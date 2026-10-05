use super::{store::Store, types::*};
use sha2::{Digest, Sha256};

const SERVICE: &str = "dev.lomi.desktop.cli-router";
const MAX_KEY_BYTES: usize = 16 * 1024;
const CLEANUP_ERROR: &str = "Credential cleanup is pending. Unlock the system key store and retry.";

trait Backend {
    fn put(&self, id: &str, key: &str) -> Result<(), String>;
    fn get(&self, id: &str) -> Result<String, String>;
    fn remove(&self, id: &str) -> Result<(), String>;
    fn verify(&self, id: &str) -> Result<bool, String> {
        let key = zeroize::Zeroizing::new(self.get(id)?);
        validate_key(&key)?;
        Ok(true)
    }
}
struct Native;
pub(super) fn valid(id: &str) -> Result<(), String> {
    if id
        .strip_prefix("router-key-")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err("Invalid router credential reference".into())
    }
}
fn validate_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty() || key.len() > MAX_KEY_BYTES || key.contains(['\0', '\r', '\n']) {
        return Err("Enter a valid API key within the supported size".into());
    }
    Ok(())
}
impl Backend for Native {
    fn put(&self, id: &str, key: &str) -> Result<(), String> {
        valid(id)?;
        crate::credential_store::entry(SERVICE, id)
            .and_then(|entry| entry.set_password(key))
            .map_err(|_| "The system key store is locked or unavailable".into())
    }
    fn get(&self, id: &str) -> Result<String, String> {
        valid(id)?;
        let key = crate::credential_store::get_password(SERVICE, id, MAX_KEY_BYTES)
            .map_err(|_| "The API key is unavailable in the system key store".to_string())?;
        validate_key(&key)
            .map_err(|_| "The API key in the system key store is invalid".to_string())?;
        Ok(key)
    }
    fn verify(&self, id: &str) -> Result<bool, String> {
        valid(id)?;
        match crate::credential_store::get_password(SERVICE, id, MAX_KEY_BYTES) {
            Ok(key) => {
                let key = zeroize::Zeroizing::new(key);
                validate_key(&key)
                    .map_err(|_| "The API key in the system key store is invalid".to_string())?;
                Ok(true)
            }
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err("The API key is unavailable in the system key store".into()),
        }
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        valid(id)?;
        match crate::credential_store::entry(SERVICE, id)
            .and_then(|entry| entry.delete_credential())
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(CLEANUP_ERROR.into()),
        }
    }
}

pub(crate) fn idle(snapshot: &Snapshot, id: &str) -> Result<(), String> {
    if snapshot.runs.iter().any(|run| {
        (run.active_profile_id.as_deref() == Some(id)
            || run.allowed_profile_ids.iter().any(|allowed| allowed == id))
            && matches!(
                run.state,
                RunState::Starting | RunState::Running | RunState::Switching
            )
            || run
                .attempts
                .iter()
                .any(|attempt| attempt.profile_id == id && attempt.state.is_active())
    }) {
        return Err("Stop the active run before changing this account credential".into());
    }
    Ok(())
}
fn revision(value: u64) -> Result<u64, String> {
    value
        .checked_add(1)
        .ok_or_else(|| "Account revision exhausted".into())
}
fn writable(snapshot: &Snapshot, id: &str, expected: u64) -> Result<(), String> {
    idle(snapshot, id)?;
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == id)
        .ok_or("Account profile does not exist")?;
    if profile.revision != expected {
        return Err("Account profile changed; reload before saving the key".into());
    }
    if profile.auth_state == AuthState::PendingRemove {
        return Err(CLEANUP_ERROR.into());
    }
    revision(profile.revision)?;
    Ok(())
}

