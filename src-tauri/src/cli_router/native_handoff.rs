//! Durable native turn boundaries. No prompt is replayed from an uncertain turn.
use crate::cli_catalog::TitleCli;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

const MAX_RECORD: usize = 1024 * 1024;
const MAX_CALLS: usize = 4096;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Ledger {
    calls: BTreeMap<String, bool>,
    approvals: BTreeSet<String>,
    pub(crate) sequence: u64,
    pub(crate) terminal: bool,
    pub(crate) drained: bool,
    pub(crate) complete_observation: bool,
}
impl Ledger {
    pub(crate) fn new() -> Self {
        Self {
            complete_observation: true,
            ..Self::default()
        }
    }
    pub(crate) fn observe(&mut self) -> Result<(), String> {
        if self.terminal {
            self.complete_observation = false;
            return Err("Native output continued after the terminal turn boundary.".into());
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Native event sequence exhausted.")?;
        Ok(())
    }
    pub(crate) fn tool(&mut self, id: &str, finished: bool) -> Result<(), String> {
        if id.is_empty()
            || id.len() > 512
            || (self.calls.len() >= MAX_CALLS && !self.calls.contains_key(id))
        {
            self.complete_observation = false;
            return Err("Native tool identity or ledger exceeded its bounds.".into());
        }
        if self.calls.get(id) == Some(&true) && !finished {
            self.complete_observation = false;
            return Err("A completed native tool was restarted with the same identity.".into());
        }
        self.calls.insert(id.into(), finished);
        Ok(())
    }
    pub(crate) fn approval(&mut self, id: &str, pending: bool) -> Result<(), String> {
        if id.is_empty() || id.len() > 512 || self.approvals.len() >= MAX_CALLS {
            self.complete_observation = false;
            return Err("Native approval ledger exceeded its bounds.".into());
        }
        if pending {
            self.approvals.insert(id.into());
        } else {
            self.approvals.remove(id);
        }
        Ok(())
    }
    pub(crate) fn has_tools(&self) -> bool {
        !self.calls.is_empty()
    }
    pub(crate) fn quiescent(&self) -> bool {
        self.complete_observation
            && self.terminal
            && self.drained
            && self.approvals.is_empty()
            && self.calls.values().all(|done| *done)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Checkpoint {
    schema: u32,
    pub(crate) cli: TitleCli,
    pub(crate) version: String,
    pub(crate) run_id: String,
    pub(crate) input_id: String,
    pub(crate) session_id: String,
    pub(crate) profile_id: String,
    pub(crate) profile_revision: u64,
    pub(crate) generation: u64,
    pub(crate) cwd: String,
    pub(crate) model: String,
    pub(crate) ledger: Ledger,
    pub(crate) completed: bool,
    pub(crate) quota_rejected: bool,
    pub(crate) history_digest: String,
}
impl Checkpoint {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cli: TitleCli,
        version: &str,
        run: &super::types::Run,
        input_id: &str,
        session_id: &str,
        profile: &super::types::Profile,
        ledger: Ledger,
        completed: bool,
        quota_rejected: bool,
        history: &[u8],
    ) -> Result<Self, String> {
        let checkpoint = Self {
            schema: 1,
            cli,
            version: version.into(),
            run_id: run.id.clone(),
            input_id: input_id.into(),
            session_id: session_id.into(),
            profile_id: profile.id.clone(),
            profile_revision: profile.revision,
            generation: run.generation,
            cwd: run.cwd.clone(),
            model: run.model.clone().ok_or("Missing native model.")?,
            ledger,
            completed,
            quota_rejected,
            history_digest: format!("{:x}", Sha256::digest(history)),
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }
    fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.generation == 0
            || self.profile_revision == 0
            || self.session_id.is_empty()
            || self.session_id.len() > 4096
            || self.version.is_empty()
            || self.version.len() > 256
            || self.model.is_empty()
            || self.model.len() > 256
            || self.cwd.len() > 32768
            || self.ledger.calls.len() > MAX_CALLS
            || self.ledger.approvals.len() > MAX_CALLS
            || self.history_digest.len() != 64
            || !self
                .history_digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !self.ledger.quiescent()
            || self.completed && self.quota_rejected
        {
            return Err("The native turn has no complete, drained continuation checkpoint.".into());
        }
        Ok(())
    }
    pub(crate) fn matches(&self, run: &super::types::Run, cli: TitleCli, version: &str) -> bool {
        self.validate().is_ok()
            && self.run_id == run.id
            && self.cli == cli
            && self.version == version
            && self.cwd == run.cwd
            && run.model.as_deref() == Some(self.model.as_str())
            && self.generation <= run.generation
            && run.allowed_profile_ids.contains(&self.profile_id)
    }
    pub(crate) fn write(&self, root: &Path) -> Result<(), String> {
        self.validate()?;
        super::gateway_profiles::check_private_directory(root)?;
        let bytes = serde_json::to_vec(self).map_err(|_| "Cannot encode native checkpoint.")?;
        if bytes.len() > MAX_RECORD {
            return Err("Native checkpoint exceeds its bounds.".into());
        }
        crate::chat::storage::atomic(&root.join("native-checkpoint.json"), &bytes)
    }
    pub(crate) fn read(root: &Path) -> Result<Option<Self>, String> {
        super::gateway_profiles::check_private_directory(root)?;
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("native-checkpoint.json"))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Cannot open native checkpoint.".into()),
        };
        let meta = file
            .metadata()
            .map_err(|_| "Cannot inspect native checkpoint.")?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.nlink() != 1
            || meta.mode() & 0o777 != 0o600
            || meta.len() > MAX_RECORD as u64
        {
            return Err("Native checkpoint is not privately owned.".into());
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_RECORD as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read native checkpoint.")?;
        if bytes.len() > MAX_RECORD {
            return Err("Native checkpoint exceeds its bounds.".into());
        }
        let record: Self =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid native checkpoint.")?;
        record.validate()?;
        // Refuse noncanonical records, including duplicate JSON object members.
        if serde_json::to_vec(&record).map_err(|_| "Invalid native checkpoint.")? != bytes {
            return Err("Native checkpoint encoding is not canonical.".into());
        }
        Ok(Some(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_result_does_not_erase_running_tools_or_permissions() {
        let mut ledger = Ledger::new();
        ledger.tool("call", false).unwrap();
        ledger.approval("permission", true).unwrap();
        ledger.terminal = true;
        ledger.drained = true;
        assert!(!ledger.quiescent());
        ledger.tool("call", true).unwrap();
        assert!(!ledger.quiescent());
        ledger.approval("permission", false).unwrap();
        assert!(ledger.quiescent());
        assert!(ledger.tool("call", false).is_err());
        assert!(!ledger.quiescent());
    }
    #[test]
    fn transport_loss_fences_even_an_empty_turn() {
        let mut ledger = Ledger::new();
        ledger.terminal = true;
        ledger.drained = true;
        assert!(ledger.quiescent());
        ledger.complete_observation = false;
        assert!(!ledger.quiescent());
    }
}
