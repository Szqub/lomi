//! Independent durable coding-effect journal. No existing router schema changes.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const MAX_RECORDS: i64 = 100_000;
const MAX_BYTES: i64 = 240 * 1024 * 1024;
fn failure() -> String {
    "The coding journal could not establish durable ownership. No effect may be replayed.".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Binding {
    pub run_id: String,
    pub attempt_id: String,
    pub generation: u64,
    pub auth_revision: u64,
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
}
impl Binding {
    pub(crate) fn key(&self) -> Result<String, String> {
        for value in [
            &self.run_id,
            &self.attempt_id,
            &self.thread_id,
            &self.turn_id,
            &self.call_id,
        ] {
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err(failure());
            }
        }
        serde_json::to_string(&(&self.run_id, &self.thread_id, &self.turn_id, &self.call_id))
            .map_err(|_| failure())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Record {
    pub binding: Binding,
    pub name: String,
    pub args_digest: String,
    pub path: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub before_dev: Option<u64>,
    pub before_ino: Option<u64>,
    pub parent_dev: u64,
    pub parent_ino: u64,
    pub before_bytes: Option<String>,
    pub after_bytes: Option<String>,
    pub temporary: Option<String>,
    pub result: serde_json::Value,
    pub result_digest: String,
}
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub record: Record,
    pub state: String,
}
pub(crate) struct Journal {
    connection: Connection,
}
impl Journal {
    pub(crate) fn open(root: &Path) -> Result<Self, String> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let metadata = fs::symlink_metadata(root).map_err(|_| failure())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(failure());
        }
        let path = root.join("effects.sqlite3");
        let existing = path.exists();
        match fs::symlink_metadata(&path) {
            Ok(meta)
                if meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.uid() == unsafe { libc::geteuid() }
                    && meta.mode() & 0o077 == 0
                    && meta.nlink() == 1 => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .map_err(|_| failure())?;
                file.sync_all().map_err(|_| failure())?;
                fs::File::open(root)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| failure())?;
            }
            _ => return Err(failure()),
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            match fs::symlink_metadata(root.join(format!("effects.sqlite3{suffix}"))) {
                Ok(meta)
                    if meta.is_file()
                        && !meta.file_type().is_symlink()
                        && meta.uid() == unsafe { libc::geteuid() }
                        && meta.mode() & 0o077 == 0
                        && meta.nlink() == 1 => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(failure()),
            }
        }
        if fs::metadata(&path).map_err(|_| failure())?.len() > 256 * 1024 * 1024 {
            return Err(failure());
        }
        // A writable first read can recover a hot rollback journal before
        // discovering future/corrupt schema. Preflight existing data read-only.
        if existing {
            let readonly =
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|_| failure())?;
            let version: i64 = readonly
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(|_| failure())?;
            if ![0, 1, 2].contains(&version) {
                return Err(failure());
            }
            let integrity: String = readonly
                .query_row("PRAGMA quick_check", [], |row| row.get(0))
                .map_err(|_| failure())?;
            if integrity != "ok" {
                return Err(failure());
            }
            if version != 0 {
                // Reject malformed supported schema before writable recovery or migration.
                for query in [
                    "SELECT call_key,payload,payload_digest,args_digest,state FROM calls LIMIT 0",
                    "SELECT key,value FROM metadata LIMIT 0",
                    "SELECT call_key,dev,ino FROM staged LIMIT 0",
                    "SELECT nonce,call_key,digest FROM approvals LIMIT 0",
                ] {
                    readonly.prepare(query).map_err(|_| failure())?;
                }
                if version == 2 {
                    readonly.prepare("SELECT call_key,payload,payload_digest,result_digest FROM denials LIMIT 0").map_err(|_|failure())?;
                }
            }
            if version == 0 {
                let count:i64=readonly.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",[],|row|row.get(0)).map_err(|_|failure())?;
                if count != 0 {
                    return Err(failure());
                }
            }
        }
        let connection = Connection::open(path).map_err(|_| failure())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| failure())?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| failure())?;
        if ![0, 1, 2].contains(&version) {
            return Err(failure());
        }
        let page_size: i64 = connection
            .pragma_query_value(None, "page_size", |row| row.get(0))
            .map_err(|_| failure())?;
        if page_size != 4096 {
            return Err(failure());
        }
        let check: String = connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(|_| failure())?;
        if check != "ok" {
            return Err(failure());
        }
        if version == 0 {
            let existing: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",[], |row| row.get(0)).map_err(|_| failure())?;
            if existing != 0 {
                return Err(failure());
            }
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA max_page_count=65536; PRAGMA wal_autocheckpoint=128;").map_err(|_| failure())?;
        let mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|_| failure())?;
        let synchronous: i64 = connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .map_err(|_| failure())?;
        if mode != "wal" || synchronous != 2 {
            return Err(failure());
        }
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE calls(call_key TEXT PRIMARY KEY, payload TEXT NOT NULL, payload_digest TEXT NOT NULL, args_digest TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('prepared','executing','committed','denied','uncertain','unapplied')));
                CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                CREATE TABLE staged(call_key TEXT PRIMARY KEY REFERENCES calls(call_key), dev TEXT NOT NULL, ino TEXT NOT NULL);
                CREATE TABLE approvals(nonce TEXT PRIMARY KEY, call_key TEXT NOT NULL UNIQUE REFERENCES calls(call_key), digest TEXT NOT NULL);
                CREATE TABLE denials(call_key TEXT PRIMARY KEY REFERENCES calls(call_key), payload TEXT NOT NULL, payload_digest TEXT NOT NULL, result_digest TEXT NOT NULL);
                PRAGMA user_version=2; COMMIT;").map_err(|_| failure())?;
        }
        if version == 1 {
            // Old denied calls lack an authoritative reply. Preserve them without
            // inventing a result; only newly journaled denials can be replayed.
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE denials(call_key TEXT PRIMARY KEY REFERENCES calls(call_key), payload TEXT NOT NULL, payload_digest TEXT NOT NULL, result_digest TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(|_|failure())?;
        }
        Ok(Self { connection })
    }
    pub(crate) fn bind_project(&mut self, identity: &str) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT value FROM metadata WHERE key='project'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| failure())?;
        if let Some(existing) = existing {
            if existing != identity {
                return Err(failure());
            }
        } else {
            let count: i64 = transaction
                .query_row("SELECT count(*) FROM calls", [], |row| row.get(0))
                .map_err(|_| failure())?;
            if count != 0 {
                return Err(failure());
            }
            transaction
                .execute(
                    "INSERT INTO metadata(key,value) VALUES('project',?)",
                    [identity],
                )
                .map_err(|_| failure())?;
        }
        transaction.commit().map_err(|_| failure())?;
        Ok(())
    }
    pub(crate) fn get(&self, binding: &Binding) -> Result<Option<Entry>, String> {
        let row = self
            .connection
            .query_row(
                "SELECT payload,state,payload_digest,args_digest,call_key FROM calls WHERE call_key=?",
                [binding.key()?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,row.get::<_,String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| failure())?;
        row.map(|(payload, state, hash, args_digest, key)| {
            checked_entry(payload, state, hash, args_digest, key)
        })
        .transpose()
    }
    pub(crate) fn insert(&mut self, record: &Record, state: &str) -> Result<(), String> {
        if !["prepared", "committed"].contains(&state) {
            return Err(failure());
        }
        let payload = serde_json::to_string(record).map_err(|_| failure())?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let (count, size): (i64, i64) = transaction
            .query_row(
                "SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0)+(SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM denials) FROM calls",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| failure())?;
        if count >= MAX_RECORDS || size.saturating_add(payload.len() as i64) > MAX_BYTES {
            return Err(
                "The coding journal capacity is exhausted; no further effects are allowed.".into(),
            );
        }
        transaction
            .execute(
                "INSERT INTO calls(call_key,payload,payload_digest,args_digest,state) VALUES(?,?,?,?,?)",
                params![record.binding.key()?, payload, format!("{:x}",Sha256::digest(payload.as_bytes())),record.args_digest, state],
            )
            .map_err(|_| failure())?;
        transaction.commit().map_err(|_| failure())?;
        Ok(())
    }
    pub(crate) fn authorize(
        &mut self,
        binding: &Binding,
        nonce: &str,
        digest: &str,
    ) -> Result<(), String> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(failure());
        }
        if nonce.len() < 32
            || nonce.len() > 256
            || !nonce
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err(failure());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let key = binding.key()?;
        transaction
            .execute(
                "INSERT INTO approvals(nonce,call_key,digest) VALUES(?,?,?)",
                params![nonce, key, digest],
            )
            .map_err(|_| failure())?;
        if transaction
            .execute(
                "UPDATE calls SET state='executing' WHERE call_key=? AND state='prepared'",
                [key],
            )
            .map_err(|_| failure())?
            != 1
        {
            return Err(failure());
        }
        transaction.commit().map_err(|_| failure())?;
        Ok(())
    }
    pub(crate) fn stage(&mut self, binding: &Binding, dev: u64, ino: u64) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let key = binding.key()?;
        let state: String = transaction
            .query_row("SELECT state FROM calls WHERE call_key=?", [&key], |row| {
                row.get(0)
            })
            .map_err(|_| failure())?;
        if state != "executing" {
            return Err(failure());
        }
        transaction
            .execute(
                "INSERT INTO staged(call_key,dev,ino) VALUES(?,?,?)",
                params![key, dev.to_string(), ino.to_string()],
            )
            .map_err(|_| failure())?;
        transaction.commit().map_err(|_| failure())?;
        Ok(())
    }
    pub(crate) fn staged(&self, binding: &Binding) -> Result<Option<(u64, u64)>, String> {
        let row = self
            .connection
            .query_row(
                "SELECT dev,ino FROM staged WHERE call_key=?",
                [binding.key()?],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        row.map(|(dev, ino)| {
            Ok((
                dev.parse().map_err(|_| failure())?,
                ino.parse().map_err(|_| failure())?,
            ))
        })
        .transpose()
    }
    pub(crate) fn denied_result(&self, binding: &Binding) -> Result<serde_json::Value, String> {
        let entry = self.get(binding)?.ok_or_else(failure)?;
        if entry.state != "denied" {
            return Err(failure());
        }
        let (payload, payload_digest, result_digest): (String, String, String) = self
            .connection
            .query_row(
                "SELECT payload,payload_digest,result_digest FROM denials WHERE call_key=?",
                [binding.key()?],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| failure())?;
        let result: serde_json::Value = serde_json::from_str(&payload).map_err(|_| failure())?;
        if format!("{:x}", Sha256::digest(payload.as_bytes())) != payload_digest
            || super::effects::value_digest(&result)? != result_digest
            || result != denial_result(&entry.record)
        {
            return Err(failure());
        }
        Ok(result)
    }
    pub(crate) fn deny(&mut self, binding: &Binding) -> Result<serde_json::Value, String> {
        let entry = self.get(binding)?.ok_or_else(failure)?;
        if entry.state == "denied" {
            return self.denied_result(binding);
        }
        if entry.state != "prepared" || entry.record.binding != *binding {
            return Err(failure());
        }
        let result = denial_result(&entry.record);
        let payload = serde_json::to_string(&result).map_err(|_| failure())?;
        let result_digest = super::effects::value_digest(&result)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let size:i64=transaction.query_row("SELECT (SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM calls)+(SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM denials)",[],|row|row.get(0)).map_err(|_|failure())?;
        if size.saturating_add(payload.len() as i64) > MAX_BYTES {
            return Err(failure());
        }
        transaction.execute("INSERT INTO denials(call_key,payload,payload_digest,result_digest) VALUES(?,?,?,?)",
            params![binding.key()?,payload,format!("{:x}",Sha256::digest(payload.as_bytes())),result_digest]).map_err(|_|failure())?;
        if transaction
            .execute(
                "UPDATE calls SET state='denied' WHERE call_key=? AND state='prepared'",
                [binding.key()?],
            )
            .map_err(|_| failure())?
            != 1
        {
            return Err(failure());
        }
        transaction.commit().map_err(|_| failure())?;
        Ok(result)
    }
    pub(crate) fn transition(
        &mut self,
        binding: &Binding,
        expected: &str,
        state: &str,
    ) -> Result<(), String> {
        if !["committed", "uncertain", "unapplied"].contains(&state) {
            return Err(failure());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        if transaction
            .execute(
                "UPDATE calls SET state=? WHERE call_key=? AND state=?",
                params![state, binding.key()?, expected],
            )
            .map_err(|_| failure())?
            != 1
        {
            return Err(failure());
        }
        transaction.commit().map_err(|_| failure())?;
        Ok(())
    }
    pub(crate) fn pending(&self) -> Result<Vec<Entry>, String> {
        let mut statement=self.connection.prepare("SELECT payload,state,payload_digest,args_digest,call_key FROM calls WHERE state IN ('executing','uncertain') ORDER BY call_key").map_err(|_|failure())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| failure())?;
        let mut entries = Vec::new();
        for row in rows {
            let (payload, state, hash, args_digest, key) = row.map_err(|_| failure())?;
            entries.push(checked_entry(payload, state, hash, args_digest, key)?);
        }
        Ok(entries)
    }
}