pub(crate) fn put(
    store: &mut Store,
    profile_id: &str,
    expected_revision: u64,
    key: &str,
) -> Result<Snapshot, String> {
    put_with(store, profile_id, expected_revision, key, &Native)
}
fn put_with(
    store: &mut Store,
    profile_id: &str,
    expected_revision: u64,
    key: &str,
    backend: &impl Backend,
) -> Result<Snapshot, String> {
    validate_key(key)?;
    let snapshot = store.snapshot()?;
    writable(&snapshot, profile_id, expected_revision)?;
    let old = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .and_then(|profile| profile.credential_ref.clone());
    if let Some(old) = &old {
        valid(old)?;
    }
    let id = format!("router-key-{}", super::new_id()?);
    store.credential_stage(&id, profile_id)?;
    if let Err(error) = backend.put(&id, key) {
        if backend.remove(&id).is_ok() {
            store.credential_finish(&id)?;
        }
        return Err(error);
    }
    let additions = old
        .as_deref()
        .map(|old| vec![(old, profile_id, "cleanup")])
        .unwrap_or_default();
    let result = store.credential_update(&additions, &[&id], |snapshot| {
        writable(snapshot, profile_id, expected_revision)?;
        let profile = snapshot
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or("Account profile does not exist")?;
        profile.credential_ref = Some(id.clone());
        profile.quota_group_key = Some(format!(
            "{:?}:{:x}",
            profile.cli,
            Sha256::digest(key.as_bytes())
        ));
        profile.auth_state = AuthState::Ready;
        profile.storage_mode = StorageMode::ApiKey;
        profile.revision = revision(profile.revision)?;
        revoke_runs(snapshot, profile_id)?;
        snapshot
            .quota
            .retain(|quota| quota.profile_id != profile_id);
        Ok(())
    });
    if result.is_err() {
        // A failed COMMIT may leave its outcome uncertain. Delete only after a
        // fresh durable read proves no committed profile references the key.
        if store.snapshot().is_ok_and(|snapshot| {
            !snapshot
                .profiles
                .iter()
                .any(|profile| profile.credential_ref.as_deref() == Some(&id))
        }) && backend.remove(&id).is_ok()
        {
            store.credential_finish(&id)?;
        }
        return result;
    }
    if let Some(old) = old {
        backend.remove(&old)?;
        store.credential_finish(&old)?;
    }
    result
}

pub(crate) fn get(profile: &Profile) -> Result<String, String> {
    if !profile.enabled
        || profile.auth_state != AuthState::Ready
        || profile.storage_mode != StorageMode::ApiKey
        || profile.quota_group_key.as_deref().is_none_or(str::is_empty)
    {
        return Err("Account API key is not ready".into());
    }
    Native.get(
        profile
            .credential_ref
            .as_deref()
            .ok_or("Account API key is not connected")?,
    )
}

/// Confirms local key presence only, never provider authentication or capacity.
pub(crate) fn verify(profile: &Profile) -> Result<bool, String> {
    verify_with(profile, &Native)
}
fn verify_with(profile: &Profile, backend: &impl Backend) -> Result<bool, String> {
    if !profile.enabled
        || matches!(
            profile.auth_state,
            AuthState::PendingRemove | AuthState::Disabled
        )
        || profile.storage_mode != StorageMode::ApiKey
    {
        return Err("Account API key verification is unavailable".into());
    }
    if profile.credential_ref.is_some()
        && profile.quota_group_key.as_deref().is_none_or(str::is_empty)
    {
        return Err(
            "Save an API key for this reviewed destination before verifying the account.".into(),
        );
    }
    let Some(id) = profile.credential_ref.as_deref() else {
        return Ok(false);
    };
    valid(id)?;
    backend.verify(id)
}

