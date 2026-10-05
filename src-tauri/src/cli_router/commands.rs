use super::{adapters, profile_directory, runtime, types::*, view, CliRouterService, View};
use crate::{cli_catalog::TitleCli, files::main_window, terminal::Shells};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::atomic::Ordering};
use tauri::{AppHandle, Emitter, Manager, State, Window};

fn trusted(window: &Window, settings_only: bool) -> Result<(), String> {
    if settings_only && window.label() != "settings" {
        return Err("Manage CLI accounts in Settings.".into());
    }
    if !matches!(window.label(), "main" | "settings") {
        return Err("Router access is unavailable in this window.".into());
    }
    Ok(())
}

fn label(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 160 || value.chars().any(char::is_control) {
        return Err("Enter a name between 1 and 160 characters.".into());
    }
    Ok(value.into())
}

fn editable(profile: &Profile) -> Result<(), String> {
    if profile.auth_state == AuthState::PendingRemove {
        return Err(
            "This account is being removed. Retry removal to finish credential cleanup.".into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn cli_router_snapshot(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
) -> Result<View, String> {
    trusted(&window, false)?;
    let settings = window.label() == "settings";
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_store(&app, |owner| Ok(view(owner.store.snapshot()?, settings)))
    })
    .await
    .map_err(|_| "Router storage task failed.")?
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Mutation {
    request_id: String,
    expected_revision: u64,
    action: Action,
}

#[derive(Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Action {
    CreateProfile {
        cli: TitleCli,
        label: String,
    },
    UpdateProfile {
        profile_id: String,
        label: Option<String>,
        enabled: Option<bool>,
    },
    ConfigureGatewayAccount {
        profile_id: String,
        provider: Option<ApiDestination>,
    },
    RemoveProfile {
        profile_id: String,
    },
    CreateRouter {
        cli: TitleCli,
        label: String,
        ordered_profile_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        balance_remaining_quota: Option<bool>,
    },
    UpdateRouter {
        router_id: String,
        label: Option<String>,
        enabled: Option<bool>,
        balance_remaining_quota: Option<bool>,
        ordered_profile_ids: Option<Vec<String>>,
    },
    RemoveRouter {
        router_id: String,
    },
}

fn pool(snapshot: &Snapshot, cli: TitleCli, ids: &[String]) -> Result<(), String> {
    let mut seen = HashSet::new();
    if ids.is_empty()
        || ids.len() > 32
        || ids.iter().any(|id| {
            !seen.insert(id)
                || !snapshot
                    .profiles
                    .iter()
                    .any(|p| p.id == *id && p.cli == cli)
        })
    {
        return Err("Choose distinct accounts belonging to this CLI.".into());
    }
    Ok(())
}

fn ready_bindings(snapshot: &Snapshot, cli: TitleCli, ids: &[String]) -> usize {
    let mut native = HashSet::new();
    let mut gateway = HashSet::new();
    for profile in snapshot.profiles.iter().filter(|profile| {
        ids.contains(&profile.id)
            && profile.cli == cli
            && profile.enabled
            && profile.auth_state == AuthState::Ready
    }) {
        let Some(key) = profile
            .quota_group_key
            .as_ref()
            .filter(|key| !key.is_empty())
        else {
            continue;
        };
        if let Some(provider) = &profile.gateway_provider {
            if profile.storage_mode == StorageMode::ApiKey && profile.credential_ref.is_some() {
                let Ok(provider) = super::gateway_config::destination(cli, provider) else {
                    continue;
                };
                let Ok(identity) =
                    super::gateway_url::dispatch_identity(&provider.base_url, provider.protocol)
                else {
                    continue;
                };
                gateway.insert((key.clone(), identity));
            }
        } else if (cli == TitleCli::Codex && profile.storage_mode != StorageMode::ApiKey)
            || (cli != TitleCli::Codex && profile.storage_mode == StorageMode::ApiKey)
        {
            native.insert(key.clone());
        }
    }
    // One subscription plus one API gateway binding cannot form a two-account
    // pool in either execution contract. Require two in one compatible family.
    let native_profiles = if super::native_accounts::available(cli) {
        snapshot
            .profiles
            .iter()
            .filter(|profile| {
                ids.contains(&profile.id)
                    && profile.cli == cli
                    && profile.enabled
                    && profile.gateway_provider.is_none()
                    && profile.storage_mode != StorageMode::ApiKey
                    && matches!(profile.auth_state, AuthState::Ready | AuthState::Unverified)
            })
            .map(|profile| {
                profile
                    .quota_group_key
                    .as_deref()
                    .filter(|key| !key.is_empty())
                    .map(|key| (true, key))
                    .unwrap_or((false, profile.id.as_str()))
            })
            .collect::<HashSet<_>>()
            .len()
    } else {
        0
    };
    native.len().max(gateway.len()).max(native_profiles)
}

fn router_options(
    snapshot: &Snapshot,
    cli: TitleCli,
    ids: &[String],
    enabling: bool,
    balance: bool,
) -> Result<(), String> {
    if enabling && ready_bindings(snapshot, cli, ids) < 2 {
        return Err("Verify at least two distinct subscription bindings or configure two compatible API gateway bindings before enabling this router. Separate keys do not prove separate billing budgets.".into());
    }
    if balance && !adapters::adapter(cli).capability.balance {
        return Err(
            "This CLI has no qualified profile quota reader. New turns can use account order."
                .into(),
        );
    }
    Ok(())
}

fn create_router(
    snapshot: &mut Snapshot,
    cli: TitleCli,
    name: &str,
    ids: &[String],
    enabled: bool,
    balance: bool,
) -> Result<(), String> {
    pool(snapshot, cli, ids)?;
    if snapshot.routers.len() >= 64 {
        return Err("Remove an unused router before adding another.".into());
    }
    router_options(snapshot, cli, ids, enabled, balance)?;
    snapshot.routers.push(Router {
        id: super::new_id()?,
        cli,
        label: label(name)?,
        enabled,
        ordered_profile_ids: ids.to_vec(),
        balance_remaining_quota: balance,
        revision: 1,
    });
    Ok(())
}

fn router_reconfiguration(
    snapshot: &Snapshot,
    router: &Router,
    enabled: Option<bool>,
    balance: Option<bool>,
    ids: Option<&[String]>,
) -> Result<(), String> {
    if let Some(ids) = ids {
        pool(snapshot, router.cli, ids)?;
    }
    let enabling = enabled.unwrap_or(router.enabled) && (enabled.is_some() || ids.is_some());
    router_options(
        snapshot,
        router.cli,
        ids.unwrap_or(&router.ordered_profile_ids),
        enabling,
        balance == Some(true),
    )
}

#[tauri::command]
pub(crate) async fn cli_router_mutate(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: Mutation,
) -> Result<View, String> {
    trusted(&window, true)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = state.with_store(&app, |owner| {
            owner.store.mutate(&format!("settings:mutate:{}", request.request_id), &request, |snapshot| {
                if snapshot.revision != request.expected_revision { return Err("Router settings changed. Refresh and try again.".into()); }
                match &request.action {
                    Action::CreateProfile { cli, label: name } => {
                        if !adapters::adapter(*cli).capability.can_create_profile { return Err(adapters::adapter(*cli).capability.reason); }
                        if snapshot.profiles.len() >= 128 { return Err("Remove an unused account before adding another.".into()); }
                        snapshot.profiles.push(Profile { id: super::new_id()?, cli: *cli, label: label(name)?, enabled: true, revision: 1, credential_ref: None, quota_group_key: None, gateway_provider: None, auth_state: AuthState::Disconnected, storage_mode: StorageMode::CliManaged });
                    }
                    Action::UpdateProfile { profile_id, label: name, enabled } => {
                        super::gateway_config::profile_idle(snapshot, profile_id)?;
                        let profile = snapshot.profiles.iter_mut().find(|p| p.id == *profile_id).ok_or("Account no longer exists.")?;
                        editable(profile)?;
                        if let Some(name) = name { profile.label = label(name)?; }
                        if let Some(enabled) = enabled { profile.enabled = *enabled; }
                        profile.revision += 1;
                    }
                    Action::ConfigureGatewayAccount { profile_id, provider } => {
                        super::credentials::idle(snapshot, profile_id)?;
                        super::gateway_config::profile_idle(snapshot, profile_id)?;
                        if super::profile_writer_busy(&state, profile_id)? { return Err("Close this account's terminal before changing its destination.".into()); }
                        let profile = snapshot.profiles.iter_mut().find(|profile| profile.id == *profile_id).ok_or("Account no longer exists.")?;
                        editable(profile)?;
                        super::gateway_config::configure(profile, provider.as_ref())?;
                        snapshot.quota.retain(|quota| quota.profile_id != *profile_id);
                        for run in &mut snapshot.runs {
                            if run.allowed_profile_ids.contains(profile_id) { run.allowed_profile_ids.retain(|id| id != profile_id); if run.pinned_profile_id.as_deref() == Some(profile_id) { run.pinned_profile_id = None; } run.revision = run.revision.checked_add(1).ok_or("Run revision exhausted.")?; }
                        }
                    }
                    Action::RemoveProfile { profile_id } => {
                        if super::profile_writer_busy(&state, profile_id)? { return Err("Close the account terminal before removing it.".into()); }
                        if !snapshot.profiles.iter().any(|p| p.id == *profile_id) { return Err("Account no longer exists.".into()); }
                        super::credentials::idle(snapshot, profile_id)?;
                        // Removal revokes dispatch and preserves the CLI-owned namespace
                        // until logout can be qualified without affecting other profiles.
                        let profile = snapshot.profiles.iter_mut().find(|p| p.id == *profile_id).ok_or("Account no longer exists.")?;
                        profile.enabled = false; profile.auth_state = AuthState::PendingRemove; profile.revision += 1;
                        snapshot.quota.retain(|q| q.profile_id != *profile_id);
                        for router in &mut snapshot.routers {
                            router.ordered_profile_ids.retain(|id| id != profile_id);
                            router.revision += 1;
                            if router.ordered_profile_ids.is_empty() { router.enabled = false; }
                        }
                    }
                    Action::CreateRouter { cli, label: name, ordered_profile_ids, enabled, balance_remaining_quota } => {
                        create_router(snapshot, *cli, name, ordered_profile_ids, enabled.unwrap_or(false), balance_remaining_quota.unwrap_or(false))?;
                    }
                    Action::UpdateRouter { router_id, label: name, enabled, balance_remaining_quota, ordered_profile_ids } => {
                        super::gateway_config::router_idle(snapshot, router_id)?;
                        let index = snapshot.routers.iter().position(|r| r.id == *router_id).ok_or("Router no longer exists.")?;
                        router_reconfiguration(snapshot, &snapshot.routers[index], *enabled, *balance_remaining_quota, ordered_profile_ids.as_deref())?;
                        let router = &mut snapshot.routers[index];
                        if let Some(name) = name { router.label = label(name)?; }
                        if let Some(enabled) = enabled { router.enabled = *enabled; }
                        if let Some(balance) = balance_remaining_quota { router.balance_remaining_quota = *balance; }
                        if let Some(ids) = ordered_profile_ids { router.ordered_profile_ids = ids.clone(); }
                        router.revision += 1;
                    }
                    Action::RemoveRouter { router_id } => {
                        if snapshot.runs.iter().any(|r| r.router_id == *router_id && matches!(r.state, RunState::Running | RunState::Starting | RunState::Switching)) { return Err("Stop active runs before removing this router.".into()); }
                        if !snapshot.routers.iter().any(|r| r.id == *router_id) { return Err("Router no longer exists.".into()); }
                        snapshot.routers.retain(|r| r.id != *router_id);
                    }
                }
                Ok(())
            })
        })?;
        state.changed(&app, result.snapshot.revision);
        if let Action::RemoveProfile { profile_id } = &request.action {
            let removed = state.with_store(&app, |owner| {
                let current = owner.store.snapshot()?;
                if current.profiles.iter().any(|profile| profile.id == *profile_id) {
                    super::credentials::remove(&mut owner.store, profile_id)
                } else { Ok(current) }
            })?;
            state.changed(&app, removed.revision);
            return Ok(view(removed, true));
        }
        Ok(view(result.snapshot, true))
    }).await.map_err(|_| "Router settings task failed.")?
}

