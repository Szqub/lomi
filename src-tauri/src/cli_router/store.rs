use super::types::*;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    time::Duration,
};

const SCHEMA_VERSION: i64 = 5;
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_REQUESTS: i64 = 100_000;

pub(crate) struct Store {
    connection: Connection,
    _owner: fs::File,
}

pub(crate) struct MutationResult {
    pub(crate) snapshot: Snapshot,
    pub(crate) replayed: bool,
}

struct RequestRecord {
    id: String,
    digest: String,
    revision: u64,
}

fn request_records(
    connection: &Connection,
    version: i64,
    latest_revision: u64,
) -> Result<(Vec<RequestRecord>, bool), String> {
    let count: i64 = connection
        .query_row("SELECT count(*) FROM requests", [], |row| row.get(0))
        .map_err(error)?;
    if count > MAX_REQUESTS {
        return Err("Router request journal exceeds its qualified capacity; existing data has been preserved".into());
    }
    let invalid_metadata: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM requests WHERE id IS NULL OR digest IS NULL OR length(CAST(id AS BLOB)) NOT BETWEEN 1 AND 512 OR length(CAST(digest AS BLOB)) != 64)", [], |row| row.get(0)).map_err(error)?;
    if invalid_metadata {
        return Err(error("invalid request metadata"));
    }
    if version == 1 {
        let oversized: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM requests WHERE length(CAST(result AS BLOB)) > ?1)",
                [MAX_SNAPSHOT_BYTES as i64],
                |row| row.get(0),
            )
            .map_err(error)?;
        if oversized {
            return Err("Router request journal exceeds its qualified record size; existing data has been preserved".into());
        }
    } else {
        let invalid_revision: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM requests WHERE committed_revision IS NULL OR length(CAST(committed_revision AS BLOB)) NOT BETWEEN 1 AND 20)", [], |row| row.get(0)).map_err(error)?;
        if invalid_revision {
            return Err(error("invalid request revision"));
        }
    }
    let value_column = if version == 1 {
        "result"
    } else {
        "committed_revision"
    };
    let mut statement = connection
        .prepare(&format!("SELECT id, digest, {value_column} FROM requests"))
        .map_err(error)?;
    let mut rows = statement.query([]).map_err(error)?;
    let mut records = Vec::new();
    let mut retired_history = false;
    let mut ids = HashSet::new();
    while let Some(row) = rows.next().map_err(error)? {
        let id: String = row.get(0).map_err(error)?;
        let digest: String = row.get(1).map_err(error)?;
        let value: String = row.get(2).map_err(error)?;
        if id.is_empty()
            || id.len() > 512
            || !ids.insert(id.clone())
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(error("invalid request record"));
        }
        let revision = if version == 1 {
            let migration = decode_migrating(&value)?;
            retired_history |= migration.changed();
            migration.snapshot.revision
        } else {
            value.parse::<u64>().map_err(error)?
        };
        if revision > latest_revision || (version >= 2 && revision.to_string() != value) {
            return Err(error("invalid request revision"));
        }
        records.push(RequestRecord {
            id,
            digest,
            revision,
        });
    }
    Ok((records, retired_history))
}

fn recover_interrupted(snapshot: &mut Snapshot) -> Result<(), String> {
    for quota in &mut snapshot.quota {
        if quota.status == QuotaStatus::Fresh {
            quota.status = QuotaStatus::Stale;
        }
    }
    for run in &mut snapshot.runs {
        if matches!(
            run.state,
            RunState::Starting | RunState::Running | RunState::Switching
        ) || run.attempts.iter().any(|attempt| attempt.state.is_active())
        {
            run.state = RunState::RecoveryRequired;
            run.generation = next(run.generation)?;
            run.revision = next(run.revision)?;
            run.status_message =
                "The previous attempt needs review before resuming; no input was replayed".into();
            for attempt in &mut run.attempts {
                if attempt.state.is_active() {
                    attempt.state = AttemptState::RecoveryRequired;
                }
            }
            for turn in &mut run.turns {
                if turn.state.is_active() {
                    turn.state = AttemptState::RecoveryRequired;
                }
            }
        }
    }
    Ok(())
}

fn error(_: impl std::fmt::Display) -> String {
    "Router storage failed; existing data has been preserved".into()
}