fn revoke(snapshot: &mut Snapshot, profile_id: &str) -> Result<(), String> {
    for router in &mut snapshot.routers {
        if router.ordered_profile_ids.contains(&profile_id.to_owned()) {
            router.ordered_profile_ids.retain(|id| id != profile_id);
            router.revision = revision(router.revision)?;
        }
    }
    revoke_runs(snapshot, profile_id)
}
fn revoke_runs(snapshot: &mut Snapshot, profile_id: &str) -> Result<(), String> {
    for run in &mut snapshot.runs {
        if run.allowed_profile_ids.iter().any(|id| id == profile_id)
            || run.pinned_profile_id.as_deref() == Some(profile_id)
            || run.active_profile_id.as_deref() == Some(profile_id)
        {
            run.allowed_profile_ids.retain(|id| id != profile_id);
            if run.pinned_profile_id.as_deref() == Some(profile_id) {
                run.pinned_profile_id = None;
            }
            if run.active_profile_id.as_deref() == Some(profile_id) {
                run.active_profile_id = None;
            }
            run.revision = revision(run.revision)?;
        }
    }
    Ok(())
}
fn finish_remove(
    store: &mut Store,
    profile_id: &str,
    id: Option<&str>,
) -> Result<Snapshot, String> {
    store.credential_update(&[], &id.into_iter().collect::<Vec<_>>(), |snapshot| {
        idle(snapshot, profile_id)?;
        revoke(snapshot, profile_id)?;
        snapshot.profiles.retain(|profile| profile.id != profile_id);
        snapshot
            .quota
            .retain(|quota| quota.profile_id != profile_id);
        Ok(())
    })
}
pub(crate) fn remove(store: &mut Store, profile_id: &str) -> Result<Snapshot, String> {
    remove_with(store, profile_id, &Native)
}
fn remove_with(
    store: &mut Store,
    profile_id: &str,
    backend: &impl Backend,
) -> Result<Snapshot, String> {
    let id = begin_remove(store, profile_id)?;
    if let Some(id) = &id {
        backend.remove(id)?;
    }
    finish_remove(store, profile_id, id.as_deref())
}

fn begin_remove(store: &mut Store, profile_id: &str) -> Result<Option<String>, String> {
    let snapshot = store.snapshot()?;
    idle(&snapshot, profile_id)?;
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or("Account profile does not exist")?;
    let id = profile.credential_ref.clone();
    if let Some(id) = &id {
        valid(id)?;
    }
    let additions = id
        .as_deref()
        .map(|id| vec![(id, profile_id, "remove")])
        .unwrap_or_default();
    store.credential_update(&additions, &[], |snapshot| {
        idle(snapshot, profile_id)?;
        revoke(snapshot, profile_id)?;
        let profile = snapshot
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or("Account profile does not exist")?;
        profile.enabled = false;
        profile.auth_state = AuthState::PendingRemove;
        profile.revision = revision(profile.revision)?;
        Ok(())
    })?;
    Ok(id)
}