#[tauri::command]
pub(crate) async fn cli_profile_open_terminal(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    profile_id: String,
) -> Result<View, String> {
    trusted(&window, true)?;
    let snapshot = state.with_store(&app, |owner| {
        let snapshot = owner.store.snapshot()?;
        let profile = snapshot
            .profiles
            .iter()
            .find(|p| p.id == profile_id && p.enabled)
            .ok_or("Account is disabled or no longer exists.")?;
        editable(profile)?;
        if !adapters::adapter(profile.cli).capability.profile_terminal {
            return Err("This CLI supports API-key router profiles only. Configure its key in Settings and use the Router dialog.".into());
        }
        if profile.storage_mode == StorageMode::ApiKey {
            return Err("This account uses an API key. Start it from the Router dialog.".into());
        }
        let directory = profile_directory(owner, &profile.id)?;
        if super::native_accounts::available(profile.cli) {
            super::native_accounts::prepare(&directory, profile.cli)?;
        }
        Ok(snapshot)
    })?;
    let main = app
        .get_webview_window("main")
        .ok_or("Open the main Lomi window to connect this account.")?;
    tauri::Emitter::emit_to(
        &app,
        tauri::EventTarget::webview("main"),
        "cli-router-open-profile",
        serde_json::json!({"profileId": profile_id}),
    )
    .map_err(|_| "Could not open the account terminal.")?;
    let _ = main.unminimize();
    let _ = main.show();
    let _ = main.set_focus();
    Ok(view(snapshot, true))
}

#[tauri::command]
pub(crate) async fn cli_router_open_terminal(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    shells: State<'_, Shells>,
    router_id: String,
    cwd: String,
    shell_profile_id: String,
) -> Result<View, String> {
    main_window(&window)?;
    runtime::shell_profile(&shells, &shell_profile_id)?;
    let cwd = crate::files::directory(&cwd)?
        .to_string_lossy()
        .into_owned();
    let cli = state.with_store(&app, |owner| {
        owner
            .store
            .snapshot()?
            .routers
            .iter()
            .find(|r| r.id == router_id)
            .map(|r| r.cli)
            .ok_or("Router no longer exists.".into())
    })?;
    if !super::native_accounts::available(cli) {
        return Err("This CLI has no qualified private native account terminal.".into());
    }
    if adapters::adapter(cli).capability.quota_read {
        let _ = refresh_quota(state.inner(), &app, &router_id).await;
    }
    let (snapshot, router_label) = state.with_store(&app, |owner| {
        if state.closing.load(Ordering::SeqCst) {
            return Err("Lomi is preparing to close.".into());
        }
        let snapshot = owner.store.snapshot()?;
        let router = snapshot
            .routers
            .iter()
            .find(|r| r.id == router_id && r.enabled && r.cli == cli)
            .ok_or("Enable this native account router in Settings first.")?;
        let router_label = router.label.clone();
        Ok((snapshot, router_label))
    })?;
    app.emit_to(tauri::EventTarget::webview("main"), "cli-router-open-profile", serde_json::json!({"routerId": router_id, "cwd": cwd, "shellProfileId": shell_profile_id, "cli": cli, "routerLabel": router_label})).map_err(|_| "Could not open the routed terminal.")?;
    Ok(view(snapshot, false))
}

/// This explicit action opens the original native account and exact saved
/// session for manual inspection. It never sends the unfinished user prompt.
#[tauri::command]
pub(crate) fn cli_run_open_native_recovery(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    run_id: String,
    expected_revision: u64,
) -> Result<View, String> {
    main_window(&window)?;
    let (snapshot, payload) = state.with_store(&app, |owner| {
        let snapshot = owner.store.snapshot()?;
        let run = snapshot.runs.iter().find(|run| run.id == run_id && run.revision == expected_revision).ok_or("The native run changed. Refresh before opening recovery.")?;
        if run.execution_mode != RunExecutionMode::Native || !matches!(run.state, RunState::RecoveryRequired | RunState::Stopped | RunState::Paused)
            || state.active.lock().map_err(|_| "Native ownership unavailable.")?.contains_key(&run.id)
            || state.draining.lock().map_err(|_| "Native close state unavailable.")?.contains(&run.id)
        { return Err("Stop the native client and wait for its process to close before opening recovery.".into()); }
        let attempt = run.attempts.last().filter(|attempt| attempt.generation == run.generation).ok_or("This run has no retained native session. Open its account terminal from Settings to inspect native history.")?;
        let profile = snapshot.profiles.iter().find(|profile| profile.id == attempt.profile_id && profile.enabled).ok_or("The original account is unavailable.")?;
        if !run.allowed_profile_ids.contains(&profile.id) || profile.storage_mode == StorageMode::ApiKey || profile.gateway_provider.is_some() || !super::native_accounts::available(profile.cli) { return Err("Approve the original native account's history access before opening recovery.".into()); }
        editable(profile)?;
        let payload = serde_json::json!({"profileId":profile.id,"cli":profile.cli,"cwd":run.cwd,"shellProfileId":run.shell_profile_id,"nativeRunId":run.id,"nativeRunRevision":run.revision});
        Ok((snapshot, payload))
    })?;
    app.emit_to(
        tauri::EventTarget::webview("main"),
        "cli-router-open-profile",
        payload,
    )
    .map_err(|_| "Cannot open the native recovery terminal.")?;
    Ok(view(snapshot, false))
}

pub(crate) struct TerminalProfile<'a> {
    pub(crate) shells: &'a Shells,
    pub(crate) shell: &'a crate::shell::Profile,
    pub(crate) cwd: &'a str,
    pub(crate) id: Option<&'a str>,
    pub(crate) cli: TitleCli,
    pub(crate) router_id: Option<&'a str>,
    pub(crate) recovery: Option<(&'a str, u64)>,
}

/// Called under the native owner lock immediately before preparing the spawn.
/// Pending main-window events never reserve or pin a pool account.
fn select_terminal_profile(
    snapshot: &Snapshot,
    id: Option<&str>,
    cli: TitleCli,
    router_id: Option<&str>,
    now: i64,
    mut writer_busy: impl FnMut(&str) -> Result<bool, String>,
) -> Result<String, String> {
    let Some(router_id) = router_id else {
        return id
            .map(str::to_owned)
            .ok_or_else(|| "Choose a CLI account or router.".into());
    };
    if !super::native_accounts::available(cli) {
        return Err("This CLI has no private native account router terminal.".into());
    }
    let router = snapshot
        .routers
        .iter()
        .find(|router| router.id == router_id && router.enabled && router.cli == cli)
        .ok_or("The router was disabled or changed before launch.")?;
    if id.is_some_and(|id| !router.ordered_profile_ids.iter().any(|member| member == id)) {
        return Err("The selected account is no longer in the router.".into());
    }
    if cli != TitleCli::Codex {
        for member in &router.ordered_profile_ids {
            if id.is_some_and(|id| id != member) {
                continue;
            }
            let Some(profile) = snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == *member && profile.cli == cli)
            else {
                continue;
            };
            if profile.enabled
                && matches!(profile.auth_state, AuthState::Ready | AuthState::Unverified)
                && profile.storage_mode != StorageMode::ApiKey
                && profile.gateway_provider.is_none()
                && !writer_busy(member)?
                && !snapshot.runs.iter().any(|run| {
                    run.allowed_profile_ids.contains(member)
                        && matches!(
                            run.state,
                            RunState::Starting | RunState::Running | RunState::Switching
                        )
                })
            {
                return Ok(member.clone());
            }
        }
        return Err("No available native account. Close active terminals and sign in to an enabled profile first.".into());
    }
    let mut candidates = snapshot.profiles.clone();
    for profile in &mut candidates {
        if profile.storage_mode == StorageMode::ApiKey
            || profile.quota_group_key.as_deref().is_none_or(str::is_empty)
            || writer_busy(&profile.id)?
            || snapshot.runs.iter().any(|run| {
                run.allowed_profile_ids.contains(&profile.id)
                    && matches!(
                        run.state,
                        RunState::Starting | RunState::Running | RunState::Switching
                    )
            })
        {
            profile.enabled = false;
        }
    }
    let run = Run {
        id: String::new(),
        router_id: router.id.clone(),
        cwd: String::new(),
        shell_profile_id: None,
        title: String::new(),
        state: RunState::Idle,
        model: None,
        reasoning_effort: None,
        execution_mode: RunExecutionMode::Text,
        continuation_requested: false,
        pinned_profile_id: id.map(str::to_owned),
        allowed_profile_ids: router.ordered_profile_ids.clone(),
        active_profile_id: None,
        generation: 0,
        revision: 0,
        inputs: vec![],
        attempts: vec![],
        turns: vec![],
        legacy_output: None,
        output: String::new(),
        status_message: String::new(),
        attempted_profile_ids: vec![],
    };
    super::policy::select(router, &run, &candidates, &snapshot.quota, now).profile_id
        .ok_or_else(|| "No available account. Close active profile terminals, verify login or refresh capacity in Settings.".into())
}