fn decode(source: &str) -> Result<Snapshot, String> {
    if source.len() > MAX_SNAPSHOT_BYTES {
        return Err("Router history exceeds the qualified storage size".into());
    }
    let snapshot: Snapshot = serde_json::from_str(source).map_err(error)?;
    validate(&snapshot)?;
    Ok(snapshot)
}

struct CatalogMigration {
    snapshot: Snapshot,
    profiles: HashSet<String>,
    routers: HashSet<String>,
    removal_intents: Vec<(String, String)>,
}

impl CatalogMigration {
    fn changed(&self) -> bool {
        !self.profiles.is_empty() || !self.routers.is_empty()
    }
}

fn decode_migrating(source: &str) -> Result<CatalogMigration, String> {
    if source.len() > MAX_SNAPSHOT_BYTES {
        return Err("Router history exceeds the qualified storage size".into());
    }
    let mut value: serde_json::Value = serde_json::from_str(source).map_err(error)?;
    let mut profiles = HashSet::new();
    let mut routers = HashSet::new();
    let mut original_clis = HashMap::new();
    let mut original_router_clis = HashMap::new();
    for collection in ["profiles", "routers"] {
        let records = value
            .get_mut(collection)
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| error("invalid catalog"))?;
        for record in records {
            let id = record["id"]
                .as_str()
                .ok_or_else(|| error("invalid identifier"))?
                .to_string();
            let cli = record["cli"]
                .as_str()
                .ok_or_else(|| error("invalid CLI"))?
                .to_string();
            if collection == "profiles" {
                original_clis.insert(id.clone(), cli.clone());
            } else {
                original_router_clis.insert(id.clone(), cli.clone());
            }
            if crate::cli_catalog::is_retired_id(&cli) {
                if collection == "profiles" {
                    profiles.insert(id);
                } else {
                    routers.insert(id);
                }
                // Validate the complete old schema and ledger before discarding
                // records. This temporary value never enters native dispatch.
                record["cli"] = serde_json::json!("codex");
            }
        }
    }
    // Surrogate enum values must not mask originally mismatched pool members.
    for router in value["routers"]
        .as_array()
        .ok_or_else(|| error("invalid catalog"))?
    {
        let id = router["id"]
            .as_str()
            .ok_or_else(|| error("invalid identifier"))?;
        let cli = original_router_clis
            .get(id)
            .ok_or_else(|| error("invalid CLI"))?;
        for member in router["orderedProfileIds"]
            .as_array()
            .ok_or_else(|| error("invalid pool"))?
        {
            let member = member.as_str().ok_or_else(|| error("invalid pool"))?;
            if original_clis.get(member) != Some(cli) {
                return Err(error("mismatched pool"));
            }
        }
    }
    let mut snapshot: Snapshot = serde_json::from_value(value).map_err(error)?;
    validate(&snapshot)?;
    let mut removal_intents = Vec::new();
    for profile in snapshot.profiles.iter().filter(|profile| {
        profiles.contains(&profile.id) && profile.auth_state == AuthState::PendingRemove
    }) {
        if let Some(id) = profile.credential_ref.as_ref() {
            super::credentials::valid(id)?;
            removal_intents.push((id.clone(), profile.id.clone()));
        }
    }
    snapshot
        .profiles
        .retain(|profile| !profiles.contains(&profile.id));
    snapshot
        .routers
        .retain(|router| !routers.contains(&router.id));
    snapshot
        .quota
        .retain(|quota| !profiles.contains(&quota.profile_id));
    snapshot.runs.retain(|run| {
        !routers.contains(&run.router_id)
            && !run
                .pinned_profile_id
                .as_ref()
                .is_some_and(|id| profiles.contains(id))
            && !run
                .active_profile_id
                .as_ref()
                .is_some_and(|id| profiles.contains(id))
            && !run
                .allowed_profile_ids
                .iter()
                .any(|id| profiles.contains(id))
            && !run
                .attempted_profile_ids
                .iter()
                .any(|id| profiles.contains(id))
            && !run
                .attempts
                .iter()
                .any(|attempt| profiles.contains(&attempt.profile_id))
            && !run
                .turns
                .iter()
                .any(|turn| profiles.contains(&turn.profile_id))
    });
    validate(&snapshot)?;
    Ok(CatalogMigration {
        snapshot,
        profiles,
        routers,
        removal_intents,
    })
}