fn denial_result(record: &Record) -> serde_json::Value {
    serde_json::json!({"denied":true,"path":record.path,"reason":"The user denied this coding effect."})
}

fn checked_entry(
    payload: String,
    state: String,
    hash: String,
    args_digest: String,
    key: String,
) -> Result<Entry, String> {
    if format!("{:x}", Sha256::digest(payload.as_bytes())) != hash {
        return Err(failure());
    }
    let record: Record = serde_json::from_str(&payload).map_err(|_| failure())?;
    if record.args_digest != args_digest
        || record.binding.key()? != key
        || !["lomi_list", "lomi_read", "lomi_write", "lomi_apply_patch"]
            .contains(&record.name.as_str())
        || ![
            "prepared",
            "executing",
            "committed",
            "denied",
            "uncertain",
            "unapplied",
        ]
        .contains(&state.as_str())
        || super::effects::value_digest(&record.result)? != record.result_digest
    {
        return Err(failure());
    }
    Ok(Entry { record, state })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn future_and_corrupt_journals_are_preserved_before_schema_changes() {
        for future in [true, false] {
            let root = tempfile::tempdir().unwrap();
            crate::chat::storage::private(root.path(), true).unwrap();
            let path = root.path().join("effects.sqlite3");
            if future {
                let connection = Connection::open(&path).unwrap();
                connection.execute_batch("CREATE TABLE future_payload(value TEXT); INSERT INTO future_payload VALUES('retain'); PRAGMA user_version=999;").unwrap();
                drop(connection);
            } else {
                fs::write(&path, b"retain this damaged journal unchanged").unwrap();
            }
            crate::chat::storage::private(&path, false).unwrap();
            let before = fs::read(&path).unwrap();
            assert!(Journal::open(root.path()).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
    #[test]
    fn journal_refuses_linked_owner_file_without_touching_its_target() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        crate::chat::storage::private(root.path(), true).unwrap();
        let target = root.path().join("original");
        fs::write(&target, b"retain").unwrap();
        symlink(&target, root.path().join("effects.sqlite3")).unwrap();
        assert!(Journal::open(root.path()).is_err());
        assert_eq!(fs::read(target).unwrap(), b"retain");
    }
    #[test]
    fn future_hot_rollback_journal_is_not_recovered_by_admission() {
        let source = tempfile::tempdir().unwrap();
        let source_path = source.path().join("source.sqlite3");
        let connection = Connection::open(&source_path).unwrap();
        connection.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL;PRAGMA cache_size=1;CREATE TABLE retained(value BLOB);PRAGMA user_version=999;").unwrap();
        for _ in 0..64 {
            connection
                .execute(
                    "INSERT INTO retained(value) VALUES(?)",
                    ["before".repeat(2048)],
                )
                .unwrap();
        }
        connection
            .execute_batch("BEGIN IMMEDIATE;UPDATE retained SET value=zeroblob(12288);")
            .unwrap();
        // Copy an open spilled transaction as a crash fixture; admission must
        // neither roll it back nor remove its rollback journal.
        let root = tempfile::tempdir().unwrap();
        crate::chat::storage::private(root.path(), true).unwrap();
        let path = root.path().join("effects.sqlite3");
        let rollback = root.path().join("effects.sqlite3-journal");
        fs::copy(&source_path, &path).unwrap();
        fs::copy(source.path().join("source.sqlite3-journal"), &rollback).unwrap();
        crate::chat::storage::private(&path, false).unwrap();
        crate::chat::storage::private(&rollback, false).unwrap();
        let before = fs::read(&path).unwrap();
        let journal_before = fs::read(&rollback).unwrap();
        assert!(Journal::open(root.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&rollback).unwrap(), journal_before);
        connection.execute_batch("ROLLBACK;").unwrap();
    }
}