pub(crate) fn terminal_command<T>(
    state: &CliRouterService,
    app: &AppHandle,
    request: TerminalProfile<'_>,
    start: impl FnOnce(
        (portable_pty::CommandBuilder, String),
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        (String, String),
    ) -> Result<T, String>,
) -> Result<T, String> {
    let TerminalProfile {
        shells,
        shell,
        cwd,
        id,
        cli,
        router_id,
        recovery,
    } = request;
    if !adapters::adapter(cli).capability.profile_terminal {
        return Err(
            "This CLI has no isolated profile terminal. Use its managed API-key router turn."
                .into(),
        );
    }
    state.with_store(app, |owner| {
        if state.closing.load(Ordering::SeqCst) { return Err("Lomi is preparing to close.".into()); }
        if router_id.is_some() || super::native_accounts::available(cli) { runtime::shell_profile(shells, &shell.id)?; }
        let snapshot = owner.store.snapshot()?;
        let chosen = select_terminal_profile(&snapshot, id, cli, router_id, super::now(), |id| super::profile_writer_busy(state, id))?;
        let id = chosen.as_str();
        let profile = snapshot.profiles.iter().find(|p| p.id == id && p.cli == cli && p.enabled).ok_or("CLI account is disabled or no longer exists.")?;
        let recovery_run = recovery.map(|(run_id, revision)| -> Result<&Run, String> {
            let run = snapshot.runs.iter().find(|run| run.id == run_id && run.revision == revision && run.execution_mode == RunExecutionMode::Native).ok_or("The native recovery request changed before launch.")?;
            if !matches!(run.state, RunState::RecoveryRequired | RunState::Stopped | RunState::Paused)
                || !run.allowed_profile_ids.contains(&profile.id) || run.cwd != cwd || run.shell_profile_id.as_deref() != Some(shell.id.as_str())
                || run.attempts.last().is_none_or(|attempt| attempt.profile_id != profile.id || attempt.generation != run.generation)
                || state.active.lock().map_err(|_| "Native ownership unavailable.")?.contains_key(run_id)
                || state.draining.lock().map_err(|_| "Native close status unavailable.")?.contains(run_id)
            { return Err("The native account, workspace or recovery ownership changed.".into()); }
            Ok(run)
        }).transpose()?;
        editable(profile)?;
        if super::profile_writer_busy(state, id)? { return Err("The account terminal is still open. Close it before connecting again.".into()); }
        if snapshot.runs.iter().any(|run| run.allowed_profile_ids.iter().any(|p| p == id) && matches!(run.state, RunState::Running | RunState::Starting | RunState::Switching)) { return Err("Stop active routed runs before opening this account terminal.".into()); }
        if profile.storage_mode == StorageMode::ApiKey { return Err("API accounts use managed Router turns.".into()); }
        let namespace = adapters::adapter(cli).namespace_env.ok_or("Account isolation is unavailable for this CLI.")?;
        let directory = profile_directory(owner, id)?;
        if cli == TitleCli::Agy { super::gateway_profiles::admit_project(cli, std::path::Path::new(cwd))?; }
        let resolved = if cli == TitleCli::Grok {
            let resource_dir = app.path().resource_dir().map_err(|_| "Cannot locate router-owned Grok resources.")?;
            (super::grok_artifact::resolve(&resource_dir)?, None)
        } else {
            let resolved = crate::cli_launch::resolve_cli(shell, cwd, &shells.integration, cli)?;
            (resolved.program, resolved.argument)
        };
        let native = if super::native_accounts::available(cli) {
            Some(super::native_accounts::prepare(&directory,cli)?)
        } else { None };
        #[cfg(unix)]
        let executable_identity = if native.is_some() { Some(super::gateway_runtime::executable(&resolved.0)?) } else { None };
        #[cfg(unix)]
        let mut admitted_version = None;
        #[cfg(unix)]
        if native.is_some() {
            super::native_accounts::admit_executable(cli,&resolved.0)?;
            if cli == TitleCli::Grok { super::grok_artifact::admit(&resolved.0)?; }
            let probe = super::gateway_profiles::prepare_probe(&directory.join(format!("version-probe-{}",super::new_id()?)))?;
            let version = super::gateway_runtime::version(&resolved.0,cli,&probe)?;
            // npm Kimi owns subscriptions in file OAuth storage. Python Kimi's
            // fallback keyring requires a separate account namespace contract.
            if cli == TitleCli::Kimi && version != "2.1.1" {
                return Err("Native Kimi subscription accounts require npm kimi-code 2.1.1. Python Kimi remains available for API Gateway mode.".into());
            }
            admitted_version = Some(version);
        }
        #[cfg(unix)]
        let history = recovery_run.map(|run| super::native_runtime::recovery_history(owner, run, profile, cli, admitted_version.as_deref().ok_or("Native recovery version is unqualified.")?, &directory)).transpose()?;
        #[cfg(not(unix))]
        if recovery_run.is_some() { return Err("Native recovery is unavailable on this platform.".into()); }
        // Apply the account binding after the interactive shell's startup files.
        let startup = if shell.kind == "fish" { "exec /usr/bin/env -i PATH=\"$PATH\" TERM=xterm-256color COLORTERM=truecolor TERM_PROGRAM=Lomi $argv" } else { "exec /usr/bin/env -i PATH=\"$PATH\" TERM=xterm-256color COLORTERM=truecolor TERM_PROGRAM=Lomi \"$@\"" };
        let (mut command, cwd) = crate::shell::build_with_command(shell, cwd, &shells.integration, startup)?;
        if shell.kind != "fish" { command.arg("lomi-router-profile"); }
        if let Some(launch) = &native {
            for (key,value) in &launch.environment {
                let mut binding = key.clone(); binding.push("="); binding.push(value);
                command.arg(binding);
            }
        } else {
            command.arg(format!("HOME={}", shell.home));
            command.arg(format!("{namespace}={}", directory.display()));
        }
        command.arg("LANG=en_US.UTF-8");
        command.arg(&resolved.0);
        if let Some(launch) = &native { command.args(launch.arguments.iter()); }
        if let Some(argument) = &resolved.1 { command.arg(argument); }
        #[cfg(unix)]
        if let Some(history) = &history { command.args(&history.arguments); }
        #[cfg(unix)]
        if let Some(run) = recovery_run { super::native_runtime::retire_checkpoint(owner, run)?; }
        let saved = owner.store.update(|snapshot| {
            if let Some(run) = recovery_run {
                let current = snapshot.runs.iter_mut().find(|current| current.id == run.id && current.revision == run.revision).ok_or("Native recovery changed before launch.")?;
                current.state = RunState::Paused;
                current.revision = current.revision.checked_add(1).ok_or("Native revision exhausted.")?;
                current.status_message = "Reviewing the exact saved session in the original account's native terminal. Its tools and permissions apply. This external interaction retires the supervised checkpoint; continue in that terminal.".into();
            }
            let profile = snapshot.profiles.iter_mut().find(|p| p.id == id).ok_or("Account no longer exists.")?;
            profile.auth_state = AuthState::Unverified; profile.revision += 1; profile.quota_group_key = None;
            snapshot.quota.retain(|q| q.profile_id != id);
            for run in &mut snapshot.runs {
                if run.allowed_profile_ids.iter().any(|p| p == id) { run.allowed_profile_ids.retain(|p| p != id); run.revision += 1; }
            }
            Ok(())
        })?;
        if state.closing.load(Ordering::SeqCst) { return Err("Lomi is preparing to close.".into()); }
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if native.is_some() {
            if cli == TitleCli::Agy {
                super::gateway_profiles::admit_project(cli, std::path::Path::new(&cwd))?;
                super::native_accounts::prepare(&directory, cli)?;
            }
            super::native_accounts::admit_executable(cli,&resolved.0)?;
            if cli == TitleCli::Grok { super::grok_artifact::admit(&resolved.0)?; }
            #[cfg(unix)]
            if executable_identity.as_ref().is_some_and(|identity| super::gateway_runtime::executable(&resolved.0).as_ref() != Ok(identity)) {
                return Err("The native account executable changed before launch.".into());
            }
        }
        #[cfg(unix)]
        if let Some(history) = &history { history.fence()?; }
        #[cfg(unix)]
        if let Some(run) = recovery_run { super::native_runtime::fence_recovery_workspace(owner, run)?; }
        let result = start((command, cwd), done.clone(), (profile.id.clone(), profile.label.clone()))?;
        state.profile_terminals.lock().map_err(|_| "Router writer state unavailable.")?.insert(id.into(), done);
        state.changed(app, saved.revision);
        Ok(result)
    })
}

#[tauri::command]
pub(crate) async fn cli_profile_verify(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    shells: State<'_, Shells>,
    profile_id: String,
) -> Result<View, String> {
    struct VerificationLease {
        state: CliRouterService,
        profile_id: String,
        active_id: String,
        done: std::sync::Arc<std::sync::atomic::AtomicBool>,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl VerificationLease {
        fn check(&self) -> Result<(), String> {
            runtime::storage_ready(&self.state)?;
            if self.state.closing.load(Ordering::SeqCst) || self.cancelled.load(Ordering::SeqCst) {
                return Err("Account verification was stopped.".into());
            }
            let writers = self
                .state
                .profile_terminals
                .lock()
                .map_err(|_| "Router writer state unavailable.")?;
            if !writers
                .get(&self.profile_id)
                .is_some_and(|done| std::sync::Arc::ptr_eq(done, &self.done))
            {
                return Err("Account ownership changed during verification.".into());
            }
            Ok(())
        }
    }
    impl Drop for VerificationLease {
        fn drop(&mut self) {
            self.done.store(true, Ordering::SeqCst);
            if let Ok(mut writers) = self.state.profile_terminals.lock() {
                if writers
                    .get(&self.profile_id)
                    .is_some_and(|done| std::sync::Arc::ptr_eq(done, &self.done))
                {
                    writers.remove(&self.profile_id);
                }
            }
            if let Ok(mut active) = self.state.active.lock() {
                if active
                    .get(&self.active_id)
                    .is_some_and(|cancel| std::sync::Arc::ptr_eq(cancel, &self.cancelled))
                {
                    active.remove(&self.active_id);
                }
            }
        }
    }
    trusted(&window, true)?;
    let state = state.inner().clone();
    let shells = shells.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (profile, directory, lease) = state.with_store(&app, |owner| {
            runtime::storage_ready(&state)?;
            if state.closing.load(Ordering::SeqCst) {
                return Err("Lomi is preparing to close.".into());
            }
            if super::profile_writer_busy(&state, &profile_id)? {
                return Err("Close the account terminal before verifying its login.".into());
            }
            let snapshot = owner.store.snapshot()?;
            let profile = snapshot
                .profiles
                .iter()
                .find(|p| p.id == profile_id)
                .cloned()
                .ok_or("Account no longer exists.")?;
            editable(&profile)?;
            if !profile.enabled {
                return Err("Enable this account before verifying it.".into());
            }
            let directory = profile_directory(owner, &profile_id)?;
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let active_id = format!("profile-verify-{}", super::new_id()?);
            let mut writers = state
                .profile_terminals
                .lock()
                .map_err(|_| "Router writer state unavailable.")?;
            let mut active = state
                .active
                .lock()
                .map_err(|_| "Router ownership unavailable.")?;
            if writers.contains_key(&profile_id)
                || active.len() >= 4
                || state.closing.load(Ordering::SeqCst)
            {
                return Err("Account verification is busy or stopping.".into());
            }
            writers.insert(profile_id.clone(), done.clone());
            active.insert(active_id.clone(), cancelled.clone());
            let lease = VerificationLease {
                state: state.clone(),
                profile_id: profile_id.clone(),
                active_id,
                done,
                cancelled,
            };
            Ok((profile, directory, lease))
        })?;
        lease.check()?;
        let verified = if profile.storage_mode == StorageMode::ApiKey {
            super::credentials::verify(&profile)?
        } else {
            runtime::verify_profile(&shells, &profile, &directory)?
        };
        if verified && profile.auth_state == AuthState::IdentityMismatch {
            return Err("The native identity differs from the saved account binding. Reconnect this account before verifying it.".into());
        }
        let snapshot = state.with_store(&app, |owner| {
            lease.check()?;
            owner.store.update(|snapshot| {
                lease.check()?;
                let current = snapshot
                    .profiles
                    .iter_mut()
                    .find(|p| p.id == profile.id)
                    .ok_or("Account no longer exists.")?;
                if current != &profile {
                    return Err("Account changed during verification. Try again.".into());
                }
                editable(current)?;
                if !current.enabled {
                    return Err("The account was disabled during verification.".into());
                }
                let auth_state = if verified {
                    AuthState::Ready
                } else {
                    AuthState::ReauthRequired
                };
                if current.auth_state != auth_state {
                    current.auth_state = auth_state;
                    current.revision = current
                        .revision
                        .checked_add(1)
                        .ok_or("Account revision exhausted.")?;
                }
                Ok(())
            })
        })?;
        state.changed(&app, snapshot.revision);
        Ok(view(snapshot, true))
    })
    .await
    .map_err(|_| "Account verification task failed.")?
}
#[tauri::command]
pub(crate) async fn cli_profile_set_api_key(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    profile_id: String,
    expected_revision: u64,
    key: String,
) -> Result<View, String> {
    trusted(&window, true)?;
    if key.trim().is_empty() || key.len() > 8192 || key.chars().any(char::is_control) {
        return Err("Enter a valid API key.".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = state.with_store(&app, |owner| {
            if super::profile_writer_busy(&state, &profile_id)? {
                return Err("Close the account terminal before changing its credentials.".into());
            }
            let current = owner.store.snapshot()?;
            let profile = current
                .profiles
                .iter()
                .find(|p| p.id == profile_id)
                .ok_or("Account no longer exists.")?;
            let capability = adapters::adapter(profile.cli).capability;
            if (!capability.managed_turns || capability.api_key_label.is_none())
                && profile.gateway_provider.is_none()
            {
                return Err("API authentication is unavailable for this CLI.".into());
            }
            if profile.cli == TitleCli::Hermes && profile.gateway_provider.is_none() {
                super::hermes::admit_api_key(key.trim())?;
            }
            if current.runs.iter().any(|run| {
                run.allowed_profile_ids.iter().any(|id| id == &profile_id)
                    && matches!(
                        run.state,
                        RunState::Starting | RunState::Running | RunState::Switching
                    )
            }) {
                return Err(
                    "Stop active routed runs before changing this account's credentials.".into(),
                );
            }
            super::credentials::put(&mut owner.store, &profile_id, expected_revision, key.trim())
        })?;
        state.changed(&app, snapshot.revision);
        Ok(view(snapshot, true))
    })
    .await
    .map_err(|_| "API key storage task failed.")?
}

