//! Durable native Codex coding dispatch/checkpoint ownership, separate from UI state.
//! An enqueue receipt is irreversible: no canonical checkpoint means recovery is
//! required, even if the child may never have received turn/start.
use super::codex::SavedThread;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
const MAX_HISTORY: usize = 32 * 1024 * 1024;
const MAX_BYTES: i64 = 240 * 1024 * 1024;
const MAX_RECORDS: i64 = 10_000;
const CONTINUATION:&str="Continue the existing native conversation from its last fully recorded turn. Do not repeat the original task or repeat completed tool effects. Use the native history and recorded tool results already present; stop and report any uncertainty.";
fn failure() -> String {
    "The native coding journal could not establish its durable binding. Existing data has been preserved.".into()
}
fn recovery() -> String {
    "RecoveryRequired: a native coding dispatch lacks a canonical checkpoint. Review retained observations; the original task cannot be replayed.".into()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(failure())
    } else {
        Ok(())
    }
}
fn serialized<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| failure())
}
fn checked<T: serde::de::DeserializeOwned>(payload: String, digest: String) -> Result<T, String> {
    if hash(payload.as_bytes()) != digest {
        return Err(failure());
    }
    serde_json::from_str(&payload).map_err(|_| failure())
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => serde_json::to_value(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap(),
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        value => value.clone(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DispatchBinding {
    pub attempt_id: String,
    pub generation: u64,
    pub auth_revision: u64,
    pub profile_id: String,
    pub input_id: String,
    pub client_id: String,
}
impl DispatchBinding {
    fn validate(&self) -> Result<(), String> {
        for value in [
            &self.attempt_id,
            &self.profile_id,
            &self.input_id,
            &self.client_id,
        ] {
            id(value)?
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InputPlan {
    pub client_id: String,
    pub prompt: String,
    pub is_continuation: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    input_id: String,
    original_text: String,
    reason: String,
    plan: InputPlan,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    saved: SavedThread,
    history: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Dispatch {
    binding: DispatchBinding,
    snapshot: Snapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Checkpoint {
    binding: DispatchBinding,
    snapshot: Snapshot,
    completed: bool,
    exhausted: bool,
}
pub(crate) struct Journal {
    connection: Connection,
    _lock: File,
    root: PathBuf,
    run_id: String,
    root_identity: (u64, u64),
    db_identity: (u64, u64),
    lock_identity: (u64, u64),
}
fn private_file(path: &Path, optional: bool) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.nlink() == 1 =>
        {
            Ok(())
        }
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(failure()),
    }
}
fn owned(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| failure())?;
    let metadata = file.metadata().map_err(|_| failure())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(failure());
    }
    Ok(file)
}
fn validate_history(saved: &SavedThread, history: &Value, root: &Path) -> Result<(), String> {
    id(&saved.id)?;
    id(&saved.creator_account_id)?;
    let turns = history["turns"].as_array().ok_or_else(failure)?;
    if history["id"].as_str() != Some(saved.id.as_str()) || serialized(history)?.len() > MAX_HISTORY
    {
        return Err(failure());
    }
    // Exact pinned CodingProtocol history_digest: serde serialization of turns,
    // preserving object insertion order. Never normalize native history.
    if hash(&serde_json::to_vec(&history["turns"]).map_err(|_| failure())?) != saved.history_sha256
    {
        return Err(failure());
    }
    if turns.iter().any(|turn| {
        !matches!(
            turn["status"].as_str(),
            Some("completed" | "failed" | "interrupted")
        )
    }) {
        return Err(failure());
    }
    if !saved.rollout_path.is_absolute() || !saved.rollout_path.starts_with(root) {
        return Err(failure());
    }
    for component in saved
        .rollout_path
        .strip_prefix(root)
        .map_err(|_| failure())?
        .components()
    {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(failure());
        }
    }
    Ok(())
}
fn sync_rollout(saved: &SavedThread, root: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in saved
        .rollout_path
        .strip_prefix(root)
        .map_err(|_| failure())?
        .components()
    {
        let std::path::Component::Normal(part) = component else {
            return Err(failure());
        };
        current.push(part);
        let metadata = fs::symlink_metadata(&current).map_err(|_| failure())?;
        if metadata.file_type().is_symlink()
            || (current != saved.rollout_path && !metadata.is_dir())
        {
            return Err(failure());
        }
    }
    let before = fs::symlink_metadata(&saved.rollout_path).map_err(|_| failure())?;
    if !before.is_file()
        || before.uid() != unsafe { libc::geteuid() }
        || before.mode() & 0o022 != 0
        || before.nlink() != 1
        || before.len() > MAX_HISTORY as u64
    {
        return Err(failure());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&saved.rollout_path)
        .map_err(|_| failure())?;
    let opened = file.metadata().map_err(|_| failure())?;
    if !opened.is_file() || (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
        return Err(failure());
    }
    file.sync_all().map_err(|_| failure())?;
    for directory in saved.rollout_path.parent().ok_or_else(failure)?.ancestors() {
        if !directory.starts_with(root) {
            break;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(directory)
            .map_err(|_| failure())?;
        file.sync_all().map_err(|_| failure())?;
    }
    Ok(())
}
impl Journal {
    pub(crate) fn open(
        root: &Path,
        run_id: &str,
        project_identity: &Value,
        model: &str,
        effort: &str,
    ) -> Result<Self, String> {
        id(run_id)?;
        super::codex::model(Some(model))?;
        super::codex::effort(model, Some(effort))?;
        if fs::symlink_metadata(root)
            .map_err(|_| failure())?
            .file_type()
            .is_symlink()
        {
            return Err(failure());
        }
        let root = root.canonicalize().map_err(|_| failure())?;
        let metadata = fs::symlink_metadata(&root).map_err(|_| failure())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(failure());
        }
        if serialized(project_identity)?.len() > 8192 || !project_identity.is_object() {
            return Err(failure());
        }
        let admission = serialized(
            &json!({"runId":run_id,"project":canonical(project_identity),"model":model,"effort":effort}),
        )?;
        let lock_path = root.join("coding.owner.lock");
        private_file(&lock_path, true)?;
        let lock = owned(&lock_path)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Another worker owns this native coding journal.".into());
        }
        let path = root.join("coding.sqlite3");
        for suffix in ["", "-wal", "-shm", "-journal"] {
            private_file(&root.join(format!("coding.sqlite3{suffix}")), true)?
        }
        let exists = path.exists();
        if exists {
            if fs::metadata(&path).map_err(|_| failure())?.len() > 256 * 1024 * 1024 {
                return Err(failure());
            }
            let readonly = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| failure())?;
            let version: i64 = readonly
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(|_| failure())?;
            if version != 1 {
                return Err(failure());
            }
            let integrity: String = readonly
                .query_row("PRAGMA quick_check", [], |row| row.get(0))
                .map_err(|_| failure())?;
            if integrity != "ok" {
                return Err(failure());
            }
            let (payload, digest): (String, String) = readonly
                .query_row(
                    "SELECT payload,digest FROM admission WHERE id=1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| failure())?;
            if payload != admission || hash(payload.as_bytes()) != digest {
                return Err(failure());
            }
            for sql in [
                "SELECT seq,client_id,input_id,payload,digest FROM plans LIMIT 0",
                "SELECT client_id,payload,digest FROM dispatches LIMIT 0",
                "SELECT client_id,payload,digest FROM checkpoints LIMIT 0",
                "SELECT seq,client_id,payload,digest FROM observations LIMIT 0",
            ] {
                readonly.prepare(sql).map_err(|_| failure())?;
            }
        } else {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|_| failure())?;
            file.sync_all().map_err(|_| failure())?;
            File::open(&root)
                .and_then(|file| file.sync_all())
                .map_err(|_| failure())?;
        }
        let connection = Connection::open(&path).map_err(|_| failure())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| failure())?;
        let page: i64 = connection
            .pragma_query_value(None, "page_size", |row| row.get(0))
            .map_err(|_| failure())?;
        if page != 4096 {
            return Err(failure());
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA max_page_count=65536; PRAGMA wal_autocheckpoint=128;").map_err(|_|failure())?;
        let mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|_| failure())?;
        let sync: i64 = connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .map_err(|_| failure())?;
        if mode != "wal" || sync != 2 {
            return Err(failure());
        }
        if !exists {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE admission(id INTEGER PRIMARY KEY CHECK(id=1),payload TEXT NOT NULL,digest TEXT NOT NULL);
            CREATE TABLE plans(seq INTEGER PRIMARY KEY AUTOINCREMENT,client_id TEXT NOT NULL UNIQUE,input_id TEXT NOT NULL,payload TEXT NOT NULL,digest TEXT NOT NULL);
            CREATE TABLE dispatches(client_id TEXT PRIMARY KEY REFERENCES plans(client_id),payload TEXT NOT NULL,digest TEXT NOT NULL);
            CREATE TABLE checkpoints(client_id TEXT PRIMARY KEY REFERENCES dispatches(client_id),payload TEXT NOT NULL,digest TEXT NOT NULL);
            CREATE TABLE observations(seq INTEGER PRIMARY KEY AUTOINCREMENT,client_id TEXT NOT NULL REFERENCES dispatches(client_id),payload TEXT NOT NULL,digest TEXT NOT NULL);
            PRAGMA user_version=1; COMMIT;").map_err(|_|failure())?;
        }
        let old: Option<(String, String)> = connection
            .query_row(
                "SELECT payload,digest FROM admission WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        if let Some((payload, digest)) = old {
            if payload != admission || hash(payload.as_bytes()) != digest {
                return Err(failure());
            }
        } else {
            if exists {
                return Err(failure());
            }
            connection
                .execute(
                    "INSERT INTO admission VALUES(1,?,?)",
                    params![admission, hash(admission.as_bytes())],
                )
                .map_err(|_| failure())?;
        }
        let root_identity = (metadata.dev(), metadata.ino());
        let db = fs::symlink_metadata(&path).map_err(|_| failure())?;
        let db_identity = (db.dev(), db.ino());
        let lock_meta = lock.metadata().map_err(|_| failure())?;
        let lock_identity = (lock_meta.dev(), lock_meta.ino());
        Ok(Self {
            connection,
            _lock: lock,
            root,
            run_id: run_id.into(),
            root_identity,
            db_identity,
            lock_identity,
        })
    }
    fn fence(&self) -> Result<(), String> {
        let metadata = fs::symlink_metadata(&self.root).map_err(|_| failure())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || (metadata.dev(), metadata.ino()) != self.root_identity
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(failure());
        }
        for (name, identity) in [
            ("coding.sqlite3", self.db_identity),
            ("coding.owner.lock", self.lock_identity),
        ] {
            let path = self.root.join(name);
            private_file(&path, false)?;
            let metadata = fs::symlink_metadata(path).map_err(|_| failure())?;
            if (metadata.dev(), metadata.ino()) != identity {
                return Err(failure());
            }
        }
        Ok(())
    }
    fn ensure_idle(&self) -> Result<(), String> {
        self.fence()?;
        let pending:i64=self.connection.query_row("SELECT count(*) FROM dispatches d LEFT JOIN checkpoints c USING(client_id) WHERE c.client_id IS NULL",[],|row|row.get(0)).map_err(|_|failure())?;
        if pending != 0 {
            return Err(recovery());
        }
        Ok(())
    }
    fn latest(&self) -> Result<Option<Checkpoint>, String> {
        self.fence()?;
        let row:Option<(String,String)>=self.connection.query_row("SELECT c.payload,c.digest FROM checkpoints c JOIN plans p USING(client_id) ORDER BY p.seq DESC LIMIT 1",[],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|_|failure())?;
        row.map(|(payload, digest)| {
            let checkpoint: Checkpoint = checked(payload, digest)?;
            validate_history(
                &checkpoint.snapshot.saved,
                &checkpoint.snapshot.history,
                &self.root,
            )?;
            Ok(checkpoint)
        })
        .transpose()
    }
    pub(crate) fn latest_saved(&self) -> Result<Option<SavedThread>, String> {
        Ok(self.latest()?.map(|checkpoint| checkpoint.snapshot.saved))
    }
    /// Knowledge-only acknowledgement: never dispatches or repairs a missing checkpoint.
    pub(crate) fn completed_input(&self, input_id: &str) -> Result<bool, String> {
        id(input_id)?;
        self.ensure_idle()?;
        let Some(checkpoint) = self.latest()? else {
            return Ok(false);
        };
        if checkpoint.binding.input_id != input_id || !checkpoint.completed {
            return Ok(false);
        }
        checkpoint.binding.validate()?;
        let dispatch = self
            .dispatched(&checkpoint.binding.client_id)?
            .ok_or_else(failure)?;
        let plan = self.plan(&checkpoint.binding.client_id)?;
        let saved = &checkpoint.snapshot.saved;
        if checkpoint.exhausted
            || dispatch.binding != checkpoint.binding
            || plan.input_id != input_id
            || saved.id != dispatch.snapshot.saved.id
            || saved.creator_account_id != dispatch.snapshot.saved.creator_account_id
            || saved.rollout_path != dispatch.snapshot.saved.rollout_path
        {
            return Err(failure());
        }
        let before = dispatch.snapshot.history["turns"]
            .as_array()
            .ok_or_else(failure)?;
        let after = checkpoint.snapshot.history["turns"]
            .as_array()
            .ok_or_else(failure)?;
        if after.len() != before.len() + 1 || after[..before.len()] != before[..] {
            return Err(failure());
        }
        let terminal = after.last().ok_or_else(failure)?;
        let items = terminal["items"].as_array().ok_or_else(failure)?;
        let users = items
            .iter()
            .filter(|item| item["type"] == "userMessage")
            .collect::<Vec<_>>();
        if terminal["status"] != "completed"
            || terminal.get("error").is_some_and(|error| !error.is_null())
            || users.len() != 1
            || users[0]["clientId"] != checkpoint.binding.client_id
            || users[0]["content"]
                != json!([{"type":"text","text":plan.plan.prompt,"text_elements":[]}])
        {
            return Err(failure());
        }
        Ok(true)
    }
    pub(crate) fn next_input(
        &mut self,
        input_id: &str,
        text: &str,
        allow_interrupted_continuation: bool,
    ) -> Result<InputPlan, String> {
        self.ensure_idle()?;
        id(input_id)?;
        let pending_plan:Option<String>=self.connection.query_row("SELECT p.input_id FROM plans p LEFT JOIN dispatches d USING(client_id) WHERE d.client_id IS NULL ORDER BY p.seq DESC LIMIT 1",[],|row|row.get(0)).optional().map_err(|_|failure())?;
        if pending_plan
            .as_deref()
            .is_some_and(|pending| pending != input_id)
        {
            return Err(
                "Another prepared native input owns this run; it cannot be silently replaced."
                    .into(),
            );
        }
        if text.trim().is_empty() || text.len() > 60 * 1024 || text.contains('\0') {
            return Err(failure());
        }
        let previous:Option<(String,String,bool)>=self.connection.query_row("SELECT p.payload,p.digest,d.client_id IS NOT NULL FROM plans p LEFT JOIN dispatches d USING(client_id) WHERE p.input_id=? ORDER BY p.seq DESC LIMIT 1",[input_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|failure())?;
        let (continuation, reason) = if let Some((payload, digest, dispatched)) = previous {
            let previous: Plan = checked(payload, digest)?;
            if previous.input_id != input_id || previous.original_text != text {
                return Err(failure());
            }
            if !dispatched {
                return Ok(previous.plan);
            }
            let checkpoint = self
                .checkpoint_for(&previous.plan.client_id)?
                .ok_or_else(recovery)?;
            if checkpoint.completed {
                return Err(
                    "This native input is already complete; send a distinct new input instead."
                        .into(),
                );
            }
            if checkpoint.exhausted {
                (true, "quotaContinuation")
            } else if allow_interrupted_continuation {
                (true, "explicitContinuation")
            } else {
                return Err("Explicit Main continuation is required for this canonical interrupted/failed turn.".into());
            }
        } else {
            (false, "newInput")
        };
        let plan = Plan {
            input_id: input_id.into(),
            original_text: text.into(),
            reason: reason.into(),
            plan: InputPlan {
                client_id: super::new_id()?,
                prompt: if continuation {
                    CONTINUATION.into()
                } else {
                    text.into()
                },
                is_continuation: continuation,
            },
        };
        let payload = serialized(&plan)?;
        let digest = hash(payload.as_bytes());
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        capacity(&transaction, payload.len())?;
        transaction
            .execute(
                "INSERT INTO plans(client_id,input_id,payload,digest) VALUES(?,?,?,?)",
                params![plan.plan.client_id, input_id, payload, digest],
            )
            .map_err(|_| failure())?;
        transaction.commit().map_err(|_| failure())?;
        Ok(plan.plan)
    }
    fn plan(&self, client_id: &str) -> Result<Plan, String> {
        let (payload, digest): (String, String) = self
            .connection
            .query_row(
                "SELECT payload,digest FROM plans WHERE client_id=?",
                [client_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| failure())?;
        let plan: Plan = checked(payload, digest)?;
        if plan.plan.client_id != client_id {
            return Err(failure());
        }
        Ok(plan)
    }
    fn dispatched(&self, client_id: &str) -> Result<Option<Dispatch>, String> {
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT payload,digest FROM dispatches WHERE client_id=?",
                [client_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        row.map(|(payload, digest)| {
            let dispatch: Dispatch = checked(payload, digest)?;
            if dispatch.binding.client_id != client_id {
                return Err(failure());
            }
            validate_history(
                &dispatch.snapshot.saved,
                &dispatch.snapshot.history,
                &self.root,
            )?;
            Ok(dispatch)
        })
        .transpose()
    }
    fn checkpoint_for(&self, client_id: &str) -> Result<Option<Checkpoint>, String> {
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT payload,digest FROM checkpoints WHERE client_id=?",
                [client_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        row.map(|(payload, digest)| {
            let checkpoint: Checkpoint = checked(payload, digest)?;
            if checkpoint.binding.client_id != client_id {
                return Err(failure());
            }
            validate_history(
                &checkpoint.snapshot.saved,
                &checkpoint.snapshot.history,
                &self.root,
            )?;
            Ok(checkpoint)
        })
        .transpose()
    }
    pub(crate) fn dispatch(
        &mut self,
        binding: &DispatchBinding,
        saved: &SavedThread,
        history: &Value,
    ) -> Result<String, String> {
        self.ensure_idle()?;
        self.fence()?;
        binding.validate()?;
        validate_history(saved, history, &self.root)?;
        let plan = self.plan(&binding.client_id)?;
        if plan.input_id != binding.input_id {
            return Err(failure());
        }
        if self.dispatched(&binding.client_id)?.is_some() {
            return Err(recovery());
        }
        if let Some(latest) = self.latest()? {
            if latest.snapshot.saved != *saved
                || latest.snapshot.history["turns"] != history["turns"]
            {
                return Err(
                    "Native coding history differs from its last durable checkpoint.".into(),
                );
            }
        } else if !history["turns"].as_array().ok_or_else(failure)?.is_empty() {
            return Err("A fresh coding run cannot adopt unseen native history.".into());
        }
        sync_rollout(saved, &self.root)?;
        self.fence()?;
        let dispatch = Dispatch {
            binding: binding.clone(),
            snapshot: Snapshot {
                saved: saved.clone(),
                history: history.clone(),
            },
        };
        let payload = serialized(&dispatch)?;
        let receipt = hash(payload.as_bytes());
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        capacity(&transaction, payload.len())?;
        transaction
            .execute(
                "INSERT INTO dispatches(client_id,payload,digest) VALUES(?,?,?)",
                params![binding.client_id, payload, receipt],
            )
            .map_err(|_| failure())?;
        transaction.commit().map_err(|_| failure())?;
        Ok(receipt)
    }
    pub(crate) fn checkpoint(
        &mut self,
        binding: &DispatchBinding,
        saved: &SavedThread,
        history: &Value,
        completed: bool,
        exhausted: bool,
    ) -> Result<String, String> {
        self.fence()?;
        binding.validate()?;
        validate_history(saved, history, &self.root)?;
        let dispatch = self.dispatched(&binding.client_id)?.ok_or_else(failure)?;
        if dispatch.binding != *binding
            || saved.id != dispatch.snapshot.saved.id
            || saved.creator_account_id != dispatch.snapshot.saved.creator_account_id
            || saved.rollout_path != dispatch.snapshot.saved.rollout_path
            || completed && exhausted
        {
            return Err(failure());
        }
        let before = dispatch.snapshot.history["turns"]
            .as_array()
            .ok_or_else(failure)?;
        let after = history["turns"].as_array().ok_or_else(failure)?;
        if after.len() != before.len() + 1 || after[..before.len()] != before[..] {
            return Err(failure());
        }
        let terminal = after.last().ok_or_else(failure)?;
        let plan = self.plan(&binding.client_id)?;
        let items = terminal["items"].as_array().ok_or_else(failure)?;
        let users = items
            .iter()
            .filter(|item| item["type"] == "userMessage")
            .collect::<Vec<_>>();
        if users.len() != 1
            || users[0]["clientId"] != binding.client_id
            || users[0]["content"]
                != json!([{"type":"text","text":plan.plan.prompt,"text_elements":[]}])
        {
            return Err(failure());
        }
        if completed
            && (terminal["status"] != "completed"
                || terminal.get("error").is_some_and(|error| !error.is_null()))
        {
            return Err(failure());
        }
        if exhausted
            && !(terminal["status"] == "failed"
                && terminal["error"]["codexErrorInfo"] == "usageLimitExceeded")
        {
            return Err(failure());
        }
        if !completed
            && !exhausted
            && !matches!(terminal["status"].as_str(), Some("failed" | "interrupted"))
        {
            return Err(failure());
        }
        sync_rollout(saved, &self.root)?;
        self.fence()?;
        let checkpoint = Checkpoint {
            binding: binding.clone(),
            snapshot: Snapshot {
                saved: saved.clone(),
                history: history.clone(),
            },
            completed,
            exhausted,
        };
        let payload = serialized(&checkpoint)?;
        let receipt = hash(payload.as_bytes());
        if let Some(existing) = self.checkpoint_for(&binding.client_id)? {
            if serialized(&existing)? == payload {
                return Ok(receipt);
            }
            return Err(failure());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        capacity(&transaction, payload.len())?;
        transaction
            .execute(
                "INSERT INTO checkpoints(client_id,payload,digest) VALUES(?,?,?)",
                params![binding.client_id, payload, receipt],
            )
            .map_err(|_| failure())?;
        transaction.commit().map_err(|_| failure())?;
        Ok(receipt)
    }
    pub(crate) fn observe(
        &mut self,
        binding: &DispatchBinding,
        observations: &Value,
    ) -> Result<String, String> {
        self.fence()?;
        binding.validate()?;
        let dispatch = self.dispatched(&binding.client_id)?.ok_or_else(failure)?;
        if dispatch.binding != *binding {
            return Err(failure());
        }
        let payload = serialized(
            &json!({"runId":self.run_id,"binding":binding,"observations":observations}),
        )?;
        if payload.len() > MAX_HISTORY {
            return Err(failure());
        }
        let receipt = hash(payload.as_bytes());
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        capacity(&transaction, payload.len())?;
        transaction
            .execute(
                "INSERT INTO observations(client_id,payload,digest) VALUES(?,?,?)",
                params![binding.client_id, payload, receipt],
            )
            .map_err(|_| failure())?;
        transaction.commit().map_err(|_| failure())?;
        Ok(receipt)
    }
}
fn capacity(connection: &Connection, additional: usize) -> Result<(), String> {
    let(count,size):(i64,i64)=connection.query_row("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM (SELECT payload FROM plans UNION ALL SELECT payload FROM dispatches UNION ALL SELECT payload FROM checkpoints UNION ALL SELECT payload FROM observations)",[],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|_|failure())?;
    if count >= MAX_RECORDS || size.saturating_add(additional as i64) > MAX_BYTES {
        return Err("The durable native coding history capacity is exhausted.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Value {
        json!({"path":"/project","dev":1,"ino":2})
    }
    fn open(root: &Path) -> Journal {
        Journal::open(root, "run", &project(), "gpt-6.1-sol", "high").unwrap()
    }
    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        crate::chat::storage::private(root.path(), true).unwrap();
        root
    }
    fn snapshot(root: &Path, turns: Value) -> (SavedThread, Value) {
        let path = root.canonicalize().unwrap().join("rollout.jsonl");
        if !path.exists() {
            fs::write(&path, b"owned native fixture\n").unwrap();
            crate::chat::storage::private(&path, false).unwrap();
        }
        let saved = SavedThread {
            id: "thread".into(),
            rollout_path: path,
            creator_account_id: "account".into(),
            history_sha256: hash(&serde_json::to_vec(&turns).unwrap()),
        };
        (
            saved,
            json!({"id":"thread","turns":turns,"historyMode":"paginated"}),
        )
    }
    fn binding(plan: &InputPlan, input: &str) -> DispatchBinding {
        DispatchBinding {
            attempt_id: "attempt".into(),
            generation: 1,
            auth_revision: 2,
            profile_id: "profile".into(),
            input_id: input.into(),
            client_id: plan.client_id.clone(),
        }
    }
    fn terminal(plan: &InputPlan, status: &str, error: Value) -> Value {
        json!({"id":"turn","status":status,"error":error,"items":[{"id":"user","type":"userMessage","clientId":plan.client_id,"content":[{"type":"text","text":plan.prompt,"text_elements":[]}]},{"id":"reason","type":"reasoning","summary":["native reasoning"],"content":["full native reasoning"]},{"id":"assistant","type":"agentMessage","text":"answer","phase":"final_answer"}]})
    }
    #[test]
    fn pending_dispatch_survives_reopen_and_blocks_even_explicit_replay() {
        let root = fixture();
        let mut journal = open(root.path());
        let plan = journal.next_input("input", "original task", false).unwrap();
        let (saved, history) = snapshot(root.path(), json!([]));
        journal
            .dispatch(&binding(&plan, "input"), &saved, &history)
            .unwrap();
        journal
            .observe(&binding(&plan, "input"), &json!({"unfinished":"partial"}))
            .unwrap();
        drop(journal);
        let mut reopened = open(root.path());
        assert!(reopened
            .next_input("input", "original task", true)
            .unwrap_err()
            .contains("RecoveryRequired"));
        assert!(reopened
            .next_input("different", "new task", false)
            .unwrap_err()
            .contains("RecoveryRequired"));
        assert!(reopened.latest_saved().unwrap().is_none());
    }
    #[test]
    fn quota_checkpoint_retains_full_history_and_continues_without_original_task() {
        let root = fixture();
        let mut journal = open(root.path());
        let first = journal
            .next_input("input", "unique original assignment 771", false)
            .unwrap();
        let (saved, before) = snapshot(root.path(), json!([]));
        let binding = binding(&first, "input");
        journal.dispatch(&binding, &saved, &before).unwrap();
        let (saved, after) = snapshot(
            root.path(),
            json!([terminal(
                &first,
                "failed",
                json!({"codexErrorInfo":"usageLimitExceeded"})
            )]),
        );
        let receipt = journal
            .checkpoint(&binding, &saved, &after, false, true)
            .unwrap();
        assert_eq!(
            journal
                .checkpoint(&binding, &saved, &after, false, true)
                .unwrap(),
            receipt
        );
        drop(journal);
        let mut reopened = open(root.path());
        assert_eq!(reopened.latest_saved().unwrap(), Some(saved));
        assert_eq!(reopened.latest().unwrap().unwrap().snapshot.history, after);
        let next = reopened
            .next_input("input", "unique original assignment 771", false)
            .unwrap();
        assert!(next.is_continuation);
        assert_ne!(next.client_id, first.client_id);
        assert_eq!(next.prompt, CONTINUATION);
        assert!(!next.prompt.contains("unique original assignment 771"));
        assert_eq!(
            reopened
                .next_input("input", "unique original assignment 771", false)
                .unwrap(),
            next
        );
    }
    #[test]
    fn canonical_interrupt_needs_explicit_main_and_completed_input_cannot_repeat() {
        for status in ["interrupted", "completed"] {
            let root = fixture();
            let mut journal = open(root.path());
            let plan = journal.next_input("input", "task", false).unwrap();
            let (saved, before) = snapshot(root.path(), json!([]));
            let binding = binding(&plan, "input");
            journal.dispatch(&binding, &saved, &before).unwrap();
            let (saved, after) =
                snapshot(root.path(), json!([terminal(&plan, status, Value::Null)]));
            journal
                .checkpoint(&binding, &saved, &after, status == "completed", false)
                .unwrap();
            assert!(journal.next_input("input", "task", false).is_err());
            if status == "interrupted" {
                assert!(
                    journal
                        .next_input("input", "task", true)
                        .unwrap()
                        .is_continuation
                )
            } else {
                assert!(journal.next_input("input", "task", true).is_err());
                let next = journal
                    .next_input("second", "exact new text", false)
                    .unwrap();
                assert_eq!(next.prompt, "exact new text");
                assert!(!next.is_continuation);
            }
        }
    }
    #[test]
    fn completed_acknowledgement_is_readonly_and_unknown_later_dispatch_blocks_it() {
        let root = fixture();
        let mut journal = open(root.path());
        assert!(!journal.completed_input("input").unwrap());
        let plan = journal.next_input("input", "task", false).unwrap();
        let (saved, before) = snapshot(root.path(), json!([]));
        let first = binding(&plan, "input");
        journal.dispatch(&first, &saved, &before).unwrap();
        assert!(journal
            .completed_input("input")
            .unwrap_err()
            .contains("RecoveryRequired"));
        let (saved, after) = snapshot(
            root.path(),
            json!([terminal(&plan, "completed", Value::Null)]),
        );
        journal
            .checkpoint(&first, &saved, &after, true, false)
            .unwrap();
        // Transport observations cannot fabricate or erase canonical completion.
        journal
            .observe(&first, &json!({"invalidTail":true}))
            .unwrap();
        drop(journal);
        let mut journal = open(root.path());
        assert!(journal.completed_input("input").unwrap());
        assert!(!journal.completed_input("other").unwrap());
        assert!(journal.next_input("input", "task", true).is_err());
        let next = journal.next_input("other", "new task", false).unwrap();
        journal
            .dispatch(&binding(&next, "other"), &saved, &after)
            .unwrap();
        assert!(journal
            .completed_input("input")
            .unwrap_err()
            .contains("RecoveryRequired"));
    }
    #[test]
    fn wrong_history_client_and_nonquota_rate_limit_never_checkpoint() {
        let root = fixture();
        let mut journal = open(root.path());
        let plan = journal.next_input("input", "task", false).unwrap();
        let binding = binding(&plan, "input");
        let (mut saved, before) = snapshot(root.path(), json!([]));
        saved.history_sha256 = hash(b"different");
        assert!(journal.dispatch(&binding, &saved, &before).is_err());
        let (saved, before) = snapshot(root.path(), json!([]));
        journal.dispatch(&binding, &saved, &before).unwrap();
        let mut wrong = terminal(&plan, "completed", Value::Null);
        wrong["items"][0]["clientId"] = json!("another-client");
        let (saved, after) = snapshot(root.path(), json!([wrong]));
        assert!(journal
            .checkpoint(&binding, &saved, &after, true, false)
            .is_err());
        let (saved, after) = snapshot(
            root.path(),
            json!([terminal(
                &plan,
                "failed",
                json!({"codexErrorInfo":"rateLimitExceeded"})
            )]),
        );
        assert!(journal
            .checkpoint(&binding, &saved, &after, false, true)
            .is_err());
        assert!(journal.next_input("input", "task", true).is_err());
    }
    #[test]
    fn owner_lock_project_and_model_binding_are_not_transferable() {
        let root = fixture();
        let journal = open(root.path());
        assert!(Journal::open(root.path(), "run", &project(), "gpt-6.1-sol", "high").is_err());
        drop(journal);
        assert!(Journal::open(
            root.path(),
            "run",
            &json!({"path":"/changed","dev":1,"ino":3}),
            "gpt-6.1-sol",
            "high"
        )
        .is_err());
        assert!(Journal::open(
            root.path(),
            "another-run",
            &project(),
            "gpt-6.1-sol",
            "high"
        )
        .is_err());
    }
    #[test]
    fn future_and_corrupt_owner_data_are_preserved_before_writable_open() {
        for future in [true, false] {
            let root = fixture();
            let path = root.path().join("coding.sqlite3");
            if future {
                let connection = Connection::open(&path).unwrap();
                connection.execute_batch("CREATE TABLE retain(value TEXT);INSERT INTO retain VALUES('original');PRAGMA user_version=999;").unwrap();
                drop(connection);
            } else {
                fs::write(&path, b"retain damaged coding history").unwrap();
            }
            crate::chat::storage::private(&path, false).unwrap();
            let original = fs::read(&path).unwrap();
            assert!(Journal::open(root.path(), "run", &project(), "gpt-6.1-sol", "high").is_err());
            assert_eq!(fs::read(path).unwrap(), original);
        }
    }
}
