//! Native Remote authority. Cloud records cannot create local grants.
mod channel;
mod policy;
mod runtime;
pub mod workspace;
use workspace::{Domain, WorkspaceInfo};

use crate::{auth::AuthController, terminal::Terminals};
use lomi_remote_crypto::{Identity, PeerApproval, Permissions, SignedBundle, SignedPeerApproval};
use policy::{LocalGrant, Policy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State, Window};

pub(crate) const LIVE_QUALIFIED: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteState {
    pub qualified: bool,
    pub enabled: bool,
    pub online: bool,
    pub host_id: Option<String>,
    pub fingerprint: Option<String>,
    pub message: Option<String>,
    pub sessions: Vec<SessionInfo>,
    pub pairings: Vec<Pairing>,
    pub grants: Vec<GrantInfo>,
    pub workspaces: Vec<WorkspaceInfo>,
    pub domain_epoch: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub epoch: String,
    pub label: String,
    pub cols: u16,
    pub rows: u16,
    pub shared: bool,
    pub available: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantInfo {
    pub id: String,
    pub fingerprint: String,
    pub session_ids: Vec<String>,
    pub permissions: Permissions,
    pub expires_at: u64,
    pub revoked: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub id: String,
    pub host_id: String,
    pub device_id: String,
    pub host_bundle: SignedBundle,
    pub device_bundle: SignedBundle,
    pub host_fingerprint: String,
    pub device_fingerprint: String,
    pub nonce: String,
    pub pairing_fingerprint: String,
    pub status: String,
    #[serde(deserialize_with = "deserialize_expiry")]
    pub expires_at: u64,
    #[serde(deserialize_with = "deserialize_expiry")]
    pub max_approval_expires_at: u64,
    #[serde(default)]
    pub signed_approval: Option<SignedPeerApproval>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_epoch: Option<String>,
    #[serde(default)]
    pub workspace_revision: Option<u64>,
}

struct Core {
    enabled: bool,
    binding: Option<crate::auth::controller::RemoteBinding>,
    identity: Option<Identity>,
    policy: Option<Policy>,
    deadline: Option<Instant>,
    message: Option<String>,
    pairings: Vec<Pairing>,
    shares: HashSet<String>,
    legacy_shares: HashSet<String>,
    channels: HashMap<String, Arc<AtomicBool>>,
    channel_workspaces: HashMap<String, String>,
    channel_grants: HashMap<String, String>,
    attempted_channels: HashMap<String, u64>,
    policy_revision: u64,
    domain: Domain,
    restore_binding: Option<crate::auth::controller::RemoteBinding>,
}

impl Core {
    fn reset_bound_authority(&mut self) {
        self.enabled = false;
        self.deadline = None;
        self.binding = None;
        self.policy = None;
        self.identity = None;
        self.restore_binding = None;
        self.shares.clear();
        self.legacy_shares.clear();
        self.pairings.clear();
        for stop in self.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        self.channels.clear();
        self.channel_workspaces.clear();
        self.channel_grants.clear();
        self.message = Some("Remote account authorization changed.".into());
    }
    fn storage_failed(&mut self) {
        self.enabled = false;
        self.deadline = None;
        self.message = Some("Remote secure policy could not be committed.".into());
        for stop in self.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        self.policy.take();
        self.identity.take();
    }
    fn reconcile_cloud_grants(
        &mut self,
        grants: &[Value],
        pending_pairings: &HashSet<String>,
    ) -> Result<(), String> {
        let policy = self.policy.as_mut().ok_or("Remote identity unavailable.")?;
        let mut changed = false;
        let mut retired = HashSet::new();
        for local in &mut policy.grants {
            let uncertain_pending = local.approval.approval.version == 2
                && local
                    .pairing_id
                    .as_ref()
                    .is_some_and(|id| pending_pairings.contains(id));
            let active = grants.iter().any(|g| {
                g.get("id").and_then(Value::as_str) == Some(local.id.as_str())
                    && g.get("revokedAt").is_none_or(Value::is_null)
            });
            if !local.revoked
                && !active
                && (local.confirmed || (local.approval.approval.version == 2 && !uncertain_pending))
            {
                local.revoked = true;
                retired.insert(local.id.clone());
                changed = true;
            }
        }
        for (id, grant) in &self.channel_grants {
            if retired.contains(grant) {
                if let Some(stop) = self.channels.get(id) {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        }
        if changed {
            self.commit_policy()?;
        }
        Ok(())
    }
    fn prune_inactive_grants(&mut self) -> Result<(), String> {
        let policy = self.policy.as_mut().ok_or("Remote identity unavailable.")?;
        let removed: HashSet<String> = policy
            .grants
            .iter()
            .filter(|g| g.revoked || g.approval.approval.expires_at <= now())
            .map(|g| g.id.clone())
            .collect();
        if removed.is_empty() {
            return Ok(());
        }
        for (id, grant) in &self.channel_grants {
            if removed.contains(grant) {
                if let Some(stop) = self.channels.get(id) {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        }
        policy.grants.retain(|g| !removed.contains(&g.id));
        self.commit_policy()
    }
    fn commit_policy(&mut self) -> Result<(), String> {
        if let Err(error) = self
            .policy
            .as_ref()
            .ok_or("Remote identity unavailable.")?
            .save()
        {
            self.storage_failed();
            return Err(error);
        }
        self.policy_revision = self.policy_revision.wrapping_add(1);
        Ok(())
    }
}

impl Default for Core {
    fn default() -> Self {
        Self {
            enabled: false,
            binding: None,
            identity: None,
            policy: None,
            deadline: None,
            message: (!LIVE_QUALIFIED)
                .then(|| "Remote live access is awaiting qualification.".into()),
            pairings: vec![],
            shares: HashSet::new(),
            legacy_shares: HashSet::new(),
            channels: HashMap::new(),
            channel_workspaces: HashMap::new(),
            channel_grants: HashMap::new(),
            attempted_channels: HashMap::new(),
            policy_revision: 0,
            domain: Domain::default(),
            restore_binding: None,
        }
    }
}

#[derive(Clone)]
pub struct Remote {
    core: Arc<Mutex<Core>>,
    runtime: Arc<Mutex<runtime::Runtime>>,
    stopped: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    events: tokio::sync::broadcast::Sender<Value>,
}

impl Default for Remote {
    fn default() -> Self {
        let (events, _) = tokio::sync::broadcast::channel(256);
        Self {
            core: Arc::new(Mutex::new(Core::default())),
            runtime: Arc::new(Mutex::new(runtime::Runtime::default())),
            stopped: Arc::new(AtomicBool::new(false)),
            healthy: Arc::new(AtomicBool::new(false)),
            events,
        }
    }
}

impl Remote {
    pub fn initialize(&self, app: tauri::AppHandle) {
        let terminals = app.state::<Terminals>().inner().clone();
        let Ok((receiver, healthy)) = terminals.observe_remote() else {
            return;
        };
        self.healthy.store(true, Ordering::SeqCst);
        let controller = self.clone();
        let observer_app = app.clone();
        std::thread::spawn(move || {
            while !controller.stopped.load(Ordering::SeqCst) {
                if !healthy.load(Ordering::SeqCst) {
                    controller.fail_closed("Remote terminal observer exceeded its budget.");
                    controller.healthy.store(false, Ordering::SeqCst);
                    break;
                }
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => {
                        let result = controller
                            .runtime
                            .lock()
                            .map_err(|_| "Terminal model unavailable.".into())
                            .and_then(|mut r| r.observe(&observer_app, event));
                        let _ = controller.reconcile_workspaces(&observer_app);
                        match result {
                            Ok(Some(delta)) => {
                                if delta.get("type").and_then(Value::as_str) == Some("unavailable")
                                {
                                    if let Some(id) = delta.get("sessionId").and_then(Value::as_str)
                                    {
                                        if let Ok(mut c) = controller.core.lock() {
                                            c.shares.remove(id);
                                            c.legacy_shares.remove(id);
                                        }
                                        observer_app.state::<Terminals>().remote_revoke(id);
                                    }
                                }
                                let _ = controller.events.send(delta);
                            }
                            Ok(None) => {}
                            Err(_) => {
                                controller.healthy.store(false, Ordering::SeqCst);
                                controller
                                    .fail_closed("Independent terminal model is unavailable.");
                                break;
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
        });
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            while !controller.stopped.load(Ordering::SeqCst) {
                interval.tick().await;
                controller.reset_changed_binding(&app);
                if LIVE_QUALIFIED && !controller.core.lock().map(|c| c.enabled).unwrap_or(false) {
                    if let Ok(binding) = app.state::<AuthController>().remote_binding() {
                        let unchecked = controller
                            .core
                            .lock()
                            .map(|c| c.restore_binding.as_ref() != Some(&binding))
                            .unwrap_or(false);
                        if unchecked {
                            let account = hex(&Sha256::digest(
                                format!("lomi-remote-account-v1:{}", binding.user_id).as_bytes(),
                            )[..16]);
                            if let Ok(shared) =
                                Policy::has_shared_consent(&account, &binding.session_id)
                            {
                                if shared && controller.enable_inner(&app, false).await.is_err() {
                                    controller.fail_closed(
                                        "Remote workspace consent could not be restored.",
                                    );
                                } else if let Ok(mut c) = controller.core.lock() {
                                    c.restore_binding = Some(binding);
                                }
                            }
                        }
                    }
                }
                if controller.core.lock().map(|c| c.enabled).unwrap_or(false)
                    && controller.poll(&app).await.is_err()
                {
                    controller.fail_closed("Remote authorization could not be confirmed.");
                }
            }
        });
    }

    fn reset_changed_binding(&self, app: &tauri::AppHandle) {
        let current = app.state::<AuthController>().remote_binding().ok();
        if let Ok(mut core) = self.core.lock() {
            if core.binding.is_some() && core.binding != current {
                core.reset_bound_authority();
            }
        }
    }
    fn fail_closed(&self, message: &str) {
        if let Ok(mut core) = self.core.lock() {
            core.deadline = None;
            core.message = Some(message.into());
            for stop in core.channels.values() {
                stop.store(true, Ordering::SeqCst);
            }
            core.channels.clear();
            core.channel_workspaces.clear();
            core.channel_grants.clear();
            core.pairings.clear();
        }
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.fail_closed("Remote stopped.");
        if let Ok(mut r) = self.runtime.lock() {
            r.stop();
        }
        if let Ok(mut c) = self.core.lock() {
            c.identity.take();
            c.enabled = false;
        }
    }

    pub fn state(&self) -> RemoteState {
        let core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        let policy = core.policy.as_ref();
        RemoteState {
            qualified: LIVE_QUALIFIED,
            enabled: core.enabled,
            online: core.deadline.is_some_and(|d| Instant::now() < d)
                && self.healthy.load(Ordering::SeqCst),
            host_id: policy.map(|p| p.host_id.clone()),
            fingerprint: policy
                .and_then(|p| p.bundle.bundle.fingerprint().ok())
                .map(|f| hex(&f)),
            message: core.message.clone(),
            sessions: runtime
                .sessions
                .values()
                .map(|s| SessionInfo {
                    id: s.id.clone(),
                    epoch: s.epoch.clone(),
                    label: s.label.clone(),
                    cols: s.cols,
                    rows: s.rows,
                    shared: core.shares.contains(&s.id),
                    available: s.available,
                })
                .collect(),
            domain_epoch: core.domain.epoch.clone(),
            workspaces: core
                .domain
                .workspaces
                .iter()
                .map(|w| {
                    let consent =
                        policy.and_then(|p| p.workspaces.iter().find(|c| c.id == w.id && c.shared));
                    let ready = w.terminals.iter().all(|t| {
                        t.session_id
                            .as_ref()
                            .is_some_and(|id| runtime.sessions.get(id).is_some_and(|s| s.available))
                    });
                    WorkspaceInfo {
                        id: w.id.clone(),
                        shared: consent.is_some(),
                        online: consent.is_some()
                            && ready
                            && core.deadline.is_some_and(|d| d > Instant::now()),
                        message: if consent.is_some() && !ready {
                            workspace::workspace_unavailable(&core.domain, &runtime, &w.id).or_else(
                                || {
                                    Some(
                                        "Waiting for every workspace terminal to become available."
                                            .into(),
                                    )
                                },
                            )
                        } else {
                            None
                        },
                    }
                })
                .collect(),
            pairings: core.pairings.clone(),
            grants: policy
                .map(|p| {
                    p.grants
                        .iter()
                        .map(|g| GrantInfo {
                            id: g.id.clone(),
                            fingerprint: hex(&g.approval.approval.device_fingerprint),
                            session_ids: g
                                .approval
                                .approval
                                .session_ids
                                .iter()
                                .map(uuid_text)
                                .collect(),
                            permissions: g.approval.approval.permissions,
                            expires_at: g.approval.approval.expires_at,
                            revoked: g.revoked,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    async fn enable(&self, app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
        if !enabled {
            self.fail_closed("Remote is disabled.");
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            core.enabled = false;
            core.shares.clear();
            core.legacy_shares.clear();
            core.restore_binding = core.binding.clone();
            if let Some(policy) = core.policy.as_mut() {
                for w in &mut policy.workspaces {
                    w.shared = false;
                    w.sessions.clear();
                    w.projection = None;
                }
                for g in &mut policy.grants {
                    g.revoked = true;
                }
                core.commit_policy()?;
            }
            return Ok(());
        }
        if !LIVE_QUALIFIED {
            return Err("This build has not qualified Remote live access.".into());
        }
        self.enable_inner(app, false).await
    }

    #[cfg(feature = "remote-probe")]
    pub(crate) async fn probe_enable(&self, app: &tauri::AppHandle) -> Result<(), String> {
        if app.state::<AuthController>().remote_environment()? != "development" {
            return Err("Remote probes require the local fixture account.".into());
        }
        self.enable_inner(app, true).await
    }

    async fn enable_inner(&self, app: &tauri::AppHandle, probe: bool) -> Result<(), String> {
        if !self.healthy.load(Ordering::SeqCst) {
            return Err(
                "Independent terminal state is unavailable. Restart Lomi before sharing.".into(),
            );
        }
        let auth = app.state::<AuthController>();
        let (binding, session) = auth
            .remote_request(reqwest::Method::GET, "/v1/remote/native/session", None)
            .await?;
        let account_id: [u8; 16] =
            Sha256::digest(format!("lomi-remote-account-v1:{}", binding.user_id).as_bytes())[..16]
                .try_into()
                .unwrap();
        if session.get("accountId").and_then(Value::as_str) != Some(hex(&account_id).as_str())
            || session.pointer("/user/id").and_then(Value::as_str) != Some(binding.user_id.as_str())
            || session.pointer("/session/id").and_then(Value::as_str)
                != Some(binding.session_id.as_str())
        {
            return Err("Remote session does not match the desktop account.".into());
        }
        #[cfg(feature = "remote-probe")]
        let (policy, identity) = if probe {
            Policy::probe(&hex(&account_id), &binding.session_id, account_id)?
        } else {
            Policy::load_or_create(&hex(&account_id), &binding.session_id, account_id)?
        };
        #[cfg(not(feature = "remote-probe"))]
        let (policy, identity) = {
            let _ = probe;
            Policy::load_or_create(&hex(&account_id), &binding.session_id, account_id)?
        };
        auth.remote_request(
            reqwest::Method::POST,
            "/v1/remote/native/hosts",
            Some(&json!({"hostId":policy.host_id,"name":"Lomi desktop","bundle":policy.bundle})),
        )
        .await?;
        if auth.remote_binding()? != binding {
            return Err("Account session changed.".into());
        }
        {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            core.binding = Some(binding);
            core.policy = Some(policy);
            core.identity = Some(identity);
            core.enabled = true;
            core.message = None;
            core.deadline = None;
        }
        self.poll(app).await
    }

    async fn poll(&self, app: &tauri::AppHandle) -> Result<(), String> {
        self.reconcile_workspaces(app)?;
        let auth = app.state::<AuthController>();
        let binding = auth.remote_binding()?;
        let (host_id, shares, workspaces, policy_revision) = {
            let core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if core.binding.as_ref() != Some(&binding) || !self.healthy.load(Ordering::SeqCst) {
                return Err("Remote authorization changed.".into());
            }
            let runtime = self
                .runtime
                .lock()
                .map_err(|_| "Terminal model unavailable.")?;
            (core.policy.as_ref().ok_or("Remote identity unavailable.")?.host_id.clone(),
                core.shares.iter().filter_map(|id| runtime.sessions.get(id)).filter(|s| s.available).map(|s| json!({"id":s.id,"epoch":s.epoch,"label":s.label,"cols":s.cols,"rows":s.rows})).collect::<Vec<_>>(), core.policy.as_ref().ok_or("Remote identity unavailable.")?.workspaces.iter().filter(|w| w.shared && core.domain.revision > 0 && core.domain.workspaces.iter().any(|d| d.id == w.id)).enumerate().map(|(i,w)| json!({"id":w.id,"epoch":w.epoch,"revision":w.revision,"label":format!("Workspace {}",i+1),"permissions":"control","sessionIds":w.sessions.iter().map(|s| s.0.clone()).collect::<Vec<_>>()})).collect::<Vec<_>>(),core.policy_revision)
        };
        auth.remote_request(
            reqwest::Method::POST,
            &format!("/v1/remote/native/hosts/{host_id}/heartbeat"),
            Some(&json!({"shares":shares,"workspaces":workspaces})),
        )
        .await?;
        let (_, state) = auth
            .remote_request(
                reqwest::Method::GET,
                &format!("/v1/remote/native/hosts/{host_id}/state"),
                None,
            )
            .await?;
        let pairings: Vec<Pairing> = serde_json::from_value(
            state
                .get("pairings")
                .cloned()
                .ok_or("Invalid Remote state.")?,
        )
        .map_err(|_| "Invalid Remote pairings.")?;
        let channels = state
            .get("channels")
            .and_then(Value::as_array)
            .ok_or("Invalid Remote channels.")?;
        let grants = state
            .get("grants")
            .and_then(Value::as_array)
            .ok_or("Invalid Remote grants.")?;
        let authorization = state
            .get("authorizationExpiresAt")
            .and_then(Value::as_str)
            .ok_or_else(|| "Invalid Remote deadline.".to_string())
            .and_then(parse_expiry)?;
        if pairings.len() > 32
            || channels.len() > 64
            || grants.len() > 64
            || authorization <= now()
            || auth.remote_binding()? != binding
        {
            return Err("Remote authorization expired.".into());
        }
        {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if core.binding.as_ref() != Some(&binding) || !core.enabled {
                return Err("Remote authorization changed.".into());
            }
            core.deadline = Some(
                Instant::now()
                    + Duration::from_secs(
                        10.min(authorization - now())
                            .min(binding.expires_at.saturating_sub(now())),
                    ),
            );
            core.message = None;
            let pending_pairings: HashSet<String> = pairings
                .iter()
                .filter(|p| p.status == "pending" && p.expires_at > now())
                .map(|p| p.id.clone())
                .collect();
            core.pairings = pairings
                .into_iter()
                .filter(|p| p.host_id == host_id && p.status == "pending" && p.expires_at > now())
                .collect();
            // Server can revoke a local grant, but can never create one.
            if core.policy_revision == policy_revision {
                core.reconcile_cloud_grants(grants, &pending_pairings)?;
            }
            let active: HashSet<&str> = channels
                .iter()
                .filter_map(|c| c.get("id").and_then(Value::as_str))
                .collect();
            core.channels.retain(|id, stop| {
                if active.contains(id.as_str()) {
                    true
                } else {
                    stop.store(true, Ordering::SeqCst);
                    false
                }
            });
        }
        self.update_workspace_grants(app).await?;
        for wire in channels {
            channel::start(self.clone(), app.clone(), wire.clone()).await?;
        }
        Ok(())
    }

    async fn approve(
        &self,
        app: &tauri::AppHandle,
        pairing_id: String,
        fingerprint: String,
        session_ids: Vec<String>,
        permissions: Permissions,
    ) -> Result<(), String> {
        let auth = app.state::<AuthController>();
        let binding = auth.remote_binding()?;
        let (signed, grant_id) = {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if core.binding.as_ref() != Some(&binding)
                || !core.deadline.is_some_and(|d| d > Instant::now())
            {
                return Err("Remote authorization expired.".into());
            }
            if session_ids.is_empty()
                || session_ids.len() > 32
                || session_ids.iter().collect::<HashSet<_>>().len() != session_ids.len()
                || session_ids.iter().any(|id| !core.shares.contains(id))
            {
                return Err("Select explicitly shared sessions.".into());
            }
            let pairing = core
                .pairings
                .iter()
                .find(|p| p.id == pairing_id && p.expires_at > now())
                .ok_or("Pairing expired.")?
                .clone();
            let policy = core.policy.as_ref().ok_or("Remote identity unavailable.")?;
            let hostfp = policy.bundle.bundle.fingerprint()?;
            let devicefp = pairing.device_bundle.bundle.fingerprint()?;
            pairing.device_bundle.verify(&devicefp)?;
            let account_id = policy.bundle.bundle.account_id;
            let host_id = uuid_bytes(&policy.host_id)?;
            let device_id = uuid_bytes(&pairing.device_id)?;
            if pairing.host_bundle != policy.bundle
                || pairing.device_bundle.bundle.account_id != account_id
                || pairing.device_bundle.bundle.subject_id != device_id
                || pairing.device_bundle.bundle.role != lomi_remote_crypto::Role::Device
                || pairing.host_fingerprint != hex(&hostfp)
                || pairing.device_fingerprint != hex(&devicefp)
            {
                return Err("Pairing identity mismatch.".into());
            }
            let nonce = hex_bytes::<32>(&pairing.nonce)?;
            let expected = hex(&lomi_remote_crypto::pairing_fingerprint(
                &account_id,
                &host_id,
                &device_id,
                &hostfp,
                &devicefp,
                &nonce,
            ));
            // Exact full paste is intentional: no suffix matching or automatic clipboard read.
            if fingerprint != expected || pairing.pairing_fingerprint != expected {
                return Err("Paste the exact complete fingerprint shown by your browser.".into());
            }
            let grant_id = uuid()?;
            let signed = core
                .identity
                .as_ref()
                .ok_or("Remote identity unavailable.")?
                .sign_peer_approval(PeerApproval {
                    version: 1,
                    workspace_id: None,
                    workspace_epoch: None,
                    session_epochs: vec![],
                    account_id,
                    host_id,
                    device_id,
                    host_fingerprint: hostfp,
                    device_fingerprint: devicefp,
                    pairing_nonce: nonce,
                    grant_id: uuid_bytes(&grant_id)?,
                    session_ids: session_ids
                        .iter()
                        .map(|id| uuid_bytes(id))
                        .collect::<Result<Vec<_>, _>>()?,
                    permissions,
                    access_epoch: 1,
                    revision: 1,
                    expires_at: binding
                        .expires_at
                        .min(pairing.max_approval_expires_at)
                        .min(now() + 12 * 3600),
                })?;
            let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
            // Removed entries cannot authorize cloud regrants: channels require a retained local grant.
            policy
                .grants
                .retain(|g| !g.revoked && g.approval.approval.expires_at > now());
            if policy.grants.len() >= 32 {
                return Err("Revoke old devices before approving another.".into());
            }
            policy.grants.push(LocalGrant {
                id: grant_id.clone(),
                device_bundle: pairing.device_bundle,
                approval: signed.clone(),
                revoked: false,
                confirmed: false,
                pairing_id: None,
            });
            // Local permission and complete key pins are committed before cloud approval.
            if let Err(error) = policy.save() {
                policy.grants.pop();
                return Err(error);
            }
            core.policy_revision = core.policy_revision.wrapping_add(1);
            (signed, grant_id)
        };
        if auth
            .remote_request(
                reqwest::Method::POST,
                &format!("/v1/remote/native/pairings/{pairing_id}/approve"),
                Some(&json!({"signedApproval":signed})),
            )
            .await
            .is_err()
        {
            self.revoke_local(app, &grant_id)?;
            return Err("Pairing approval was not confirmed. Local access was revoked.".into());
        }
        {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
            let local = policy
                .grants
                .iter_mut()
                .find(|g| g.id == grant_id && !g.revoked)
                .ok_or("Local approval changed.")?;
            local.confirmed = true;
            if let Err(error) = policy.save() {
                if let Some(local) = policy.grants.iter_mut().find(|g| g.id == grant_id) {
                    local.confirmed = false;
                    local.revoked = true;
                }
                core.storage_failed();
                return Err(error);
            }
            core.policy_revision = core.policy_revision.wrapping_add(1);
        }
        self.poll(app).await
    }

    fn revoke_local(&self, app: &tauri::AppHandle, id: &str) -> Result<(), String> {
        let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
        // Disconnect before attempting secure persistence or network access.
        for stop in core.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        core.channels.clear();
        for session in &core.shares {
            app.state::<Terminals>().remote_revoke(session);
        }
        core.policy_revision = core.policy_revision.wrapping_add(1);
        let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
        let grant = policy
            .grants
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or("Unknown Remote grant.")?;
        grant.revoked = true;
        if let Err(error) = policy.save() {
            core.storage_failed();
            return Err(error);
        }
        Ok(())
    }
}

pub(super) fn parse_expiry(text: &str) -> Result<u64, String> {
    let stamp = chrono::DateTime::parse_from_rfc3339(text)
        .map_err(|_| "Invalid Remote expiry.")?
        .timestamp();
    u64::try_from(stamp).map_err(|_| "Invalid Remote expiry.".into())
}
fn deserialize_expiry<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse_expiry(&text).map_err(serde::de::Error::custom)
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX)
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn hex_bytes<const N: usize>(text: &str) -> Result<[u8; N], String> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Invalid identity encoding.".into());
    }
    let mut out = [0; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| "Invalid identity encoding.")?;
    }
    Ok(out)
}
pub(super) fn uuid() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Entropy unavailable.")?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    Ok(uuid_text(&bytes))
}
pub(super) fn uuid_text(bytes: &[u8; 16]) -> String {
    let h = hex(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}
pub(super) fn uuid_bytes(text: &str) -> Result<[u8; 16], String> {
    if text.len() != 36 || ![8, 13, 18, 23].iter().all(|&i| text.as_bytes()[i] == b'-') {
        return Err("Invalid UUID.".into());
    }
    hex_bytes(&text.replace('-', ""))
}

fn settings(window: &Window) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("Remote management is available only in Settings.".into())
    }
}
#[tauri::command]
pub fn remote_get_state(window: Window, remote: State<'_, Remote>) -> Result<RemoteState, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Untrusted Remote caller.".into());
    }
    Ok(remote.state())
}
#[tauri::command]
pub async fn remote_set_enabled(
    window: Window,
    remote: State<'_, Remote>,
    enabled: bool,
) -> Result<RemoteState, String> {
    settings(&window)?;
    remote.enable(window.app_handle(), enabled).await?;
    Ok(remote.state())
}
#[tauri::command]
pub fn remote_share_session(
    window: Window,
    remote: State<'_, Remote>,
    id: String,
    shared: bool,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&id)?;
    let mut core = remote.core.lock().map_err(|_| "Remote unavailable.")?;
    if shared {
        if !core.enabled {
            return Err("Enable Remote first.".into());
        }
        if !remote
            .runtime
            .lock()
            .map_err(|_| "Terminal unavailable.")?
            .sessions
            .get(&id)
            .is_some_and(|s| s.available)
        {
            return Err("Terminal is no longer running.".into());
        }
        if core.shares.len() >= 32 {
            return Err("Remote supports at most 32 shared sessions.".into());
        }
        core.legacy_shares.insert(id.clone());
        core.shares.insert(id);
    } else {
        core.legacy_shares.remove(&id);
        core.shares.remove(&id);
        window.state::<Terminals>().remote_revoke(&id);
        for stop in core.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        core.channels.clear();
    }
    drop(core);
    Ok(remote.state())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_approve_pairing(
    window: Window,
    remote: State<'_, Remote>,
    pairing_id: String,
    fingerprint: String,
    session_ids: Vec<String>,
    permissions: Permissions,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&pairing_id)?;
    remote
        .approve(
            window.app_handle(),
            pairing_id,
            fingerprint,
            session_ids,
            permissions,
        )
        .await?;
    Ok(remote.state())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_deny_pairing(
    window: Window,
    remote: State<'_, Remote>,
    pairing_id: String,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&pairing_id)?;
    window
        .state::<AuthController>()
        .remote_request(
            reqwest::Method::POST,
            &format!("/v1/remote/native/pairings/{pairing_id}/deny"),
            Some(&json!({})),
        )
        .await?;
    Ok(remote.state())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_revoke_grant(
    window: Window,
    remote: State<'_, Remote>,
    grant_id: String,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&grant_id)?;
    remote.revoke_local(window.app_handle(), &grant_id)?;
    window
        .state::<AuthController>()
        .remote_request(
            reqwest::Method::POST,
            &format!("/v1/remote/native/grants/{grant_id}/revoke"),
            Some(&json!({})),
        )
        .await?;
    Ok(remote.state())
}

#[tauri::command]
pub fn hide_main_window(window: Window) -> Result<(), String> {
    crate::files::main_window(&window)?;
    window
        .hide()
        .map_err(|_| "Could not hide the workspace.".into())
}
#[tauri::command]
pub fn request_quit(window: Window) -> Result<(), String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Untrusted caller.".into());
    }
    window.app_handle().exit(0);
    Ok(())
}
#[tauri::command]
pub fn reopen_main_window(window: Window) -> Result<(), String> {
    settings(&window)?;
    let main = window
        .app_handle()
        .get_window("main")
        .ok_or("Workspace unavailable.")?;
    main.unminimize().map_err(|_| "Workspace unavailable.")?;
    main.show().map_err(|_| "Workspace unavailable.")?;
    main.set_focus()
        .map_err(|_| "Workspace unavailable.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_grant() -> (Policy, LocalGrant) {
        let (policy, host) = Policy::probe("account", "parent", [1; 16]).unwrap();
        let device =
            Identity::generate([1; 16], [3; 16], lomi_remote_crypto::Role::Device, 1).unwrap();
        let id = uuid().unwrap();
        let signed = host
            .sign_peer_approval(PeerApproval {
                version: 2,
                account_id: [1; 16],
                host_id: uuid_bytes(&policy.host_id).unwrap(),
                device_id: [3; 16],
                host_fingerprint: policy.bundle.bundle.fingerprint().unwrap(),
                device_fingerprint: device.public_bundle().bundle.fingerprint().unwrap(),
                pairing_nonce: [8; 32],
                grant_id: uuid_bytes(&id).unwrap(),
                workspace_id: Some([9; 16]),
                workspace_epoch: Some([10; 16]),
                session_ids: vec![],
                session_epochs: vec![],
                permissions: Permissions::Control,
                access_epoch: 1,
                revision: 1,
                expires_at: now() + 60,
            })
            .unwrap();
        (
            policy,
            LocalGrant {
                id,
                device_bundle: device.public_bundle(),
                approval: signed,
                revoked: false,
                confirmed: false,
                pairing_id: Some("initial-pairing".into()),
            },
        )
    }
    #[test]
    fn account_switch_fences_old_credentials_and_preserves_domain_for_new_parent() {
        let (policy, _) = fixture_grant();
        let mut core = Core {
            enabled: true,
            policy: Some(policy),
            binding: Some(crate::auth::controller::RemoteBinding {
                user_id: "old-account".into(),
                session_id: "old-parent".into(),
                expires_at: now() + 60,
            }),
            ..Core::default()
        };
        let domain_epoch = core.domain.begin().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        core.channels.insert("channel".into(), stop.clone());
        core.shares.insert(uuid().unwrap());
        core.reset_bound_authority();
        assert!(!core.enabled);
        assert!(core.binding.is_none());
        assert!(core.policy.is_none());
        assert!(core.identity.is_none());
        assert!(core.shares.is_empty());
        assert!(core.restore_binding.is_none());
        assert!(stop.load(Ordering::SeqCst));
        assert_eq!(core.domain.epoch.as_deref(), Some(domain_epoch.as_str()));
        core.binding = Some(crate::auth::controller::RemoteBinding {
            user_id: "new-account".into(),
            session_id: "new-parent".into(),
            expires_at: now() + 60,
        });
        assert_ne!(core.binding.as_ref().unwrap().session_id, "old-parent");
    }
    #[test]
    fn replaced_pending_approval_and_revoked_inflight_scope_do_not_poison_other_enrollment() {
        let (mut policy, grant) = fixture_grant();
        let id = grant.id.clone();
        policy.grants.push(grant);
        let mut core = Core {
            policy: Some(policy),
            ..Core::default()
        };
        let pending = HashSet::from(["initial-pairing".into()]);
        core.reconcile_cloud_grants(&[], &pending).unwrap();
        assert!(!core.policy.as_ref().unwrap().grants[0].revoked);
        // The initial response may be lost after commit; its active cloud grant preserves retry.
        core.reconcile_cloud_grants(&[json!({"id":id})], &HashSet::new())
            .unwrap();
        assert!(!core.policy.as_ref().unwrap().grants[0].revoked);
        let old_stop = Arc::new(AtomicBool::new(false));
        core.channels.insert("old-channel".into(), old_stop.clone());
        core.channel_grants.insert("old-channel".into(), id);
        // Replacement removed both the old pending pairing and its unconfirmed grant.
        core.reconcile_cloud_grants(&[], &HashSet::new()).unwrap();
        assert!(core.policy.as_ref().unwrap().grants[0].revoked);
        assert!(old_stop.load(Ordering::SeqCst));
        let (_, mut next) = fixture_grant();
        next.pairing_id = None;
        let next_id = next.id.clone();
        core.policy.as_mut().unwrap().grants.push(next);
        core.reconcile_cloud_grants(&[], &pending).unwrap();
        assert!(
            core.policy
                .as_ref()
                .unwrap()
                .grants
                .iter()
                .find(|g| g.id == next_id)
                .unwrap()
                .revoked
        );
        core.prune_inactive_grants().unwrap();
        assert!(core.policy.as_ref().unwrap().grants.is_empty());
        let (_, fresh) = fixture_grant();
        core.policy.as_mut().unwrap().grants.push(fresh);
        core.reconcile_cloud_grants(&[], &pending).unwrap();
        assert!(!core.policy.as_ref().unwrap().grants[0].revoked);
    }
    #[test]
    fn identifiers_expiries_and_disabled_defaults_fail_closed() {
        let text = uuid().unwrap();
        assert_eq!(uuid_text(&uuid_bytes(&text).unwrap()), text);
        assert!(uuid_bytes("AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA").is_err());
        assert!(uuid_bytes("12345678_1234_1234_1234_123456789abc").is_err());
        assert!(hex_bytes::<32>(&"a".repeat(63)).is_err());
        assert_eq!(parse_expiry("1970-01-01T00:01:00Z").unwrap(), 60);
        assert!(parse_expiry("-1").is_err());
        let remote = Remote::default();
        assert!(!remote.state().enabled);
        assert_eq!(
            remote.state().qualified,
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        );
        assert_eq!(remote.state().message.is_none(), LIVE_QUALIFIED);
        assert!(!remote.state().online);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };
    let show = MenuItem::with_id(
        app,
        "remote-show-workspace",
        "Show Lomi",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "remote-quit", "Quit Lomi", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("lomi-background")
        .tooltip("Lomi")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "remote-show-workspace" => {
                if let Some(main) = app.get_window("main") {
                    let _ = main.unminimize();
                    let _ = main.show();
                    let _ = main.set_focus();
                }
            }
            "remote-quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    app.manage(builder.build(app)?);
    Ok(())
}