#[tauri::command]
pub(crate) async fn cli_router_refresh_quota(
    window: Window,
    state: State<'_, CliRouterService>,
    app: AppHandle,
    router_id: String,
) -> Result<View, String> {
    trusted(&window, false)?;
    let settings = window.label() == "settings";
    let snapshot = refresh_quota(state.inner(), &app, &router_id).await?;
    Ok(view(snapshot, settings))
}

pub(crate) async fn refresh_quota(
    state: &CliRouterService,
    app: &AppHandle,
    router_id: &str,
) -> Result<Snapshot, String> {
    use futures_util::{stream, StreamExt};
    let _reader = state.quota_reader.lock().await;
    let prepared = state.with_store(app, |owner| {
        let snapshot = owner.store.snapshot()?;
        let router = snapshot.routers.iter().find(|r| r.id == router_id).ok_or("Router no longer exists.")?;
        if !adapters::adapter(router.cli).capability.quota_read { return Err("This CLI has no qualified profile quota reader. API keys have no subscription percentage.".into()); }
        if owner.quota_checks.get(router_id).is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5)) { return Ok((snapshot.clone(), vec![], snapshot.revision + 1)); }
        owner.quota_checks.insert(router_id.into(), std::time::Instant::now());
        let epoch = snapshot.revision + 1;
        let mut entries = Vec::new();
        for id in &router.ordered_profile_ids {
            let profile = snapshot.profiles.iter().find(|p| p.id == *id).cloned().ok_or("Account no longer exists.")?;
            if profile.storage_mode == StorageMode::ApiKey || !profile.enabled || super::profile_writer_busy(state, id)? || owner.quota_retry_at.get(id).is_some_and(|deadline| *deadline > std::time::Instant::now()) { continue; }
            owner.quota_retry_at.insert(id.clone(), std::time::Instant::now() + std::time::Duration::from_secs(5));
            let block = snapshot.quota.iter().find(|q| q.profile_id == *id).map_or(0, |q| q.block_revision);
            entries.push((profile, profile_directory(owner, id)?, block));
        }
        Ok((snapshot, entries, epoch))
    })?;
    let (baseline, entries, epoch) = prepared;
    if entries.is_empty() {
        return Ok(baseline);
    }
    let usage = app.state::<crate::cli_usage::CliUsage>();
    let results = stream::iter(entries)
        .map(|(profile, directory, block)| {
            let usage = usage.inner();
            async move {
                let result = super::usage::read_codex(usage, &directory, None).await;
                (profile, block, result)
            }
        })
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
    let observed = super::now();
    let snapshot = state.with_store(app, |owner| {
        for (profile, _, result) in &results {
            if let Err(super::usage::CodexQuotaError::Throttled { retry_after_ms }) = result {
                let delay =
                    std::time::Duration::from_millis(retry_after_ms.unwrap_or(30_000).max(5_000));
                // Fail closed if a server delay cannot be represented on this host.
                let deadline = std::time::Instant::now()
                    .checked_add(delay)
                    .unwrap_or_else(|| {
                        std::time::Instant::now()
                            + std::time::Duration::from_secs(365 * 24 * 60 * 60)
                    });
                owner.quota_retry_at.insert(profile.id.clone(), deadline);
            }
        }
        let mut busy = HashSet::new();
        for (profile, _, _) in &results {
            if super::profile_writer_busy(state, &profile.id)? {
                busy.insert(profile.id.clone());
            }
        }
        owner.store.update(|snapshot| {
            apply_quota_results(snapshot, &baseline, &results, epoch, observed, &busy)
        })
    })?;
    state.changed(app, snapshot.revision);
    Ok(snapshot)
}

type QuotaResult = (
    Profile,
    u64,
    Result<super::usage::AuthenticatedCodexQuota, super::usage::CodexQuotaError>,
);

