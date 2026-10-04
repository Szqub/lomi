//! Persistent native-client identity and generation binding; contains no secrets.
use super::{gateway_profiles::Protocol, gateway_store::Continuity, types::Run};
use crate::cli_catalog::TitleCli;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Session {
    schema: u32,
    cli: TitleCli,
    version: String,
    model: String,
    cwd: String,
    protocol: Protocol,
    pub(crate) generation: u64,
    pub(crate) continuity: Option<Continuity>,
}
impl Session {
    pub(crate) fn new(
        run: &Run,
        cli: TitleCli,
        version: &str,
        protocol: Protocol,
        continuity: Option<Continuity>,
    ) -> Result<Self, String> {
        Ok(Self {
            schema: 1,
            cli,
            version: version.into(),
            model: run.model.clone().ok_or("Missing native model.")?,
            cwd: run.cwd.clone(),
            protocol,
            generation: run.generation,
            continuity,
        })
    }
    pub(crate) fn journal_root(&self, root: &Path) -> PathBuf {
        if self.generation == 1 {
            root.into()
        } else {
            root.join("generations").join(self.generation.to_string())
        }
    }
    pub(crate) fn read(
        root: &Path,
        run: &Run,
        cli: TitleCli,
        version: &str,
        protocol: Protocol,
    ) -> Result<Self, String> {
        super::gateway_profiles::check_private_directory(root)?;
        let path = root.join("native-session.json");
        let mut file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path).map_err(|_| "This saved session has no qualified native continuation record. Its history is retained.")?;
        let meta = file
            .metadata()
            .map_err(|_| "Cannot inspect native continuation record.")?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.nlink() != 1
            || meta.mode() & 0o777 != 0o600
            || meta.len() > 16 * 1024
        {
            return Err("Native continuation record is not privately owned.".into());
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read native continuation record.")?;
        if bytes.len() > 16 * 1024 {
            return Err("Native continuation record exceeded its limit.".into());
        }
        let session: Self =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid native continuation record.")?;
        if session.schema != 1
            || session.cli != cli
            || session.version != version
            || session.model != run.model.as_deref().unwrap_or("")
            || session.cwd != run.cwd
            || session.protocol != protocol
            || session.generation == 0
            || session.generation > run.generation
        {
            return Err("The native client, model or saved session binding changed. Existing history has been preserved.".into());
        }
        Ok(session)
    }
    pub(crate) fn write(&self, root: &Path) -> Result<(), String> {
        super::gateway_profiles::check_private_directory(root)?;
        let path = root.join("native-session.json");
        crate::chat::storage::reject_link(&path)?;
        let bytes =
            serde_json::to_vec(self).map_err(|_| "Cannot encode native continuation record.")?;
        crate::chat::storage::atomic(&path, &bytes)
    }
}