pub(crate) fn recover(store: &mut Store) -> Result<(), String> {
    recover_with(store, &Native)
}
fn recover_with(store: &mut Store, backend: &impl Backend) -> Result<(), String> {
    let mut failed = false;
    // Removal intent can be committed before its native cleanup call starts.
    // Repair that crash gap durably before touching any referenced secret.
    let snapshot = store.snapshot()?;
    let journal = store.credential_journal()?;
    for profile in snapshot
        .profiles
        .iter()
        .filter(|profile| profile.auth_state == AuthState::PendingRemove)
    {
        if let Some(id) = profile.credential_ref.as_deref() {
            if !journal.iter().any(|(saved_id, saved_profile, status)| {
                saved_id == id && saved_profile == &profile.id && status == "remove"
            }) {
                begin_remove(store, &profile.id)?;
            }
        }
    }
    for (id, profile_id, status) in store.credential_journal()? {
        valid(&id)?;
        let snapshot = store.snapshot()?;
        if status != "remove"
            && snapshot
                .profiles
                .iter()
                .any(|profile| profile.credential_ref.as_deref() == Some(&id))
        {
            // A committed pointer is authoritative; never delete its credential.
            store.credential_finish(&id)?;
            continue;
        }
        if backend.remove(&id).is_err() {
            failed = true;
            continue;
        }
        if status == "remove" {
            finish_remove(store, &profile_id, Some(&id))?;
        } else {
            store.credential_finish(&id)?;
        }
    }
    // Removal of a profile without a credential may have stopped after revocation.
    let snapshot = store.snapshot()?;
    for profile in snapshot.profiles.iter().filter(|profile| {
        profile.auth_state == AuthState::PendingRemove && profile.credential_ref.is_none()
    }) {
        finish_remove(store, &profile.id, None)?;
    }
    if failed {
        Err(CLEANUP_ERROR.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_catalog::TitleCli;
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
    };

    #[derive(Default)]
    struct Fake {
        keys: RefCell<HashMap<String, String>>,
        fail_put: Cell<bool>,
        fail_remove: Cell<bool>,
        fail_get: Cell<bool>,
        get_calls: Cell<usize>,
        verify_calls: Cell<usize>,
    }
    impl Backend for Fake {
        fn put(&self, id: &str, key: &str) -> Result<(), String> {
            self.keys.borrow_mut().insert(id.into(), key.into());
            if self.fail_put.get() {
                Err("fixture write failed".into())
            } else {
                Ok(())
            }
        }
        fn get(&self, id: &str) -> Result<String, String> {
            self.get_calls.set(self.get_calls.get() + 1);
            if self.fail_get.get() {
                return Err("fixture keyring unavailable".into());
            }
            self.keys
                .borrow()
                .get(id)
                .cloned()
                .ok_or("fixture missing".into())
        }
        fn verify(&self, id: &str) -> Result<bool, String> {
            self.verify_calls.set(self.verify_calls.get() + 1);
            if self.fail_get.get() {
                return Err("fixture keyring unavailable".into());
            }
            let keys = self.keys.borrow();
            let Some(key) = keys.get(id) else {
                return Ok(false);
            };
            validate_key(key)?;
            Ok(true)
        }
        fn remove(&self, id: &str) -> Result<(), String> {
            if self.fail_remove.get() {
                return Err(CLEANUP_ERROR.into());
            }
            self.keys.borrow_mut().remove(id);
            Ok(())
        }
    }
    fn fixture() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(&directory.path().join("router.sqlite")).unwrap();
        store
            .update(|snapshot| {
                snapshot.profiles.push(Profile {
                    id: "profile".into(),
                    cli: TitleCli::Codex,
                    label: "Account".into(),
                    enabled: true,
                    revision: 1,
                    auth_state: AuthState::Disconnected,
                    storage_mode: StorageMode::ApiKey,
                    credential_ref: None,
                    quota_group_key: None,
                    gateway_provider: None,
                });
                Ok(())
            })
            .unwrap();
        (directory, store)
    }
    #[test]
    fn retired_profile_migration_preserves_cleanup_obligations_until_key_store_recovers() {
        for status in ["staged", "cleanup", "remove", "metadata_only"] {
            let (directory, mut store) = fixture();
            let backend = Fake::default();
            let id = format!("router-key-{}", super::super::new_id().unwrap());
            if status != "metadata_only" {
                store.credential_stage(&id, "profile").unwrap();
            }
            backend.put(&id, "pending-key").unwrap();
            let mut original = serde_json::to_value(store.snapshot().unwrap()).unwrap();
            original["profiles"][0]["cli"] = serde_json::json!("goose");
            if status == "metadata_only" {
                original["profiles"][0]["authState"] = serde_json::json!("pending_remove");
                original["profiles"][0]["credentialRef"] = serde_json::json!(id);
            }
            drop(store);
            let path = directory.path().join("router.sqlite");
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute("UPDATE snapshot SET data = ?1", [original.to_string()])
                .unwrap();
            if status != "metadata_only" {
                connection
                    .execute("UPDATE credential_journal SET status = ?1", [status])
                    .unwrap();
            }
            drop(connection);
            let mut store = Store::open(&path).unwrap();
            assert!(store.snapshot().unwrap().profiles.is_empty());
            assert_eq!(store.credential_journal().unwrap().len(), 1);
            backend.fail_remove.set(true);
            assert!(recover_with(&mut store, &backend).is_err());
            assert_eq!(store.credential_journal().unwrap().len(), 1);
            assert_eq!(backend.keys.borrow().len(), 1);
            backend.fail_remove.set(false);
            recover_with(&mut store, &backend).unwrap();
            assert!(store.credential_journal().unwrap().is_empty());
            assert!(backend.keys.borrow().is_empty());
        }
    }
    #[test]
    fn immutable_rotation_and_revision_checks_preserve_current_key() {
        let (_directory, mut store) = fixture();
        let backend = Fake::default();
        let first = put_with(&mut store, "profile", 1, "first-key", &backend).unwrap();
        let first_id = first.profiles[0].credential_ref.clone().unwrap();
        assert!(put_with(&mut store, "profile", 1, "stale-key", &backend).is_err());
        assert_eq!(backend.keys.borrow().len(), 1);
        let second = put_with(&mut store, "profile", 2, "second-key", &backend).unwrap();
        let second_id = second.profiles[0].credential_ref.as_ref().unwrap();
        assert_ne!(&first_id, second_id);
        assert_eq!(backend.get(second_id).unwrap(), "second-key");
        assert!(!backend.keys.borrow().contains_key(&first_id));
        assert!(store.credential_journal().unwrap().is_empty());
        let data = std::fs::read(_directory.path().join("router.sqlite")).unwrap();
        assert!(!String::from_utf8_lossy(&data).contains("first-key"));
    }
    #[test]
    fn failed_write_is_journaled_and_recovers_after_restart() {
        let (directory, mut store) = fixture();
        let backend = Fake::default();
        backend.fail_put.set(true);
        backend.fail_remove.set(true);
        assert!(put_with(&mut store, "profile", 1, "failed-key", &backend).is_err());
        assert!(store.snapshot().unwrap().profiles[0]
            .credential_ref
            .is_none());
        assert_eq!(store.credential_journal().unwrap().len(), 1);
        drop(store);
        let mut store = Store::open(&directory.path().join("router.sqlite")).unwrap();
        assert!(recover_with(&mut store, &backend).is_err());
        backend.fail_remove.set(false);
        recover_with(&mut store, &backend).unwrap();
        assert!(backend.keys.borrow().is_empty());
        assert!(store.credential_journal().unwrap().is_empty());
    }
    #[test]
    fn committed_rotation_retries_old_cleanup_without_deleting_new_key() {
        let (directory, mut store) = fixture();
        let backend = Fake::default();
        put_with(&mut store, "profile", 1, "old-key", &backend).unwrap();
        backend.fail_remove.set(true);
        assert!(put_with(&mut store, "profile", 2, "new-key", &backend).is_err());
        let id = store.snapshot().unwrap().profiles[0]
            .credential_ref
            .clone()
            .unwrap();
        assert_eq!(backend.get(&id).unwrap(), "new-key");
        assert_eq!(backend.keys.borrow().len(), 2);
        drop(store);
        let mut store = Store::open(&directory.path().join("router.sqlite")).unwrap();
        backend.fail_remove.set(false);
        recover_with(&mut store, &backend).unwrap();
        assert_eq!(backend.get(&id).unwrap(), "new-key");
        assert_eq!(backend.keys.borrow().len(), 1);
    }
    #[test]
    fn metadata_commit_failure_cleans_new_key_and_keeps_old_pointer() {
        let (_directory, mut store) = fixture();
        let backend = Fake::default();
        put_with(&mut store, "profile", 1, "current-key", &backend).unwrap();
        let before = store.snapshot().unwrap();
        store.reject_credential_test_commit();
        assert!(put_with(&mut store, "profile", 2, "rejected-key", &backend).is_err());
        assert_eq!(store.snapshot().unwrap(), before);
        assert_eq!(backend.keys.borrow().len(), 1);
        assert_eq!(
            backend
                .get(before.profiles[0].credential_ref.as_ref().unwrap())
                .unwrap(),
            "current-key"
        );
        assert!(store.credential_journal().unwrap().is_empty());
    }

    #[test]
    fn staged_precommit_crash_discards_only_unreferenced_key() {
        let (_directory, mut store) = fixture();
        let backend = Fake::default();
        put_with(&mut store, "profile", 1, "current-key", &backend).unwrap();
        let orphan = format!("router-key-{}", crate::cli_router::new_id().unwrap());
        store.credential_stage(&orphan, "profile").unwrap();
        backend.put(&orphan, "orphan-key").unwrap();
        recover_with(&mut store, &backend).unwrap();
        assert!(!backend.keys.borrow().contains_key(&orphan));
        assert_eq!(backend.keys.borrow().len(), 1);
    }
    #[test]
    fn rotation_revokes_historical_run_grants_and_preserves_attempt_history() {
        let (_directory, mut store) = fixture();
        let backend = Fake::default();
        put_with(&mut store, "profile", 1, "old-key", &backend).unwrap();
        store
            .update(|snapshot| {
                snapshot.runs.push(Run {
                    id: "history".into(),
                    router_id: "old-router".into(),
                    cwd: "/fixture".into(),
                    shell_profile_id: None,
                    title: "History".into(),
                    state: RunState::Running,
                    model: None,
                    reasoning_effort: None,
                    execution_mode: RunExecutionMode::Text,
                    continuation_requested: false,
                    pinned_profile_id: Some("profile".into()),
                    allowed_profile_ids: vec!["profile".into()],
                    active_profile_id: Some("profile".into()),
                    generation: 1,
                    revision: 1,
                    inputs: vec![RunInput {
                        id: "input".into(),
                        text: "hello".into(),
                    }],
                    attempts: vec![RunAttempt {
                        id: "attempt".into(),
                        input_id: "input".into(),
                        profile_id: "profile".into(),
                        generation: 1,
                        state: AttemptState::Completed,
                        reason: "complete".into(),
                    }],
                    output: "history".into(),
                    turns: vec![],
                    legacy_output: None,
                    status_message: String::new(),
                    attempted_profile_ids: vec!["profile".into()],
                });
                Ok(())
            })
            .unwrap();
        assert!(put_with(&mut store, "profile", 2, "active-key", &backend).is_err());
        assert_eq!(backend.keys.borrow().len(), 1);
        assert!(store.credential_journal().unwrap().is_empty());
        store
            .update(|snapshot| {
                snapshot.runs[0].state = RunState::Completed;
                Ok(())
            })
            .unwrap();
        let snapshot = put_with(&mut store, "profile", 2, "new-key", &backend).unwrap();
        let run = &snapshot.runs[0];
        assert!(run.allowed_profile_ids.is_empty());
        assert!(run.pinned_profile_id.is_none());
        assert!(run.active_profile_id.is_none());
        assert_eq!(run.attempts[0].profile_id, "profile");
        assert_eq!(run.output, "history");
    }

    #[test]
    fn removal_revokes_immediately_and_finishes_on_restart() {
        let (directory, mut store) = fixture();
        let backend = Fake::default();
        put_with(&mut store, "profile", 1, "remove-key", &backend).unwrap();
        store
            .update(|snapshot| {
                snapshot.routers.push(Router {
                    id: "router".into(),
                    cli: TitleCli::Codex,
                    label: "Router".into(),
                    enabled: true,
                    ordered_profile_ids: vec!["profile".into()],
                    balance_remaining_quota: false,
                    revision: 1,
                });
                Ok(())
            })
            .unwrap();
        backend.fail_remove.set(true);
        assert!(remove_with(&mut store, "profile", &backend).is_err());
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.profiles[0].auth_state, AuthState::PendingRemove);
        assert!(!snapshot.profiles[0].enabled);
        assert!(snapshot.routers[0].ordered_profile_ids.is_empty());
        drop(store);
        let mut store = Store::open(&directory.path().join("router.sqlite")).unwrap();
        backend.fail_remove.set(false);
        recover_with(&mut store, &backend).unwrap();
        assert!(store.snapshot().unwrap().profiles.is_empty());
        assert!(backend.keys.borrow().is_empty());
    }
    #[test]
    fn metadata_only_removal_intent_is_journaled_and_cleaned_after_restart() {
        let (directory, mut store) = fixture();
        let backend = Fake::default();
        put_with(&mut store, "profile", 1, "pending-key", &backend).unwrap();
        store
            .update(|snapshot| {
                snapshot.profiles[0].enabled = false;
                snapshot.profiles[0].auth_state = AuthState::PendingRemove;
                snapshot.profiles[0].revision += 1;
                Ok(())
            })
            .unwrap();
        assert!(store.credential_journal().unwrap().is_empty());
        drop(store);
        let mut store = Store::open(&directory.path().join("router.sqlite")).unwrap();
        backend.fail_remove.set(true);
        assert!(recover_with(&mut store, &backend).is_err());
        assert_eq!(store.credential_journal().unwrap()[0].2, "remove");
        assert_eq!(
            store.snapshot().unwrap().profiles[0].auth_state,
            AuthState::PendingRemove
        );
        assert_eq!(backend.keys.borrow().len(), 1);
        drop(store);
        let mut store = Store::open(&directory.path().join("router.sqlite")).unwrap();
        backend.fail_remove.set(false);
        recover_with(&mut store, &backend).unwrap();
        assert!(backend.keys.borrow().is_empty());
        assert!(store.credential_journal().unwrap().is_empty());
        assert!(store.snapshot().unwrap().profiles.is_empty());
    }
    #[test]
    fn failed_delete_rejects_key_replacement_until_removal_recovers() {
        let (_directory, mut store) = fixture();
        let backend = Fake::default();
        let initial = put_with(&mut store, "profile", 1, "original-key", &backend).unwrap();
        let id = initial.profiles[0].credential_ref.clone().unwrap();
        backend.fail_remove.set(true);
        assert!(remove_with(&mut store, "profile", &backend).is_err());
        let pending = store.snapshot().unwrap();
        assert!(put_with(
            &mut store,
            "profile",
            pending.profiles[0].revision,
            "replacement-key",
            &backend
        )
        .is_err());
        assert_eq!(store.snapshot().unwrap(), pending);
        assert_eq!(backend.keys.borrow().len(), 1);
        assert_eq!(backend.get(&id).unwrap(), "original-key");
        backend.fail_remove.set(false);
        recover_with(&mut store, &backend).unwrap();
        assert!(store.snapshot().unwrap().profiles.is_empty());
        assert!(backend.keys.borrow().is_empty());
        assert!(store.credential_journal().unwrap().is_empty());
    }
    #[test]
    fn api_key_presence_verification_repairs_reauth_without_key_reentry() {
        let (_directory, mut store) = fixture();
        let backend = Fake::default();
        let snapshot = put_with(&mut store, "profile", 1, "present-key", &backend).unwrap();
        let mut profile = snapshot.profiles[0].clone();
        for state in [AuthState::ReauthRequired, AuthState::Disconnected] {
            profile.auth_state = state;
            assert!(verify_with(&profile, &backend).unwrap());
        }
        backend.fail_get.set(true);
        assert!(verify_with(&profile, &backend).is_err());
        assert_eq!(profile.auth_state, AuthState::Disconnected);
        backend.fail_get.set(false);
        profile.auth_state = AuthState::PendingRemove;
        assert!(verify_with(&profile, &backend).is_err());
        profile.auth_state = AuthState::Ready;
        profile.enabled = false;
        assert!(verify_with(&profile, &backend).is_err());
        profile.enabled = true;
        profile.storage_mode = StorageMode::CliManaged;
        assert!(verify_with(&profile, &backend).is_err());
        assert_eq!(backend.keys.borrow().len(), 1);
        profile.storage_mode = StorageMode::ApiKey;
        backend.keys.borrow_mut().clear();
        assert!(!verify_with(&profile, &backend).unwrap());
        profile.credential_ref = None;
        assert!(!verify_with(&profile, &backend).unwrap());
    }
    #[test]
    fn retained_key_without_destination_binding_is_refused_before_backend_access() {
        let (_directory, mut store) = fixture();
        let backend = Fake::default();
        let snapshot =
            put_with(&mut store, "profile", 1, "retained-fixture-key", &backend).unwrap();
        let mut profile = snapshot.profiles[0].clone();
        let reference = profile.credential_ref.clone().unwrap();
        profile.auth_state = AuthState::Ready;
        profile.quota_group_key = None;
        profile.gateway_provider = Some(ApiDestination {
            protocol: super::super::gateway_profiles::Protocol::OpenAiResponses,
            base_url: "https://reviewed.example/v1".into(),
        });
        backend.get_calls.set(0);
        backend.verify_calls.set(0);
        assert!(verify_with(&profile, &backend).is_err());
        assert_eq!(backend.get_calls.get(), 0);
        assert_eq!(backend.verify_calls.get(), 0);
        assert_eq!(
            backend.keys.borrow().get(&reference).unwrap(),
            "retained-fixture-key"
        );
    }
}