/// Apply one fenced collection atomically. Contradictory reports for a shared
/// account remain blocked until a later collection observes confirmed recovery.
fn apply_quota_results(
    snapshot: &mut Snapshot,
    baseline: &Snapshot,
    results: &[QuotaResult],
    epoch: u64,
    observed: i64,
    busy: &HashSet<String>,
) -> Result<(), String> {
    let blocked_groups = results
        .iter()
        .filter_map(|(profile, block, result)| {
            let authenticated = result.as_ref().ok()?;
            let windows = authenticated.windows.as_ref().ok()?;
            if busy.contains(&profile.id)
                || !windows.iter().any(|w| w.remaining_percent == Some(0.0))
                || !snapshot
                    .profiles
                    .iter()
                    .any(|p| p.id == profile.id && p.revision == profile.revision)
            {
                return None;
            }
            let current = snapshot.quota.iter().find(|q| q.profile_id == profile.id);
            if current.map_or(0, |q| q.block_revision) != *block
                || current.is_some_and(|q| q.epoch > epoch || q.observed_at > observed)
            {
                return None;
            }
            Some(authenticated.group_key.clone())
        })
        .collect::<HashSet<_>>();
    for (profile, block, result) in results {
        let Some(index) = snapshot
            .profiles
            .iter()
            .position(|p| p.id == profile.id && p.revision == profile.revision)
        else {
            continue;
        };
        if busy.contains(&profile.id) {
            continue;
        }
        let mut current = snapshot
            .quota
            .iter()
            .find(|q| q.profile_id == profile.id)
            .cloned();
        if current.as_ref().map_or(0, |q| q.block_revision) != *block
            || current
                .as_ref()
                .is_some_and(|q| q.epoch > epoch || q.observed_at > observed)
        {
            continue;
        }
        let mut report_block = *block;
        let (windows, authenticated_group) = match result {
            Ok(authenticated) => {
                if authenticated.group_key.is_empty() {
                    continue;
                }
                let group = &authenticated.group_key;
                let changed = snapshot.profiles[index]
                    .quota_group_key
                    .as_ref()
                    .is_some_and(|old| old != group);
                let unbound_quota =
                    snapshot.profiles[index].quota_group_key.is_none() && current.is_some();
                if changed || unbound_quota {
                    // Quota from a different or unbound identity cannot be
                    // inherited by this authenticated account.
                    snapshot.quota.retain(|q| q.profile_id != profile.id);
                    current = None;
                    report_block = 0;
                }
                if changed {
                    // Profile revision invalidates prepared dispatch bindings.
                    snapshot.profiles[index].revision = snapshot.profiles[index]
                        .revision
                        .checked_add(1)
                        .ok_or("Account revision exhausted.")?;
                    for run in &mut snapshot.runs {
                        if run.allowed_profile_ids.contains(&profile.id) {
                            run.allowed_profile_ids.retain(|id| id != &profile.id);
                            run.revision = run
                                .revision
                                .checked_add(1)
                                .ok_or("Run revision exhausted.")?;
                        }
                    }
                }
                // Account verification is independent of numeric quota coverage.
                snapshot.profiles[index].quota_group_key = Some(group.clone());
                snapshot.profiles[index].auth_state = AuthState::Ready;
                (authenticated.windows.as_ref(), Some(group))
            }
            Err(failure) => {
                if matches!(failure, super::usage::CodexQuotaError::Unauthenticated) {
                    snapshot.profiles[index].auth_state = AuthState::ReauthRequired;
                    snapshot.profiles[index].revision = snapshot.profiles[index]
                        .revision
                        .checked_add(1)
                        .ok_or("Account revision exhausted.")?;
                }
                (Err(failure), None)
            }
        };
        let had_block = current.as_ref().is_some_and(|quota| {
            quota.status == QuotaStatus::Exhausted
                || (matches!(quota.status, QuotaStatus::Fresh | QuotaStatus::Stale)
                    && quota
                        .windows
                        .iter()
                        .any(|window| window.remaining_percent == Some(0.0)))
        });
        match windows {
            Ok(windows) => {
                let group = authenticated_group
                    .ok_or("Quota report has no authenticated account binding.")?;
                let exhausted = windows.iter().any(|w| w.remaining_percent == Some(0.0));
                if !exhausted && blocked_groups.contains(group) {
                    continue;
                }
                let incoming = Quota {
                    profile_id: profile.id.clone(),
                    status: if exhausted {
                        QuotaStatus::Exhausted
                    } else {
                        QuotaStatus::Fresh
                    },
                    windows: windows.clone(),
                    observed_at: observed,
                    expires_at: observed + 30_000,
                    epoch,
                    block_revision: report_block,
                };
                let recovered = super::policy::apply_group_recovery(
                    snapshot,
                    baseline,
                    &profile.id,
                    group,
                    &incoming,
                    observed,
                );
                let accepted = if !recovered.is_empty() {
                    true
                } else if let Some(mut existing) = current {
                    if super::policy::apply_quota(&mut existing, incoming.clone(), report_block) {
                        snapshot.quota.retain(|q| q.profile_id != profile.id);
                        snapshot.quota.push(existing);
                        true
                    } else {
                        false
                    }
                } else {
                    snapshot.quota.push(incoming);
                    true
                };
                if accepted {
                    if exhausted {
                        let next_block = report_block
                            .checked_add(1)
                            .ok_or("Quota block revision exhausted.")?
                            .max(epoch);
                        if let Some(quota) = snapshot
                            .quota
                            .iter_mut()
                            .find(|q| q.profile_id == profile.id)
                        {
                            quota.block_revision = next_block;
                        }
                    }
                    if !recovered.is_empty() {
                        for run in &mut snapshot.runs {
                            run.attempted_profile_ids
                                .retain(|id| !recovered.contains(id));
                        }
                    }
                    if had_block && !exhausted {
                        for run in &mut snapshot.runs {
                            run.attempted_profile_ids.retain(|id| id != &profile.id);
                        }
                    }
                }
            }
            Err(failure) => {
                if had_block {
                    continue;
                }
                let status = match failure {
                    super::usage::CodexQuotaError::Throttled { .. } => QuotaStatus::ReaderThrottled,
                    super::usage::CodexQuotaError::Unsupported
                    | super::usage::CodexQuotaError::UnknownScope => QuotaStatus::Unsupported,
                    _ => QuotaStatus::ReaderError,
                };
                let windows = current.map_or_else(Vec::new, |q| q.windows);
                snapshot.quota.retain(|q| q.profile_id != profile.id);
                snapshot.quota.push(Quota {
                    profile_id: profile.id.clone(),
                    status,
                    windows,
                    observed_at: observed,
                    expires_at: observed,
                    epoch,
                    block_revision: report_block,
                });
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn cli_router_closing(
    window: Window,
    state: State<'_, CliRouterService>,
    closing: bool,
) -> Result<(), String> {
    main_window(&window)?;
    // The owner mutex serializes shutdown with the last dispatch fence.
    let _guard = state
        .inner
        .lock()
        .map_err(|_| "Router service is unavailable.")?;
    state.closing.store(closing, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub(crate) async fn cli_router_drain(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
) -> Result<(), String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.drain(&app))
        .await
        .map_err(|_| "Router shutdown task failed.")?
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StartRun {
    request_id: String,
    router_id: String,
    cwd: String,
    shell_profile_id: String,
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
    // Preserve the digest of committed legacy text-creation requests.
    #[serde(default, skip_serializing_if = "RunExecutionMode::is_text")]
    execution_mode: RunExecutionMode,
    title: String,
}

#[tauri::command]
pub(crate) async fn cli_run_start(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    shells: State<'_, Shells>,
    request: StartRun,
) -> Result<View, String> {
    main_window(&window)?;
    if request.request_id.len() != 36
        || !request.request_id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        })
        || request.request_id.as_bytes()[14] != b'4'
        || !b"89ab".contains(&request.request_id.as_bytes()[19])
    {
        return Err("Create a fresh run request identifier.".into());
    }
    let state = state.inner().clone();
    let shells = shells.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if state.closing.load(Ordering::SeqCst) { return Err("Lomi is preparing to close.".into()); }
        runtime::shell_profile(&shells, &request.shell_profile_id)?;
        let cwd = crate::files::directory(&request.cwd)?.to_string_lossy().into_owned();
        if let Some(model) = &request.model {
            if model.is_empty() || model.len() > 200 || !model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._:/".contains(&b)) { return Err("Enter a valid model ID.".into()); }
        }
        let result = state.with_store(&app, |owner| owner.store.mutate(&format!("main:start:{}", request.request_id), &request, |snapshot| {
            if state.closing.load(Ordering::SeqCst) { return Err("Lomi is preparing to close.".into()); }
            let router = snapshot.routers.iter().find(|r| r.id == request.router_id && r.enabled).ok_or("Enable this router in Settings first.")?;
            let capability = adapters::adapter(router.cli).capability;
            let allowed_profile_ids = if request.execution_mode == RunExecutionMode::Gateway {
                if !capability.gateway_terminal { return Err("This CLI has no admitted native gateway terminal.".into()); }
                if request.reasoning_effort.is_some() { return Err("Choose reasoning preferences inside the native gateway CLI.".into()); }
                super::gateway_profiles::admit_project(router.cli, std::path::Path::new(&cwd))?;
                super::gateway_config::pool(snapshot, router, &state)?.into_iter().map(|profile| profile.id).collect()
            } else if request.execution_mode == RunExecutionMode::Native {
                if !capability.native_turns { return Err("This CLI requires its native interactive terminal for subscription coding.".into()); }
                router.ordered_profile_ids.iter().filter(|id| snapshot.profiles.iter().any(|profile| profile.id == **id && profile.enabled && profile.gateway_provider.is_none() && profile.storage_mode != StorageMode::ApiKey && matches!(profile.auth_state, AuthState::Ready | AuthState::Unverified))).cloned().collect()
            } else {
                if !capability.managed_turns { return Err(capability.reason); }
                router.ordered_profile_ids.iter().filter(|id| snapshot.profiles.iter().any(|profile| profile.id == **id && profile.gateway_provider.is_none())).cloned().collect()
            };
            if request.execution_mode == RunExecutionMode::Coding && (router.cli != TitleCli::Codex || !cfg!(unix)) {
                return Err("Mediated coding runs require the pinned native Codex client on macOS or Linux.".into());
            }
            if request.model.as_ref().is_none_or(|model| model.trim().is_empty()) { return Err("Choose the model before creating this run.".into()); }
            if request.execution_mode != RunExecutionMode::Gateway && router.cli == TitleCli::Codex && request.reasoning_effort.as_deref().is_none_or(|effort| !["low", "medium", "high", "xhigh"].contains(&effort)) {
                return Err("Choose the Codex reasoning effort explicitly before creating this run.".into());
            }
            if router.cli != TitleCli::Codex && request.reasoning_effort.is_some() {
                return Err("This CLI has no qualified configurable reasoning effort.".into());
            }
            if snapshot.runs.len() >= 128 { return Err("Router history is full. Export or remove old runs before starting another.".into()); }
            if snapshot.runs.iter().any(|run| run.id == request.request_id) { return Err("This run identifier already exists.".into()); }
            // Bind the result to its idempotent creation request. The UI must
            // not infer ownership from unrelated runs in a newer snapshot.
            snapshot.runs.push(Run { id: request.request_id.clone(), router_id: router.id.clone(), cwd: cwd.clone(), shell_profile_id: Some(request.shell_profile_id.clone()), title: label(&request.title)?, state: RunState::Idle, model: request.model.clone(), reasoning_effort: request.reasoning_effort.clone(), execution_mode: request.execution_mode, continuation_requested: false, pinned_profile_id: None, allowed_profile_ids, active_profile_id: None, generation: 0, revision: 1, inputs: vec![], attempts: vec![], turns: vec![], legacy_output: None, output: String::new(), status_message: if request.execution_mode == RunExecutionMode::Gateway { "Ready for an explicit native CLI terminal start. Selected API accounts may receive the full native context; tools and approvals remain native.".into() } else { "Ready. Accounts added to the pool later will not receive this run's history.".into() }, attempted_profile_ids: vec![] });
            Ok(())
        }))?;
        state.changed(&app, result.snapshot.revision); Ok(view(result.snapshot, false))
    }).await.map_err(|_| "Could not create the router run.")?
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SendRun {
    request_id: String,
    run_id: String,
    expected_revision: u64,
    text: String,
    #[serde(default)]
    handoff: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RemoveRun {
    request_id: String,
    run_id: String,
    expected_revision: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AcknowledgeCodingCompletion {
    request_id: String,
    run_id: String,
    expected_run_revision: u64,
}

/// A completed native checkpoint can be acknowledged after a failed drain.
/// This operation never launches a CLI or redispatches the saved input.
#[tauri::command]
pub(crate) async fn cli_run_acknowledge_coding_completion(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: AcknowledgeCodingCompletion,
) -> Result<View, String> {
    main_window(&window)?;
    #[cfg(not(unix))]
    {
        let _ = (app, state, request);
        Err("Coding checkpoints are unavailable on this platform.".into())
    }
    #[cfg(unix)]
    {
        let state = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            use std::os::unix::fs::MetadataExt;
            let saved = state.with_store(&app, |owner| {
                if state.active.lock().map_err(|_| "Router ownership unavailable.")?.contains_key(&request.run_id)
                    || state.draining.lock().map_err(|_| "Router close status unavailable.")?.contains(&request.run_id) {
                    return Err("Wait for this coding process to finish before acknowledging its checkpoint.".into());
                }
                let current = owner.store.snapshot()?;
                let run = current.runs.iter().find(|run| run.id == request.run_id).ok_or("Run no longer exists.")?;
                if run.revision != request.expected_run_revision || !matches!(run.execution_mode, RunExecutionMode::Coding | RunExecutionMode::Native)
                    || !matches!(run.state, RunState::RecoveryRequired | RunState::Stopped | RunState::Paused) {
                    return Err("Refresh and review the saved coding outcome before acknowledging it.".into());
                }
                if run.execution_mode == RunExecutionMode::Native {
                    super::native_runtime::completed_checkpoint(owner, &current, run)?;
                } else {
                let input = run.inputs.last().ok_or("There is no saved coding input.")?;
                let root = owner.root.join("runs").join(&run.id).join("coding");
                if !root.join("coding.sqlite3").is_file() { return Err("This run has no retained native coding journal to acknowledge.".into()); }
                let cwd = crate::files::directory(&run.cwd)?;
                let metadata = std::fs::symlink_metadata(&cwd).map_err(|_| "Cannot verify the coding project.")?;
                let identity = serde_json::json!({"path":cwd,"device":metadata.dev(),"inode":metadata.ino()});
                let journal = super::coding_journal::Journal::open(&root, &run.id, &identity,
                    run.model.as_deref().ok_or("Missing coding model.")?, run.reasoning_effort.as_deref().ok_or("Missing coding reasoning effort.")?)?;
                if !journal.completed_input(&input.id)? {
                    return Err("This input has no complete, unambiguous native checkpoint. Its retained observations require recovery; the task will not be replayed.".into());
                }
                }
                owner.store.mutate(&format!("main:ack-coding:{}", request.request_id), &request, |snapshot| {
                    let run = snapshot.runs.iter_mut().find(|run| run.id == request.run_id).ok_or("Run no longer exists.")?;
                    if run.revision != request.expected_run_revision { return Err("Run changed. Refresh and review it again.".into()); }
                    run.state = RunState::Idle;
                    run.continuation_requested = false;
                    run.attempted_profile_ids.clear();
                    run.revision = run.revision.checked_add(1).ok_or("Run revision exhausted.")?;
                    run.status_message = "The completed native checkpoint was acknowledged. Ready for a new message; no task was launched.".into();
                    Ok(())
                })
            })?;
            state.changed(&app, saved.snapshot.revision);
            Ok(view(saved.snapshot, false))
        }).await.map_err(|_| "Could not acknowledge the coding checkpoint.")?
    }
}

#[tauri::command]
pub(crate) async fn cli_run_remove(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: RemoveRun,
) -> Result<View, String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let saved = state.with_store(&app, |owner| {
            owner.store.mutate(
                &format!("main:remove:{}", request.request_id),
                &request,
                |snapshot| {
                    let run = snapshot
                        .runs
                        .iter()
                        .find(|run| run.id == request.run_id)
                        .ok_or("Run no longer exists.")?;
                    if run.execution_mode == RunExecutionMode::Native && run.active_profile_id.as_deref().is_some_and(|id| super::profile_writer_busy(&state, id).unwrap_or(true)) {
                        return Err("Close the original account's native recovery terminal before removing this run.".into());
                    }
                    if run.revision != request.expected_revision {
                        return Err("Run changed. Refresh and try again.".into());
                    }
                    if state
                        .draining
                        .lock()
                        .map_err(|_| "Router close status unavailable.")?
                        .contains(&run.id)
                    {
                        return Err(
                            "Wait for the run to finish closing before removing history.".into(),
                        );
                    }
                    if matches!(
                        run.state,
                        RunState::Starting | RunState::Running | RunState::Switching
                    ) || state
                        .active
                        .lock()
                        .map_err(|_| "Router ownership unavailable.")?
                        .contains_key(&run.id)
                    {
                        return Err(
                            "Stop this run and wait for its CLI to finish before removing history."
                                .into(),
                        );
                    }
                    snapshot.runs.retain(|run| run.id != request.run_id);
                    Ok(())
                },
            )
        })?;
        state.changed(&app, saved.snapshot.revision);
        Ok(view(saved.snapshot, false))
    })
    .await
    .map_err(|_| "Could not remove the run from history.")?
}