fn read_source(connection: &Connection) -> Result<String, String> {
    let bytes: i64 = connection
        .query_row(
            "SELECT length(CAST(data AS BLOB)) FROM snapshot WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(error)?;
    if bytes < 0 || bytes as u64 > MAX_SNAPSHOT_BYTES as u64 {
        return Err("Router history exceeds the qualified storage size".into());
    }
    let source: String = connection
        .query_row("SELECT data FROM snapshot WHERE id = 1", [], |row| {
            row.get(0)
        })
        .map_err(error)?;
    Ok(source)
}

fn read(connection: &Connection) -> Result<Snapshot, String> {
    decode(&read_source(connection)?)
}

fn backup_before_catalog_removal(connection: &Connection, directory: &Path) -> Result<(), String> {
    let path = directory.join(format!(
        "router-before-cli-removal-{}.sqlite",
        super::new_id()?
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path).map_err(error)?;
    // SQLite backup includes committed WAL pages, unlike copying the main file.
    connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .map_err(error)?;
    file.sync_all().map_err(error)?;
    fs::File::open(directory)
        .and_then(|parent| parent.sync_all())
        .map_err(error)
}

fn preserve_removal_intents(
    connection: &Connection,
    intents: &[(String, String)],
) -> Result<(), String> {
    for (credential, profile) in intents {
        connection.execute("INSERT OR IGNORE INTO credential_journal (id, profile_id, status) VALUES (?1, ?2, 'remove')", params![credential, profile]).map_err(error)?;
    }
    Ok(())
}

fn write(connection: &Connection, snapshot: &Snapshot) -> Result<(), String> {
    validate(snapshot)?;
    let source = serde_json::to_string(snapshot).map_err(error)?;
    if source.len() > MAX_SNAPSHOT_BYTES {
        return Err("Router history exceeds the qualified storage size".into());
    }
    connection
        .execute("UPDATE snapshot SET data = ?1 WHERE id = 1", [source])
        .map_err(error)?;
    Ok(())
}

fn unique<'a>(ids: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = HashSet::new();
    ids.into_iter()
        .all(|id| !id.is_empty() && id.len() <= 256 && seen.insert(id))
}

fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if !unique(snapshot.profiles.iter().map(|p| p.id.as_str()))
        || !unique(snapshot.routers.iter().map(|r| r.id.as_str()))
        || !unique(snapshot.runs.iter().map(|r| r.id.as_str()))
        || !unique(snapshot.quota.iter().map(|q| q.profile_id.as_str()))
    {
        return Err("Invalid or duplicate router record identifier".into());
    }
    for router in &snapshot.routers {
        if !unique(router.ordered_profile_ids.iter().map(String::as_str))
            || router.ordered_profile_ids.iter().any(|id| {
                !snapshot
                    .profiles
                    .iter()
                    .any(|p| p.id == *id && p.cli == router.cli)
            })
        {
            return Err("Router profiles must exist and use the same CLI".into());
        }
    }
    for quota in &snapshot.quota {
        if !snapshot.profiles.iter().any(|p| p.id == quota.profile_id)
            || !unique(quota.windows.iter().map(|w| w.id.as_str()))
            || quota.windows.iter().any(|w| {
                w.remaining_percent
                    .is_some_and(|v| !v.is_finite() || !(0.0..=100.0).contains(&v))
            })
        {
            return Err("Invalid sanitized quota observation".into());
        }
    }
    for run in &snapshot.runs {
        // History survives removal of its router/profile records.
        if !unique(run.inputs.iter().map(|input| input.id.as_str()))
            || !unique(run.attempts.iter().map(|attempt| attempt.id.as_str()))
            || !unique(run.allowed_profile_ids.iter().map(String::as_str))
            || !unique(run.attempted_profile_ids.iter().map(String::as_str))
            || run.attempts.iter().filter(|a| a.state.is_active()).count() > 1
            || run.attempts.iter().any(|attempt| {
                !run.inputs.iter().any(|input| input.id == attempt.input_id)
                    || attempt.generation > run.generation
                    || (attempt.state.is_active()
                        && !run.allowed_profile_ids.contains(&attempt.profile_id))
            })
            || !unique(run.turns.iter().map(|turn| turn.attempt_id.as_str()))
            || run.turns.iter().any(|turn| {
                !run.attempts.iter().any(|attempt| {
                    attempt.id == turn.attempt_id
                        && attempt.input_id == turn.input_id
                        && attempt.profile_id == turn.profile_id
                        && attempt.generation == turn.generation
                        && attempt.state == turn.state
                }) || turn.text.len() > super::runtime::MAX_OUTPUT
            })
            || run
                .legacy_output
                .as_ref()
                .is_some_and(|text| text.len() > super::runtime::MAX_OUTPUT)
        {
            return Err("Invalid logical input or run attempt ledger".into());
        }
        if run.output.len() > super::runtime::MAX_OUTPUT
            || run
                .turns
                .windows(2)
                .any(|turns| turns[0].generation >= turns[1].generation)
        {
            return Err("Invalid managed-turn history size or order".into());
        }
        if !run.turns.is_empty() || run.legacy_output.is_some() {
            let mut output = run.legacy_output.clone().unwrap_or_default();
            for turn in &run.turns {
                if output.len().saturating_add(turn.text.len()) > super::runtime::MAX_OUTPUT {
                    return Err("Managed-turn history exceeds its size limit".into());
                }
                output.push_str(&turn.text);
            }
            if output != run.output {
                return Err("Managed-turn records do not match retained output".into());
            }
        }
    }
    Ok(())
}

