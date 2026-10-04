//! Native coding supervisor. OAuth and builtin tools remain native-owned.
//! Native startup effects prohibit automatic subscription-task redispatch.
//! Settled turns retain checkpoints; uncertain sessions require native review.
use super::{
    native_approvals,
    native_handoff::{Checkpoint, Ledger},
    native_process::{Prepared, Process},
    native_wire::*,
    types::*,
    CliRouterService,
};
use crate::{cli_catalog::TitleCli, terminal::Shells};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::AppHandle;

const MAX_HISTORY: usize = 16 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    schema: u32,
    run_id: String,
    input_id: String,
    profile_id: String,
    profile_revision: u64,
    generation: u64,
    session_id: String,
    model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    version: String,
    cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    directory_identity: Option<(u64, u64)>,
    ledger: Ledger,
    output: String,
    tools: Vec<NativeTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_file: Option<String>,
}
fn workspace(cwd: &str) -> Result<(u64, u64), String> {
    let path = crate::files::directory(cwd)?;
    if path != Path::new(cwd) {
        return Err("The native workspace path changed.".into());
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Cannot inspect the native workspace.")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("The native workspace is unavailable.".into());
    }
    Ok((metadata.dev(), metadata.ino()))
}
fn private_child(parent: &Path, name: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    crate::chat::storage::reject_link(parent)?;
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "Cannot create private native history.")?,
        Err(_) => return Err("Cannot inspect private native history.".into()),
        Ok(_) => {}
    }
    super::gateway_profiles::check_private_directory(&path)?;
    Ok(path)
}
fn root(owner: &super::Owner, id: &str) -> Result<PathBuf, String> {
    root_at(&owner.root, id)
}
fn root_at(owner_root: &Path, id: &str) -> Result<PathBuf, String> {
    if !crate::chat::process::valid_id(id) {
        return Err("Invalid native run identity.".into());
    }
    let runs = private_child(owner_root, "runs")?;
    let run = private_child(&runs, id)?;
    private_child(&run, "native")
}
fn encoded(record: &Record) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(record).map_err(|_| "Cannot encode native history.")?;
    if bytes.len() > MAX_HISTORY {
        return Err(
            "Native history reached its bound. Its uncertain result is retained for recovery."
                .into(),
        );
    }
    Ok(bytes)
}
fn save(root: &Path, record: &Record) -> Result<Vec<u8>, String> {
    let bytes = encoded(record)?;
    crate::chat::storage::atomic(
        &root.join(format!("attempt-{}.json", record.generation)),
        &bytes,
    )?;
    Ok(bytes)
}
fn validate_history(root: &Path, checkpoint: &Checkpoint) -> Result<Record, String> {
    let (record, bytes) = read_record(root, checkpoint.generation)?;
    if record.directory_identity != Some(workspace(&record.cwd)?) {
        return Err("The native checkpoint belongs to a replaced workspace directory.".into());
    }
    if format!("{:x}", Sha256::digest(&bytes)) != checkpoint.history_digest {
        return Err(
            "Native history changed after checkpoint. Review recovery before continuing.".into(),
        );
    }
    if record.schema != 1
        || record.run_id != checkpoint.run_id
        || record.input_id != checkpoint.input_id
        || record.profile_id != checkpoint.profile_id
        || record.profile_revision != checkpoint.profile_revision
        || record.generation != checkpoint.generation
        || record.session_id != checkpoint.session_id
        || record.model != checkpoint.model
        || record.version != checkpoint.version
        || record.cwd != checkpoint.cwd
        || !record.ledger.quiescent()
    {
        return Err("Native history does not match its owned checkpoint.".into());
    }
    Ok(record)
}
/// Review only a settled, complete native file. Grants never imply portability.
pub(crate) fn handoff_review_at_root(
    state: &CliRouterService,
    owner_root: &Path,
    snapshot: &Snapshot,
    request: &super::native_transfer_commands::PreviewRequest,
) -> Result<(super::native_transfer::ForkRequest, String, u64, bool), String> {
    super::runtime::storage_ready(state)?;
    if state.closing.load(Ordering::SeqCst)
        || state
            .active
            .lock()
            .map_err(|_| "Native ownership unavailable.")?
            .contains_key(&request.run_id)
        || state
            .draining
            .lock()
            .map_err(|_| "Native close status unavailable.")?
            .contains(&request.run_id)
    {
        return Err("Wait for the native process to finish before transferring history.".into());
    }
    let run = snapshot
        .runs
        .iter()
        .find(|run| run.id == request.run_id)
        .ok_or("The native run no longer exists.")?;
    let router = snapshot
        .routers
        .iter()
        .find(|router| router.id == run.router_id && router.enabled)
        .ok_or("The native router is unavailable.")?;
    if run.revision != request.expected_revision
        || run.execution_mode != RunExecutionMode::Native
        || router.cli != TitleCli::Pi
        || !matches!(
            run.state,
            RunState::Idle | RunState::Completed | RunState::RecoveryRequired
        )
    {
        return Err(
            "Refresh and review a settled native Pi run before transferring its history.".into(),
        );
    }
    let root = root_at(owner_root, &run.id)?;
    let checkpoint =
        Checkpoint::read(&root)?.ok_or("This native run has no settled checkpoint.")?;
    if checkpoint.generation != run.generation
        || !checkpoint.matches(run, router.cli, &checkpoint.version)
        || run
            .inputs
            .last()
            .is_none_or(|input| input.id != checkpoint.input_id)
    {
        return Err(
            "The latest native attempt is not fully checkpointed. Its history requires recovery."
                .into(),
        );
    }
    let record = validate_history(&root, &checkpoint)?;
    let source = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == checkpoint.profile_id)
        .ok_or("The source native profile no longer exists.")?;
    let destination = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == request.destination_profile_id)
        .ok_or("The destination native profile no longer exists.")?;
    if source.revision != checkpoint.profile_revision
        || destination.revision != request.destination_profile_revision
        || source.id == destination.id
        || [source, destination].iter().any(|profile| {
            profile.cli != TitleCli::Pi
                || !profile.enabled
                || profile.storage_mode == StorageMode::ApiKey
                || profile.gateway_provider.is_some()
                || !matches!(profile.auth_state, AuthState::Ready | AuthState::Unverified)
                || !run.allowed_profile_ids.contains(&profile.id)
                || !router.ordered_profile_ids.contains(&profile.id)
                || super::profile_writer_busy(state, &profile.id).unwrap_or(true)
        })
    {
        return Err("Review both current native profiles and approve their history access before transferring.".into());
    }
    let fork = super::native_transfer::ForkRequest {
        run_id: run.id.clone(),
        input_id: checkpoint.input_id.clone(),
        source_generation: checkpoint.generation,
        next_generation: checkpoint
            .generation
            .checked_add(1)
            .ok_or("Native generation exhausted.")?,
        source_file: record
            .session_file
            .ok_or("The Pi checkpoint has no qualified native file.")?,
        source_session: checkpoint.session_id.clone(),
        source_profile: source.id.clone(),
        source_revision: source.revision,
        destination_profile: destination.id.clone(),
        destination_revision: destination.revision,
        cwd: run.cwd.clone(),
        cwd_identity: workspace(&run.cwd)?,
        model: checkpoint.model,
        version: checkpoint.version,
    };
    let (digest, bytes) = super::native_transfer::inspect(&root, &fork)?;
    Ok((fork, digest, bytes, checkpoint.completed))
}
fn read_record(root: &Path, generation: u64) -> Result<(Record, Vec<u8>), String> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join(format!("attempt-{generation}.json")))
        .map_err(|_| "The native checkpoint history is missing.")?;
    let meta = file
        .metadata()
        .map_err(|_| "Cannot inspect native history.")?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.nlink() != 1
        || meta.mode() & 0o777 != 0o600
        || meta.len() > MAX_HISTORY as u64
    {
        return Err("Native history is not privately owned.".into());
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_HISTORY as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read native history.")?;
    if bytes.len() > MAX_HISTORY {
        return Err("Native history exceeds its bound.".into());
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| "Invalid native history.")?;
    if encoded(&record)? != bytes {
        return Err("Native history is not canonically encoded.".into());
    }
    Ok((record, bytes))
}
struct AccountLease {
    state: CliRouterService,
    id: String,
    marker: Arc<AtomicBool>,
}
impl AccountLease {
    fn acquire(state: &CliRouterService, id: &str) -> Result<Self, String> {
        let mut leases = state
            .profile_terminals
            .lock()
            .map_err(|_| "Native account ownership unavailable.")?;
        if leases
            .get(id)
            .is_some_and(|marker| !marker.load(Ordering::SeqCst))
        {
            return Err(
                "Close this account's CLI or wait for its native reader before starting coding."
                    .into(),
            );
        }
        let marker = Arc::new(AtomicBool::new(false));
        leases.insert(id.into(), marker.clone());
        Ok(Self {
            state: state.clone(),
            id: id.into(),
            marker,
        })
    }
}
impl Drop for AccountLease {
    fn drop(&mut self) {
        self.marker.store(true, Ordering::SeqCst);
        if let Ok(mut leases) = self.state.profile_terminals.lock() {
            if leases
                .get(&self.id)
                .is_some_and(|marker| Arc::ptr_eq(marker, &self.marker))
            {
                leases.remove(&self.id);
            }
        }
    }
}
struct Binding<'a> {
    state: &'a CliRouterService,
    app: &'a AppHandle,
    run: &'a Run,
    router: &'a Router,
    profile: &'a Profile,
    transfer_source: Option<&'a Profile>,
    cancel: &'a Arc<AtomicBool>,
    directory_identity: (u64, u64),
}
impl Binding<'_> {
    fn fence(&self) -> Result<(), String> {
        if workspace(&self.run.cwd)? != self.directory_identity {
            return Err("The native workspace directory changed before dispatch.".into());
        }
        self.state.with_store(self.app, |owner| {
            super::runtime::storage_ready(self.state)?;
            if self.cancel.load(Ordering::SeqCst) || self.state.closing.load(Ordering::SeqCst) { return Err("The native run is stopping.".into()); }
            let snapshot = owner.store.snapshot()?;
            let current = snapshot.runs.iter().find(|run| run.id == self.run.id).ok_or("Native run no longer exists.")?;
            let router = snapshot.routers.iter().find(|router| router.id == self.router.id).ok_or("Native router no longer exists.")?;
            let profile = snapshot.profiles.iter().find(|profile| profile.id == self.profile.id).ok_or("Native account no longer exists.")?;
            if current.execution_mode != RunExecutionMode::Native || current.generation != self.run.generation
                || current.state != RunState::Running || current.active_profile_id.as_deref() != Some(profile.id.as_str())
                || current.model != self.run.model || current.cwd != self.run.cwd || current.reasoning_effort != self.run.reasoning_effort
                || !current.allowed_profile_ids.contains(&profile.id) || !router.enabled || router.revision != self.router.revision
                || !router.ordered_profile_ids.contains(&profile.id) || !profile.enabled || profile.revision != self.profile.revision
                || !matches!(profile.auth_state, AuthState::Ready | AuthState::Unverified) || profile.storage_mode == StorageMode::ApiKey
                || !self.state.active.lock().map_err(|_| "Native ownership unavailable.")?.get(&current.id).is_some_and(|cancel| Arc::ptr_eq(cancel, self.cancel))
            { return Err("Native account, model, history grant or execution generation changed. The request was fenced.".into()); }
            if let Some(source) = self.transfer_source {
                let current_source = snapshot.profiles.iter().find(|profile| profile.id == source.id)
                    .ok_or("The transferred history's source account no longer exists.")?;
                if current_source.revision != source.revision || !current_source.enabled
                    || !current.allowed_profile_ids.contains(&source.id) || !router.ordered_profile_ids.contains(&source.id)
                    || current_source.storage_mode == StorageMode::ApiKey || current_source.gateway_provider.is_some()
                {
                    return Err("The transferred history's source account or consent changed before dispatch.".into());
                }
            }
            Ok(())
        })
    }
}
fn send(
    process: &mut Process,
    wire: &mut NativeWire,
    request: WireRequest,
    id: &str,
    binding: &Binding<'_>,
) -> Result<Vec<NativeEvent>, String> {
    binding.fence()?;
    match request {
        WireRequest::Stdio(value) => {
            process.send(&value)?;
            Ok(vec![])
        }
        WireRequest::Http { method, path, body } => {
            if path == "/event" {
                process.open_events(&path)?;
                return Ok(vec![]);
            }
            let (status, body) = process.request_with_status(method, &path, body.as_ref())?;
            Ok(vec![wire.accepted_http(id, status, body)?])
        }
        WireRequest::Websocket { path } => {
            process.open_websocket(&path)?;
            Ok(vec![])
        }
    }
}
fn startup_observations(
    wire: &NativeWire,
    events: Vec<NativeEvent>,
    record: &mut Record,
    root: &Path,
) -> Result<(), String> {
    for event in events {
        record.ledger.observe()?;
        match event {
            NativeEvent::Tool(item) => {
                record.ledger.tool(
                    &item.tool_id,
                    matches!(item.state, ToolState::Completed | ToolState::Failed),
                )?;
                record.tools = wire.tools();
            }
            NativeEvent::Session { session_id, .. } => record.session_id = session_id,
            NativeEvent::Error { .. } | NativeEvent::Permission(_) | NativeEvent::Terminal(_) => {
                record.ledger.complete_observation = false;
                save(root, record)?;
                return Err("Native initialization requested interaction or failed before the user prompt. Review the retained native account session.".into());
            }
            _ => {}
        }
    }
    record.session_id = wire.session_id().into();
    save(root, record)?;
    Ok(())
}
fn startup_send(
    process: &mut Process,
    wire: &mut NativeWire,
    request: WireRequest,
    id: &str,
    binding: &Binding<'_>,
    record: &mut Record,
    root: &Path,
) -> Result<(), String> {
    let events = send(process, wire, request, id, binding)?;
    startup_observations(wire, events, record, root)
}
fn startup(
    process: &mut Process,
    wire: &mut NativeWire,
    binding: &Binding<'_>,
    record: &mut Record,
    root: &Path,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !wire.pending_requests().is_empty() {
        binding.fence()?;
        if Instant::now() >= deadline || process.exited() {
            return Err(
                "Native initialization did not confirm the selected account, session and model."
                    .into(),
            );
        }
        if let Some(raw) = process.poll()? {
            let events = wire.observe(raw)?;
            startup_observations(wire, events, record, root)?;
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}
fn model_parts(cli: TitleCli, model: &str) -> Result<(Option<&str>, &str), String> {
    if matches!(cli, TitleCli::Pi | TitleCli::Kilo | TitleCli::Opencode) {
        let (provider, model) = model
            .split_once('/')
            .filter(|(provider, model)| !provider.is_empty() && !model.is_empty())
            .ok_or("Choose an exact provider/model ID for this native CLI.")?;
        Ok((Some(provider), model))
    } else {
        Ok((None, model))
    }
}
fn prompt_id(id: &str) -> Result<String, String> {
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid native prompt identity.".into());
    }
    let mut bytes = id.as_bytes().to_vec();
    bytes[12] = b'4';
    bytes[16] = b'8';
    let id = std::str::from_utf8(&bytes).map_err(|_| "Invalid native prompt identity.")?;
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &id[..8],
        &id[8..12],
        &id[12..16],
        &id[16..20],
        &id[20..]
    ))
}
fn permission(
    binding: &Binding<'_>,
    wire: &mut NativeWire,
    process: &mut Process,
    item: NativePermission,
    record: &mut Record,
    root: &Path,
) -> Result<(), String> {
    permission_inner(binding, wire, process, item, record, root, 0)
}
fn permission_inner(
    binding: &Binding<'_>,
    wire: &mut NativeWire,
    process: &mut Process,
    item: NativePermission,
    record: &mut Record,
    root: &Path,
    depth: usize,
) -> Result<(), String> {
    if depth >= 32 {
        return Err("Native permission nesting exceeded its bounded queue. Review the retained native session.".into());
    }
    let id = item.request_id.to_string();
    record.ledger.approval(&id, true)?;
    save(root, record)?;
    let mut values = Vec::new();
    let mut choices = Vec::new();
    let mut offered = if item.choices.is_empty()
        && wire.kind() == NativeKind::Pi
        && item.raw["method"] == "confirm"
    {
        vec![json!(false), json!(true)]
    } else {
        item.choices.clone()
    };
    if wire.kind() == NativeKind::Pi {
        offered.push(Value::Null);
    }
    let mut denial = None;
    for (index, value) in offered.iter().enumerate() {
        let label = value
            .get("name")
            .or_else(|| value.get("label"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| {
                if value == &json!(true) {
                    "Allow".into()
                } else if value == &json!(false) {
                    "Deny".into()
                } else if value.is_null() {
                    "Cancel".into()
                } else {
                    value.to_string()
                }
            });
        let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
        let allow = kind.starts_with("allow")
            || matches!(
                value.as_str(),
                Some("allow" | "accept" | "acceptForSession" | "once" | "always" | "approved")
            )
            || value == &json!(true)
            || value
                .as_object()
                .is_some_and(|object| object.keys().any(|key| key.starts_with("accept")))
            || wire.kind() == NativeKind::Pi && item.raw["method"] == "select" && value.is_string();
        let reply = if wire.kind() == NativeKind::Grok {
            value
                .get("optionId")
                .cloned()
                .ok_or("ACP permission has no offered option identity.")?
        } else {
            value.clone()
        };
        if kind.starts_with("reject")
            || matches!(
                value.as_str(),
                Some("deny" | "decline" | "reject" | "cancel" | "rejected" | "cancelled")
            )
            || value == &json!(false)
            || wire.kind() == NativeKind::Pi && value.is_null()
        {
            denial = Some(reply.clone());
        }
        let choice_id = format!("choice-{index}");
        values.push((choice_id.clone(), reply));
        choices.push(native_approvals::Choice {
            id: choice_id,
            label,
            allow,
        });
    }
    if choices.is_empty() {
        return Err("This native interaction needs its CLI's interactive UI. No tool decision was inferred.".into());
    }
    let text_input = if wire.kind() == NativeKind::Pi
        && matches!(item.raw["method"].as_str(), Some("input" | "editor"))
    {
        choices.push(native_approvals::Choice {
            id: "submit-text".into(),
            label: "Submit".into(),
            allow: true,
        });
        Some(native_approvals::TextInput {
            label: item.raw["title"]
                .as_str()
                .unwrap_or("Native CLI input")
                .into(),
            initial: item.raw["prefill"].as_str().unwrap_or_default().into(),
            multiline: item.raw["method"] == "editor",
        })
    } else {
        None
    };
    let tool = item
        .raw
        .pointer("/request/tool_name")
        .or_else(|| item.raw.pointer("/params/toolCall/title"))
        .or_else(|| item.raw.pointer("/properties/permission"))
        .or_else(|| item.raw.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("Native tool permission")
        .to_owned();
    let input = item
        .raw
        .pointer("/request/input")
        .or_else(|| item.raw.pointer("/params/toolCall/rawInput"))
        .cloned()
        .unwrap_or_else(|| item.raw.clone());
    let ticket = binding.state.native_approvals.request(
        native_approvals::Preview {
            id: String::new(),
            run_id: binding.run.id.clone(),
            generation: binding.run.generation,
            native_request_id: id.clone(),
            tool,
            input,
            choices,
            text_input,
        },
        binding.profile,
    )?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut queued_permissions = Vec::new();
    let chosen = loop {
        binding.fence()?;
        if let Some(choice) = ticket.decision()? {
            break Some(choice);
        }
        if Instant::now() >= deadline {
            break None;
        }
        if let Some(raw) = process.poll()? {
            for event in wire.observe(raw)? {
                if let NativeEvent::Permission(other) = event {
                    if queued_permissions.len() >= 32 {
                        return Err("Too many simultaneous native permission requests.".into());
                    }
                    record
                        .ledger
                        .approval(&other.request_id.to_string(), true)?;
                    queued_permissions.push(other);
                } else {
                    observe(binding, process, wire, vec![event], record, root)?;
                }
            }
            if !wire.permission_pending(&item.request_id) || wire.terminal().is_some() {
                record.ledger.approval(&id, false)?;
                save(root, record)?;
                drop(ticket);
                for queued in queued_permissions {
                    if wire.permission_pending(&queued.request_id) {
                        if wire.terminal().is_some() {
                            return Err("Native turn ended with unresolved permission requests. Review its retained task.".into());
                        }
                        permission_inner(binding, wire, process, queued, record, root, depth + 1)?;
                    }
                }
                return Ok(());
            }
        } else if process.exited() {
            return Err("The native client stopped while awaiting a permission decision.".into());
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    };
    binding.fence()?;
    let value = chosen
        .and_then(|chosen| {
            if chosen.choice_id == "submit-text" {
                chosen.text.map(Value::String)
            } else {
                values
                    .iter()
                    .find(|(id, _)| id == &chosen.choice_id)
                    .map(|(_, value)| value.clone())
            }
        })
        .or(denial)
        .ok_or("Native permission expired without an offered denial. Its process will stop.")?;
    let reply = wire.permission_reply(&item.request_id, value)?;
    // HTTP permission replies are not turn acceptance records.
    match reply {
        WireRequest::Stdio(value) => process.send(&value)?,
        WireRequest::Http { method, path, body } => {
            process.request(method, &path, body.as_ref())?;
        }
        WireRequest::Websocket { .. } => {
            return Err("Unqualified native permission transport.".into())
        }
    }
    record.ledger.approval(&id, false)?;
    save(root, record)?;
    drop(ticket);
    for queued in queued_permissions {
        if wire.permission_pending(&queued.request_id) {
            permission_inner(binding, wire, process, queued, record, root, depth + 1)?;
        }
    }
    Ok(())
}
fn observe(
    binding: &Binding<'_>,
    process: &mut Process,
    wire: &mut NativeWire,
    events: Vec<NativeEvent>,
    record: &mut Record,
    root: &Path,
) -> Result<(), String> {
    for event in events {
        if record.ledger.terminal {
            if matches!(
                event,
                NativeEvent::Observed(_) | NativeEvent::Accepted { .. }
            ) {
                continue;
            }
            record.ledger.complete_observation = false;
            save(root, record)?;
            return Err("Native activity continued after the recorded terminal boundary. Automatic continuation was fenced.".into());
        }
        record.ledger.observe()?;
        match event {
            NativeEvent::Text { text, .. } => {
                if record.output.len().saturating_add(text.len()) > super::runtime::MAX_OUTPUT {
                    return Err("Native output reached its saved history limit.".into());
                }
                record.output.push_str(&text);
            }
            NativeEvent::Tool(item) => {
                record.ledger.tool(
                    &item.tool_id,
                    matches!(item.state, ToolState::Completed | ToolState::Failed),
                )?;
                record.tools = wire.tools();
                save(root, record)?;
            }
            NativeEvent::Permission(item) => {
                permission(binding, wire, process, item, record, root)?
            }
            NativeEvent::PermissionWithdrawn { request_id } => {
                record.ledger.approval(&request_id.to_string(), false)?;
            }
            NativeEvent::Terminal(_) => {
                record.ledger.terminal = true;
                save(root, record)?;
            }
            NativeEvent::Error {
                class: ErrorClass::Protocol,
                ..
            } => {
                record.ledger.complete_observation = false;
                return Err(
                    "Native event continuity was lost. Review the retained task before continuing."
                        .into(),
                );
            }
            NativeEvent::Observed(raw)
                if raw["type"] == "resync_required"
                    || raw["payload"]["type"] == "resync_required" =>
            {
                record.ledger.complete_observation = false;
                return Err(
                    "Native event history requires resynchronization. Automatic replay was fenced."
                        .into(),
                );
            }
            NativeEvent::Observed(_) => {}
            _ => {}
        }
        record.session_id = wire.session_id().into();
    }
    Ok(())
}
pub(crate) fn execute(
    state: &CliRouterService,
    app: &AppHandle,
    shells: &Shells,
    run_id: &str,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), String> {
    attempt(state, app, shells, run_id, cancelled)
}
fn attempt(
    state: &CliRouterService,
    app: &AppHandle,
    shells: &Shells,
    run_id: &str,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), String> {
    let (mut run, router, profile, directory, root, lease, source_lease) = state.with_store(app, |owner| {
        super::runtime::storage_ready(state)?;
        if cancelled.load(Ordering::SeqCst) || state.closing.load(Ordering::SeqCst) { return Err("The native task stopped before dispatch.".into()); }
        let snapshot = owner.store.snapshot()?;
        let run = snapshot.runs.iter().find(|run| run.id == run_id && run.execution_mode == RunExecutionMode::Native && matches!(run.state, RunState::Starting | RunState::Switching)).cloned().ok_or("The native task changed before dispatch.")?;
        let router = snapshot.routers.iter().find(|router| router.id == run.router_id && router.enabled).cloned().ok_or("Native router is unavailable.")?;
        let checkpoint = Checkpoint::read(&root(owner, &run.id)?)?;
        let preferred = run.pinned_profile_id.as_ref().or_else(|| checkpoint.as_ref().map(|checkpoint| &checkpoint.profile_id));
        let mut groups: HashSet<_> = snapshot.profiles.iter().filter(|profile| run.attempted_profile_ids.contains(&profile.id)).filter_map(|profile| profile.quota_group_key.clone()).collect();
        let profile = router.ordered_profile_ids.iter().filter_map(|id| snapshot.profiles.iter().find(|profile| profile.id == *id)).find(|profile| {
            profile.cli == router.cli && profile.enabled && profile.gateway_provider.is_none() && profile.storage_mode != StorageMode::ApiKey
                && matches!(profile.auth_state, AuthState::Ready | AuthState::Unverified) && run.allowed_profile_ids.contains(&profile.id)
                && !run.attempted_profile_ids.contains(&profile.id) && preferred.is_none_or(|pin| pin == &profile.id)
                && !super::profile_writer_busy(state, &profile.id).unwrap_or(true)
                && profile.quota_group_key.as_ref().is_none_or(|key| groups.insert(key.clone()))
        }).cloned().ok_or("No unattempted native account remains. Sign in or select an account before explicitly continuing.")?;
        let lease = AccountLease::acquire(state, &profile.id)?;
        let source_lease = checkpoint.as_ref().filter(|checkpoint| checkpoint.profile_id != profile.id).map(|checkpoint| {
            let source = snapshot.profiles.iter().find(|source| source.id == checkpoint.profile_id)
                .cloned().ok_or("The history source account is unavailable.")?;
            let lease = AccountLease::acquire(state, &source.id)?;
            Ok::<_, String>((source, lease))
        }).transpose()?;
        Ok((run.clone(), router, profile.clone(), super::profile_directory(owner, &profile.id)?, root(owner, &run.id)?, lease, source_lease))
    })?;
    let prepared = Prepared::prepare(
        app,
        shells,
        run.shell_profile_id
            .as_deref()
            .ok_or("Choose a local shell for native coding.")?,
        &run.cwd,
        router.cli,
        &directory,
        cancelled.clone(),
    )?;
    let version = prepared.version().to_owned();
    let directory_identity = workspace(&run.cwd)?;
    let kind = prepared.kind();
    let previous = Checkpoint::read(&root)?;
    let input = run
        .inputs
        .last()
        .cloned()
        .ok_or("No native message is saved.")?;
    let mut session = String::new();
    let mut resume = false;
    let mut previous_record = None;
    let mut transfer = None;
    if let Some(checkpoint) = &previous {
        if checkpoint.generation != run.generation
            && run.attempts.last().is_some_and(|attempt| {
                attempt.generation == run.generation && attempt.state != AttemptState::Rejected
            })
        {
            return Err("The latest native attempt has no settled checkpoint. Review its exact session in the native account terminal; an older checkpoint cannot authorize replay.".into());
        }
        if !checkpoint.matches(&run, router.cli, &version) {
            return Err("The native model, workspace, client or history grant changed. Existing history is retained.".into());
        }
        previous_record = Some(validate_history(&root, checkpoint)?);
        if checkpoint.profile_id != profile.id {
            let source = &source_lease
                .as_ref()
                .ok_or("The native history source is not leased.")?
                .0;
            if kind != NativeKind::Pi
                || checkpoint.generation != run.generation
                || source.revision != checkpoint.profile_revision
                || source.cli != TitleCli::Pi
                || !source.enabled
                || source.storage_mode == StorageMode::ApiKey
                || source.gateway_provider.is_some()
                || !matches!(source.auth_state, AuthState::Ready | AuthState::Unverified)
                || !router.ordered_profile_ids.contains(&source.id)
            {
                return Err("The saved native history requires a qualified, reviewed transfer to this account.".into());
            }
            let request = super::native_transfer::ForkRequest {
                run_id: run.id.clone(),
                input_id: checkpoint.input_id.clone(),
                source_generation: checkpoint.generation,
                next_generation: run
                    .generation
                    .checked_add(1)
                    .ok_or("Native generation exhausted.")?,
                source_file: previous_record
                    .as_ref()
                    .and_then(|record| record.session_file.clone())
                    .ok_or("The source Pi session file is unavailable.")?,
                source_session: checkpoint.session_id.clone(),
                source_profile: source.id.clone(),
                source_revision: source.revision,
                destination_profile: profile.id.clone(),
                destination_revision: profile.revision,
                cwd: run.cwd.clone(),
                cwd_identity: directory_identity,
                model: checkpoint.model.clone(),
                version: version.clone(),
            };
            let receipt = super::native_transfer::read(&root, request.next_generation)?;
            super::native_transfer::fence(&root, &request, &receipt)?;
            session = receipt.session_id.clone();
            transfer = Some((request, receipt));
        } else if checkpoint.profile_revision != profile.revision {
            return Err("The saved native conversation belongs to a different account revision. Review a handoff before transferring its history.".into());
        }
        if checkpoint.input_id == input.id && checkpoint.completed {
            return Err(
                "This native input already completed. Its checkpoint blocks duplicate execution."
                    .into(),
            );
        }
        if checkpoint.input_id == input.id && !run.continuation_requested {
            return Err("Explicitly continue the retained native task before resuming.".into());
        }
        if transfer.is_none() {
            session = checkpoint.session_id.clone();
        }
        resume = true;
    } else if !run.attempts.is_empty()
        && !(run.state == RunState::Switching
            && run
                .attempts
                .last()
                .is_some_and(|attempt| attempt.state == AttemptState::Rejected))
    {
        return Err("This native run has an earlier uncheckpointed attempt. Automatic replay is unavailable.".into());
    }
    let (provider, model) = model_parts(
        router.cli,
        run.model
            .as_deref()
            .ok_or("Choose an exact native model.")?,
    )?;
    let mut arguments = kind.launch_for_model(model, provider)?.arguments;
    if kind == NativeKind::Claude {
        if session.is_empty() {
            session = run.id.clone();
            arguments.extend(["--session-id".into(), session.clone()]);
        } else {
            arguments.extend(["--resume".into(), session.clone()]);
        }
    }
    let pi_name = if kind == NativeKind::Pi && resume {
        if let Some((_, receipt)) = &transfer {
            receipt.destination_file.clone()
        } else {
            previous_record
                .as_ref()
                .and_then(|record| record.session_file.clone())
                .ok_or("The native Pi checkpoint has no qualified session file binding.")?
        }
    } else {
        format!(
            "pi-session-{}.jsonl",
            run.generation
                .checked_add(1)
                .ok_or("Native generation exhausted.")?
        )
    };
    if !pi_name
        .strip_prefix("pi-session-")
        .and_then(|name| name.strip_suffix(".jsonl"))
        .is_some_and(|id| id.parse::<u64>().is_ok())
    {
        return Err("Invalid native Pi session file binding.".into());
    }
    let pi_path = root.join(&pi_name);
    let pi_history = if kind == NativeKind::Pi && resume {
        Some(super::native_history::pi_file(
            &root, &pi_path, &session, &run.cwd,
        )?)
    } else {
        None
    };
    if kind == NativeKind::Pi {
        arguments.extend(["--session".into(), pi_path.to_string_lossy().into_owned()]);
    }
    run.generation = run
        .generation
        .checked_add(1)
        .ok_or("Native generation exhausted.")?;
    let attempt_id = super::new_id()?;
    let generation = run.generation;
    let project = super::project_lease::activate(Path::new(&run.cwd), &run.id, generation)?;
    let saved = state.with_store(app, |owner| {
        owner.store.update(|snapshot| {
            let current = snapshot
                .runs
                .iter_mut()
                .find(|current| current.id == run.id && current.revision == run.revision)
                .ok_or("Native run changed during preparation.")?;
            current.generation = generation;
            current.active_profile_id = Some(profile.id.clone());
            current.state = RunState::Running;
            current.continuation_requested = false;
            current.attempted_profile_ids.push(profile.id.clone());
            current.attempts.push(RunAttempt {
                id: attempt_id.clone(),
                input_id: input.id.clone(),
                profile_id: profile.id.clone(),
                generation,
                state: AttemptState::DispatchIntent,
                reason: "Native CLI retains its own OAuth, tools and permission policy.".into(),
            });
            current.turns.push(RunTurn {
                input_id: input.id.clone(),
                attempt_id: attempt_id.clone(),
                profile_id: profile.id.clone(),
                generation,
                state: AttemptState::DispatchIntent,
                text: String::new(),
            });
            current.status_message =
                "Starting the native coding client. Native authentication and tool policies apply."
                    .into();
            current.revision += 1;
            Ok(())
        })
    })?;
    state.changed(app, saved.revision);
    let binding = Binding {
        state,
        app,
        run: &run,
        router: &router,
        profile: &profile,
        transfer_source: source_lease.as_ref().map(|(source, _)| source),
        cancel: cancelled,
        directory_identity,
    };
    let mut wire = NativeWire::new(kind, &session, &attempt_id)?;
    let mut record = Record {
        schema: 1,
        run_id: run.id.clone(),
        input_id: input.id.clone(),
        profile_id: profile.id.clone(),
        profile_revision: profile.revision,
        generation,
        session_id: wire.session_id().into(),
        model: run.model.clone().unwrap_or_default(),
        version: version.clone(),
        cwd: run.cwd.clone(),
        directory_identity: Some(directory_identity),
        ledger: Ledger::new(),
        output: String::new(),
        tools: vec![],
        session_file: (kind == NativeKind::Pi).then_some(pi_name),
    };
    save(&root, &record)?;
    let mut process = prepared.spawn(&arguments, || {
        binding.fence()?;
        if let Some(history) = &pi_history {
            history.fence()?;
        }
        if let Some((request, receipt)) = &transfer {
            let retained = Checkpoint::read(&root)?
                .ok_or("The source checkpoint disappeared before transfer.")?;
            if serde_json::to_vec(&Some(retained))
                .map_err(|_| "Cannot bind the source checkpoint.")?
                != serde_json::to_vec(&previous)
                    .map_err(|_| "Cannot bind the source checkpoint.")?
            {
                return Err("The source checkpoint changed before transfer dispatch.".into());
            }
            super::native_transfer::fence(&root, request, receipt)?;
        }
        Ok(())
    })?;
    for request in wire.initialize("initialize")? {
        startup_send(
            &mut process,
            &mut wire,
            request,
            "initialize",
            &binding,
            &mut record,
            &root,
        )?;
    }
    startup(&mut process, &mut wire, &binding, &mut record, &root)?;
    if let Some(request) = wire.authentication_request("authenticate")? {
        startup_send(
            &mut process,
            &mut wire,
            request,
            "authenticate",
            &binding,
            &mut record,
            &root,
        )?;
        startup(&mut process, &mut wire, &binding, &mut record, &root)?;
    }
    if let Some(request) = wire.initialized() {
        startup_send(
            &mut process,
            &mut wire,
            request,
            "initialized",
            &binding,
            &mut record,
            &root,
        )?;
    }
    if !matches!(kind, NativeKind::Claude | NativeKind::Pi) {
        let request = wire.session_request("session", &run.cwd, resume)?;
        startup_send(
            &mut process,
            &mut wire,
            request,
            "session",
            &binding,
            &mut record,
            &root,
        )?;
        startup(&mut process, &mut wire, &binding, &mut record, &root)?;
    }
    for request in
        wire.configure_model("model", model, provider, run.reasoning_effort.as_deref())?
    {
        startup_send(
            &mut process,
            &mut wire,
            request,
            "model",
            &binding,
            &mut record,
            &root,
        )?;
    }
    startup(&mut process, &mut wire, &binding, &mut record, &root)?;
    if matches!(
        kind,
        NativeKind::Kimi | NativeKind::Kilo | NativeKind::OpenCode
    ) {
        let request = wire.event_stream()?;
        startup_send(
            &mut process,
            &mut wire,
            request,
            "events",
            &binding,
            &mut record,
            &root,
        )?;
        if kind == NativeKind::Kimi {
            binding.fence()?;
            process.send(&wire.websocket_hello("hello")?)?;
            let subscription_deadline = Instant::now() + Duration::from_secs(30);
            while !wire.subscription_ready() {
                binding.fence()?;
                if Instant::now() >= subscription_deadline {
                    return Err("Native event subscription was not acknowledged. No user prompt was dispatched.".into());
                }
                if let Some(raw) = process.poll()? {
                    let events = wire.observe(raw)?;
                    startup_observations(&wire, events, &mut record, &root)?;
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
    save(&root, &record)?;
    let prompt = if run.continuation_requested && resume {
        "Continue the unfinished user task from the saved native conversation. Respect completed tool results and inspect uncertain changes before performing another operation."
    } else {
        &input.text
    };
    let prompt_id = prompt_id(&attempt_id)?;
    let request = wire.prompt(&prompt_id, prompt)?;
    let events = send(&mut process, &mut wire, request, &prompt_id, &binding)?;
    observe(
        &binding,
        &mut process,
        &mut wire,
        events,
        &mut record,
        &root,
    )?;
    let deadline = Instant::now() + Duration::from_secs(3600);
    let mut last_save = Instant::now();
    while wire.terminal().is_none() {
        binding.fence()?;
        project.check(&run.id, generation)?;
        if Instant::now() >= deadline {
            record.ledger.complete_observation = false;
            save(&root, &record)?;
            return Err("Native coding ended without a confirmed terminal boundary. Its history is retained; no prompt was replayed.".into());
        }
        if let Some(raw) = process.poll()? {
            let events = wire.observe(raw)?;
            observe(
                &binding,
                &mut process,
                &mut wire,
                events,
                &mut record,
                &root,
            )?;
        } else if process.exited() {
            record.ledger.complete_observation = false;
            save(&root, &record)?;
            return Err(
                "Native process ended without a confirmed turn result. Its history is retained."
                    .into(),
            );
        } else {
            thread::sleep(Duration::from_millis(10));
        }
        if last_save.elapsed() >= Duration::from_millis(250) {
            save(&root, &record)?;
            publish_output(&binding, &attempt_id, &record.output)?;
            last_save = Instant::now();
        }
    }
    let terminal = wire
        .terminal()
        .cloned()
        .ok_or("Missing native turn result.")?;
    if matches!(
        kind,
        NativeKind::Kimi | NativeKind::Kilo | NativeKind::OpenCode
    ) {
        let snapshot_deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() >= snapshot_deadline {
                return Err(
                    "Native idle snapshot could not be reconciled with the complete event journal."
                        .into(),
                );
            }
            let mut idle_views = Vec::new();
            for request in wire.idle_snapshot_requests()? {
                binding.fence()?;
                let WireRequest::Http { method, path, body } = request else {
                    return Err("Unqualified native idle transport.".into());
                };
                idle_views.push(process.request(method, &path, body.as_ref())?);
            }
            if let Some((target, epoch)) = wire.idle_snapshot_watermark(&idle_views)? {
                loop {
                    binding.fence()?;
                    let (observed, observed_epoch) = wire
                        .observed_journal_cursor()
                        .ok_or("Native journal cursor was lost.")?;
                    if epoch != observed_epoch {
                        return Err("Native journal epoch changed before drain.".into());
                    }
                    if observed >= target {
                        break;
                    }
                    if Instant::now() >= snapshot_deadline {
                        return Err(
                            "Native terminal journal did not reach its snapshot watermark.".into(),
                        );
                    }
                    if let Some(raw) = process.poll()? {
                        let events = wire.observe(raw)?;
                        observe(
                            &binding,
                            &mut process,
                            &mut wire,
                            events,
                            &mut record,
                            &root,
                        )?;
                    } else if process.exited() {
                        return Err(
                            "Native server exited before its terminal journal was observed.".into(),
                        );
                    } else {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                if wire.observed_journal_cursor() != Some((target, epoch)) {
                    continue;
                }
            }
            wire.validate_idle_snapshots(&run.cwd, &idle_views)?;
            break;
        }
    }
    let remaining = process.drain()?;
    for raw in remaining {
        let events = wire.observe(raw)?;
        observe(
            &binding,
            &mut process,
            &mut wire,
            events,
            &mut record,
            &root,
        )?;
    }
    record.ledger.drained = process.drained();
    binding.fence()?;
    record.session_id = wire.session_id().into();
    record.tools = wire.tools();
    let bytes = save(&root, &record)?;
    let completed = terminal.outcome == NativeOutcome::Completed;
    let quota = terminal.error_class == Some(ErrorClass::Quota);
    if record.ledger.quiescent() && !record.session_id.is_empty() {
        Checkpoint::new(
            router.cli,
            &version,
            &run,
            &input.id,
            &record.session_id,
            &profile,
            record.ledger.clone(),
            completed,
            quota,
            &bytes,
        )?
        .write(&root)?;
    }
    // Native hooks/plugins may have effects outside the tool stream. Until a
    // complete no-effect contract is qualified, subscription coding retains
    // its creator account and requires explicit review on quota failure.
    let status = if completed && record.ledger.quiescent() {
        AttemptState::Completed
    } else {
        AttemptState::RecoveryRequired
    };
    let saved = state.with_store(app, |owner| owner.store.update(|snapshot| {
        let current = snapshot.runs.iter_mut().find(|current| current.id == run.id && current.generation == generation).ok_or("Native run changed while recording its result.")?;
        if let Some(attempt) = current.attempts.iter_mut().find(|attempt| attempt.id == attempt_id) { attempt.state = status; }
        if let Some(turn) = current.turns.iter_mut().find(|turn| turn.attempt_id == attempt_id) { turn.state = status; turn.text = record.output.clone(); }
        current.output = record.output.clone(); current.revision += 1;
        current.state = if status == AttemptState::Completed { current.attempted_profile_ids.clear(); RunState::Completed } else { RunState::RecoveryRequired };
        current.status_message = if completed && record.ledger.quiescent() { "Native coding turn completed. Its conversation and tool boundary were checkpointed.".into() } else if quota && record.ledger.has_tools() { "Native quota ended an effectful turn. Review the original account's saved session and tool results before continuing.".into() } else if quota { "Native quota rejected this turn. Native startup effects are not fully qualified; review the original session before choosing another account.".into() } else { "The native turn did not complete safely. Its history and tool observations are retained; explicitly review continuation.".into() };
        Ok(())
    }))?;
    state.changed(app, saved.revision);
    drop(lease);
    drop(project);
    Ok(())
}
fn publish_output(binding: &Binding<'_>, attempt_id: &str, text: &str) -> Result<(), String> {
    let snapshot = binding.state.with_store(binding.app, |owner| {
        owner.store.update(|snapshot| {
            let run = snapshot
                .runs
                .iter_mut()
                .find(|run| run.id == binding.run.id && run.generation == binding.run.generation)
                .ok_or("Native output owner changed.")?;
            run.output = text.into();
            if let Some(turn) = run
                .turns
                .iter_mut()
                .find(|turn| turn.attempt_id == attempt_id)
            {
                turn.text = text.into();
                turn.state = AttemptState::Running;
            }
            if let Some(attempt) = run
                .attempts
                .iter_mut()
                .find(|attempt| attempt.id == attempt_id)
            {
                attempt.state = AttemptState::Running;
            }
            run.revision += 1;
            Ok(())
        })
    })?;
    binding.state.changed(binding.app, snapshot.revision);
    Ok(())
}

/// Reconcile a completed checkpoint after a database publication interruption.
/// This operation reads owned history and never starts a native client.
pub(crate) fn completed_checkpoint(
    owner: &super::Owner,
    snapshot: &Snapshot,
    run: &Run,
) -> Result<(), String> {
    let router = snapshot
        .routers
        .iter()
        .find(|router| router.id == run.router_id)
        .ok_or("Native router no longer exists.")?;
    let root = root(owner, &run.id)?;
    let checkpoint = Checkpoint::read(&root)?
        .ok_or("This native turn has no complete checkpoint to acknowledge.")?;
    let input = run.inputs.last().ok_or("There is no saved native input.")?;
    if !checkpoint.completed
        || checkpoint.input_id != input.id
        || checkpoint.generation != run.generation
        || !checkpoint.matches(run, router.cli, &checkpoint.version)
    {
        return Err(
            "This input has no complete owned native checkpoint. No task will be replayed.".into(),
        );
    }
    validate_history(&root, &checkpoint)?;
    Ok(())
}

/// Open an uncertain session for explicit inspection in the original account.
/// Its external interaction retires automatic checkpoint authority.
pub(crate) fn recovery_history(
    owner: &super::Owner,
    run: &Run,
    profile: &Profile,
    cli: TitleCli,
    version: &str,
    account: &Path,
) -> Result<super::native_history::Continuation, String> {
    let history_root = root(owner, &run.id)?;
    let (record, _) = read_record(&history_root, run.generation)?;
    if record.directory_identity != Some(workspace(&run.cwd)?) {
        return Err(
            "The retained native session belongs to a replaced workspace directory.".into(),
        );
    }
    if record.schema != 1
        || record.run_id != run.id
        || record.generation != run.generation
        || run
            .inputs
            .last()
            .is_none_or(|input| input.id != record.input_id)
        || record.profile_id != profile.id
        || record.profile_revision != profile.revision
        || record.cwd != run.cwd
        || run.model.as_deref() != Some(record.model.as_str())
        || record.version != version
        || record.session_id.is_empty()
    {
        return Err("The retained native session is unavailable or its account binding changed. Review the original account's own session picker.".into());
    }
    if cli == TitleCli::Pi {
        let name = record
            .session_file
            .as_deref()
            .ok_or("The retained Pi session has no owned file binding.")?;
        if !name
            .strip_prefix("pi-session-")
            .and_then(|name| name.strip_suffix(".jsonl"))
            .is_some_and(|id| id.parse::<u64>().is_ok())
        {
            return Err("Invalid native Pi history binding.".into());
        }
        super::native_history::pi_file(
            &history_root,
            &history_root.join(name),
            &record.session_id,
            &run.cwd,
        )
    } else {
        super::native_history::select_account(account, cli, version, &run.cwd, &record.session_id)
    }
}
pub(crate) fn retire_checkpoint(owner: &super::Owner, run: &Run) -> Result<(), String> {
    let history_root = root(owner, &run.id)?;
    let (mut record, _) = read_record(&history_root, run.generation)?;
    record.ledger.complete_observation = false;
    save(&history_root, &record)?;
    let checkpoint = history_root.join("native-checkpoint.json");
    crate::chat::storage::reject_link(&checkpoint)?;
    match fs::remove_file(checkpoint) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Cannot retire the native checkpoint before interactive recovery.".into()),
    }
}

pub(crate) fn fence_recovery_workspace(owner: &super::Owner, run: &Run) -> Result<(), String> {
    let history_root = root(owner, &run.id)?;
    let (record, _) = read_record(&history_root, run.generation)?;
    if record.run_id != run.id
        || record.generation != run.generation
        || record.cwd != run.cwd
        || record.directory_identity != Some(workspace(&run.cwd)?)
    {
        return Err("The native recovery workspace changed before terminal launch.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, os::unix::fs::PermissionsExt};

    struct Fixture {
        owner: super::super::Owner,
        run: Run,
        snapshot: Snapshot,
        checkpoint: Checkpoint,
        history: PathBuf,
        _directory: tempfile::TempDir,
    }

    impl Fixture {
        fn completed() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let storage = fs::canonicalize(directory.path()).unwrap();
            let cwd = storage.join("workspace");
            fs::create_dir(&cwd).unwrap();
            let owner = super::super::Owner {
                store: super::super::store::Store::open(&storage.join("router.sqlite")).unwrap(),
                root: storage,
                quota_checks: HashMap::new(),
                quota_retry_at: HashMap::new(),
            };
            let profile = Profile {
                id: "profile".into(),
                cli: TitleCli::Codex,
                label: "Native".into(),
                enabled: true,
                revision: 1,
                auth_state: AuthState::Ready,
                storage_mode: StorageMode::CliManaged,
                credential_ref: None,
                quota_group_key: None,
                gateway_provider: None,
            };
            let run = Run {
                id: "run".into(),
                router_id: "router".into(),
                cwd: cwd.to_str().unwrap().into(),
                shell_profile_id: None,
                title: "Native task".into(),
                state: RunState::RecoveryRequired,
                model: Some("native-model".into()),
                reasoning_effort: None,
                execution_mode: RunExecutionMode::Native,
                continuation_requested: false,
                pinned_profile_id: None,
                allowed_profile_ids: vec![profile.id.clone()],
                active_profile_id: Some(profile.id.clone()),
                generation: 1,
                revision: 1,
                inputs: vec![RunInput {
                    id: "input".into(),
                    text: "Inspect workspace".into(),
                }],
                attempts: vec![],
                output: String::new(),
                turns: vec![],
                legacy_output: None,
                status_message: String::new(),
                attempted_profile_ids: vec![],
            };
            let snapshot = Snapshot {
                routers: vec![Router {
                    id: run.router_id.clone(),
                    cli: TitleCli::Codex,
                    label: "Native".into(),
                    enabled: true,
                    ordered_profile_ids: vec![profile.id.clone()],
                    balance_remaining_quota: false,
                    revision: 1,
                }],
                profiles: vec![profile.clone()],
                runs: vec![run.clone()],
                ..Snapshot::default()
            };
            let history = root(&owner, &run.id).unwrap();
            let mut ledger = Ledger::new();
            ledger.observe().unwrap();
            ledger.tool("read-workspace", true).unwrap();
            ledger.terminal = true;
            ledger.drained = true;
            let record = Record {
                schema: 1,
                run_id: run.id.clone(),
                input_id: "input".into(),
                profile_id: profile.id.clone(),
                profile_revision: profile.revision,
                generation: run.generation,
                session_id: "native-session".into(),
                model: run.model.clone().unwrap(),
                version: "0.160.0".into(),
                cwd: run.cwd.clone(),
                directory_identity: Some(workspace(&run.cwd).unwrap()),
                ledger: ledger.clone(),
                output: "Workspace inspected".into(),
                tools: vec![],
                session_file: None,
            };
            let bytes = save(&history, &record).unwrap();
            let checkpoint = Checkpoint::new(
                TitleCli::Codex,
                &record.version,
                &run,
                &record.input_id,
                &record.session_id,
                &profile,
                ledger,
                true,
                false,
                &bytes,
            )
            .unwrap();
            checkpoint.write(&history).unwrap();
            Self {
                owner,
                run,
                snapshot,
                checkpoint,
                history,
                _directory: directory,
            }
        }
    }

    #[test]
    fn owned_canonical_history_acknowledges_completion_without_replay() {
        let fixture = Fixture::completed();
        let path = fixture.history.join("attempt-1.json");
        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.nlink(), 1);
        let record = validate_history(&fixture.history, &fixture.checkpoint).unwrap();
        assert_eq!(record.output, "Workspace inspected");
        completed_checkpoint(&fixture.owner, &fixture.snapshot, &fixture.run).unwrap();
        fence_recovery_workspace(&fixture.owner, &fixture.run).unwrap();
    }

    #[test]
    fn pi_handoff_review_requires_latest_settled_history_and_both_exact_grants() {
        let mut fixture = Fixture::completed();
        fixture.run.model = Some("openai/gpt-4.1".into());
        fixture.run.state = RunState::Completed;
        fixture.run.allowed_profile_ids.push("destination".into());
        fixture.snapshot.routers[0].cli = TitleCli::Pi;
        fixture.snapshot.routers[0]
            .ordered_profile_ids
            .push("destination".into());
        fixture.snapshot.profiles[0].cli = TitleCli::Pi;
        let mut destination = fixture.snapshot.profiles[0].clone();
        destination.id = "destination".into();
        destination.label = "Destination".into();
        fixture.snapshot.profiles.push(destination);
        fixture.snapshot.runs[0] = fixture.run.clone();
        let (mut record, _) = read_record(&fixture.history, 1).unwrap();
        record.model = fixture.run.model.clone().unwrap();
        record.version = "1.0.1".into();
        record.session_file = Some("pi-session-1.jsonl".into());
        let usage = json!({"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
            "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}});
        let entries = [
            json!({"type":"session","version":3,"id":record.session_id,"timestamp":"2026-01-01T00:00:00Z","cwd":fixture.run.cwd}),
            json!({"type":"message","id":"u","parentId":null,"timestamp":"2026-01-01T00:00:00Z",
                "message":{"role":"user","content":"Retained input","timestamp":1}}),
            json!({"type":"message","id":"a","parentId":"u","timestamp":"2026-01-01T00:00:00Z",
                "message":{"role":"assistant","content":[{"type":"text","text":"Completed"}],
                    "api":"openai-completions","provider":"openai","model":"gpt-4.1","usage":usage,"stopReason":"stop","timestamp":2}}),
        ];
        let bytes: Vec<u8> = entries
            .iter()
            .flat_map(|entry| {
                let mut bytes = serde_json::to_vec(entry).unwrap();
                bytes.push(b'\n');
                bytes
            })
            .collect();
        crate::chat::storage::atomic(&fixture.history.join("pi-session-1.jsonl"), &bytes).unwrap();
        let record_bytes = save(&fixture.history, &record).unwrap();
        fixture.checkpoint = Checkpoint::new(
            TitleCli::Pi,
            &record.version,
            &fixture.run,
            &record.input_id,
            &record.session_id,
            &fixture.snapshot.profiles[0],
            record.ledger,
            true,
            false,
            &record_bytes,
        )
        .unwrap();
        fixture.checkpoint.write(&fixture.history).unwrap();
        let state = CliRouterService::default();
        let request = super::super::native_transfer_commands::PreviewRequest {
            run_id: fixture.run.id.clone(),
            expected_revision: fixture.run.revision,
            destination_profile_id: "destination".into(),
            destination_profile_revision: 1,
        };
        let (fork, digest, size, completed) =
            handoff_review_at_root(&state, &fixture.owner.root, &fixture.snapshot, &request)
                .unwrap();
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(digest, format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(fork.next_generation, 2);
        assert!(completed);
        assert!(!fixture.history.join("pi-session-2.jsonl").exists());
        assert!(!fixture.history.join("pi-transfer-2.json").exists());
        for index in 0..6 {
            let mut changed = fixture.snapshot.clone();
            match index {
                0 => changed.runs[0]
                    .allowed_profile_ids
                    .retain(|id| id != "destination"),
                1 => changed.runs[0]
                    .allowed_profile_ids
                    .retain(|id| id != "profile"),
                2 => changed.profiles[1].revision += 1,
                3 => changed.profiles[0].revision += 1,
                4 => changed.runs[0].generation += 1,
                _ => changed.runs[0].state = RunState::Running,
            }
            assert!(
                handoff_review_at_root(&state, &fixture.owner.root, &changed, &request).is_err(),
                "gate {index}"
            );
        }
        let marker = Arc::new(AtomicBool::new(false));
        state
            .profile_terminals
            .lock()
            .unwrap()
            .insert("destination".into(), marker);
        assert!(
            handoff_review_at_root(&state, &fixture.owner.root, &fixture.snapshot, &request)
                .is_err()
        );
        assert_eq!(
            fs::read(fixture.history.join("pi-session-1.jsonl")).unwrap(),
            bytes
        );
    }

    #[test]
    fn changed_private_canonical_history_cannot_acknowledge_old_checkpoint() {
        let fixture = Fixture::completed();
        let (mut record, _) = read_record(&fixture.history, fixture.run.generation).unwrap();
        record.output.push_str("; changed after completion");
        save(&fixture.history, &record).unwrap();
        let error =
            completed_checkpoint(&fixture.owner, &fixture.snapshot, &fixture.run).unwrap_err();
        assert!(
            error.contains("history changed after checkpoint"),
            "{error}"
        );
    }

    #[test]
    fn replaced_workspace_blocks_checkpoint_and_final_recovery_fence() {
        let fixture = Fixture::completed();
        // Retain the old inode so the replacement cannot accidentally reuse it.
        let old_workspace = fixture.owner.root.join("retained-workspace");
        fs::rename(&fixture.run.cwd, &old_workspace).unwrap();
        fs::create_dir(&fixture.run.cwd).unwrap();
        let (record, _) = read_record(&fixture.history, fixture.run.generation).unwrap();
        assert_ne!(
            record.directory_identity,
            Some(workspace(&fixture.run.cwd).unwrap())
        );
        let error =
            completed_checkpoint(&fixture.owner, &fixture.snapshot, &fixture.run).unwrap_err();
        assert!(error.contains("replaced workspace directory"), "{error}");
        let error = fence_recovery_workspace(&fixture.owner, &fixture.run).unwrap_err();
        assert!(
            error.contains("workspace changed before terminal launch"),
            "{error}"
        );
    }

    #[test]
    fn history_admission_rejects_public_and_noncanonical_records() {
        let fixture = Fixture::completed();
        let path = fixture.history.join("attempt-1.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let error =
            completed_checkpoint(&fixture.owner, &fixture.snapshot, &fixture.run).unwrap_err();
        assert!(error.contains("not privately owned"), "{error}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let (record, _) = read_record(&fixture.history, fixture.run.generation).unwrap();
        let pretty = serde_json::to_vec_pretty(&record).unwrap();
        crate::chat::storage::atomic(&path, &pretty).unwrap();
        // Even a checkpoint rebound to these bytes cannot authorize noncanonical history.
        let mut checkpoint = fixture.checkpoint.clone();
        checkpoint.history_digest = format!("{:x}", Sha256::digest(&pretty));
        checkpoint.write(&fixture.history).unwrap();
        let error =
            completed_checkpoint(&fixture.owner, &fixture.snapshot, &fixture.run).unwrap_err();
        assert!(error.contains("not canonically encoded"), "{error}");
    }

    #[test]
    fn interactive_retirement_removes_completion_authority_but_keeps_recovery_fence() {
        let fixture = Fixture::completed();
        completed_checkpoint(&fixture.owner, &fixture.snapshot, &fixture.run).unwrap();
        retire_checkpoint(&fixture.owner, &fixture.run).unwrap();
        assert!(Checkpoint::read(&fixture.history).unwrap().is_none());
        let (record, _) = read_record(&fixture.history, fixture.run.generation).unwrap();
        assert!(!record.ledger.complete_observation);
        assert!(!record.ledger.quiescent());
        let error =
            completed_checkpoint(&fixture.owner, &fixture.snapshot, &fixture.run).unwrap_err();
        assert!(
            error.contains("no complete checkpoint to acknowledge"),
            "{error}"
        );
        fence_recovery_workspace(&fixture.owner, &fixture.run).unwrap();
    }
}