#[tauri::command]
pub(crate) async fn cli_run_send(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    shells: State<'_, Shells>,
    request: SendRun,
) -> Result<View, String> {
    main_window(&window)?;
    if request.text.len() > 64 * 1024 || (!request.handoff && request.text.trim().is_empty()) {
        return Err("Enter a message up to 64 KiB.".into());
    }
    let state = state.inner().clone();
    let shells = shells.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut reserved = false;
        let result = state.with_store(&app, |owner| {
            if state.closing.load(Ordering::SeqCst) { return Err("Lomi is preparing to close.".into()); }
            if state.draining.lock().map_err(|_| "Router close status unavailable.")?.contains(&request.run_id) {
                return Err("This run is closing. Wait for its CLI to finish.".into());
            }
            let result = owner.store.mutate(&format!("main:send:{}", request.request_id), &request, |snapshot| {
                let index = snapshot.runs.iter().position(|r| r.id == request.run_id).ok_or("Run no longer exists.")?;
                let run = &snapshot.runs[index];
                if run.execution_mode == RunExecutionMode::Gateway {
                    return Err("Open this run's CLI terminal to send native messages.".into());
                }
                if run.execution_mode == RunExecutionMode::Coding && request.text.len() > 60 * 1024 {
                    return Err("Coding messages are limited to 60 KiB of UTF-8 text.".into());
                }
                if run.revision != request.expected_revision { return Err("Run changed. Refresh and try again.".into()); }
                if matches!(run.state, RunState::Running | RunState::Starting | RunState::Switching) { return Err("Wait for the current turn or stop it first.".into()); }
                if !run.inputs.is_empty() && matches!(run.state, RunState::RecoveryRequired | RunState::WaitingForCapacity | RunState::Stopped | RunState::Paused) && !request.handoff { return Err("Review the saved outcome and explicitly continue before sending again.".into()); }
                if request.handoff && !request.text.trim().is_empty() { return Err("Recovery retains the original message. Send a new message after recovery completes.".into()); }
                if run.output.len() > runtime::MAX_OUTPUT || (!request.handoff && run.inputs.len() >= 128) { return Err("Run history reached its limit. Start a new run with a summary.".into()); }
                let router = snapshot.routers.iter().find(|r| r.id == run.router_id).ok_or("Router no longer exists.")?;
                let capability = adapters::adapter(router.cli).capability;
                if !(if run.execution_mode == RunExecutionMode::Native { capability.native_turns } else { capability.managed_turns }) { return Err(capability.reason); }
                let run = &mut snapshot.runs[index];
                if !request.handoff { run.inputs.push(RunInput { id: request.request_id.clone(), text: request.text.clone() }); run.attempted_profile_ids.clear(); }
                // The explicit recovery action authorizes one new attempt with
                // reviewed context. Durable prior attempts and quota blocks stay.
                if request.handoff { run.attempted_profile_ids.clear(); }
                run.continuation_requested = request.handoff && matches!(run.execution_mode, RunExecutionMode::Coding | RunExecutionMode::Native);
                if run.inputs.is_empty() { return Err("There is no saved message to continue.".into()); }
                run.state = RunState::Starting; run.revision += 1;
                run.status_message = if request.handoff && run.execution_mode == RunExecutionMode::Coding { "Resuming the saved native coding thread after verifying its durable checkpoint.".into() } else if request.handoff { "Continuing in a new CLI conversation with the saved context you approved.".into() } else { "Selecting an account…".into() };
                runtime::reserve(&state, &request.run_id)?;
                reserved = true;
                Ok(())
            });
            if result.is_err() && reserved { if let Ok(mut active) = state.active.lock() { active.remove(&request.run_id); } }
            result
        })?;
        state.changed(&app, result.snapshot.revision);
        if !result.replayed {
            if let Err(error) = runtime::launch(state.clone(), app.clone(), shells, request.run_id.clone()) {
                let snapshot = state.with_store(&app, |owner| owner.store.update(|snapshot| {
                    let run = snapshot.runs.iter_mut().find(|r| r.id == request.run_id).ok_or("Run no longer exists.")?;
                    run.state = RunState::RecoveryRequired; run.revision += 1; run.status_message = error.clone(); Ok(())
                }))?;
                state.changed(&app, snapshot.revision);
                return Err(error);
            }
        }
        Ok(view(result.snapshot, false))
    }).await.map_err(|_| "Could not send the router message.")?
}

struct RunDrainLease {
    state: CliRouterService,
    run_id: String,
    retained: bool,
}

impl Drop for RunDrainLease {
    fn drop(&mut self) {
        if !self.retained {
            if let Ok(mut draining) = self.state.draining.lock() {
                draining.remove(&self.run_id);
            }
        }
    }
}

#[tauri::command]
pub(crate) async fn cli_run_drain(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    run_id: String,
) -> Result<View, String> {
    main_window(&window)?;
    let state = state.inner().clone();
    // Cancellation must remain available even if storage cannot currently save.
    if let Some(cancel) = state
        .active
        .lock()
        .map_err(|_| "Router ownership unavailable.")?
        .get(&run_id)
    {
        cancel.store(true, Ordering::SeqCst);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut lease = state.with_store(&app, |owner| {
            if !owner.store.snapshot()?.runs.iter().any(|run| run.id == run_id) {
                return Err("Run no longer exists.".into());
            }
            if !state.draining.lock().map_err(|_| "Router close status unavailable.")?.insert(run_id.clone()) {
                return Err("This run is already closing.".into());
            }
            let lease = RunDrainLease { state: state.clone(), run_id: run_id.clone(), retained: false };
            if let Some(cancel) = state.active.lock().map_err(|_| "Router ownership unavailable.")?.get(&run_id) {
                cancel.store(true, Ordering::SeqCst);
            }
            Ok(lease)
        })?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if !state.active.lock().map_err(|_| "Router ownership unavailable.")?.contains_key(&run_id) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("The CLI has not confirmed shutdown. Its saved panel remains open; stop the run and retry.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Worker ownership ends only after process cleanup and its final save.
        // with_store also fences and persists interrupted attempts after a save failure.
        let snapshot = state.with_store(&app, |owner| owner.store.snapshot())?;
        let run = snapshot.runs.iter().find(|run| run.id == run_id).ok_or("Run no longer exists.")?;
        if run.attempts.iter().any(|attempt| attempt.state.is_active())
            || matches!(run.state, RunState::Starting | RunState::Running | RunState::Switching)
        {
            return Err("The run still has an unresolved attempt. Its panel remains open.".into());
        }
        // The caller holds this fence through other view guards and the domain
        // removal. Explicit release handles cancellation or final reference removal.
        lease.retained = true;
        Ok(view(snapshot, false))
    }).await.map_err(|_| "Could not finish closing the CLI run.")?
}

#[tauri::command]
pub(crate) fn cli_run_close_release(
    window: Window,
    state: State<'_, CliRouterService>,
    run_ids: Vec<String>,
) -> Result<(), String> {
    main_window(&window)?;
    if run_ids.len() > 128 {
        return Err("Too many closing runs.".into());
    }
    // Serialize temporary close admission with the final process dispatch fence.
    let _owner = state
        .inner
        .lock()
        .map_err(|_| "Router service is unavailable.")?;
    let mut draining = state
        .draining
        .lock()
        .map_err(|_| "Router close status unavailable.")?;
    for id in run_ids {
        draining.remove(&id);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn cli_run_stop(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    run_id: String,
) -> Result<View, String> {
    main_window(&window)?;
    // Cancellation remains available if persistence or reconciliation fails.
    if let Some(cancel) = state
        .active
        .lock()
        .map_err(|_| "Router ownership unavailable.")?
        .get(&run_id)
    {
        cancel.store(true, Ordering::SeqCst);
    }
    let snapshot = state.with_store(&app, |owner| {
        if let Some(cancel) = state
            .active
            .lock()
            .map_err(|_| "Router service is unavailable.")?
            .get(&run_id)
        {
            cancel.store(true, Ordering::SeqCst);
        }
        owner.store.update(|snapshot| {
            let run = snapshot
                .runs
                .iter_mut()
                .find(|r| r.id == run_id)
                .ok_or("Run no longer exists.")?;
            run.revision += 1;
            if matches!(
                run.state,
                RunState::Running | RunState::Starting | RunState::Switching
            ) {
                run.status_message =
                    "Stopping the CLI. Review any partial work before continuing.".into();
            } else {
                run.state = RunState::Paused;
            }
            Ok(())
        })
    })?;
    state.changed(&app, snapshot.revision);
    Ok(view(snapshot, false))
}

#[tauri::command]
pub(crate) async fn cli_run_update_data_grant(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: super::grants::UpdateDataGrant,
) -> Result<View, String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let saved = state.with_store(&app, |owner| {
            if state.closing.load(Ordering::SeqCst)
                || state.draining.lock().map_err(|_| "Router close status unavailable.")?.contains(&request.run_id)
                || state.active.lock().map_err(|_| "Router ownership unavailable.")?.contains_key(&request.run_id)
            {
                return Err("Wait for this run to finish stopping or closing before changing account access.".into());
            }
            owner.store.mutate(&format!("main:grant:{}", request.request_id), &request, |snapshot| {
                super::grants::apply(snapshot, &request, |id| super::profile_writer_busy(&state, id))
            })
        })?;
        state.changed(&app, saved.snapshot.revision);
        Ok(view(saved.snapshot, false))
    }).await.map_err(|_| "Could not update saved-history access.")?
}