fn next(value: u64) -> Result<u64, String> {
    value
        .checked_add(1)
        .ok_or_else(|| "Router revision exhausted".into())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    // The caller supplies a dedicated app-owned router directory, not a project.
    if !path.exists() {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(error)?;
    }
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err("Router directory must be private and owned by this user".into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(error)
}

#[cfg(not(unix))]
fn private_directory(_: &Path) -> Result<(), String> {
    Err("Private router storage has not been qualified on this platform".into())
}

fn checked_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err("Router storage cannot follow links or special files".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
                    return Err("Router storage file ownership is invalid".into());
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(error(e)),
    }
}

fn owner_lock(path: &Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path).map_err(error)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Router storage already has a native owner".into());
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        options.open(path).map_err(error)
    }
}

impl Store {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let directory = path
            .parent()
            .ok_or_else(|| "Router storage path is invalid".to_string())?;
        let filename = path
            .file_name()
            .ok_or_else(|| "Router storage filename is invalid".to_string())?
            .to_string_lossy();
        private_directory(directory)?;
        let lock_path = directory.join("router.owner.lock");
        for file in [
            path.to_path_buf(),
            path.with_file_name(format!("{filename}-wal")),
            path.with_file_name(format!("{filename}-shm")),
            lock_path.clone(),
        ] {
            checked_file(&file)?;
        }
        let owner = owner_lock(&lock_path)?;
        let existing = path.exists();
        let mut legacy_requests = None;
        let mut legacy_snapshot = None;
        let mut removal_intents = Vec::new();
        if existing {
            // Validate before changing journal mode or performing recovery writes.
            let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(error)?;
            let version: i64 = connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(error)?;
            if !matches!(version, 1 | 2 | 3 | 4 | SCHEMA_VERSION) {
                return Err(
                    "Router database schema is unsupported; existing data has been preserved"
                        .into(),
                );
            }
            let integrity: String = connection
                .query_row("PRAGMA quick_check", [], |row| row.get(0))
                .map_err(error)?;
            if integrity != "ok" {
                return Err(error(integrity));
            }
            let migration = decode_migrating(&read_source(&connection)?)?;
            let (requests, retired_history) =
                request_records(&connection, version, migration.snapshot.revision)?;
            let removed = migration.changed();
            if removed || retired_history {
                backup_before_catalog_removal(&connection, directory)?;
            }
            let mut snapshot = migration.snapshot;
            removal_intents = migration.removal_intents;
            if removed {
                snapshot.revision = next(snapshot.revision)?;
            }
            if version == 1 {
                legacy_requests = Some(requests);
            }
            if version < SCHEMA_VERSION || removed {
                legacy_snapshot = Some(snapshot);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(error)?;
            }
        } else {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(path).map_err(error)?;
        }
        let mut connection = Connection::open(path).map_err(error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(error)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(error)?;
        if !existing {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(error)?;
            transaction.execute_batch("CREATE TABLE snapshot (id INTEGER PRIMARY KEY CHECK(id = 1), data TEXT NOT NULL); CREATE TABLE requests (id TEXT PRIMARY KEY, digest TEXT NOT NULL, committed_revision TEXT NOT NULL); CREATE TABLE credential_journal (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('staged', 'cleanup', 'remove')));").map_err(error)?;
            transaction
                .execute(
                    "INSERT INTO snapshot VALUES (1, ?1)",
                    [serde_json::to_string(&Snapshot::default()).map_err(error)?],
                )
                .map_err(error)?;
            transaction
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(error)?;
            transaction.commit().map_err(error)?;
        } else if let Some(requests) = legacy_requests {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(error)?;
            transaction.execute_batch("CREATE TABLE requests_v2 (id TEXT PRIMARY KEY, digest TEXT NOT NULL, committed_revision TEXT NOT NULL); CREATE TABLE IF NOT EXISTS credential_journal (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('staged', 'cleanup', 'remove')));").map_err(error)?;
            for request in requests {
                transaction.execute("INSERT INTO requests_v2 (id, digest, committed_revision) VALUES (?1, ?2, ?3)", params![request.id, request.digest, request.revision.to_string()]).map_err(error)?;
            }
            transaction
                .execute_batch("DROP TABLE requests; ALTER TABLE requests_v2 RENAME TO requests;")
                .map_err(error)?;
            if let Some(snapshot) = legacy_snapshot.as_ref() {
                write(&transaction, snapshot)?;
            }
            // A removal can be committed before its key-store journal exists.
            // Preserve that obligation atomically with retirement of its profile.
            preserve_removal_intents(&transaction, &removal_intents)?;
            transaction
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(error)?;
            transaction.commit().map_err(error)?;
        } else if let Some(snapshot) = legacy_snapshot {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(error)?;
            write(&transaction, &snapshot)?;
            preserve_removal_intents(&transaction, &removal_intents)?;
            transaction
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(error)?;
            transaction.commit().map_err(error)?;
        }
        let mut store = Self {
            connection,
            _owner: owner,
        };
        let snapshot = store.snapshot()?;
        let needs_recovery = snapshot.runs.iter().any(|r| {
            matches!(
                r.state,
                RunState::Starting | RunState::Running | RunState::Switching
            ) || r.attempts.iter().any(|a| a.state.is_active())
        });
        let needs_stale = snapshot
            .quota
            .iter()
            .any(|q| q.status == QuotaStatus::Fresh);
        if needs_recovery || needs_stale {
            store.reconcile_interrupted()?;
        }
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn reject_credential_test_commit(&self) {
        self.connection.execute_batch("CREATE TEMP TRIGGER reject_credential_commit BEFORE UPDATE ON snapshot BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    }

    #[cfg(test)]
    pub(crate) fn restore_test_writes(&self) {
        self.connection
            .execute_batch("DROP TRIGGER IF EXISTS temp.reject_credential_commit;")
            .unwrap();
    }

    /// Call only after native workers have exited. A successful write fences all
    /// unresolved intents before the owner clears its storage-failure latch.
    pub(crate) fn reconcile_interrupted(&mut self) -> Result<Snapshot, String> {
        self.update(recover_interrupted)
    }

    /// Only opaque credential identifiers belong here; never credential payloads.
    pub(crate) fn credential_journal(&self) -> Result<Vec<(String, String, String)>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT id, profile_id, status FROM credential_journal ORDER BY id LIMIT 257")
            .map_err(error)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(error)?;
        let rows = rows.collect::<Result<Vec<_>, _>>().map_err(error)?;
        if rows.len() > 256 {
            return Err("Router credential cleanup exceeds its qualified limit".into());
        }
        Ok(rows)
    }

    pub(crate) fn credential_stage(&mut self, id: &str, profile_id: &str) -> Result<(), String> {
        if self.credential_journal()?.len() >= 256 {
            return Err("Finish pending credential cleanup before saving another key".into());
        }
        self.connection
            .execute(
                "INSERT INTO credential_journal (id, profile_id, status) VALUES (?1, ?2, 'staged')",
                params![id, profile_id],
            )
            .map_err(error)?;
        Ok(())
    }

    pub(crate) fn credential_finish(&mut self, id: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM credential_journal WHERE id = ?1", [id])
            .map_err(error)?;
        Ok(())
    }

    /// Atomically commit profile changes and the exact cleanup obligations.
    pub(crate) fn credential_update<F>(
        &mut self,
        additions: &[(&str, &str, &str)],
        completed: &[&str],
        mutation: F,
    ) -> Result<Snapshot, String>
    where
        F: FnOnce(&mut Snapshot) -> Result<(), String>,
    {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let mut snapshot = read(&transaction)?;
        let revision = snapshot.revision;
        mutation(&mut snapshot)?;
        snapshot.revision = next(revision)?;
        write(&transaction, &snapshot)?;
        for (id, profile_id, status) in additions {
            transaction.execute("INSERT INTO credential_journal (id, profile_id, status) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET status = excluded.status", params![id, profile_id, status]).map_err(error)?;
        }
        for id in completed {
            transaction
                .execute("DELETE FROM credential_journal WHERE id = ?1", [id])
                .map_err(error)?;
        }
        let pending: i64 = transaction
            .query_row("SELECT count(*) FROM credential_journal", [], |row| {
                row.get(0)
            })
            .map_err(error)?;
        if pending > 256 {
            return Err("Finish pending credential cleanup before changing another account".into());
        }
        transaction.commit().map_err(error)?;
        Ok(snapshot)
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        read(&self.connection)
    }

    pub(crate) fn update<F>(&mut self, mutation: F) -> Result<Snapshot, String>
    where
        F: FnOnce(&mut Snapshot) -> Result<(), String>,
    {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let mut snapshot = read(&transaction)?;
        let revision = snapshot.revision;
        mutation(&mut snapshot)?;
        snapshot.revision = next(revision)?;
        write(&transaction, &snapshot)?;
        transaction.commit().map_err(error)?;
        Ok(snapshot)
    }

    /// request_id must be namespaced by native caller scope and command. The
    /// journal stores only a digest of arguments, never credential payloads.
    /// A replay returns the current snapshot without invoking revision checks or
    /// resurrecting history deleted since the request originally committed.
    pub(crate) fn mutate<T: Serialize, F>(
        &mut self,
        request_id: &str,
        payload: &T,
        mutation: F,
    ) -> Result<MutationResult, String>
    where
        F: FnOnce(&mut Snapshot) -> Result<(), String>,
    {
        if request_id.is_empty() || request_id.len() > 512 {
            return Err("Invalid router request identifier".into());
        }
        let value = serde_json::to_value(payload).map_err(error)?;
        fn canonical(value: serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Object(map) => {
                    let sorted = map
                        .into_iter()
                        .map(|(key, value)| (key, canonical(value)))
                        .collect::<std::collections::BTreeMap<_, _>>();
                    serde_json::Value::Object(sorted.into_iter().collect())
                }
                serde_json::Value::Array(values) => {
                    serde_json::Value::Array(values.into_iter().map(canonical).collect())
                }
                other => other,
            }
        }
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&canonical(value)).map_err(error)?)
        );
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let previous: Option<String> = transaction
            .query_row(
                "SELECT digest FROM requests WHERE id = ?1",
                [request_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)?;
        if let Some(saved_digest) = previous {
            if saved_digest != digest {
                return Err("Request identifier was already used with different arguments".into());
            }
            return Ok(MutationResult {
                snapshot: read(&transaction)?,
                replayed: true,
            });
        }
        let count: i64 = transaction
            .query_row("SELECT count(*) FROM requests", [], |row| row.get(0))
            .map_err(error)?;
        if count >= MAX_REQUESTS {
            return Err("Router request journal reached its qualified capacity; new mutations are blocked without expiring prior requests".into());
        }
        let mut snapshot = read(&transaction)?;
        let revision = snapshot.revision;
        mutation(&mut snapshot)?;
        snapshot.revision = next(revision)?;
        write(&transaction, &snapshot)?;
        transaction
            .execute(
                "INSERT INTO requests (id, digest, committed_revision) VALUES (?1, ?2, ?3)",
                params![request_id, digest, snapshot.revision.to_string()],
            )
            .map_err(error)?;
        transaction.commit().map_err(error)?;
        Ok(MutationResult {
            snapshot,
            replayed: false,
        })
    }
}

