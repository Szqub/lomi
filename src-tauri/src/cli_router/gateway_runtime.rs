//! Native CLI terminals with frozen API credentials and durable HTTP receipts.
//! Tools, permission prompts and native history remain owned by the CLI.
use super::{
    credentials, gateway_config,
    gateway_http::{self, Receipt, RequestSummary, Upstream, UpstreamOwner},
    gateway_profiles::{self, Protocol},
    gateway_session, gateway_store, runtime,
    types::*,
    CliRouterService,
};
use crate::{cli_catalog::TitleCli, terminal::Shells};
use std::{
    fs,
    io::Read,
    os::unix::{fs::MetadataExt, io::AsRawFd},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

struct Account {
    profile: Profile,
    key: Zeroizing<String>,
}
struct Selection {
    current: Option<String>,
    pinned: bool,
}
struct Owner {
    service: CliRouterService,
    app: AppHandle,
    run: Run,
    router: Router,
    accounts: Vec<Account>,
    cancelled: Arc<AtomicBool>,
    journal: gateway_store::Journal,
    selection: Mutex<Selection>,
}
impl Owner {
    fn fence(&self, native: &super::Owner, profile: Option<&Profile>) -> Result<(), String> {
        runtime::storage_ready(&self.service)?;
        if self.service.closing.load(Ordering::SeqCst) || self.cancelled.load(Ordering::SeqCst) {
            return Err("The gateway terminal is stopping.".into());
        }
        let active = self
            .service
            .active
            .lock()
            .map_err(|_| "Gateway ownership unavailable.")?;
        if active
            .get(&self.run.id)
            .is_none_or(|owner| !Arc::ptr_eq(owner, &self.cancelled))
        {
            return Err("The gateway terminal lost native ownership.".into());
        }
        drop(active);
        let snapshot = native.store.snapshot()?;
        let run = snapshot
            .runs
            .iter()
            .find(|run| {
                run.id == self.run.id
                    && run.generation == self.run.generation
                    && run.execution_mode == RunExecutionMode::Gateway
                    && matches!(run.state, RunState::Starting | RunState::Running)
            })
            .ok_or("The gateway run changed before dispatch.")?;
        if run.allowed_profile_ids != self.run.allowed_profile_ids
            || !snapshot.routers.iter().any(|router| {
                router.id == self.router.id
                    && router.enabled
                    && router.revision == self.router.revision
            })
        {
            return Err("The frozen gateway pool changed before dispatch.".into());
        }
        if let Some(profile) = profile {
            if !snapshot.profiles.iter().any(|current| current == profile)
                || !run.allowed_profile_ids.contains(&profile.id)
                || super::profile_writer_busy(&self.service, &profile.id)?
            {
                return Err("The approved gateway account changed before dispatch.".into());
            }
        }
        gateway_profiles::admit_project(self.router.cli, Path::new(&self.run.cwd))
    }
    fn portable_anthropic(&self) -> bool {
        // Official Claude documentation explicitly preserves signatures across
        // Claude API, Bedrock and Vertex. Admit the narrower same-origin native
        // Claude API pool; custom providers make no such portability promise.
        self.accounts.iter().all(|account| {
            account
                .profile
                .gateway_provider
                .as_ref()
                .is_some_and(|provider| {
                    provider.protocol == Protocol::Anthropic
                        && super::gateway_url::official_anthropic(&provider.base_url)
                })
        })
    }
}
impl UpstreamOwner for Owner {
    fn select(&self, request: &RequestSummary, excluded: &[String]) -> Result<Upstream, String> {
        let mut selection = self
            .selection
            .lock()
            .map_err(|_| "Gateway account affinity unavailable.")?;
        let account = self.service.with_store(&self.app, |native| {
            self.fence(native, None)?;
            let snapshot = native.store.snapshot()?;
            if !snapshot.runs.iter().any(|run| run.id == self.run.id && run.generation == self.run.generation && run.state == RunState::Running) { return Err("The gateway CLI has not crossed its native launch fence.".into()); }
            let usable = |account: &&Account| !excluded.contains(&account.profile.id) && !snapshot.quota.iter().any(|quota| quota.profile_id == account.profile.id && quota.status == QuotaStatus::Exhausted && quota.expires_at > super::now());
            let current = selection.current.as_deref().and_then(|id| self.accounts.iter().find(|account| account.profile.id == id));
            let account = if selection.pinned || request.account_bound && !self.portable_anthropic() {
                current.filter(|account| !excluded.contains(&account.profile.id))
            } else {
                current.filter(|account| usable(account)).or_else(|| self.accounts.iter().find(usable))
            }.ok_or("No approved gateway API account is available; opaque native context remains with its creator account.")?;
            self.fence(native, Some(&account.profile))?;
            Ok(Upstream { protocol: account.profile.gateway_provider.as_ref().ok_or("Missing approved API destination.")?.protocol, profile_id: account.profile.id.clone(), revision: account.profile.revision, base_url: account.profile.gateway_provider.as_ref().ok_or("Missing approved API destination.")?.base_url.clone(), api_key: account.key.clone() })
        })?;
        selection.current = Some(account.profile_id.clone());
        Ok(account)
    }
    fn fence_and_checkpoint(
        &self,
        request: &RequestSummary,
        upstream: &Upstream,
    ) -> Result<(), String> {
        self.service.with_store(&self.app, |native| {
            let account = self.accounts.iter().find(|account| account.profile.id == upstream.profile_id && account.profile.revision == upstream.revision).ok_or("Unknown gateway account binding.")?;
            self.fence(native, Some(&account.profile))?;
            let provider = account.profile.gateway_provider.as_ref().ok_or("Missing approved API destination.")?;
            if provider.protocol != upstream.protocol || provider.base_url != upstream.base_url {
                return Err("Gateway dispatch destination changed.".into());
            }
            if !native.store.snapshot()?.runs.iter().any(|run| run.id == self.run.id && run.generation == self.run.generation && run.state == RunState::Running) { return Err("The gateway CLI is not admitted for provider dispatch.".into()); }
            self.journal.intent(request, upstream)?;
            let snapshot = native.store.update(|snapshot| {
                let run = snapshot.runs.iter_mut().find(|run| run.id == self.run.id && run.generation == self.run.generation).ok_or("Gateway generation changed.")?;
                run.active_profile_id = Some(upstream.profile_id.clone());
                run.revision = run.revision.checked_add(1).ok_or("Run revision exhausted.")?;
                run.status_message = format!("Native CLI API request {} on {}. Tools and permissions are controlled by the CLI.", request.attempt_index + 1, account.profile.label);
                Ok(())
            })?;
            self.service.changed(&self.app, snapshot.revision);
            self.fence(native, Some(&account.profile))
        })
    }
    fn rate_limited(
        &self,
        _: &RequestSummary,
        upstream: &Upstream,
        retry_after: Option<&str>,
    ) -> Result<(), String> {
        let seconds = retry_after
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|seconds| (1..=86400).contains(seconds))
            .unwrap_or(60);
        self.service.with_store(&self.app, |native| {
            let snapshot = native.store.update(|snapshot| {
                snapshot
                    .quota
                    .retain(|quota| quota.profile_id != upstream.profile_id);
                let now = super::now();
                snapshot.quota.push(Quota {
                    profile_id: upstream.profile_id.clone(),
                    status: QuotaStatus::Exhausted,
                    windows: vec![],
                    observed_at: now,
                    expires_at: now.saturating_add(seconds.saturating_mul(1000)),
                    epoch: 0,
                    block_revision: upstream.revision,
                });
                Ok(())
            })?;
            self.service.changed(&self.app, snapshot.revision);
            Ok(())
        })
    }
    fn can_failover(&self, request: &RequestSummary, _: &Upstream) -> bool {
        self.selection.lock().is_ok_and(|selection| {
            !selection.pinned && (!request.account_bound || self.portable_anthropic())
        })
    }
    fn finish(&self, receipt: &Receipt) -> Result<(), String> {
        self.journal.finish(receipt)?;
        if receipt.downstream_started
            && self.journal.context_receipt(receipt)?
            && receipt
                .status
                .is_some_and(|status| (200..300).contains(&status))
        {
            let protocol = gateway_profiles::support(self.router.cli)
                .ok_or("Missing native gateway protocol.")?
                .protocol;
            let selected = self
                .accounts
                .iter()
                .find(|account| account.profile.id == receipt.profile_id)
                .ok_or("Unknown gateway receipt account.")?;
            let selected_native = selected
                .profile
                .gateway_provider
                .as_ref()
                .is_some_and(|provider| provider.protocol == protocol);
            if matches!(protocol, Protocol::OpenAiResponses | Protocol::Gemini)
                || protocol == Protocol::Anthropic
                    && (!selected_native || !self.portable_anthropic())
            {
                self.selection
                    .lock()
                    .map_err(|_| "Gateway affinity unavailable.")?
                    .pinned = true;
            }
        }
        Ok(())
    }
}

