//! Explicit account observations from admitted native control planes.
//! Source windows remain metadata until their model/execution scope is qualified.
use super::{
    native_capacity::{self, CapacityStatus, Observation, Window as CapacityWindow},
    native_process::{Prepared, Process},
    native_wire::{strict_json, NativeKind},
    runtime,
    types::{AuthState, Profile, Snapshot, StorageMode},
    CliRouterService,
};
use crate::{cli_catalog::TitleCli, terminal::Shells};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, Window};

const REPORT_FILE: &str = "native-observation.json";
const REPORT_LIMIT: usize = 256 * 1024;
const REPORT_TTL: i64 = 60_000;
const TIMEOUT: Duration = Duration::from_secs(30);
const UNAVAILABLE: &str = "The native account observation could not be verified. Try again after checking the native login.";
const UNSUPPORTED: &str =
    "This native client has no qualified read-only identity and quota collector.";
const CHANGED: &str = "The account or router configuration changed during refresh. Refresh again.";
const UNKNOWN_SCOPE: &str =
    "Native account authenticated; quota scope is not qualified for router balancing.";
const MISMATCH: &str = "The native account identity differs from the saved binding. Reconnect this account before using it.";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeReport {
    pub(crate) profile_id: String,
    pub(crate) profile_revision: u64,
    pub(crate) observed_at: i64,
    pub(crate) expires_at: i64,
    pub(crate) authenticated: bool,
    pub(crate) observation: Observation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}
struct Collected {
    authenticated: bool,
    observation: Observation,
    // A successful existing Codex reader already supplies its authenticated group.
    group: Option<String>,
    error: Option<String>,
}
impl Collected {
    fn unknown(cli: TitleCli, error: &'static str) -> Self {
        Self {
            authenticated: false,
            observation: native_capacity::unsupported_capacity(cli_name(cli)),
            group: None,
            error: Some(error.into()),
        }
    }
}
fn cli_name(cli: TitleCli) -> &'static str {
    match cli {
        TitleCli::Claude => "claude",
        TitleCli::Codex => "codex",
        TitleCli::Grok => "grok",
        TitleCli::Kimi => "kimi",
        TitleCli::Kilo => "kilo",
        TitleCli::Pi => "pi",
        TitleCli::Opencode => "opencode",
        TitleCli::Agy => "antigravity",
        _ => "unsupported",
    }
}
fn trusted(window: &Window) -> Result<(), String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Native account access is unavailable in this window.".into());
    }
    Ok(())
}
fn eligible(profile: &Profile) -> Result<(), String> {
    if !profile.enabled
        || matches!(
            profile.auth_state,
            AuthState::PendingRemove | AuthState::Disabled
        )
    {
        return Err("Enable an available native account before refreshing it.".into());
    }
    if profile.storage_mode != StorageMode::CliManaged || profile.gateway_provider.is_some() {
        return Err(
            "Native refresh requires a CLI-managed account without an API destination.".into(),
        );
    }
    Ok(())
}

fn check_profile(snapshot: &Snapshot, expected: &Profile) -> Result<(), String> {
    let current = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == expected.id)
        .ok_or(CHANGED)?;
    if current != expected {
        return Err(CHANGED.into());
    }
    eligible(current)
}

struct Lease {
    state: CliRouterService,
    app: AppHandle,
    profile: Profile,
    directory: PathBuf,
    active_id: String,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}