#[cfg(all(test, unix))]
mod catalog_migration_tests {
    use super::*;
    use serde_json::{json, Value};

    fn raw_run(id: &str, router: &str, profile: &str) -> Value {
        json!({
            "id": id, "routerId": router, "cwd": "/project", "title": "Saved task",
            "state": "idle", "model": "gpt-6-sol", "pinnedProfileId": null,
            "allowedProfileIds": [profile], "activeProfileId": null,
            "generation": 1, "revision": 1, "inputs": [], "attempts": [],
            "output": "kept output", "statusMessage": "", "attemptedProfileIds": []
        })
    }

    fn raw_snapshot(cli: &str) -> Value {
        json!({
            "revision": 7,
            "profiles": [
                {"id":"kept", "cli":"codex", "label":"Keep", "enabled":true,
                 "revision":1, "authState":"ready", "storageMode":"cli_managed"},
                {"id":"retired", "cli":cli, "label":"Retire", "enabled":true,
                 "revision":1, "authState":"ready", "storageMode":"api_key"}
            ],
            "routers": [
                {"id":"kept-pool", "cli":"codex", "label":"Keep", "enabled":true,
                 "orderedProfileIds":["kept"], "balanceRemainingQuota":false, "revision":1},
                {"id":"retired-pool", "cli":cli, "label":"Retire", "enabled":true,
                 "orderedProfileIds":["retired"], "balanceRemainingQuota":false, "revision":1}
            ],
            "quota": [{"profileId":"retired", "status":"stale", "windows":[],
                "observedAt":0, "expiresAt":0, "epoch":0, "blockRevision":0}],
            "runs": [raw_run("kept-run", "kept-pool", "kept"), raw_run("retired-run", "retired-pool", "retired")]
        })
    }

