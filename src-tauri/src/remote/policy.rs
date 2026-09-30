use lomi_remote_crypto::{Identity, SignedBundle, SignedPeerApproval};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const SERVICE: &str = "dev.lomi.remote.live.v1";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Policy {
    version: u8,
    pub account: String,
    pub parent: String,
    pub host_id: String,
    pub bundle: SignedBundle,
    seed: Zeroizing<Vec<u8>>,
    pub grants: Vec<LocalGrant>,
    #[serde(default)]
    pub workspaces: Vec<super::workspace::Consent>,
    #[cfg(any(feature = "remote-probe", test))]
    #[serde(skip)]
    memory_only: bool,
    #[cfg(test)]
    #[serde(skip)]
    pub fail_save: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LocalGrant {
    pub id: String,
    pub device_bundle: SignedBundle,
    pub approval: SignedPeerApproval,
    pub revoked: bool,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default)]
    pub pairing_id: Option<String>,
}

impl Policy {
    pub fn has_shared_consent(account: &str, parent: &str) -> Result<bool, String> {
        match Self::entry(account, parent)?.get_secret() {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                if bytes.len() > 128 * 1024 {
                    return Err("Remote secure policy exceeds its limit.".into());
                }
                let policy: Self = serde_json::from_slice(&bytes)
                    .map_err(|_| "Remote secure policy is invalid.")?;
                if policy.account != account || policy.parent != parent {
                    return Err("Remote secure policy account changed.".into());
                }
                Ok(policy.workspaces.iter().any(|w| w.shared))
            }
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err("Unlock secure storage before restoring Remote.".into()),
        }
    }

    fn entry(account: &str, parent: &str) -> Result<keyring::Entry, String> {
        // Both identifiers are hashes. No credential enumeration or plaintext file.
        use sha2::{Digest, Sha256};
        let subject = format!("{account}-{:x}", Sha256::digest(parent.as_bytes()));
        keyring::Entry::new(SERVICE, &subject)
            .map_err(|_| "Remote secure storage is unavailable.".into())
    }

    pub fn load_or_create(
        account: &str,
        parent: &str,
        account_id: [u8; 16],
    ) -> Result<(Self, Identity), String> {
        let entry = Self::entry(account, parent)?;
        match entry.get_secret() {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                if bytes.len() > 128 * 1024 {
                    return Err("Remote secure policy exceeds its limit.".into());
                }
                let result = serde_json::from_slice::<Self>(&bytes)
                    .map_err(|_| "Remote secure policy is invalid.");
                let policy = result?;
                if policy.version != 1
                    || policy.account != account
                    || policy.parent != parent
                    || policy.bundle.bundle.account_id != account_id
                    || policy.bundle.bundle.subject_id != super::uuid_bytes(&policy.host_id)?
                    || policy.grants.len() > 32
                    || policy.workspaces.len() > 1024
                {
                    return Err("Remote secure policy does not match this account session.".into());
                }
                let mut workspace_ids = std::collections::HashSet::new();
                let mut shared_sessions = std::collections::HashSet::new();
                if policy.workspaces.iter().filter(|w| w.shared).count() > 32 {
                    return Err("Remote secure workspace policy exceeds its limit.".into());
                }
                for w in &policy.workspaces {
                    super::uuid_bytes(&w.id)?;
                    super::uuid_bytes(&w.epoch)?;
                    if !workspace_ids.insert(&w.id)
                        || w.revision == 0
                        || w.revision > 9_007_199_254_740_991
                        || w.sessions.len() > 32
                    {
                        return Err("Remote secure workspace policy is invalid.".into());
                    }
                    for (id, epoch) in &w.sessions {
                        super::uuid_bytes(id)?;
                        super::uuid_bytes(epoch)?;
                        if w.shared && !shared_sessions.insert(id) {
                            return Err("Remote secure workspace membership is invalid.".into());
                        }
                    }
                }
                if shared_sessions.len() > 32 {
                    return Err("Remote secure workspace policy exceeds its limit.".into());
                }
                let identity = Identity::import_secret_seed_blob(&policy.seed, &policy.bundle)?;
                Ok((policy, identity))
            }
            Err(keyring::Error::NoEntry) => {
                let host_id = super::uuid()?;
                let identity = Identity::generate(
                    account_id,
                    super::uuid_bytes(&host_id)?,
                    lomi_remote_crypto::Role::Host,
                    1,
                )?;
                let policy = Self {
                    version: 1,
                    account: account.into(),
                    parent: parent.into(),
                    host_id,
                    bundle: identity.public_bundle(),
                    seed: identity.export_secret_seed_blob()?,
                    grants: vec![],
                    workspaces: vec![],
                    #[cfg(any(feature = "remote-probe", test))]
                    memory_only: false,
                    #[cfg(test)]
                    fail_save: false,
                };
                policy.save()?;
                Ok((policy, identity))
            }
            Err(_) => Err("Unlock secure storage before enabling Remote.".into()),
        }
    }

    #[cfg(any(feature = "remote-probe", test))]
    pub fn probe(
        account: &str,
        parent: &str,
        account_id: [u8; 16],
    ) -> Result<(Self, Identity), String> {
        let host_id = super::uuid()?;
        let identity = Identity::generate(
            account_id,
            super::uuid_bytes(&host_id)?,
            lomi_remote_crypto::Role::Host,
            1,
        )?;
        Ok((
            Self {
                version: 1,
                account: account.into(),
                parent: parent.into(),
                host_id,
                bundle: identity.public_bundle(),
                seed: identity.export_secret_seed_blob()?,
                grants: vec![],
                workspaces: vec![],
                memory_only: true,
                #[cfg(test)]
                fail_save: false,
            },
            identity,
        ))
    }

    pub fn save(&self) -> Result<(), String> {
        #[cfg(test)]
        if self.fail_save {
            return Err("Injected secure storage failure.".into());
        }
        #[cfg(any(feature = "remote-probe", test))]
        if self.memory_only {
            return Ok(());
        }
        let bytes = Zeroizing::new(
            serde_json::to_vec(self).map_err(|_| "Remote secure policy cannot be encoded.")?,
        );
        let result = if bytes.len() > 128 * 1024 {
            Err("Remote secure policy exceeds its limit.")
        } else {
            Self::entry(&self.account, &self.parent)?
                .set_secret(&bytes)
                .map_err(|_| "Remote secure policy could not be committed.")
        };
        Ok(result?)
    }
}