impl Lease {
    fn acquire(state: CliRouterService, app: AppHandle, id: &str) -> Result<Self, String> {
        let active_id = format!("native-refresh-{}", super::new_id()?);
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (profile, directory) = state.with_store(&app, |owner| {
            runtime::storage_ready(&state)?;
            if state.closing.load(Ordering::SeqCst) {
                return Err("Lomi is preparing to close.".into());
            }
            if super::profile_writer_busy(&state, id)? {
                return Err("Close this account's native terminal before refreshing it.".into());
            }
            let snapshot = owner.store.snapshot()?;
            let profile = snapshot
                .profiles
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .ok_or("Account no longer exists.")?;
            eligible(&profile)?;
            let directory = super::profile_directory(owner, id)?;
            let mut writers = state.profile_terminals.lock().map_err(|_| UNAVAILABLE)?;
            let mut active = state.active.lock().map_err(|_| UNAVAILABLE)?;
            if writers.contains_key(id) || active.len() >= 4 || state.closing.load(Ordering::SeqCst)
            {
                return Err("The native account reader is busy or stopping.".into());
            }
            writers.insert(id.into(), done.clone());
            active.insert(active_id.clone(), cancel.clone());
            Ok((profile, directory))
        })?;
        Ok(Self {
            state,
            app,
            profile,
            directory,
            active_id,
            cancel,
            done,
        })
    }
    fn check_snapshot(&self, snapshot: &Snapshot) -> Result<(), String> {
        runtime::storage_ready(&self.state)?;
        if self.state.closing.load(Ordering::SeqCst)
            || self.cancel.load(Ordering::SeqCst)
            || self.done.load(Ordering::SeqCst)
        {
            return Err("The native account refresh was stopped.".into());
        }
        check_profile(snapshot, &self.profile)?;
        let writers = self
            .state
            .profile_terminals
            .lock()
            .map_err(|_| UNAVAILABLE)?;
        if !writers
            .get(&self.profile.id)
            .is_some_and(|done| Arc::ptr_eq(done, &self.done))
        {
            return Err(CHANGED.into());
        }
        Ok(())
    }
    fn check(&self) -> Result<(), String> {
        self.state.with_store(&self.app, |owner| {
            self.check_snapshot(&owner.store.snapshot()?)
        })
    }
    fn spawn(&self, prepared: Prepared, args: &[String]) -> Result<Process, String> {
        self.state.with_store(&self.app, |owner| {
            let snapshot = owner.store.snapshot()?;
            prepared.spawn(args, || self.check_snapshot(&snapshot))
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.done.store(true, Ordering::SeqCst);
        if let Ok(mut active) = self.state.active.lock() {
            if active
                .get(&self.active_id)
                .is_some_and(|v| Arc::ptr_eq(v, &self.cancel))
            {
                active.remove(&self.active_id);
            }
        }
        if let Ok(mut writers) = self.state.profile_terminals.lock() {
            if writers
                .get(&self.profile.id)
                .is_some_and(|v| Arc::ptr_eq(v, &self.done))
            {
                writers.remove(&self.profile.id);
            }
        }
    }
}
struct Deadline {
    complete: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    at: Instant,
}
impl Deadline {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let at = Instant::now() + TIMEOUT;
        let thread = thread::spawn(move || {
            if receiver.recv_timeout(TIMEOUT).is_err() {
                cancel.store(true, Ordering::SeqCst);
            }
        });
        Self {
            complete: Some(sender),
            thread: Some(thread),
            at,
        }
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        if let Some(sender) = self.complete.take() {
            let _ = sender.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[tauri::command]
pub(crate) async fn cli_profile_refresh_native(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    shells: State<'_, Shells>,
    profile_id: String,
) -> Result<NativeReport, String> {
    trusted(&window)?;
    let state = state.inner().clone();
    let shells = shells.inner().clone();
    tauri::async_runtime::spawn_blocking(move || refresh(state, app, shells, &profile_id))
        .await
        .map_err(|_| UNAVAILABLE.to_string())?
}
fn refresh(
    state: CliRouterService,
    app: AppHandle,
    shells: Shells,
    profile_id: &str,
) -> Result<NativeReport, String> {
    let lease = Lease::acquire(state, app, profile_id)?;
    let deadline = Deadline::new(lease.cancel.clone());
    let collected = match collect(&lease, &shells, deadline.at) {
        Ok(result) => result,
        Err(_) => Collected::unknown(lease.profile.cli, UNAVAILABLE),
    };
    lease.check()?;
    publish(&lease, collected)
}

fn collect(lease: &Lease, shells: &Shells, deadline: Instant) -> Result<Collected, String> {
    let cli = lease.profile.cli;
    if matches!(cli, TitleCli::Pi | TitleCli::Opencode) || NativeKind::from_cli(cli).is_none() {
        return Ok(Collected::unknown(cli, UNSUPPORTED));
    }
    let shell = shells
        .profiles
        .iter()
        .find(|shell| {
            shell.distro.is_none() && matches!(shell.kind.as_str(), "bash" | "zsh" | "fish" | "sh")
        })
        .ok_or(UNAVAILABLE)?;
    let cwd = lease.directory.to_str().ok_or(UNAVAILABLE)?;
    lease.check()?;
    let prepared = Prepared::prepare(
        &lease.app,
        shells,
        &shell.id,
        cwd,
        cli,
        &lease.directory,
        lease.cancel.clone(),
    )?;
    match cli {
        TitleCli::Claude => {
            let bytes =
                prepared.collect_command(&["auth".into(), "status".into()], || lease.check())?;
            let raw = strict_json(&bytes)?;
            let authenticated = raw["loggedIn"] == true
                && matches!(raw["authMethod"].as_str(), Some("claude.ai" | "oauth"));
            Ok(Collected {
                authenticated,
                observation: native_capacity::parse_claude_auth(&raw),
                group: None,
                error: if authenticated {
                    None
                } else {
                    Some(UNAVAILABLE.into())
                },
            })
        }
        TitleCli::Codex => {
            // Success is the native status command's exit contract. No text or
            // credential/JWT payload is interpreted as an account identity.
            prepared.collect_command(&["login".into(), "status".into()], || lease.check())?;
            lease.check()?;
            let usage = lease.app.state::<crate::cli_usage::CliUsage>();
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(UNAVAILABLE)?;
            let quota = tauri::async_runtime::block_on(async {
                tokio::time::timeout(
                    remaining,
                    super::usage::read_codex(usage.inner(), &lease.directory, None),
                )
                .await
            })
            .map_err(|_| UNAVAILABLE)?
            .map_err(|_| UNAVAILABLE)?;
            let mut observation = native_capacity::unsupported_capacity("codex");
            observation.source = "codex:authenticated-native-usage".into();
            if let Ok(windows) = quota.windows {
                observation.windows = windows
                    .into_iter()
                    .map(|window| CapacityWindow {
                        id: window.id.clone(),
                        name: window.id,
                        remaining_percent: window.remaining_percent,
                        remaining_amount: None,
                        unit: None,
                        reset_at: window.reset_at,
                        disabled: false,
                        native_id: None,
                        native_window: None,
                    })
                    .collect();
            }
            Ok(Collected {
                authenticated: true,
                observation,
                group: Some(quota.group_key),
                error: None,
            })
        }
        TitleCli::Grok | TitleCli::Kimi | TitleCli::Kilo => {
            let args = prepared.kind().launch().arguments;
            let mut process = lease.spawn(prepared, &args)?;
            let result = match cli {
                TitleCli::Grok => grok(&mut process, lease, deadline),
                TitleCli::Kimi => kimi(&mut process, lease),
                TitleCli::Kilo => kilo(&mut process, lease),
                _ => unreachable!(),
            };
            // Stop and reap the owned server even when parsing or authentication
            // fails. The writer lease remains held until this drain completes.
            let drained = process.stop();
            match (result, drained) {
                (Ok(result), Ok(())) => Ok(result),
                _ => Err(UNAVAILABLE.into()),
            }
        }
        TitleCli::Agy => {
            let bytes = prepared.collect_command(
                &[
                    "-p".into(),
                    "/usage".into(),
                    "--output-format".into(),
                    "json".into(),
                ],
                || lease.check(),
            )?;
            let raw = strict_json(&bytes)?;
            Ok(Collected {
                // The report proves its native usage envelope, but this pin has
                // no independent fresh authenticated-principal observation.
                authenticated: false,
                observation: native_capacity::parse_agy_report(&raw).map_err(|_| UNAVAILABLE)?,
                group: None,
                error: None,
            })
        }
        _ => Ok(Collected::unknown(cli, UNSUPPORTED)),
    }
}
fn get(process: &mut Process, lease: &Lease, path: &str) -> Result<Value, String> {
    lease.check()?;
    let (status, value) = process.request_with_status("GET", path, None)?;
    if status != 200 {
        return Err(UNAVAILABLE.into());
    }
    Ok(value)
}
fn kimi(process: &mut Process, lease: &Lease) -> Result<Collected, String> {
    let user = get(process, lease, "/oauth/userinfo")?;
    if user["data"]["kind"] != "ok" {
        return Err(UNAVAILABLE.into());
    }
    let usage = get(process, lease, "/oauth/usage").unwrap_or(Value::Null);
    let observation = native_capacity::parse_kimi(&user, &usage);
    if observation.identity.is_none() {
        return Err(UNAVAILABLE.into());
    }
    Ok(Collected {
        authenticated: true,
        observation,
        group: None,
        error: None,
    })
}
fn kilo(process: &mut Process, lease: &Lease) -> Result<Collected, String> {
    let status = get(process, lease, "/kilo/auth-status")?;
    if status["authenticated"] != true || status["type"] != "oauth" {
        return Err(UNAVAILABLE.into());
    }
    let profile = get(process, lease, "/kilo/profile")?;
    if !profile["profile"].is_object() {
        return Err(UNAVAILABLE.into());
    }
    Ok(Collected {
        authenticated: true,
        observation: native_capacity::parse_kilo(&profile),
        group: None,
        error: None,
    })
}
fn rpc(
    process: &mut Process,
    lease: &Lease,
    deadline: Instant,
    id: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    lease.check()?;
    process.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
    let mut events = 0usize;
    loop {
        lease.check()?;
        if Instant::now() >= deadline {
            return Err(UNAVAILABLE.into());
        }
        if let Some(value) = process.poll()? {
            events += 1;
            if events > 256 {
                return Err(UNAVAILABLE.into());
            }
            if value["jsonrpc"] != "2.0" {
                return Err(UNAVAILABLE.into());
            }
            if value["id"] == id {
                if value.get("error").is_some() || value.get("method").is_some() {
                    return Err(UNAVAILABLE.into());
                }
                return value
                    .get("result")
                    .filter(|value| value.is_object())
                    .cloned()
                    .ok_or_else(|| UNAVAILABLE.into());
            }
            // Native callbacks cannot turn a read-only refresh into an approval
            // or an interactive login. Uncorrelated response IDs also fail closed.
            if value.get("id").is_some() {
                return Err(UNAVAILABLE.into());
            }
        } else {
            if process.exited() {
                return Err(UNAVAILABLE.into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
fn cached_method(initialized: &Value) -> bool {
    initialized["authMethods"]
        .as_array()
        .is_some_and(|methods| methods.iter().any(|method| method["id"] == "cached_token"))
}
fn grok_authenticated(authenticated: &Value) -> bool {
    authenticated["_meta"]
        .as_object()
        .is_some_and(|meta| meta.get("auth_mode").is_some_and(|mode| mode.is_string()))
}
fn grok(process: &mut Process, lease: &Lease, deadline: Instant) -> Result<Collected, String> {
    let initialized = rpc(
        process,
        lease,
        deadline,
        "native-init",
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"lomi","version":"1"}}),
    )?;
    if !cached_method(&initialized) {
        return Err(UNAVAILABLE.into());
    }
    let authenticated = rpc(
        process,
        lease,
        deadline,
        "native-auth",
        "authenticate",
        json!({"methodId":"cached_token","_meta":{"headless":true}}),
    )?;
    if !grok_authenticated(&authenticated) {
        return Err(UNAVAILABLE.into());
    }
    let auth = rpc(
        process,
        lease,
        deadline,
        "native-info",
        "_x.ai/auth/info",
        json!({}),
    )?;
    let billing = rpc(
        process,
        lease,
        deadline,
        "native-billing",
        "_x.ai/billing",
        json!({}),
    )
    .unwrap_or(Value::Null);
    let observation = native_capacity::parse_grok(&auth, &billing);
    if observation.identity.is_none() {
        return Err(UNAVAILABLE.into());
    }
    Ok(Collected {
        authenticated: true,
        observation,
        group: None,
        error: None,
    })
}
fn identity_group(observation: &Observation) -> Option<String> {
    let identity = observation.identity.as_ref()?;
    let mut hash = Sha256::new();
    hash.update(b"lomi-native-observation-group-v1\0");
    hash.update(identity.fingerprint().as_bytes());
    Some(
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}
fn updated_profile(
    profile: &Profile,
    authenticated: bool,
    group: Option<&str>,
) -> Result<(Profile, bool), String> {
    let mut current = profile.clone();
    let mismatched = authenticated
        && profile
            .quota_group_key
            .as_deref()
            .zip(group)
            .is_some_and(|(old, new)| old != new);
    if authenticated {
        current.auth_state = if mismatched {
            AuthState::IdentityMismatch
        } else {
            AuthState::Ready
        };
        if !mismatched {
            current.quota_group_key = group.map(str::to_owned);
        }
    } else {
        current.auth_state = AuthState::Unverified;
    }
    if current.auth_state != profile.auth_state
        || current.quota_group_key != profile.quota_group_key
    {
        current.revision = profile.revision.checked_add(1).ok_or(UNAVAILABLE)?;
    }
    Ok((current, mismatched))
}

fn publish(lease: &Lease, mut collected: Collected) -> Result<NativeReport, String> {
    // Neither a current native balance nor a usage endpoint establishes which
    // dimensions block a router's selected model and execution mode.
    collected.observation.scope = None;
    collected.observation.status = CapacityStatus::Unknown;
    if !collected.authenticated {
        collected.observation.identity = None;
        collected.group = None;
    }
    let group = collected
        .group
        .or_else(|| identity_group(&collected.observation));
    let observed_at = super::now();
    let expires_at = observed_at.checked_add(REPORT_TTL).ok_or(UNAVAILABLE)?;
    let (report, revision) = lease.state.with_store(&lease.app, |owner| {
        let snapshot = owner.store.snapshot()?;
        lease.check_snapshot(&snapshot)?;
        let (current, mismatched) =
            updated_profile(&lease.profile, collected.authenticated, group.as_deref())?;
        let report = NativeReport {
            profile_id: current.id.clone(),
            profile_revision: current.revision,
            observed_at,
            expires_at,
            authenticated: collected.authenticated,
            observation: collected.observation,
            error: if mismatched {
                Some(MISMATCH.into())
            } else {
                collected.error
            },
        };
        validate_report(&report, &current)?;
        // Commit live authentication proof before replacing metadata. A failed
        // store commit cannot publish a fresh same-revision observation.
        let snapshot = owner.store.update(|snapshot| {
            lease.check_snapshot(snapshot)?;
            let profile = snapshot
                .profiles
                .iter_mut()
                .find(|p| p.id == current.id)
                .ok_or(CHANGED)?;
            *profile = current.clone();
            // Native metadata cannot leave an old percentage qualified for a
            // newly refreshed, explicitly unqualified execution scope.
            snapshot
                .quota
                .retain(|quota| quota.profile_id != lease.profile.id);
            Ok(())
        })?;
        persist(&lease.directory, &report)?;
        Ok((report, snapshot.revision))
    })?;
    lease.state.changed(&lease.app, revision);
    Ok(report)
}
fn directory(path: &Path) -> Result<PathBuf, String> {
    for path in [path.parent().ok_or(UNAVAILABLE)?, path] {
        let metadata = fs::symlink_metadata(path).map_err(|_| UNAVAILABLE)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(UNAVAILABLE.into());
        }
    }
    let canonical = path.canonicalize().map_err(|_| UNAVAILABLE)?;
    if canonical.parent()
        != Some(
            path.parent()
                .ok_or(UNAVAILABLE)?
                .canonicalize()
                .map_err(|_| UNAVAILABLE)?
                .as_path(),
        )
    {
        return Err(UNAVAILABLE.into());
    }
    Ok(canonical)
}
fn persist(directory_path: &Path, report: &NativeReport) -> Result<(), String> {
    let bytes = serde_json::to_vec(report).map_err(|_| UNAVAILABLE)?;
    if bytes.len() > REPORT_LIMIT {
        return Err(UNAVAILABLE.into());
    }
    let directory = directory(directory_path)?;
    let path = directory.join(REPORT_FILE);
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(UNAVAILABLE.into());
        }
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&directory).map_err(|_| UNAVAILABLE)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| UNAVAILABLE)?;
    temporary.write_all(&bytes).map_err(|_| UNAVAILABLE)?;
    temporary.as_file().sync_all().map_err(|_| UNAVAILABLE)?;
    // Revalidate the private parent after preparing the bounded receipt.
    if self::directory(directory_path)? != directory {
        return Err(UNAVAILABLE.into());
    }
    temporary.persist(&path).map_err(|_| UNAVAILABLE)?;
    File::open(&directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| UNAVAILABLE)?;
    Ok(())
}
fn bounded_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn validate_report(report: &NativeReport, profile: &Profile) -> Result<(), String> {
    let expected_source = match profile.cli {
        TitleCli::Claude => "claude:auth-status",
        TitleCli::Codex => "codex:authenticated-native-usage",
        TitleCli::Grok => "grok:acp-billing",
        TitleCli::Kimi => "kimi:native-oauth-usage",
        TitleCli::Kilo => "kilo:native-profile",
        TitleCli::Pi => "pi:unsupported-native-capacity",
        TitleCli::Opencode => "opencode:unsupported-native-capacity",
        TitleCli::Agy => "antigravity:native-usage",
        _ => "unsupported-native-capacity",
    };
    if report.observation.source != expected_source
        && !(report.observation.source == "unsupported-native-capacity" && !report.authenticated)
    {
        return Err(UNAVAILABLE.into());
    }
    if report.authenticated && matches!(profile.cli, TitleCli::Pi | TitleCli::Opencode) {
        return Err(UNAVAILABLE.into());
    }
    if report.authenticated
        && matches!(profile.cli, TitleCli::Grok | TitleCli::Kimi)
        && report.observation.identity.is_none()
    {
        return Err(UNAVAILABLE.into());
    }
    if report.profile_id != profile.id
        || report.profile_revision != profile.revision
        || report.observed_at < 0
        || report.observed_at > super::now().saturating_add(5000)
        || report.expires_at.checked_sub(report.observed_at) != Some(REPORT_TTL)
        || !bounded_text(&report.observation.source)
        || report.observation.scope.is_some()
        || report.observation.status != CapacityStatus::Unknown
        || report.observation.windows.len() > 256
        || report.error.as_deref().is_some_and(|error| {
            !matches!(error, UNAVAILABLE | UNSUPPORTED | UNKNOWN_SCOPE | MISMATCH)
        })
    {
        return Err(UNAVAILABLE.into());
    }
    if let Some(identity) = &report.observation.identity {
        if report.error.as_deref() != Some(MISMATCH)
            && identity_group(&report.observation).as_ref() != profile.quota_group_key.as_ref()
        {
            return Err(UNAVAILABLE.into());
        }
        if !matches!(
            (
                profile.cli,
                identity.source.as_str(),
                identity.provider.as_str()
            ),
            (TitleCli::Grok, "grok:acp-auth-info", "xai")
                | (TitleCli::Kimi, "kimi:native-oauth-userinfo", "kimi")
        ) {
            return Err(UNAVAILABLE.into());
        }
        if !report.authenticated
            || ![
                &identity.source,
                &identity.provider,
                &identity.principal_id,
                &identity.display_label,
            ]
            .into_iter()
            .all(|text| bounded_text(text))
            || identity
                .organization_id
                .as_ref()
                .is_some_and(|text| !bounded_text(text))
        {
            return Err(UNAVAILABLE.into());
        }
    }
    for window in &report.observation.windows {
        if !bounded_text(&window.id)
            || !bounded_text(&window.name)
            || window.unit.as_ref().is_some_and(|unit| !bounded_text(unit))
            || window
                .native_id
                .as_ref()
                .is_some_and(|id| !bounded_text(id))
            || window
                .native_window
                .as_ref()
                .is_some_and(|period| !bounded_text(period))
            || window
                .remaining_percent
                .is_some_and(|number| !number.is_finite() || !(0.0..=100.0).contains(&number))
            || window.remaining_amount.is_some_and(|number| {
                !number.is_finite() || !(0.0..=9_007_199_254_740_991.0).contains(&number)
            })
            || window.reset_at.is_some_and(|time| time < 0)
        {
            return Err(UNAVAILABLE.into());
        }
    }
    Ok(())
}
fn load(directory_path: &Path, profile: &Profile) -> Result<Option<NativeReport>, String> {
    if !directory_path.try_exists().map_err(|_| UNAVAILABLE)? {
        return Ok(None);
    }
    let path = directory(directory_path)?.join(REPORT_FILE);
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UNAVAILABLE.into()),
    };
    let metadata = file.metadata().map_err(|_| UNAVAILABLE)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() > REPORT_LIMIT as u64
    {
        return Err(UNAVAILABLE.into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(REPORT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNAVAILABLE)?;
    if bytes.len() > REPORT_LIMIT {
        return Err(UNAVAILABLE.into());
    }
    let raw = strict_json(&bytes).map_err(|_| UNAVAILABLE)?;
    let report: NativeReport = serde_json::from_value(raw).map_err(|_| UNAVAILABLE)?;
    if report.profile_id != profile.id || report.profile_revision != profile.revision {
        return Ok(None);
    }
    validate_report(&report, profile)?;
    if serde_json::to_vec(&report).map_err(|_| UNAVAILABLE)? != bytes {
        return Err(UNAVAILABLE.into());
    }
    Ok(Some(report))
}

/// Load only the saved sanitized receipt. This command never starts a client,
/// contacts a provider, or promotes a saved observation into current auth.
#[tauri::command]
pub(crate) async fn cli_profile_native_report(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    profile_id: String,
) -> Result<Option<NativeReport>, String> {
    trusted(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_store(&app, |owner| {
            let snapshot = owner.store.snapshot()?;
            let profile = snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .ok_or("Account no longer exists.")?;
            if !crate::chat::process::valid_id(&profile.id) || profile.id.len() != 32 {
                return Err(UNAVAILABLE.into());
            }
            load(&owner.root.join("profiles").join(&profile.id), profile)
        })
    })
    .await
    .map_err(|_| UNAVAILABLE.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_grok_identity_is_not_authenticated_without_native_auth_response() {
        assert!(cached_method(
            &json!({"authMethods":[{"id":"cached_token"}]})
        ));
        assert!(!cached_method(&json!({"authMethods":[{"id":"grok.com"}]})));
        assert!(!grok_authenticated(
            &json!({"principalId":"expired","email":"a@b"})
        ));
        assert!(!grok_authenticated(&json!({})));
        assert!(grok_authenticated(
            &json!({"_meta":{"auth_mode":"GrokCom"}})
        ));
    }
    #[test]
    fn stable_group_binds_native_principal_and_organization_only() {
        let mut observation = native_capacity::parse_grok(
            &json!({"principalId":"u1","principalType":"user","organizationId":"o1","email":"a@b","token":"private"}),
            &Value::Null,
        );
        let first = identity_group(&observation).unwrap();
        assert_eq!(first.len(), 64);
        assert!(!first.contains("private"));
        observation.identity.as_mut().unwrap().display_label = "different@b".into();
        assert_eq!(
            identity_group(&observation).as_deref(),
            Some(first.as_str())
        );
        observation.identity.as_mut().unwrap().organization_id = Some("o2".into());
        assert_ne!(
            identity_group(&observation).as_deref(),
            Some(first.as_str())
        );
    }
    #[test]
    fn unsupported_collectors_do_not_invent_auth_or_capacity() {
        for cli in [TitleCli::Pi, TitleCli::Opencode, TitleCli::Agy] {
            let collected = Collected::unknown(cli, UNSUPPORTED);
            assert!(!collected.authenticated);
            assert!(collected.observation.identity.is_none());
            assert!(collected.observation.windows.is_empty());
            assert!(collected.group.is_none());
        }
    }
    fn profile_fixture() -> Profile {
        Profile {
            id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            cli: TitleCli::Claude,
            label: "Native account".into(),
            enabled: true,
            revision: 2,
            auth_state: AuthState::Ready,
            storage_mode: StorageMode::CliManaged,
            credential_ref: None,
            quota_group_key: None,
            gateway_provider: None,
        }
    }
    fn report_fixture(profile: &Profile) -> NativeReport {
        let observed_at = super::super::now();
        NativeReport {
            profile_id: profile.id.clone(),
            profile_revision: profile.revision,
            observed_at,
            expires_at: observed_at + REPORT_TTL,
            authenticated: true,
            observation: native_capacity::parse_claude_auth(&json!({"loggedIn":true})),
            error: None,
        }
    }
    #[test]
    fn saved_report_is_private_bounded_and_bound_to_exact_profile_revision() {
        let parent = tempfile::tempdir().unwrap();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let directory = parent.path().join("profile");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let profile = profile_fixture();
        let report = report_fixture(&profile);
        persist(&directory, &report).unwrap();
        assert_eq!(
            fs::metadata(directory.join(REPORT_FILE)).unwrap().mode() & 0o777,
            0o600
        );
        assert!(load(&directory, &profile).unwrap().unwrap().authenticated);
        let mut changed = profile.clone();
        changed.revision += 1;
        assert!(load(&directory, &changed).unwrap().is_none());
        let mut future = report.clone();
        future.profile_revision += 1;
        persist(&directory, &future).unwrap();
        assert!(load(&directory, &profile).unwrap().is_none());
        fs::write(directory.join(REPORT_FILE), vec![b'x'; REPORT_LIMIT + 1]).unwrap();
        assert!(load(&directory, &profile).is_err());
    }
    #[test]
    fn saved_report_cannot_smuggle_raw_credentials_or_qualified_scope() {
        let profile = profile_fixture();
        let report = report_fixture(&profile);
        let mut raw = serde_json::to_value(&report).unwrap();
        raw["observation"]["token"] = json!("private");
        assert!(serde_json::from_value::<NativeReport>(raw).is_err());
        let mut qualified = report.clone();
        qualified.observation.scope = Some("all-models".into());
        assert!(validate_report(&qualified, &profile).is_err());
        let mut qualified = report;
        qualified.observation.status = CapacityStatus::Fresh;
        assert!(validate_report(&qualified, &profile).is_err());
    }
    #[test]
    fn agy_report_admits_raw_windows_without_auth_identity_or_balancing() {
        let mut profile = profile_fixture();
        profile.cli = TitleCli::Agy;
        let mut report = report_fixture(&profile);
        report.authenticated = false;
        report.observation = native_capacity::parse_agy_report(&json!({"status":"SUCCESS", "num_turns":0,
            "conversation_id":"", "usage":{"input_tokens":0,"output_tokens":0,"thinking_tokens":0,
                "cache_read_tokens":0,"total_tokens":0}, "command":{"name":"usage","data":{
                    "groups":[{"name":"Models","buckets":[{"id":"native","name":"Daily", "window":"day",
                        "remaining_fraction":0.25}]}]}}})).unwrap();
        report.observation.status = CapacityStatus::Unknown;
        validate_report(&report, &profile).unwrap();
        assert!(identity_group(&report.observation).is_none());
        assert_eq!(report.observation.windows[0].remaining_percent, Some(25.0));
        let parent = tempfile::tempdir().unwrap();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let directory = parent.path().join("profile");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        persist(&directory, &report).unwrap();
        let saved = load(&directory, &profile).unwrap().unwrap();
        assert_eq!(
            saved.observation.windows[0].native_id.as_deref(),
            Some("native")
        );
        assert_eq!(
            saved.observation.windows[0].native_window.as_deref(),
            Some("day")
        );
        report.observation.scope = Some("all-models".into());
        assert!(validate_report(&report, &profile).is_err());
    }
    #[test]
    fn saved_report_rejects_links_and_noncanonical_json() {
        let parent = tempfile::tempdir().unwrap();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let directory = parent.path().join("profile");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let profile = profile_fixture();
        let report = report_fixture(&profile);
        persist(&directory, &report).unwrap();
        let bytes = serde_json::to_vec_pretty(&report).unwrap();
        fs::write(directory.join(REPORT_FILE), bytes).unwrap();
        assert!(load(&directory, &profile).is_err());
        fs::remove_file(directory.join(REPORT_FILE)).unwrap();
        let outside = parent.path().join("outside.json");
        fs::write(&outside, serde_json::to_vec(&report).unwrap()).unwrap();
        std::os::unix::fs::symlink(outside, directory.join(REPORT_FILE)).unwrap();
        assert!(load(&directory, &profile).is_err());
        assert!(persist(&directory, &report).is_err());
    }
    #[test]
    fn repeated_verified_observations_preserve_credential_revision() {
        let mut profile = profile_fixture();
        profile.quota_group_key = Some("verified-account".into());
        let (first, mismatched) =
            updated_profile(&profile, true, Some("verified-account")).unwrap();
        assert!(!mismatched);
        assert_eq!(first, profile);
        let (second, mismatched) = updated_profile(&first, true, Some("verified-account")).unwrap();
        assert!(!mismatched);
        assert_eq!(second.revision, profile.revision);
        let (changed, mismatched) =
            updated_profile(&profile, true, Some("different-account")).unwrap();
        assert!(mismatched);
        assert_eq!(changed.auth_state, AuthState::IdentityMismatch);
        assert_eq!(changed.quota_group_key, profile.quota_group_key);
        assert_eq!(changed.revision, profile.revision + 1);
        let (unverified, _) = updated_profile(&profile, false, None).unwrap();
        assert_eq!(unverified.auth_state, AuthState::Unverified);
        assert_eq!(unverified.revision, profile.revision + 1);
    }
    #[test]
    fn unrelated_snapshot_output_does_not_release_profile_fence() {
        let profile = profile_fixture();
        let mut snapshot = Snapshot {
            revision: 10,
            profiles: vec![profile.clone()],
            ..Snapshot::default()
        };
        check_profile(&snapshot, &profile).unwrap();
        snapshot.revision = 99;
        check_profile(&snapshot, &profile).unwrap();
        snapshot.profiles[0].credential_ref = Some("changed-binding".into());
        assert!(check_profile(&snapshot, &profile).is_err());
        snapshot.profiles[0] = profile.clone();
        snapshot.profiles[0].revision += 1;
        assert!(check_profile(&snapshot, &profile).is_err());
        snapshot.profiles[0] = profile.clone();
        snapshot.profiles[0].enabled = false;
        assert!(check_profile(&snapshot, &profile).is_err());
    }
}