    fn backups(directory: &Path) -> Vec<std::path::PathBuf> {
        fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("router-before-cli-removal-")
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "sqlite")
            })
            .collect()
    }

    fn seed(path: &Path, value: &Value) -> Connection {
        drop(Store::open(path).unwrap());
        let connection = Connection::open(path).unwrap();
        connection
            .execute("UPDATE snapshot SET data = ?1", [value.to_string()])
            .unwrap();
        connection
    }

    #[test]
    fn every_retired_identifier_migrates_but_is_rejected_by_live_enum() {
        for cli in [
            "aider",
            "cline",
            "goose",
            "droid",
            "openhands",
            "continue",
            "amp",
            "auggie",
            "crush",
            "interpreter",
            "junie",
            "freebuff",
            "sweagent",
            "deepagents",
            "trae",
        ] {
            assert!(serde_json::from_value::<crate::cli_catalog::TitleCli>(json!(cli)).is_err());
            let migration = decode_migrating(&raw_snapshot(cli).to_string()).unwrap();
            assert!(migration.changed());
            assert_eq!(migration.snapshot.profiles.len(), 1);
            assert_eq!(migration.snapshot.routers.len(), 1);
            assert_eq!(migration.snapshot.runs[0].id, "kept-run");
            assert_eq!(migration.snapshot.runs.len(), 1);
            assert!(migration.snapshot.quota.is_empty());
        }
    }

    #[test]
    fn migration_backs_up_wal_and_credential_journal_before_atomic_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("router.sqlite");
        let original = raw_snapshot("goose");
        // Keep the writer open so committed state can remain in the WAL.
        let connection = seed(&path, &original);
        connection
            .execute(
                "INSERT INTO credential_journal VALUES ('retired-key', 'retired', 'remove')",
                [],
            )
            .unwrap();
        let store = Store::open(&path).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.revision, 8);
        assert_eq!(snapshot.profiles[0].id, "kept");
        assert_eq!(snapshot.runs[0].output, "kept output");
        assert_eq!(store.credential_journal().unwrap().len(), 1);
        let saved = backups(directory.path());
        assert_eq!(saved.len(), 1);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&saved[0]).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let backup =
            Connection::open_with_flags(&saved[0], OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let data: String = backup
            .query_row("SELECT data FROM snapshot", [], |row| row.get(0))
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&data).unwrap(), original);
        let count: i64 = backup
            .query_row("SELECT count(*) FROM credential_journal", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.snapshot().unwrap(), snapshot);
        assert_eq!(backups(directory.path()).len(), 1);
        drop(connection);
    }

    #[test]
    fn retired_profile_references_remove_affected_runs_without_expanding_grants() {
        for field in [
            "pinnedProfileId",
            "activeProfileId",
            "allowedProfileIds",
            "attemptedProfileIds",
            "attempts",
            "turns",
        ] {
            let mut source = raw_snapshot("goose");
            let mut affected = raw_run("affected", "kept-pool", "kept");
            match field {
                "pinnedProfileId" | "activeProfileId" => affected[field] = json!("retired"),
                "allowedProfileIds" | "attemptedProfileIds" => affected[field] = json!(["retired"]),
                "attempts" | "turns" => {
                    affected["inputs"] = json!([{"id":"input", "text":"saved question"}]);
                    affected["attempts"] = json!([{"id":"attempt", "inputId":"input", "profileId":"retired", "generation":1, "state":"completed", "reason":""}]);
                    if field == "turns" {
                        affected["turns"] = json!([{"inputId":"input", "attemptId":"attempt", "profileId":"retired", "generation":1, "state":"completed", "text":"kept output"}]);
                    }
                }
                _ => unreachable!(),
            }
            source["runs"].as_array_mut().unwrap().push(affected);
            let migrated = decode_migrating(&source.to_string()).unwrap().snapshot;
            assert_eq!(migrated.runs.len(), 1, "{field}");
            assert_eq!(migrated.runs[0].allowed_profile_ids, ["kept"]);
        }
    }

    #[test]
    fn unknown_cli_and_malformed_retired_data_fail_before_backup_or_writes() {
        for case in ["unknown", "malformed", "mismatched"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("router.sqlite");
            let mut source = raw_snapshot("goose");
            match case {
                "unknown" => source["profiles"][1]["cli"] = json!("unknown-cli"),
                "malformed" => source["profiles"][1]["unexpected"] = json!(true),
                "mismatched" => source["routers"][1]["cli"] = json!("aider"),
                _ => unreachable!(),
            }
            drop(seed(&path, &source));
            assert!(Store::open(&path).is_err(), "{case}");
            let connection = Connection::open(&path).unwrap();
            let saved: String = connection
                .query_row("SELECT data FROM snapshot", [], |row| row.get(0))
                .unwrap();
            assert_eq!(serde_json::from_str::<Value>(&saved).unwrap(), source);
            assert!(backups(directory.path()).is_empty());
        }
    }

    #[test]
    fn schema_one_retired_archives_keep_request_tombstones_and_original_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("router.sqlite");
        let mut archive = raw_snapshot("goose");
        archive["revision"] = json!(6);
        let mut current = decode_migrating(&archive.to_string()).unwrap().snapshot;
        current.revision = 7;
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE snapshot (id INTEGER PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE requests (id TEXT PRIMARY KEY, digest TEXT NOT NULL, result TEXT NOT NULL);").unwrap();
        connection
            .execute(
                "INSERT INTO snapshot VALUES (1, ?1)",
                [serde_json::to_string(&current).unwrap()],
            )
            .unwrap();
        let digest = format!("{:x}", Sha256::digest(b"\"input\""));
        connection
            .execute(
                "INSERT INTO requests VALUES ('legacy:request', ?1, ?2)",
                params![digest, archive.to_string()],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        drop(connection);
        let mut store = Store::open(&path).unwrap();
        let replay = store
            .mutate("legacy:request", &"input", |_| {
                panic!("must not replay retired task")
            })
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.snapshot, current);
        let saved = backups(directory.path());
        assert_eq!(saved.len(), 1);
        let backup =
            Connection::open_with_flags(&saved[0], OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let result: String = backup
            .query_row("SELECT result FROM requests", [], |row| row.get(0))
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&result).unwrap(), archive);
    }
}
