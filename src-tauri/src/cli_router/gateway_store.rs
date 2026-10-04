//! Durable native HTTP request ownership. A receipt records HTTP transport,
//! never CLI/agent completion. Unfinished or uncertain requests remain retained.
use super::{
    gateway_http::{Receipt, RequestSummary, Upstream, WireSummary},
    gateway_profiles::Protocol,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::{
    fd::AsRawFd,
    unix::fs::{MetadataExt, OpenOptionsExt},
};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

const VERSION: i64 = 1;
const MAX_RECORDS: i64 = 10_000;
const MAX_PAYLOAD: usize = 16 * 1024;
const MAX_DB: u64 = 64 * 1024 * 1024;
const MAX_DATA: i64 = 32 * 1024 * 1024;
const SCHEMA: &str = "CREATE TABLE binding(id INTEGER PRIMARY KEY CHECK(id=1),payload TEXT NOT NULL,digest TEXT NOT NULL);
CREATE TABLE intents(seq INTEGER PRIMARY KEY,attempt_id TEXT NOT NULL UNIQUE,request_id TEXT NOT NULL,attempt_index INTEGER NOT NULL,payload TEXT NOT NULL,digest TEXT NOT NULL,UNIQUE(request_id,attempt_index));
CREATE TABLE receipts(attempt_id TEXT PRIMARY KEY REFERENCES intents(attempt_id),payload TEXT NOT NULL,digest TEXT NOT NULL);";
fn failure() -> String {
    "The native gateway journal could not establish durable ownership. Existing data has been preserved.".into()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
}
fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    let value = serde_json::to_string(value).map_err(|_| failure())?;
    if value.len() > MAX_PAYLOAD {
        return Err(failure());
    }
    Ok(value)
}
fn decode<T: serde::de::DeserializeOwned>(payload: &str, hash: &str) -> Result<T, String> {
    if payload.len() > MAX_PAYLOAD || digest(payload.as_bytes()) != hash {
        return Err(failure());
    }
    serde_json::from_str(payload).map_err(|_| failure())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Binding {
    run_id: String,
    generation: u64,
    model: String,
    protocol: Protocol,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Intent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wire: Option<WireSummary>,
    request_id: String,
    upstream_attempt_id: String,
    attempt_index: usize,
    protocol: Protocol,
    path: String,
    model: String,
    body_bytes: usize,
    body_sha256: String,
    account_bound: bool,
    profile_id: String,
    profile_revision: u64,
    destination: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ReceiptView {
    pub(crate) request_id: String,
    pub(crate) upstream_attempt_id: String,
    pub(crate) attempt_index: usize,
    pub(crate) profile_id: String,
    pub(crate) status: Option<u16>,
    pub(crate) downstream_started: bool,
    pub(crate) complete: bool,
    pub(crate) cancelled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Continuity {
    pub(crate) profile_id: String,
    pub(crate) profile_revision: u64,
    pub(crate) destination: String,
}
impl From<&Receipt> for ReceiptView {
    fn from(r: &Receipt) -> Self {
        Self {
            request_id: r.request_id.clone(),
            upstream_attempt_id: r.upstream_attempt_id.clone(),
            attempt_index: r.attempt_index,
            profile_id: r.profile_id.clone(),
            status: r.status,
            downstream_started: r.downstream_started,
            complete: r.complete,
            cancelled: r.cancelled,
        }
    }
}
impl Intent {
    fn validate(&self, binding: &Binding) -> Result<(), String> {
        if !id(&self.request_id)
            || !id(&self.upstream_attempt_id)
            || !id(&self.profile_id)
            || self.attempt_index >= 32
            || self.protocol != binding.protocol
            || self.model != binding.model
            || self.body_bytes > 16 * 1024 * 1024
            || self.body_sha256.len() != 71
            || !self.body_sha256.starts_with("sha256:")
            || !self.body_sha256[7..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !valid_path(self.protocol, &self.path, &self.model)
        {
            return Err(failure());
        }
        if let Some(wire) = &self.wire {
            if self.path.ends_with("countTokens")
                || self.path.ends_with("count_tokens")
                || !super::gateway_transform::supports(self.protocol, wire.upstream_protocol)
                || !valid_path(wire.upstream_protocol, &wire.path, &self.model)
                || wire.path.ends_with("countTokens")
                || wire.path.ends_with("count_tokens")
                || wire.body_bytes > 16 * 1024 * 1024
                || !valid_digest(&wire.body_sha256)
            {
                return Err(failure());
            }
        }
        let upstream_protocol = self
            .wire
            .as_ref()
            .map_or(self.protocol, |wire| wire.upstream_protocol);
        super::gateway_url::join(
            &self.destination,
            upstream_protocol,
            self.wire
                .as_ref()
                .map_or(self.path.as_str(), |wire| wire.path.as_str()),
        )
        .map_err(|_| failure())?;
        if destination(&self.destination)? != self.destination {
            return Err(failure());
        }
        Ok(())
    }
    fn same_request(&self, other: &Self) -> bool {
        self.request_id == other.request_id
            && self.protocol == other.protocol
            && self.path == other.path
            && self.model == other.model
            && self.body_bytes == other.body_bytes
            && self.body_sha256 == other.body_sha256
            && self.account_bound == other.account_bound
    }
}
fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_path(protocol: Protocol, path: &str, model: &str) -> bool {
    match protocol {
        Protocol::Anthropic => matches!(path, "/v1/messages" | "/v1/messages/count_tokens"),
        Protocol::OpenAiChat => path == "/v1/chat/completions",
        Protocol::OpenAiResponses => path == "/v1/responses",
        Protocol::Gemini => ["/v1/", "/v1beta/"].iter().any(|p| {
            path == format!("{p}models/{model}:generateContent")
                || path == format!("{p}models/{model}:countTokens")
                || path == format!("{p}models/{model}:streamGenerateContent?alt=sse")
        }),
    }
}
fn destination(value: &str) -> Result<String, String> {
    super::gateway_url::parse(value)
        .map(|url| url.to_string())
        .map_err(|_| failure())
}
fn receipt_matches(receipt: &ReceiptView, intent: &Intent) -> Result<(), String> {
    if receipt.request_id != intent.request_id
        || receipt.upstream_attempt_id != intent.upstream_attempt_id
        || receipt.attempt_index != intent.attempt_index
        || receipt.profile_id != intent.profile_id
        || receipt.status.is_some_and(|s| !(100..=599).contains(&s))
        || receipt.downstream_started && receipt.status.is_none()
        || receipt.complete && receipt.status.is_none()
        || receipt.complete && !receipt.downstream_started && receipt.status != Some(429)
    {
        return Err(failure());
    }
    Ok(())
}
pub(crate) struct Journal {
    connection: Mutex<Connection>,
    root: PathBuf,
    binding: Binding,
    _lock: File,
    _directory: File,
    root_identity: (u64, u64),
    db_identity: (u64, u64),
    lock_identity: (u64, u64),
}
#[cfg(unix)]
fn private(path: &Path, optional: bool, max: u64) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(m)
            if m.is_file()
                && !m.file_type().is_symlink()
                && m.uid() == unsafe { libc::geteuid() }
                && m.mode() & 0o077 == 0
                && m.nlink() == 1
                && m.len() <= max =>
        {
            Ok(Some(m))
        }
        Err(e) if optional && e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        _ => Err(failure()),
    }
}
#[cfg(unix)]
fn directory(path: &Path) -> Result<fs::Metadata, String> {
    if !path.is_absolute() || fs::canonicalize(path).map_err(|_| failure())? != path {
        return Err(failure());
    }
    let m = fs::symlink_metadata(path).map_err(|_| failure())?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err(failure());
    }
    for parent in path.parent().into_iter().flat_map(Path::ancestors) {
        let p = fs::symlink_metadata(parent).map_err(|_| failure())?;
        if !p.is_dir()
            || p.file_type().is_symlink()
            || ![0, unsafe { libc::geteuid() }].contains(&p.uid())
            || p.mode() & 0o022 != 0 && !(p.uid() == 0 && p.mode() & 0o1000 != 0)
        {
            return Err(failure());
        }
    }
    Ok(m)
}
#[cfg(unix)]
fn regular(path: &Path, create: bool) -> Result<File, String> {
    let mut o = fs::OpenOptions::new();
    o.read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    if create {
        o.create_new(true);
    }
    let f = o.open(path).map_err(|_| failure())?;
    let m = f.metadata().map_err(|_| failure())?;
    let p = private(path, false, MAX_DB)?.ok_or_else(failure)?;
    if (m.dev(), m.ino()) != (p.dev(), p.ino()) {
        return Err(failure());
    }
    Ok(f)
}
fn validate(connection: &Connection, binding: &Binding) -> Result<(), String> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| failure())?;
    if version != VERSION {
        return Err(failure());
    }
    let check: String = connection
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|_| failure())?;
    if check != "ok" {
        return Err(failure());
    }
    let mut schema = connection
        .prepare("SELECT type,name,sql FROM sqlite_master ORDER BY name")
        .map_err(|_| failure())?;
    let rows = schema
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|_| failure())?;
    let expected: BTreeMap<_, _> = SCHEMA
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let s = s.trim();
            let name = s
                .strip_prefix("CREATE TABLE ")
                .unwrap()
                .split('(')
                .next()
                .unwrap()
                .trim();
            (name.to_owned(), s.to_owned())
        })
        .collect();
    let mut found = HashSet::new();
    for row in rows {
        let (kind, name, sql) = row.map_err(|_| failure())?;
        if kind == "index" && name.starts_with("sqlite_autoindex_") && sql.is_none() {
            continue;
        }
        if kind != "table" || expected.get(&name) != sql.as_ref() || !found.insert(name) {
            return Err(failure());
        }
    }
    if found.len() != 3 {
        return Err(failure());
    }
    let count: i64 = connection
        .query_row("SELECT count(*) FROM binding", [], |r| r.get(0))
        .map_err(|_| failure())?;
    if count != 1 {
        return Err(failure());
    }
    let (payload, hash): (String, String) = connection
        .query_row("SELECT payload,digest FROM binding WHERE id=1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|_| failure())?;
    if decode::<Binding>(&payload, &hash)? != *binding || payload != encode(binding)? {
        return Err(failure());
    }
    let count: i64 = connection
        .query_row("SELECT count(*) FROM intents", [], |r| r.get(0))
        .map_err(|_| failure())?;
    let receipt_count: i64 = connection
        .query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
        .map_err(|_| failure())?;
    let bytes:i64=connection.query_row("SELECT coalesce((SELECT sum(length(CAST(payload AS BLOB))) FROM intents),0)+coalesce((SELECT sum(length(CAST(payload AS BLOB))) FROM receipts),0)",[],|r|r.get(0)).map_err(|_|failure())?;
    if count > MAX_RECORDS || receipt_count > count || bytes > MAX_DATA {
        return Err(failure());
    }
    let oversized:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM intents WHERE length(CAST(payload AS BLOB))>16384 OR length(CAST(digest AS BLOB))!=64) OR EXISTS(SELECT 1 FROM receipts WHERE length(CAST(payload AS BLOB))>16384 OR length(CAST(digest AS BLOB))!=64)",[],|r|r.get(0)).map_err(|_|failure())?;
    if oversized {
        return Err(failure());
    }
    let mut st=connection.prepare("SELECT seq,attempt_id,request_id,attempt_index,payload,digest FROM intents ORDER BY seq").map_err(|_|failure())?;
    let mut rows = st.query([]).map_err(|_| failure())?;
    let mut attempts = BTreeMap::<String, Intent>::new();
    let mut logical = BTreeMap::<String, Vec<Intent>>::new();
    let mut profiles = BTreeMap::<String, (u64, String, Protocol)>::new();
    let mut seq = 0;
    while let Some(row) = rows.next().map_err(|_| failure())? {
        seq += 1;
        let number: i64 = row.get(0).map_err(|_| failure())?;
        let attempt: String = row.get(1).map_err(|_| failure())?;
        let request: String = row.get(2).map_err(|_| failure())?;
        let index: i64 = row.get(3).map_err(|_| failure())?;
        let (payload, hash): (String, String) = (
            row.get(4).map_err(|_| failure())?,
            row.get(5).map_err(|_| failure())?,
        );
        let intent: Intent = decode(&payload, &hash)?;
        intent.validate(binding)?;
        let profile_binding = (
            intent.profile_revision,
            intent.destination.clone(),
            intent
                .wire
                .as_ref()
                .map_or(intent.protocol, |wire| wire.upstream_protocol),
        );
        if profiles
            .get(&intent.profile_id)
            .is_some_and(|old| old != &profile_binding)
        {
            return Err(failure());
        }
        profiles.insert(intent.profile_id.clone(), profile_binding);
        if number != seq
            || attempt != intent.upstream_attempt_id
            || request != intent.request_id
            || index != intent.attempt_index as i64
            || payload != encode(&intent)?
            || attempts.insert(attempt, intent.clone()).is_some()
        {
            return Err(failure());
        }
        logical.entry(request).or_default().push(intent);
    }
    let mut recorded = BTreeMap::<String, ReceiptView>::new();
    let mut st = connection
        .prepare("SELECT attempt_id,payload,digest FROM receipts")
        .map_err(|_| failure())?;
    let mut rows = st.query([]).map_err(|_| failure())?;
    while let Some(row) = rows.next().map_err(|_| failure())? {
        let attempt: String = row.get(0).map_err(|_| failure())?;
        let payload: String = row.get(1).map_err(|_| failure())?;
        let hash: String = row.get(2).map_err(|_| failure())?;
        let receipt: ReceiptView = decode(&payload, &hash)?;
        let intent = attempts.get(&attempt).ok_or_else(failure)?;
        receipt_matches(&receipt, intent)?;
        if receipt.upstream_attempt_id != attempt
            || payload != encode(&receipt)?
            || recorded.insert(attempt, receipt).is_some()
        {
            return Err(failure());
        }
    }
    for values in logical.values() {
        let mut profiles = HashSet::new();
        for (index, intent) in values.iter().enumerate() {
            if intent.attempt_index != index
                || !values[0].same_request(intent)
                || !profiles.insert(&intent.profile_id)
            {
                return Err(failure());
            }
            if index > 0 {
                let prior = recorded
                    .get(&values[index - 1].upstream_attempt_id)
                    .ok_or_else(failure)?;
                if prior.status != Some(429)
                    || prior.downstream_started
                    || !prior.complete
                    || prior.cancelled
                {
                    return Err(failure());
                }
            }
        }
    }
    Ok(())
}
struct Inspection(PathBuf);
impl Drop for Inspection {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Journal {
    /// Missing journals must never be recreated when admitting saved history.
    pub(crate) fn open_existing(
        root: &Path,
        run_id: &str,
        generation: u64,
        model: &str,
        protocol: Protocol,
    ) -> Result<Self, String> {
        private(&root.join("gateway.sqlite3"), false, MAX_DB)?;
        private(&root.join("gateway.owner.lock"), false, 0)?;
        Self::open(root, run_id, generation, model, protocol)
    }
    #[cfg(unix)]
    pub(crate) fn open(
        root: &Path,
        run_id: &str,
        generation: u64,
        model: &str,
        protocol: Protocol,
    ) -> Result<Self, String> {
        if !id(run_id) || !id(model) {
            return Err(failure());
        }
        let root = root.to_owned();
        let meta = directory(&root)?;
        let directory_file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY)
            .open(&root)
            .map_err(|_| failure())?;
        let lock_path = root.join("gateway.owner.lock");
        let lock = if private(&lock_path, true, 0)?.is_some() {
            regular(&lock_path, false)?
        } else {
            regular(&lock_path, true)?
        };
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(failure());
        }
        let binding = Binding {
            run_id: run_id.into(),
            generation,
            model: model.into(),
            protocol,
        };
        let path = root.join("gateway.sqlite3");
        let mut files = Vec::new();
        for (suffix, max) in [
            ("", MAX_DB),
            ("-wal", MAX_DB),
            ("-shm", 1024 * 1024),
            ("-journal", MAX_DB),
        ] {
            if let Some(m) = private(&root.join(format!("gateway.sqlite3{suffix}")), true, max)? {
                files.push((suffix, m));
            }
        }
        let exists = files.iter().any(|(s, _)| s.is_empty());
        if !exists && !files.is_empty() {
            return Err(failure());
        }
        if exists {
            // SQLite READ_ONLY can recreate WAL SHM state. Inspect an owned
            // byte-for-byte snapshot instead, never touching original sidecars.
            let inspection = Inspection(
                std::env::temp_dir().join(format!("lomi-gateway-inspect-{}", super::new_id()?)),
            );
            fs::create_dir(&inspection.0).map_err(|_| failure())?;
            crate::chat::storage::private(&inspection.0, true)?;
            for (suffix, before) in &files {
                if *suffix == "-shm" {
                    continue;
                }
                let source = root.join(format!("gateway.sqlite3{suffix}"));
                let mut input = fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&source)
                    .map_err(|_| failure())?;
                let opened = input.metadata().map_err(|_| failure())?;
                if (opened.dev(), opened.ino()) != (before.dev(), before.ino()) {
                    return Err(failure());
                }
                let mut output =
                    regular(&inspection.0.join(format!("gateway.sqlite3{suffix}")), true)?;
                std::io::copy(
                    &mut std::io::Read::take(&mut input, MAX_DB + 1),
                    &mut output,
                )
                .map_err(|_| failure())?;
                let after = private(&source, false, MAX_DB)?.ok_or_else(failure)?;
                if (
                    after.dev(),
                    after.ino(),
                    after.len(),
                    after.mtime(),
                    after.mtime_nsec(),
                    after.ctime(),
                    after.ctime_nsec(),
                ) != (
                    before.dev(),
                    before.ino(),
                    before.len(),
                    before.mtime(),
                    before.mtime_nsec(),
                    before.ctime(),
                    before.ctime_nsec(),
                ) {
                    return Err(failure());
                }
            }
            let readonly = Connection::open_with_flags(
                inspection.0.join("gateway.sqlite3"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|_| failure())?;
            readonly
                .busy_timeout(Duration::from_secs(2))
                .map_err(|_| failure())?;
            readonly
                .execute_batch("PRAGMA trusted_schema=OFF;PRAGMA query_only=ON;")
                .map_err(|_| failure())?;
            validate(&readonly, &binding)?;
            drop(readonly);
            // Recheck all snapshot identities after the complete inspection,
            // including sidecars that were initially absent.
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let after = private(&root.join(format!("gateway.sqlite3{suffix}")), true, MAX_DB)?;
                let before = files.iter().find(|(s, _)| *s == suffix).map(|(_, m)| m);
                match (before, after) {
                    (None, None) => {}
                    (Some(b), Some(a))
                        if (
                            a.dev(),
                            a.ino(),
                            a.len(),
                            a.mtime(),
                            a.mtime_nsec(),
                            a.ctime(),
                            a.ctime_nsec(),
                        ) == (
                            b.dev(),
                            b.ino(),
                            b.len(),
                            b.mtime(),
                            b.mtime_nsec(),
                            b.ctime(),
                            b.ctime_nsec(),
                        ) => {}
                    _ => return Err(failure()),
                }
            }
        } else {
            let file = regular(&path, true)?;
            file.sync_all().map_err(|_| failure())?;
            directory_file.sync_all().map_err(|_| failure())?;
        }
        let db = private(&path, false, MAX_DB)?.ok_or_else(failure)?;
        let lock_meta = lock.metadata().map_err(|_| failure())?;
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| failure())?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|_| failure())?;
        connection.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;PRAGMA foreign_keys=ON;PRAGMA trusted_schema=OFF;PRAGMA max_page_count=16384;PRAGMA wal_autocheckpoint=128;").map_err(|_|failure())?;
        let mode: String = connection
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .map_err(|_| failure())?;
        let sync: i64 = connection
            .pragma_query_value(None, "synchronous", |r| r.get(0))
            .map_err(|_| failure())?;
        let page: i64 = connection
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(|_| failure())?;
        if mode != "wal" || sync != 2 || page != 4096 {
            return Err(failure());
        }
        if !exists {
            let tx = connection.unchecked_transaction().map_err(|_| failure())?;
            tx.execute_batch(SCHEMA).map_err(|_| failure())?;
            let payload = encode(&binding)?;
            tx.execute(
                "INSERT INTO binding VALUES(1,?1,?2)",
                params![payload, digest(payload.as_bytes())],
            )
            .map_err(|_| failure())?;
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(|_| failure())?;
            tx.commit().map_err(|_| failure())?;
            directory_file.sync_all().map_err(|_| failure())?;
        }
        validate(&connection, &binding)?;
        directory_file.sync_all().map_err(|_| failure())?;
        let journal = Self {
            connection: Mutex::new(connection),
            root,
            binding,
            _lock: lock,
            _directory: directory_file,
            root_identity: (meta.dev(), meta.ino()),
            db_identity: (db.dev(), db.ino()),
            lock_identity: (lock_meta.dev(), lock_meta.ino()),
        };
        journal.fence()?;
        Ok(journal)
    }
    #[cfg(not(unix))]
    pub(crate) fn open(_: &Path, _: &str, _: u64, _: &str, _: Protocol) -> Result<Self, String> {
        Err(failure())
    }
    fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.connection.try_lock() {
                Ok(g) => return Ok(g),
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(failure()),
                Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => return Err(failure()),
            }
        }
    }
    #[cfg(unix)]
    fn fence(&self) -> Result<(), String> {
        let root = directory(&self.root)?;
        if (root.dev(), root.ino()) != self.root_identity {
            return Err(failure());
        }
        for (name, identity, max) in [
            ("gateway.sqlite3", self.db_identity, MAX_DB),
            ("gateway.owner.lock", self.lock_identity, 0),
        ] {
            let m = private(&self.root.join(name), false, max)?.ok_or_else(failure)?;
            if (m.dev(), m.ino()) != identity {
                return Err(failure());
            }
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            private(
                &self.root.join(format!("gateway.sqlite3{suffix}")),
                true,
                MAX_DB,
            )?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    fn fence(&self) -> Result<(), String> {
        Err(failure())
    }
    pub(crate) fn intent(
        &self,
        request: &RequestSummary,
        upstream: &Upstream,
    ) -> Result<(), String> {
        self.fence()?;
        let intent = Intent {
            wire: request.wire.clone(),
            request_id: request.id.clone(),
            upstream_attempt_id: request.upstream_attempt_id.clone(),
            attempt_index: request.attempt_index,
            protocol: request.protocol,
            path: request.path.clone(),
            model: request.model.clone(),
            body_bytes: request.body_bytes,
            body_sha256: request.body_sha256.clone(),
            account_bound: request.account_bound,
            profile_id: upstream.profile_id.clone(),
            profile_revision: upstream.revision,
            destination: destination(&upstream.base_url)?,
        };
        if request
            .wire
            .as_ref()
            .map_or(request.protocol, |wire| wire.upstream_protocol)
            != upstream.protocol
        {
            return Err(failure());
        }
        intent.validate(&self.binding)?;
        let payload = encode(&intent)?;
        let mut connection = self.lock()?;
        validate(&connection, &self.binding)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT payload,digest FROM intents WHERE attempt_id=?1",
                [&intent.upstream_attempt_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        if let Some((old, hash)) = old {
            if decode::<Intent>(&old, &hash)? != intent || old != payload {
                return Err(failure());
            }
            return Ok(());
        }
        let count: i64 = tx
            .query_row("SELECT count(*) FROM intents", [], |r| r.get(0))
            .map_err(|_| failure())?;
        if count >= MAX_RECORDS {
            return Err(failure());
        }
        tx.execute("INSERT INTO intents(seq,attempt_id,request_id,attempt_index,payload,digest) VALUES(?1,?2,?3,?4,?5,?6)",params![count+1,intent.upstream_attempt_id,intent.request_id,intent.attempt_index as i64,payload,digest(payload.as_bytes())]).map_err(|_|failure())?;
        validate(&tx, &self.binding)?;
        self.fence()?;
        tx.commit().map_err(|_| failure())?;
        self._directory.sync_all().map_err(|_| failure())?;
        self.fence()
    }
    pub(crate) fn finish(&self, receipt: &Receipt) -> Result<(), String> {
        self.fence()?;
        let receipt = ReceiptView::from(receipt);
        let payload = encode(&receipt)?;
        let mut connection = self.lock()?;
        validate(&connection, &self.binding)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let row: Option<(String, String)> = tx
            .query_row(
                "SELECT payload,digest FROM intents WHERE attempt_id=?1",
                [&receipt.upstream_attempt_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        let (body, hash) = row.ok_or_else(failure)?;
        let intent: Intent = decode(&body, &hash)?;
        receipt_matches(&receipt, &intent)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT payload,digest FROM receipts WHERE attempt_id=?1",
                [&receipt.upstream_attempt_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        if let Some((old, hash)) = old {
            if decode::<ReceiptView>(&old, &hash)? != receipt || old != payload {
                return Err(failure());
            }
            return Ok(());
        }
        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3)",
            params![
                receipt.upstream_attempt_id,
                payload,
                digest(payload.as_bytes())
            ],
        )
        .map_err(|_| failure())?;
        validate(&tx, &self.binding)?;
        self.fence()?;
        tx.commit().map_err(|_| failure())?;
        self._directory.sync_all().map_err(|_| failure())?;
        self.fence()
    }
    pub(crate) fn idle(&self) -> Result<bool, String> {
        self.fence()?;
        let connection = self.lock()?;
        validate(&connection, &self.binding)?;
        let missing:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM intents i LEFT JOIN receipts r USING(attempt_id) WHERE r.attempt_id IS NULL)",[],|r|r.get(0)).map_err(|_|failure())?;
        if missing {
            return Ok(false);
        }
        Ok(self
            .read_receipts(&connection)?
            .iter()
            .all(|r| r.complete && !r.cancelled))
    }
    /// Bind retained native context to its last successful credential, not to
    /// a later rejected/uncertain intent's active-profile display field.
    pub(crate) fn continuity(&self) -> Result<Option<Continuity>, String> {
        if !self.idle()? {
            return Err(failure());
        }
        self.fence()?;
        let connection = self.lock()?;
        validate(&connection, &self.binding)?;
        let mut statement = connection.prepare("SELECT i.payload,i.digest,r.payload,r.digest FROM intents i JOIN receipts r USING(attempt_id) ORDER BY i.seq DESC").map_err(|_| failure())?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| failure())?;
        for row in rows {
            let (payload, hash, receipt, receipt_hash) = row.map_err(|_| failure())?;
            let receipt: ReceiptView = decode(&receipt, &receipt_hash)?;
            if receipt.complete
                && !receipt.cancelled
                && receipt.downstream_started
                && receipt.status.is_some_and(|s| (200..300).contains(&s))
            {
                let intent: Intent = decode(&payload, &hash)?;
                if !creates_context(&intent.path) {
                    continue;
                }
                return Ok(Some(Continuity {
                    profile_id: intent.profile_id,
                    profile_revision: intent.profile_revision,
                    destination: intent.destination,
                }));
            }
        }
        Ok(None)
    }
    pub(crate) fn context_receipt(&self, receipt: &Receipt) -> Result<bool, String> {
        self.fence()?;
        let connection = self.lock()?;
        validate(&connection, &self.binding)?;
        let (payload, hash): (String, String) = connection
            .query_row(
                "SELECT payload,digest FROM intents WHERE attempt_id=?1",
                [&receipt.upstream_attempt_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| failure())?;
        let intent: Intent = decode(&payload, &hash)?;
        receipt_matches(&ReceiptView::from(receipt), &intent)?;
        Ok(creates_context(&intent.path))
    }
    fn read_receipts(&self, connection: &Connection) -> Result<Vec<ReceiptView>, String> {
        let mut st=connection.prepare("SELECT r.payload,r.digest FROM receipts r JOIN intents i USING(attempt_id) ORDER BY i.seq").map_err(|_|failure())?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|_| failure())?;
        rows.map(|row| {
            let (p, h) = row.map_err(|_| failure())?;
            decode(&p, &h)
        })
        .collect()
    }
    #[cfg(test)]
    pub(crate) fn receipts(&self) -> Result<Vec<ReceiptView>, String> {
        self.fence()?;
        let connection = self.lock()?;
        validate(&connection, &self.binding)?;
        self.read_receipts(&connection)
    }
}

fn creates_context(path: &str) -> bool {
    !path.ends_with("/count_tokens") && !path.ends_with(":countTokens")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "lomi-gateway-store-test-{}",
            super::super::new_id().unwrap()
        ));
        fs::create_dir(&p).unwrap();
        crate::chat::storage::private(&p, true).unwrap();
        fs::canonicalize(p).unwrap()
    }
    fn request() -> RequestSummary {
        RequestSummary {
            wire: None,
            id: "request".into(),
            upstream_attempt_id: "attempt".into(),
            attempt_index: 0,
            protocol: Protocol::Anthropic,
            path: "/v1/messages".into(),
            model: "owned".into(),
            body_bytes: 3,
            body_sha256: format!("sha256:{}", digest(b"abc")),
            account_bound: false,
        }
    }
    fn upstream() -> Upstream {
        Upstream {
            protocol: Protocol::Anthropic,
            profile_id: "profile".into(),
            revision: 1,
            base_url: "https://api.anthropic.com".into(),
            api_key: zeroize::Zeroizing::new("never-journal-this-key".into()),
        }
    }
    #[test]
    fn converted_intents_bind_wire_bytes_without_changing_native_retry_identity() {
        let binding = Binding {
            run_id: "run".into(),
            generation: 1,
            model: "owned".into(),
            protocol: Protocol::Anthropic,
        };
        let mut intent = Intent {
            wire: None,
            request_id: "request".into(),
            upstream_attempt_id: "attempt".into(),
            attempt_index: 0,
            protocol: Protocol::Anthropic,
            path: "/v1/messages".into(),
            model: "owned".into(),
            body_bytes: 3,
            body_sha256: format!("sha256:{}", digest(b"abc")),
            account_bound: false,
            profile_id: "profile".into(),
            profile_revision: 1,
            destination: "https://example.com/".into(),
        };
        let native = intent.clone();
        intent.wire = Some(WireSummary {
            upstream_protocol: Protocol::OpenAiChat,
            path: "/v1/chat/completions".into(),
            body_bytes: 5,
            body_sha256: format!("sha256:{}", digest(b"other")),
        });
        assert!(intent.validate(&binding).is_ok());
        assert!(intent.same_request(&native));
        intent.wire.as_mut().unwrap().path = "/v1/messages".into();
        assert!(intent.validate(&binding).is_err());
        intent.wire.as_mut().unwrap().path = "/v1/chat/completions".into();
        intent.wire.as_mut().unwrap().body_sha256 = "unbound".into();
        assert!(intent.validate(&binding).is_err());
        let legacy = encode(&native).unwrap();
        assert!(!legacy.contains("wire"));
        assert_eq!(
            decode::<Intent>(&legacy, &digest(legacy.as_bytes())).unwrap(),
            native
        );
        let partial = legacy.replacen("{", "{\"wire\":{\"upstreamProtocol\":\"openai_chat\"},", 1);
        assert!(serde_json::from_str::<Intent>(&partial).is_err());
    }
    fn receipt() -> Receipt {
        Receipt {
            request_id: "request".into(),
            upstream_attempt_id: "attempt".into(),
            attempt_index: 0,
            profile_id: "profile".into(),
            status: Some(200),
            downstream_started: true,
            complete: true,
            cancelled: false,
        }
    }
    #[test]
    fn saved_context_uses_accepted_receipts_and_never_recreates_missing_journal() {
        let root = root();
        assert!(Journal::open_existing(&root, "run", 1, "owned", Protocol::Anthropic).is_err());
        assert!(!root.join("gateway.sqlite3").exists());
        let journal = Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).unwrap();
        let mut count = request();
        count.path = "/v1/messages/count_tokens".into();
        journal.intent(&count, &upstream()).unwrap();
        journal.finish(&receipt()).unwrap();
        assert!(!journal.context_receipt(&receipt()).unwrap());
        assert!(journal.continuity().unwrap().is_none());
        let mut next = request();
        next.id = "next".into();
        next.upstream_attempt_id = "next-attempt".into();
        journal.intent(&next, &upstream()).unwrap();
        assert!(journal.continuity().is_err());
        let mut accepted = receipt();
        accepted.request_id = next.id.clone();
        accepted.upstream_attempt_id = next.upstream_attempt_id.clone();
        journal.finish(&accepted).unwrap();
        assert!(journal.context_receipt(&accepted).unwrap());
        assert_eq!(journal.continuity().unwrap().unwrap().profile_id, "profile");
        drop(journal);
        let reopened =
            Journal::open_existing(&root, "run", 1, "owned", Protocol::Anthropic).unwrap();
        assert_eq!(reopened.continuity().unwrap().unwrap().profile_revision, 1);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn intent_precedes_receipt_and_only_exact_repeats_are_idempotent() {
        let root = root();
        let journal = Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).unwrap();
        assert!(journal.finish(&receipt()).is_err());
        journal.intent(&request(), &upstream()).unwrap();
        assert!(!journal.idle().unwrap());
        journal.intent(&request(), &upstream()).unwrap();
        let mut changed = request();
        changed.body_sha256 = format!("sha256:{}", digest(b"xyz"));
        assert!(journal.intent(&changed, &upstream()).is_err());
        journal.finish(&receipt()).unwrap();
        journal.finish(&receipt()).unwrap();
        assert_eq!(
            journal.receipts().unwrap(),
            vec![ReceiptView::from(&receipt())]
        );
        assert!(journal.idle().unwrap());
        let mut changed = receipt();
        changed.complete = false;
        assert!(journal.finish(&changed).is_err());
        drop(journal);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn explicit_api_prefixes_and_existing_origin_spelling_survive_journal_reopen() {
        for (protocol, base, path) in [
            (
                Protocol::Anthropic,
                "https://api.anthropic.com",
                "/v1/messages",
            ),
            (
                Protocol::Anthropic,
                "https://example.com/anthropic",
                "/v1/messages",
            ),
            (
                Protocol::OpenAiChat,
                "https://openrouter.ai/api/v1",
                "/v1/chat/completions",
            ),
            (
                Protocol::OpenAiResponses,
                "https://example.com/compatible-mode/v1",
                "/v1/responses",
            ),
            (
                Protocol::Gemini,
                "https://example.com/gemini/v1beta",
                "/v1beta/models/owned:streamGenerateContent?alt=sse",
            ),
        ] {
            let root = root();
            let journal = Journal::open(&root, "run", 1, "owned", protocol).unwrap();
            let mut request = request();
            request.protocol = protocol;
            request.path = path.into();
            let mut upstream = upstream();
            upstream.protocol = protocol;
            upstream.base_url = base.into();
            journal.intent(&request, &upstream).unwrap();
            journal.finish(&receipt()).unwrap();
            let payload: String = journal
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT payload FROM intents", [], |row| row.get(0))
                .unwrap();
            let recorded: Intent = serde_json::from_str(&payload).unwrap();
            assert_eq!(
                recorded.destination,
                reqwest::Url::parse(base).unwrap().to_string()
            );
            drop(journal);
            let journal = Journal::open(&root, "run", 1, "owned", protocol).unwrap();
            assert!(journal.idle().unwrap());
            assert_eq!(
                journal.receipts().unwrap(),
                vec![ReceiptView::from(&receipt())]
            );
            drop(journal);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn unfinished_or_incomplete_intent_survives_reopen() {
        let root = root();
        {
            let j = Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).unwrap();
            j.intent(&request(), &upstream()).unwrap();
        }
        let j = Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).unwrap();
        assert!(!j.idle().unwrap());
        let mut r = receipt();
        r.complete = false;
        j.finish(&r).unwrap();
        assert!(!j.idle().unwrap());
        drop(j);
        let j = Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).unwrap();
        assert!(!j.idle().unwrap());
        drop(j);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn future_and_corrupt_database_bytes_are_preserved() {
        for future in [true, false] {
            let root = root();
            let path = root.join("gateway.sqlite3");
            if future {
                let c = Connection::open(&path).unwrap();
                c.execute_batch("CREATE TABLE retained(value TEXT);INSERT INTO retained VALUES('original');PRAGMA user_version=999;").unwrap();
            } else {
                fs::write(&path, b"retained corrupt native journal").unwrap();
            }
            crate::chat::storage::private(&path, false).unwrap();
            let before = fs::read(&path).unwrap();
            assert!(Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn future_wal_database_and_all_original_sidecars_are_preserved() {
        let root = root();
        let path = root.join("gateway.sqlite3");
        let connection = Connection::open(&path).unwrap();
        crate::chat::storage::private(&path, false).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE retained(value TEXT);INSERT INTO retained VALUES('original');PRAGMA user_version=999;").unwrap();
        let mut before = Vec::new();
        for suffix in ["", "-wal", "-shm"] {
            let path = root.join(format!("gateway.sqlite3{suffix}"));
            crate::chat::storage::private(&path, false).unwrap();
            before.push((path.clone(), fs::read(path).unwrap()));
        }
        assert!(Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).is_err());
        for (path, bytes) in before {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn keys_are_absent_from_intent_and_receipt_records() {
        let root = root();
        let journal = Journal::open(&root, "run", 1, "owned", Protocol::Anthropic).unwrap();
        journal.intent(&request(), &upstream()).unwrap();
        journal.finish(&receipt()).unwrap();
        for suffix in ["", "-wal"] {
            let bytes = fs::read(root.join(format!("gateway.sqlite3{suffix}"))).unwrap();
            assert!(!bytes
                .windows(b"never-journal-this-key".len())
                .any(|v| v == b"never-journal-this-key"));
        }
        drop(journal);
        fs::remove_dir_all(root).unwrap();
    }
}