pub(crate) struct Request<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) expected_revision: u64,
    pub(crate) terminal_id: &'a str,
    pub(crate) shells: &'a Shells,
    pub(crate) shell: &'a crate::shell::Profile,
    pub(crate) cwd: &'a str,
    pub(crate) cli: TitleCli,
}
fn directory(path: &Path) -> Result<(), String> {
    crate::chat::storage::reject_link(path)?;
    fs::create_dir_all(path).map_err(|_| "Cannot create private gateway storage.")?;
    crate::chat::storage::private(path, true)
}
fn ambient(cli: TitleCli, cwd: &Path) -> Result<(), String> {
    gateway_profiles::admit_project(cli, cwd)?;
    if cli == TitleCli::Codex {
        super::codex::ambient_policy()?;
    }
    if cli == TitleCli::Goose {
        super::goose::admit_system_config()?;
    }
    if cfg!(target_os = "macos")
        && matches!(cli, TitleCli::Claude | TitleCli::Opencode | TitleCli::Kilo)
    {
        match fs::symlink_metadata("/Library/Managed Preferences") {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                return Err("Gateway launch requires absent shared managed endpoint policy.".into())
            }
        }
    }
    Ok(())
}
pub(crate) fn executable(path: &Path) -> Result<(PathBuf, u64, u64, u64, i64, i64), String> {
    let path = path
        .canonicalize()
        .map_err(|_| "Cannot resolve the native gateway executable.")?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| "Cannot inspect the gateway executable.")?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || ![0, unsafe { libc::geteuid() }].contains(&metadata.uid())
    {
        return Err("The native gateway executable is not privately owned or trusted.".into());
    }
    Ok((
        path,
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    ))
}
pub(crate) fn version(
    program: &Path,
    cli: TitleCli,
    launch: &gateway_profiles::Launch,
) -> Result<String, String> {
    let support = gateway_profiles::support(cli).ok_or("Unsupported gateway CLI.")?;
    let expected = support
        .native_version
        .ok_or("This native CLI family still needs release qualification before gateway launch.")?;
    let mut command = Command::new(program);
    command
        .env_clear()
        .envs(launch.environment.iter().cloned())
        .env(
            "PATH",
            std::env::var_os("PATH")
                .unwrap_or_else(|| "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin".into()),
        )
        .env("LANG", "en_US.UTF-8")
        .arg("--version")
        .current_dir(&launch.private_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let mut child = runtime::OwnedChild::new(
        command
            .spawn()
            .map_err(|_| "Cannot inspect the gateway CLI version.")?,
    );
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Gateway version output unavailable.")?;
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("Cannot bound gateway version inspection.".into());
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut eof = false;
    while Instant::now() < deadline {
        if eof {
            if runtime::exit_pending(&child)? {
                break;
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        match stdout.read(&mut buffer) {
            Ok(0) => {
                eof = true;
            }
            Ok(count) if bytes.len() + count <= 32768 => bytes.extend_from_slice(&buffer[..count]),
            Ok(_) => return Err("Gateway version output exceeded its limit.".into()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return Err("Cannot read gateway CLI version.".into()),
        }
    }
    let status = child.stop_and_wait()?;
    let admitted = gateway_profiles::admitted_version(cli, &bytes);
    if !eof || !status.success() || admitted.is_none() {
        return Err(format!(
            "Gateway configuration requires a reviewed {} version ({}; baseline {}). This executable is not admitted.",
            cli.name(), support.native_family, expected
        ));
    }
    admitted.ok_or_else(|| "Unrecognized native client version.".into())
}

pub(crate) fn terminal_command<T>(
    service: &CliRouterService,
    app: &AppHandle,
    request: Request<'_>,
    start: impl FnOnce(
        (portable_pty::CommandBuilder, String),
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    ) -> Result<T, String>,
    close: impl Fn(&str) + Send + 'static,
) -> Result<T, String> {
    if request.run_id.len() != 36
        || !request
            .run_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("Invalid saved gateway run identifier.".into());
    }
    runtime::shell_profile(request.shells, &request.shell.id)?;
    ambient(request.cli, Path::new(request.cwd))?;
    let program = if request.cli == TitleCli::Grok {
        let resource_dir = app
            .path()
            .resource_dir()
            .map_err(|_| "Cannot locate router-owned Grok resources.")?;
        super::grok_artifact::resolve(&resource_dir)?
    } else {
        crate::cli_launch::resolve_cli(
            request.shell,
            request.cwd,
            &request.shells.integration,
            request.cli,
        )?
        .program
    };
    let identity = executable(&program)?;
    if request.cli == TitleCli::Kimi {
        super::native_accounts::admit_executable(request.cli, &identity.0)?;
    }
    let grok_identity = if request.cli == TitleCli::Grok {
        Some(super::grok_artifact::admit(&program)?)
    } else {
        None
    };
    let (run, router, profiles, root, cancelled, resume) = service.with_store(app, |native| {
        let snapshot = native.store.snapshot()?;
        let run = snapshot.runs.iter().find(|run| run.id == request.run_id).ok_or("Gateway run no longer exists.")?;
        let resume = run.state == RunState::Stopped && run.generation > 0 && gateway_profiles::can_resume(request.cli);
        let initial = run.state == RunState::Idle && run.generation == 0;
        if run.execution_mode != RunExecutionMode::Gateway || run.revision != request.expected_revision || (!initial && !resume) || run.cwd != request.cwd || run.shell_profile_id.as_deref() != Some(&request.shell.id) { return Err("Explicitly start a fresh session or resume a cleanly stopped native CLI. Uncertain native requests cannot be replayed.".into()); }
        let router = snapshot.routers.iter().find(|router| router.id == run.router_id && router.cli == request.cli && router.enabled).ok_or("The gateway router changed.")?.clone();
        let mut profiles = gateway_config::pool(&snapshot, &router, service)?;
        profiles.retain(|profile| run.allowed_profile_ids.contains(&profile.id));
        if profiles.len() < 2 { return Err("Approve at least two compatible API accounts before starting this gateway terminal.".into()); }
        if let Some(pin) = &run.pinned_profile_id {
            if !profiles.iter().any(|profile| profile.id == *pin) { return Err("The pinned API account is not eligible for this gateway session.".into()); }
            profiles.sort_by_key(|profile| profile.id != *pin);
        }
        runtime::reserve(service, &run.id)?;
        let cancelled = service.active.lock().map_err(|_| "Gateway ownership unavailable.")?.get(&run.id).cloned().ok_or("Gateway reservation unavailable.")?;
        let id = run.id.clone();
        let saved = native.store.update(|snapshot| {
            let run = snapshot.runs.iter_mut().find(|run| run.id == id).ok_or("Gateway run disappeared.")?;
            run.generation = run.generation.checked_add(1).ok_or("Gateway generation exhausted.")?;
            run.revision = run.revision.checked_add(1).ok_or("Gateway revision exhausted.")?;
            run.state = RunState::Starting;
            run.status_message = "Preparing the private native CLI and API gateway. No native task was replayed.".into();
            Ok(())
        });
        let snapshot = match saved { Ok(snapshot) => snapshot, Err(error) => { service.active.lock().map_err(|_| "Gateway ownership unavailable.")?.remove(&id); return Err(error); } };
        service.changed(app, snapshot.revision);
        let run = snapshot.runs.into_iter().find(|run| run.id == id).ok_or("Gateway run disappeared.")?;
        Ok((run, router, profiles, native.root.join("runs").join(&id).join("gateway"), cancelled, resume))
    })?;
    let prepared = (|| {
        for parent in [
            root.parent()
                .and_then(Path::parent)
                .ok_or("Missing gateway storage parent.")?,
            root.parent().ok_or("Missing gateway run directory.")?,
            root.as_path(),
        ] {
            directory(parent)?;
        }
        let protocol = gateway_profiles::support(router.cli)
            .ok_or("Unsupported native gateway.")?
            .protocol;
        let model = run
            .model
            .as_deref()
            .ok_or("Choose a gateway model explicitly.")?;
        let probe = gateway_profiles::prepare_probe(
            &root.join(format!("version-probe-{}", run.generation)),
        )?;
        if let Some(expected) = &grok_identity {
            if super::grok_artifact::admit(&identity.0)? != *expected {
                return Err(
                    "The router-owned Grok artifact changed before version inspection.".into(),
                );
            }
        }
        let native_version = version(&identity.0, router.cli, &probe)?;
        let continuity = if resume {
            let previous =
                gateway_session::Session::read(&root, &run, router.cli, &native_version, protocol)?;
            let journal = gateway_store::Journal::open_existing(
                &previous.journal_root(&root),
                &run.id,
                previous.generation,
                model,
                protocol,
            )?;
            let continuity = journal.continuity()?.or(previous.continuity);
            if let Some(binding) = &continuity {
                let profile = profiles.iter().find(|p| p.id == binding.profile_id && p.revision == binding.profile_revision)
                    .ok_or("The account that created native context is no longer approved at its saved credential revision.")?;
                let destination = profile
                    .gateway_provider
                    .as_ref()
                    .ok_or("Missing saved API destination.")?;
                if super::gateway_url::parse(&destination.base_url)?.to_string()
                    != binding.destination
                    || run
                        .pinned_profile_id
                        .as_ref()
                        .is_some_and(|pin| *pin != binding.profile_id)
                {
                    return Err("Retained native context is bound to its original API account and destination.".into());
                }
            }
            continuity
        } else {
            None
        };
        let history = if resume {
            Some(super::native_history::select(
                &root.join("native-home"),
                router.cli,
                &native_version,
                &run.cwd,
            )?)
        } else {
            None
        };
        let session = gateway_session::Session::new(
            &run,
            router.cli,
            &native_version,
            protocol,
            continuity.clone(),
        )?;
        let journal_root = session.journal_root(&root);
        if run.generation > 1 {
            directory(&root.join("generations"))?;
            directory(&journal_root)?;
        }
        let journal =
            gateway_store::Journal::open(&journal_root, &run.id, run.generation, model, protocol)?;
        session.write(&root)?;
        let accounts = profiles
            .into_iter()
            .map(|profile| {
                Ok(Account {
                    key: Zeroizing::new(credentials::get(&profile)?),
                    profile,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let owner = Arc::new(Owner {
            service: service.clone(),
            app: app.clone(),
            run: run.clone(),
            router: router.clone(),
            accounts,
            cancelled: cancelled.clone(),
            journal,
            selection: Mutex::new(Selection {
                current: continuity
                    .as_ref()
                    .map(|b| b.profile_id.clone())
                    .or_else(|| run.pinned_profile_id.clone()),
                pinned: run.pinned_profile_id.is_some(),
            }),
        });
        if continuity.is_some()
            && (matches!(protocol, Protocol::OpenAiResponses | Protocol::Gemini)
                || protocol == Protocol::Anthropic && !owner.portable_anthropic())
        {
            owner
                .selection
                .lock()
                .map_err(|_| "Gateway affinity unavailable.")?
                .pinned = true;
        }
        let token = format!("{}{}", super::new_id()?, super::new_id()?);
        let controller = gateway_http::Controller::bind(
            protocol,
            token.clone(),
            model.into(),
            owner.clone(),
            cancelled.clone(),
        )?;
        let mut launch = gateway_profiles::prepare_native(
            &root.join("native-home"),
            router.cli,
            &controller.base_url(),
            &token,
            model,
            Path::new(&run.cwd),
            &native_version,
            resume,
        )?;
        if let Some(history) = &history {
            launch.arguments.extend(history.arguments.clone());
        }
        let mut command = portable_pty::CommandBuilder::new(&identity.0);
        command.env_clear();
        command.env(
            "PATH",
            std::env::var_os("PATH")
                .unwrap_or_else(|| "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin".into()),
        );
        command.env("LANG", "en_US.UTF-8");
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "Lomi");
        for (key, value) in launch.environment {
            command.env(key, value);
        }
        command.args(launch.arguments);
        command.cwd(&run.cwd);
        let done = Arc::new(AtomicBool::new(false));
        let drain_healthy = Arc::new(AtomicBool::new(false));
        let result = service.with_store(app, |native| {
            owner.fence(native, None)?;
            if controller.is_finished() { return Err("The gateway closed before native CLI launch.".into()); }
            for account in &owner.accounts { owner.fence(native, Some(&account.profile))?; }
            ambient(router.cli, Path::new(&run.cwd))?;
            if let Some(history) = &history { history.fence()?; }
            if executable(&identity.0)? != identity { return Err("The gateway executable changed before launch.".into()); }
            if router.cli == TitleCli::Kimi { super::native_accounts::admit_executable(router.cli,&identity.0)?; }
            if let Some(expected) = &grok_identity { if super::grok_artifact::admit(&identity.0)? != *expected { return Err("The router-owned Grok artifact changed before launch.".into()); } }
            let saved = native.store.update(|snapshot| {
                let current = snapshot.runs.iter_mut().find(|current| current.id == run.id && current.generation == run.generation).ok_or("Gateway run changed.")?;
                current.state = RunState::Running; current.revision = current.revision.checked_add(1).ok_or("Gateway revision exhausted.")?;
                current.status_message = "Native CLI terminal active. The gateway preserves native requests and switches only qualified pre-response HTTP 429 rejections.".into(); Ok(())
            })?;
            let result = start((command, run.cwd.clone()), done.clone(), drain_healthy.clone())?;
            service.changed(app, saved.revision); Ok(result)
        })?;
        let service = service.clone();
        let app = app.clone();
        let terminal_id = request.terminal_id.to_owned();
        let id = run.id.clone();
        let generation = run.generation;
        let cancelled = cancelled.clone();
        thread::spawn(move || {
            let mut controller = controller;
            while !done.load(Ordering::SeqCst) {
                if controller.is_finished() {
                    cancelled.store(true, Ordering::SeqCst);
                }
                if cancelled.load(Ordering::SeqCst) {
                    close(&terminal_id);
                }
                thread::sleep(Duration::from_millis(20));
            }
            let drained = controller.stop_and_join().and_then(|_| {
                if !drain_healthy.load(Ordering::SeqCst) {
                    return Err("The native terminal tail could not be fully drained.".into());
                }
                if owner.journal.idle()? {
                    Ok(())
                } else {
                    Err("Uncertain native API receipts were retained.".into())
                }
            });
            let saved = service.with_store(&app, |native| native.store.update(|snapshot| {
                let current = snapshot.runs.iter_mut().find(|current| current.id == id && current.generation == generation).ok_or("Gateway generation changed during drain.")?;
                current.state = if drained.is_err() { RunState::RecoveryRequired } else { RunState::Stopped };
                current.revision = current.revision.checked_add(1).ok_or("Gateway revision exhausted.")?;
                current.status_message = if let Err(error) = &drained { format!("Gateway closed with uncertain receipts: {error} Native history was retained; no request will be replayed.") } else { "Gateway terminal closed. Its private native history is retained. Explicit Resume checks the saved client, history and account bindings before launching.".into() }; Ok(())
            }));
            if let Ok(snapshot) = saved {
                service.changed(&app, snapshot.revision);
            } else {
                if let Ok(mut error) = service.storage_error.lock() {
                    *error = Some("Gateway drain could not commit its final state.".into());
                }
            }
            if let Ok(mut active) = service.active.lock() {
                if active
                    .get(&id)
                    .is_some_and(|owner| Arc::ptr_eq(owner, &cancelled))
                {
                    active.remove(&id);
                }
            }
        });
        Ok(result)
    })();
    if prepared.is_err() {
        cancelled.store(true, Ordering::SeqCst);
        let _ = service.with_store(app, |native| native.store.update(|snapshot| {
            let current = snapshot.runs.iter_mut().find(|current| current.id == run.id).ok_or("Gateway run disappeared.")?;
            current.state = RunState::RecoveryRequired; current.revision = current.revision.checked_add(1).ok_or("Gateway revision exhausted.")?;
            current.status_message = "Gateway startup failed before native dispatch. Its private state was retained; create a new run after correcting admission.".into(); Ok(())
        }));
        if let Ok(mut active) = service.active.lock() {
            active.remove(&run.id);
        }
    }
    prepared
}