#[tauri::command]
pub(crate) fn cli_run_pin(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    run_id: String,
    profile_id: Option<String>,
) -> Result<View, String> {
    main_window(&window)?;
    let snapshot = state.with_store(&app, |owner| {
        if state.closing.load(Ordering::SeqCst)
            || state
                .draining
                .lock()
                .map_err(|_| "Router close status unavailable.")?
                .contains(&run_id)
            || state
                .active
                .lock()
                .map_err(|_| "Router ownership unavailable.")?
                .contains_key(&run_id)
        {
            return Err(
                "Wait for this run to finish stopping or closing before selecting an account."
                    .into(),
            );
        }
        owner.store.update(|snapshot| {
            let run = snapshot
                .runs
                .iter_mut()
                .find(|r| r.id == run_id)
                .ok_or("Run no longer exists.")?;
            if matches!(
                run.state,
                RunState::Starting | RunState::Running | RunState::Switching
            ) || run.attempts.iter().any(|attempt| attempt.state.is_active())
            {
                return Err("Stop this run before selecting its next account.".into());
            }
            if profile_id
                .as_ref()
                .is_some_and(|id| !run.allowed_profile_ids.contains(id))
            {
                return Err("This account is outside the run's approved pool.".into());
            }
            let revision = run
                .revision
                .checked_add(1)
                .ok_or("Run revision exhausted.")?;
            run.pinned_profile_id = profile_id;
            run.revision = revision;
            Ok(())
        })
    })?;
    state.changed(&app, snapshot.revision);
    Ok(view(snapshot, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline(blocked: bool) -> Snapshot {
        let mut snapshot = Snapshot::default();
        for id in ["a", "b"] {
            snapshot.profiles.push(Profile {
                id: id.into(),
                cli: TitleCli::Codex,
                label: id.into(),
                enabled: true,
                revision: 1,
                credential_ref: None,
                quota_group_key: Some("shared".into()),
                gateway_provider: None,
                auth_state: AuthState::Ready,
                storage_mode: StorageMode::Keyring,
            });
            snapshot.quota.push(Quota {
                profile_id: id.into(),
                status: if blocked {
                    QuotaStatus::Exhausted
                } else {
                    QuotaStatus::Fresh
                },
                windows: vec![QuotaWindow {
                    id: "window".into(),
                    remaining_percent: Some(if blocked { 0.0 } else { 80.0 }),
                    reset_at: None,
                }],
                observed_at: 1,
                expires_at: 1_000,
                epoch: 1,
                block_revision: 7,
            });
        }
        snapshot
    }

    fn reports(snapshot: &Snapshot, values: [f64; 2]) -> Vec<QuotaResult> {
        snapshot
            .profiles
            .iter()
            .zip(values)
            .map(|(profile, value)| {
                (
                    profile.clone(),
                    snapshot
                        .quota
                        .iter()
                        .find(|q| q.profile_id == profile.id)
                        .unwrap()
                        .block_revision,
                    Ok(super::super::usage::AuthenticatedCodexQuota {
                        windows: Ok(vec![QuotaWindow {
                            id: "window".into(),
                            remaining_percent: Some(value),
                            reset_at: None,
                        }]),
                        group_key: "shared".into(),
                    }),
                )
            })
            .collect()
    }

    fn unknown_report(profile: &Profile, block: u64, group: &str) -> QuotaResult {
        (
            profile.clone(),
            block,
            Ok(super::super::usage::AuthenticatedCodexQuota {
                group_key: group.into(),
                windows: Err(super::super::usage::CodexQuotaError::UnknownScope),
            }),
        )
    }

    #[test]
    fn authenticated_unknown_quota_verifies_distinct_groups_without_percentages() {
        let mut baseline = baseline(false);
        baseline.quota.clear();
        for profile in &mut baseline.profiles {
            profile.quota_group_key = None;
            profile.auth_state = AuthState::Unverified;
        }
        let results = vec![
            unknown_report(&baseline.profiles[0], 0, "group-a"),
            unknown_report(&baseline.profiles[1], 0, "group-b"),
        ];
        let mut current = baseline.clone();
        apply_quota_results(
            &mut current,
            &baseline,
            &results,
            10,
            1_000,
            &HashSet::new(),
        )
        .unwrap();
        assert!(current
            .profiles
            .iter()
            .all(|profile| profile.auth_state == AuthState::Ready));
        assert_eq!(
            current.profiles[0].quota_group_key.as_deref(),
            Some("group-a")
        );
        assert_eq!(
            current.profiles[1].quota_group_key.as_deref(),
            Some("group-b")
        );
        assert!(current
            .quota
            .iter()
            .all(|quota| { quota.status == QuotaStatus::Unsupported && quota.windows.is_empty() }));
        assert_eq!(
            ready_bindings(&current, TitleCli::Codex, &["a".into(), "b".into()]),
            2
        );
    }

    fn granted_run() -> Run {
        Run {
            id: "run".into(),
            router_id: "router".into(),
            cwd: "/project".into(),
            shell_profile_id: None,
            title: "Task".into(),
            state: RunState::Paused,
            model: None,
            reasoning_effort: None,
            execution_mode: RunExecutionMode::Text,
            continuation_requested: false,
            pinned_profile_id: None,
            allowed_profile_ids: vec!["a".into(), "b".into()],
            active_profile_id: None,
            generation: 1,
            revision: 1,
            inputs: vec![],
            attempts: vec![],
            turns: vec![],
            legacy_output: None,
            output: String::new(),
            status_message: String::new(),
            attempted_profile_ids: vec!["a".into()],
        }
    }

    #[test]
    fn same_identity_unknown_quota_preserves_block_and_attempt_budget() {
        for status in [
            QuotaStatus::Exhausted,
            QuotaStatus::Fresh,
            QuotaStatus::Stale,
        ] {
            let mut baseline = baseline(true);
            baseline.quota[0].status = status;
            baseline.runs.push(granted_run());
            let results = vec![unknown_report(&baseline.profiles[0], 7, "shared")];
            let mut current = baseline.clone();
            apply_quota_results(
                &mut current,
                &baseline,
                &results,
                10,
                1_000,
                &HashSet::new(),
            )
            .unwrap();
            assert_eq!(current, baseline);
        }
    }

    #[test]
    fn first_authenticated_binding_does_not_inherit_unbound_quota() {
        let mut baseline = baseline(true);
        baseline.profiles[0].quota_group_key = None;
        baseline.profiles[0].auth_state = AuthState::Unverified;
        let results = vec![unknown_report(&baseline.profiles[0], 7, "authenticated")];
        let mut current = baseline.clone();
        apply_quota_results(
            &mut current,
            &baseline,
            &results,
            10,
            1_000,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            current.profiles[0].quota_group_key.as_deref(),
            Some("authenticated")
        );
        assert_eq!(current.profiles[0].auth_state, AuthState::Ready);
        assert_eq!(current.profiles[0].revision, baseline.profiles[0].revision);
        let quota = current
            .quota
            .iter()
            .find(|quota| quota.profile_id == "a")
            .unwrap();
        assert_eq!(quota.status, QuotaStatus::Unsupported);
        assert_eq!(quota.block_revision, 0);
        assert!(quota.windows.is_empty());
    }

    #[test]
    fn different_authenticated_identity_with_unknown_quota_drops_old_blocks_and_grants() {
        let mut baseline = baseline(true);
        baseline.runs.push(granted_run());
        let results = vec![unknown_report(&baseline.profiles[0], 7, "replacement")];
        let mut current = baseline.clone();
        apply_quota_results(
            &mut current,
            &baseline,
            &results,
            10,
            1_000,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            current.profiles[0].quota_group_key.as_deref(),
            Some("replacement")
        );
        assert_eq!(current.profiles[0].auth_state, AuthState::Ready);
        assert_eq!(
            current.profiles[0].revision,
            baseline.profiles[0].revision + 1
        );
        let quota = current
            .quota
            .iter()
            .find(|quota| quota.profile_id == "a")
            .unwrap();
        assert_eq!(quota.status, QuotaStatus::Unsupported);
        assert!(quota.windows.is_empty());
        assert_eq!(quota.block_revision, 0);
        assert_eq!(
            current
                .quota
                .iter()
                .find(|quota| quota.profile_id == "b")
                .unwrap(),
            &baseline.quota[1]
        );
        assert_eq!(current.runs[0].allowed_profile_ids, ["b"]);
        assert_eq!(current.runs[0].revision, baseline.runs[0].revision + 1);
        assert_eq!(
            current.runs[0].attempted_profile_ids,
            baseline.runs[0].attempted_profile_ids
        );
    }

    #[test]
    fn unfenced_unknown_quota_cannot_publish_an_authenticated_binding() {
        for race in 0..5 {
            let baseline = baseline(false);
            let results = vec![unknown_report(&baseline.profiles[0], 7, "replacement")];
            let mut current = baseline.clone();
            let mut busy = HashSet::new();
            match race {
                0 => {
                    busy.insert("a".into());
                }
                1 => current.profiles[0].revision += 1,
                2 => current.quota[0].block_revision += 1,
                3 => current.quota[0].epoch = 11,
                _ => current.quota[0].observed_at = 1_001,
            }
            let before = current.clone();
            apply_quota_results(&mut current, &baseline, &results, 10, 1_000, &busy).unwrap();
            assert_eq!(current, before);
        }
    }

    #[test]
    fn outer_fetch_failure_never_publishes_an_authenticated_binding() {
        let mut baseline = baseline(false);
        baseline.profiles[0].quota_group_key = None;
        baseline.profiles[0].auth_state = AuthState::Unverified;
        baseline.quota.clear();
        let results = vec![(
            baseline.profiles[0].clone(),
            0,
            Err(super::super::usage::CodexQuotaError::Unavailable),
        )];
        let mut current = baseline.clone();
        apply_quota_results(
            &mut current,
            &baseline,
            &results,
            10,
            1_000,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(current.profiles[0], baseline.profiles[0]);
        assert_eq!(current.quota[0].status, QuotaStatus::ReaderError);
        assert!(current.quota[0].windows.is_empty());
    }

    #[test]
    fn mixed_shared_group_batch_keeps_confirmed_zero_in_either_order() {
        for previously_blocked in [false, true] {
            for reversed in [false, true] {
                let baseline = baseline(previously_blocked);
                let mut current = baseline.clone();
                let mut results = reports(&baseline, [90.0, 0.0]);
                if reversed {
                    results.reverse();
                }
                apply_quota_results(
                    &mut current,
                    &baseline,
                    &results,
                    10,
                    1_000,
                    &HashSet::new(),
                )
                .unwrap();
                let zero = current.quota.iter().find(|q| q.profile_id == "b").unwrap();
                assert_eq!(zero.status, QuotaStatus::Exhausted);
                assert_eq!(zero.block_revision, 10);
                assert_eq!(zero.epoch, 10);
                // A later authenticated collection can recover the entire group;
                // a positive sibling from the contradictory collection cannot.
                let after_block = current.clone();
                let next = reports(&after_block, [90.0, 90.0]);
                apply_quota_results(
                    &mut current,
                    &after_block,
                    &next,
                    20,
                    2_000,
                    &HashSet::new(),
                )
                .unwrap();
                assert!(current.quota.iter().all(|q| q.status == QuotaStatus::Fresh
                    && q.windows[0].remaining_percent == Some(90.0)));
            }
        }
    }

    #[test]
    fn late_positive_collection_cannot_release_a_new_group_block() {
        let baseline = baseline(false);
        let mut current = baseline.clone();
        let zero = reports(&baseline, [90.0, 0.0]);
        apply_quota_results(&mut current, &baseline, &zero, 10, 1_000, &HashSet::new()).unwrap();
        let late = reports(&baseline, [90.0, 90.0]);
        apply_quota_results(&mut current, &baseline, &late, 9, 1_001, &HashSet::new()).unwrap();
        assert_eq!(
            current
                .quota
                .iter()
                .find(|q| q.profile_id == "b")
                .unwrap()
                .status,
            QuotaStatus::Exhausted
        );
    }

    #[test]
    fn stale_or_busy_zero_does_not_override_a_fenced_positive_report() {
        for busy_profile in [false, true] {
            let baseline = baseline(false);
            let mut current = baseline.clone();
            let results = reports(&baseline, [90.0, 0.0]);
            let mut busy = HashSet::new();
            if busy_profile {
                busy.insert("b".into());
            } else {
                current.profiles[1].revision += 1;
            }
            apply_quota_results(&mut current, &baseline, &results, 10, 1_000, &busy).unwrap();
            let positive = current.quota.iter().find(|q| q.profile_id == "a").unwrap();
            let unchanged = current.quota.iter().find(|q| q.profile_id == "b").unwrap();
            assert_eq!(positive.windows[0].remaining_percent, Some(90.0));
            assert_eq!(unchanged, &baseline.quota[1]);
        }
    }

    #[test]
    fn create_router_applies_reviewed_options_atomically() {
        let mut snapshot = baseline(false);
        snapshot.profiles[1].quota_group_key = Some("other".into());
        let ids = vec!["b".into(), "a".into()];
        create_router(
            &mut snapshot,
            TitleCli::Codex,
            "  Daily  ",
            &ids,
            true,
            true,
        )
        .unwrap();
        let router = &snapshot.routers[0];
        assert_eq!(router.label, "Daily");
        assert_eq!(router.ordered_profile_ids, ids);
        assert!(router.enabled);
        assert!(router.balance_remaining_quota);
        assert_eq!(router.revision, 1);
    }

    #[test]
    fn create_router_rejects_unqualified_options_without_creating_a_router() {
        let mut snapshot = baseline(false);
        let before = serde_json::to_value(&snapshot).unwrap();
        let ids = vec!["a".into(), "b".into()];
        assert!(create_router(&mut snapshot, TitleCli::Codex, "Daily", &ids, true, false).is_err());
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), before);
        assert!(create_router(
            &mut snapshot,
            TitleCli::Codex,
            "Daily",
            &["a".into()],
            true,
            false
        )
        .is_err());
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), before);

        for profile in &mut snapshot.profiles {
            profile.cli = TitleCli::Pi;
        }
        let before = serde_json::to_value(&snapshot).unwrap();
        assert!(create_router(&mut snapshot, TitleCli::Pi, "Daily", &ids, false, true).is_err());
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), before);
        assert!(
            create_router(&mut snapshot, TitleCli::Codex, "Daily", &ids, false, false).is_err()
        );
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), before);
    }

    #[test]
    fn legacy_router_creation_defaults_to_disabled_account_order() {
        let payload = serde_json::json!({
            "type": "create_router",
            "cli": "codex",
            "label": "Draft",
            "orderedProfileIds": ["a"]
        });
        let action: Action = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(serde_json::to_value(&action).unwrap(), payload);
        let Action::CreateRouter {
            cli,
            label,
            ordered_profile_ids,
            enabled,
            balance_remaining_quota,
        } = action
        else {
            panic!("Expected a create-router action");
        };
        let mut snapshot = baseline(false);
        create_router(
            &mut snapshot,
            cli,
            &label,
            &ordered_profile_ids,
            enabled.unwrap_or(false),
            balance_remaining_quota.unwrap_or(false),
        )
        .unwrap();
        assert!(!snapshot.routers[0].enabled);
        assert!(!snapshot.routers[0].balance_remaining_quota);
    }

    #[test]
    fn disabling_allows_shrinking_a_pool_without_weakening_enable_checks() {
        let snapshot = terminal_pool();
        let router = &snapshot.routers[0];
        let ids = vec!["a".into()];
        assert!(router_reconfiguration(&snapshot, router, Some(false), None, Some(&ids)).is_ok());
        assert!(router_reconfiguration(&snapshot, router, Some(true), None, Some(&ids)).is_err());
        assert!(router_reconfiguration(&snapshot, router, None, None, Some(&ids)).is_err());
        assert!(router_reconfiguration(&snapshot, router, Some(false), None, Some(&[])).is_err());
    }

    #[test]
    fn enable_counts_verified_groups_and_qualified_key_bindings() {
        let mut snapshot = baseline(false);
        let ids = vec!["a".into(), "b".into()];
        assert_eq!(ready_bindings(&snapshot, TitleCli::Codex, &ids), 1);
        snapshot.profiles[1].quota_group_key = None;
        assert_eq!(ready_bindings(&snapshot, TitleCli::Codex, &ids), 2);
        snapshot.profiles[1].quota_group_key = Some("other".into());
        assert_eq!(ready_bindings(&snapshot, TitleCli::Codex, &ids), 2);
        for profile in &mut snapshot.profiles {
            profile.cli = TitleCli::Claude;
            profile.storage_mode = StorageMode::ApiKey;
        }
        assert_eq!(ready_bindings(&snapshot, TitleCli::Claude, &ids), 2);
        snapshot.profiles[1].enabled = false;
        assert_eq!(ready_bindings(&snapshot, TitleCli::Claude, &ids), 1);
    }

    #[test]
    fn native_pool_deduplicates_known_accounts_and_admits_unverified_profiles() {
        let ids = vec!["a".into(), "b".into()];
        for cli in [TitleCli::Codex, TitleCli::Claude, TitleCli::Pi] {
            let mut snapshot = baseline(false);
            for profile in &mut snapshot.profiles {
                profile.cli = cli;
                profile.auth_state = AuthState::Unverified;
            }
            assert_eq!(ready_bindings(&snapshot, cli, &ids), 1);
            snapshot.profiles[1].quota_group_key = None;
            assert_eq!(ready_bindings(&snapshot, cli, &ids), 2);
            snapshot.profiles[0].quota_group_key = None;
            assert_eq!(ready_bindings(&snapshot, cli, &ids), 2);
            snapshot.profiles[1].auth_state = AuthState::PendingRemove;
            assert_eq!(ready_bindings(&snapshot, cli, &ids), 1);
        }
    }

    #[test]
    fn ready_gateway_bindings_deduplicate_upstream_dispatch_aliases() {
        let mut snapshot = baseline(false);
        let ids = vec!["a".into(), "b".into()];
        for profile in &mut snapshot.profiles {
            profile.cli = TitleCli::Claude;
            profile.storage_mode = StorageMode::ApiKey;
            profile.credential_ref = Some("fixture-reference".into());
            profile.gateway_provider = Some(ApiDestination {
                protocol: super::super::gateway_profiles::Protocol::Anthropic,
                base_url: "https://example.com".into(),
            });
        }
        for (first, second) in [
            ("https://example.com", "https://example.com/v1"),
            ("https://example.com/api", "https://example.com/api/v1"),
        ] {
            snapshot.profiles[0]
                .gateway_provider
                .as_mut()
                .unwrap()
                .base_url = first.into();
            snapshot.profiles[1]
                .gateway_provider
                .as_mut()
                .unwrap()
                .base_url = second.into();
            snapshot.profiles[1].quota_group_key = Some("shared".into());
            assert_eq!(ready_bindings(&snapshot, TitleCli::Claude, &ids), 1);
            snapshot.profiles[1].quota_group_key = Some("different-key".into());
            assert_eq!(ready_bindings(&snapshot, TitleCli::Claude, &ids), 2);
        }
        snapshot.profiles[1].quota_group_key = Some("shared".into());
        for different in [
            "https://other.example/api/v1",
            "https://example.com/different/v1",
        ] {
            snapshot.profiles[1]
                .gateway_provider
                .as_mut()
                .unwrap()
                .base_url = different.into();
            assert_eq!(ready_bindings(&snapshot, TitleCli::Claude, &ids), 2);
        }
        snapshot.profiles[1]
            .gateway_provider
            .as_mut()
            .unwrap()
            .protocol = super::super::gateway_profiles::Protocol::OpenAiChat;
        assert_eq!(ready_bindings(&snapshot, TitleCli::Claude, &ids), 2);
        snapshot.profiles[1]
            .gateway_provider
            .as_mut()
            .unwrap()
            .protocol = super::super::gateway_profiles::Protocol::Anthropic;
        snapshot.profiles[1]
            .gateway_provider
            .as_mut()
            .unwrap()
            .base_url = "http://example.com".into();
        assert_eq!(ready_bindings(&snapshot, TitleCli::Claude, &ids), 1);
    }

    #[test]
    fn pending_removal_rejects_profile_edits() {
        let mut profile = baseline(false).profiles.remove(0);
        profile.auth_state = AuthState::PendingRemove;
        assert!(editable(&profile).is_err());
    }
    fn terminal_pool() -> Snapshot {
        let mut snapshot = baseline(false);
        snapshot.profiles[0].quota_group_key = Some("group-a".into());
        snapshot.profiles[1].quota_group_key = Some("group-b".into());
        snapshot.routers.push(Router {
            id: "pool".into(),
            cli: TitleCli::Codex,
            label: "Pool".into(),
            enabled: true,
            ordered_profile_ids: vec!["a".into(), "b".into()],
            balance_remaining_quota: false,
            revision: 1,
        });
        snapshot
    }

    #[test]
    fn pending_terminal_events_choose_from_the_latest_native_pool_at_spawn() {
        let mut snapshot = terminal_pool();
        let choose = |snapshot: &Snapshot, busy: &HashSet<String>| {
            select_terminal_profile(snapshot, None, TitleCli::Codex, Some("pool"), 10, |id| {
                Ok(busy.contains(id))
            })
        };
        let mut busy = HashSet::new();
        let first = choose(&snapshot, &busy).unwrap();
        assert_eq!(first, "a");
        // First spawn commits its unverified state and native writer before the
        // owner admits the second event. The event itself carries only pool ID.
        snapshot.profiles[0].auth_state = AuthState::Unverified;
        snapshot.profiles[0].quota_group_key = None;
        busy.insert(first);
        assert_eq!(choose(&snapshot, &busy).unwrap(), "b");
        busy.insert("b".into());
        assert!(choose(&snapshot, &busy).is_err());
    }

    #[test]
    fn late_terminal_selection_rejects_stale_membership_disabled_and_unknown_accounts() {
        let mut snapshot = terminal_pool();
        snapshot.routers[0].ordered_profile_ids.remove(0);
        assert_eq!(
            select_terminal_profile(&snapshot, None, TitleCli::Codex, Some("pool"), 10, |_| Ok(
                false
            ))
            .unwrap(),
            "b"
        );
        assert!(select_terminal_profile(
            &snapshot,
            Some("a"),
            TitleCli::Codex,
            Some("pool"),
            10,
            |_| Ok(false)
        )
        .is_err());
        snapshot.profiles[1].quota_group_key = None;
        assert!(select_terminal_profile(
            &snapshot,
            None,
            TitleCli::Codex,
            Some("pool"),
            10,
            |_| Ok(false)
        )
        .is_err());
        snapshot.profiles[1].quota_group_key = Some("group-b".into());
        snapshot.profiles[1].enabled = false;
        assert!(select_terminal_profile(
            &snapshot,
            None,
            TitleCli::Codex,
            Some("pool"),
            10,
            |_| Ok(false)
        )
        .is_err());
        snapshot.profiles[1].enabled = true;
        snapshot.routers[0].enabled = false;
        assert!(select_terminal_profile(
            &snapshot,
            None,
            TitleCli::Codex,
            Some("pool"),
            10,
            |_| Ok(false)
        )
        .is_err());
    }

    #[test]
    fn late_terminal_selection_preserves_explicit_profile_login_and_capacity_fences() {
        let mut snapshot = terminal_pool();
        snapshot.profiles[0].auth_state = AuthState::Disconnected;
        snapshot.profiles[0].quota_group_key = None;
        assert_eq!(
            select_terminal_profile(&snapshot, Some("a"), TitleCli::Codex, None, 10, |_| Ok(
                false
            ))
            .unwrap(),
            "a"
        );
        snapshot.quota[1].status = QuotaStatus::Exhausted;
        assert!(select_terminal_profile(
            &snapshot,
            None,
            TitleCli::Codex,
            Some("pool"),
            10,
            |_| Ok(false)
        )
        .is_err());
        assert!(select_terminal_profile(
            &snapshot,
            None,
            TitleCli::Claude,
            Some("pool"),
            10,
            |_| Ok(false)
        )
        .is_err());
    }
}
