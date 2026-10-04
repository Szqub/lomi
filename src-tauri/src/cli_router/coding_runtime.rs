//! Persistent Codex run ownership and mediated project effects.
use super::{
    codex, coding_approvals,
    coding_journal::{DispatchBinding, Journal},
    coding_transport::{self, DurableToolReply, Supervisor},
    effects, now, policy, project_lease, runtime,
    types::*,
    CliRouterService,
};
use crate::{cli_catalog::TitleCli, terminal::Shells};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::AppHandle;

fn health(healthy: &Arc<AtomicBool>) -> Result<(), String> {
    if healthy.load(Ordering::SeqCst) {
        Ok(())
    } else {
        Err(
            "The native coding transport lost admission. Review retained results before recovery."
                .into(),
        )
    }
}
fn receipt(binding: &effects::CallBinding, result: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(binding, result))
        .map_err(|_| "Cannot bind the durable coding result.")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn protected(root: &Path) -> Result<Vec<PathBuf>, String> {
    let home = effects::account_home()?;
    let mut paths = vec![root
        .parent()
        .ok_or("Cannot protect native router storage.")?
        .to_owned()];
    for name in [
        ".config",
        "Library",
        ".codex",
        ".claude",
        ".gemini",
        ".copilot",
        ".cursor",
        ".qwen",
        ".kimi",
        ".hermes",
        ".openclaw",
        ".pi",
        ".goose",
        ".aider",
        ".cline",
        ".kilo",
        ".kiro",
        ".factory",
        ".openhands",
        ".continue",
        ".amp",
        ".augment",
        ".crush",
        ".vibe",
        ".grok",
        ".junie",
        ".deepagents",
        ".freebuff",
        ".trae",
        ".swe-agent",
        ".ssh",
        ".aws",
        ".azure",
        ".gnupg",
    ] {
        paths.push(home.join(name));
    }
    // Include application-visible native namespace overrides. No environment
    // values are accepted from the webview or included in UI/history receipts.
    for namespace in super::adapters::namespace_variables() {
        if namespace != "HOME" {
            if let Some(path) = std::env::var_os(namespace) {
                let path = PathBuf::from(path);
                if path.is_absolute() && path != home {
                    paths.push(path);
                }
            }
        }
    }
    let mut existing = Vec::new();
    for path in paths {
        match fs::symlink_metadata(&path) {
            Ok(_) => existing.push(
                path.canonicalize()
                    .map_err(|_| "Cannot protect native CLI configuration.")?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect native CLI configuration ownership.".into()),
        }
    }
    existing.sort();
    existing.dedup();
    Ok(existing)
}
pub(crate) fn home(owner: &super::Owner, run_id: &str, model: &str) -> Result<PathBuf, String> {
    if run_id.len() != 36
        || run_id
            .chars()
            .any(|character| !character.is_ascii_hexdigit() && character != '-')
    {
        return Err("Invalid coding run identifier.".into());
    }
    let runs = owner.root.join("runs");
    let run = runs.join(run_id);
    let coding = run.join("coding");
    for path in [&runs, &run, &coding] {
        crate::chat::storage::reject_link(path)?;
        fs::create_dir_all(path).map_err(|_| "Cannot create private coding run storage.")?;
        crate::chat::storage::private(path, true)?;
    }
    codex::coding_home(&coding, model)
}

struct Owner<'a> {
    state: &'a CliRouterService,
    app: &'a AppHandle,
    run_id: &'a str,
    cancelled: &'a Arc<AtomicBool>,
    lease: project_lease::Lease,
    binding: DispatchBinding,
    journal: Journal,
    broker: effects::Broker,
    persisted: String,
    dispatched: bool,
}
impl Owner<'_> {
    fn call(&self, request: &codex::ToolRequest) -> effects::CallBinding {
        effects::CallBinding {
            run_id: self.run_id.into(),
            attempt_id: self.binding.attempt_id.clone(),
            generation: self.binding.generation,
            auth_revision: self.binding.auth_revision,
            thread_id: request.thread_id.clone(),
            turn_id: request.turn_id.clone(),
            call_id: request.call_id.clone(),
        }
    }
    fn preturn_binding(&self) -> effects::CallBinding {
        effects::CallBinding {
            run_id: self.run_id.into(),
            attempt_id: self.binding.attempt_id.clone(),
            generation: self.binding.generation,
            auth_revision: self.binding.auth_revision,
            thread_id: "pre-turn".into(),
            turn_id: "pre-turn".into(),
            call_id: "pre-turn".into(),
        }
    }
}
impl Supervisor for Owner<'_> {
    fn release_turn(
        &mut self,
        protocol: &mut codex::CodingProtocol,
        enqueue: &mut dyn FnMut(Value) -> Result<(), String>,
    ) -> Result<(), String> {
        let binding = self.preturn_binding();
        self.state.with_store(self.app, |owner| {
            coding_approvals::fence_locked(self.state, owner, &binding, self.cancelled)?;
            self.lease.check(self.run_id, self.binding.generation)?;
            let receipt = self.journal.dispatch(
                &self.binding,
                protocol
                    .native_thread()
                    .ok_or("Missing native coding thread.")?,
                protocol
                    .native_history()
                    .ok_or("Missing native coding history.")?,
            )?;
            self.dispatched = true;
            let saved = owner.store.update(|snapshot| {
                let run = snapshot
                    .runs
                    .iter_mut()
                    .find(|run| run.id == self.run_id && run.generation == self.binding.generation)
                    .ok_or("Coding run generation changed.")?;
                run.attempts
                    .last_mut()
                    .filter(|attempt| attempt.id == self.binding.attempt_id)
                    .ok_or("Coding attempt changed.")?
                    .state = AttemptState::Running;
                run.turns
                    .last_mut()
                    .filter(|turn| turn.attempt_id == self.binding.attempt_id)
                    .ok_or("Coding turn changed.")?
                    .state = AttemptState::Running;
                run.revision = run
                    .revision
                    .checked_add(1)
                    .ok_or("Coding run revision exhausted.")?;
                run.status_message =
                    "Running in the saved native thread. Project writes require your approval."
                        .into();
                Ok(())
            })?;
            coding_approvals::fence_locked(self.state, owner, &binding, self.cancelled)?;
            enqueue(
                protocol.release_turn_start(&receipt).map_err(|_| {
                    "Native coding input intent does not match its durable receipt."
                })?,
            )?;
            self.state.changed(self.app, saved.revision);
            Ok(())
        })
    }
    fn tool(
        &mut self,
        request: &codex::ToolRequest,
        cancelled: &Arc<AtomicBool>,
        healthy: &Arc<AtomicBool>,
    ) -> Result<DurableToolReply, String> {
        let binding = self.call(request);
        health(healthy)?;
        coding_approvals::fence(self.state, self.app, &binding, cancelled)?;
        self.lease.check(self.run_id, self.binding.generation)?;
        let prepared = self
            .broker
            .prepare(binding.clone(), &request.tool, &request.arguments)?;
        let result = match prepared {
            effects::Prepared::Result(result) => result,
            effects::Prepared::ApprovalRequired(preview) => {
                let ticket = self.state.approvals.request(
                    self.state,
                    self.app,
                    self.lease.root(),
                    preview.as_ref().clone(),
                    cancelled,
                )?;
                let consent = ticket.wait(cancelled, healthy)?;
                let result = if let Some(consent) = consent {
                    let target = self.lease.root().join(&preview.path);
                    let _write = self.lease.admit_owned(&[&target])?;
                    // Keep the router owner lock through both final fences,
                    // atomic exchange, directory fsync and durable result.
                    self.state.with_store(self.app, |owner| {
                        self.broker.apply_approved(&consent, || {
                            health(healthy)?;
                            self.lease.check(self.run_id, self.binding.generation)?;
                            coding_approvals::fence_locked(self.state, owner, &binding, cancelled)
                        })
                    })
                } else {
                    self.broker.deny(&binding)
                };
                ticket.finish(result.as_ref().map(|_| ()).map_err(Clone::clone));
                result?
            }
        };
        Ok(DurableToolReply {
            receipt: receipt(&binding, &result)?,
            content_items: json!([{"type":"inputText","text":serde_json::to_string(&result).map_err(|_| "Cannot encode durable tool result.")?}]),
            success: result["denied"] != true,
        })
    }
    fn deliver_tool(
        &mut self,
        protocol: &mut codex::CodingProtocol,
        request: &codex::ToolRequest,
        reply: DurableToolReply,
        healthy: &Arc<AtomicBool>,
        enqueue: &mut dyn FnMut(Value) -> Result<(), String>,
    ) -> Result<(), String> {
        let binding = self.call(request);
        self.state.with_store(self.app, |owner| {
            coding_approvals::fence_locked(self.state, owner, &binding, self.cancelled)?;
            self.lease.check(self.run_id, self.binding.generation)?;
            health(healthy)?;
            let frame = protocol
                .complete_tool(
                    &request.call_id,
                    reply.content_items,
                    reply.success,
                    &reply.receipt,
                )
                .map_err(|_| "Native coding tool result does not match its durable receipt.")?;
            health(healthy)?;
            enqueue(frame)
        })
    }
    fn output(&mut self, text: &str) -> Result<(), String> {
        runtime::checkpoint_output(
            self.state,
            self.app,
            self.run_id,
            self.binding.generation,
            &mut self.persisted,
            text,
        )
    }
    fn checkpoint(&mut self, protocol: &codex::CodingProtocol) -> Result<(), String> {
        self.state.with_store(self.app, |owner| {
            // Stop can still persist a fully reconciled interrupted native
            // turn; it never authorizes a new effect or input.
            let snapshot = owner.store.snapshot()?;
            let run = snapshot
                .runs
                .iter()
                .find(|run| run.id == self.run_id && run.generation == self.binding.generation)
                .ok_or("Coding run generation changed.")?;
            if !run
                .attempts
                .last()
                .is_some_and(|attempt| attempt.id == self.binding.attempt_id)
            {
                return Err("Coding checkpoint owner changed.".into());
            }
            self.lease.check(self.run_id, self.binding.generation)?;
            for recovery in self.broker.recover()? {
                recovery.validate_committed(self.run_id)?;
            }
            self.journal.checkpoint(
                &self.binding,
                protocol
                    .native_thread()
                    .ok_or("Missing coding thread checkpoint.")?,
                protocol
                    .native_history()
                    .ok_or("Missing coding history checkpoint.")?,
                protocol.completed,
                protocol.exhausted,
            )?;
            Ok(())
        })
    }
    fn observations(&mut self, protocol: &codex::CodingProtocol) -> Result<(), String> {
        if self.dispatched {
            self.journal
                .observe(&self.binding, &protocol.uncertain_observations())?;
        }
        Ok(())
    }
}

pub(crate) struct Execution<'a> {
    pub(crate) run: Run,
    pub(crate) router: Router,
    pub(crate) profile: Profile,
    pub(crate) directory: PathBuf,
    pub(crate) credential_directory: PathBuf,
    pub(crate) cancelled: &'a Arc<AtomicBool>,
}

pub(crate) fn execute(
    state: &CliRouterService,
    app: &AppHandle,
    shells: &Shells,
    execution: Execution<'_>,
) -> Result<bool, String> {
    let Execution {
        run,
        router,
        profile,
        directory,
        credential_directory,
        cancelled,
    } = execution;
    if profile.cli != TitleCli::Codex || run.execution_mode != RunExecutionMode::Coding {
        return Err("Persistent coding is not implemented for this CLI.".into());
    }
    let generation = run
        .generation
        .checked_add(1)
        .ok_or("Coding run generation exhausted.")?;
    let lease = project_lease::activate(Path::new(&run.cwd), &run.id, generation)?;
    let project_meta =
        fs::symlink_metadata(lease.root()).map_err(|_| "Cannot bind coding project ownership.")?;
    let identity =
        json!({"path":lease.root(),"device":project_meta.dev(),"inode":project_meta.ino()});
    let root = directory
        .parent()
        .ok_or("Invalid private coding storage.")?;
    let model = codex::model(run.model.as_deref())?;
    let effort = codex::effort(model, run.reasoning_effort.as_deref())?;
    let mut journal = Journal::open(root, &run.id, &identity, model, effort)?;
    let input = run.inputs.last().ok_or("No saved coding input.")?;
    let plan = journal.next_input(&input.id, &input.text, run.continuation_requested)?;
    let binding = DispatchBinding {
        attempt_id: super::new_id()?,
        generation,
        auth_revision: profile.revision,
        profile_id: profile.id.clone(),
        input_id: input.id.clone(),
        client_id: plan.client_id.clone(),
    };
    let denied = state.with_store(app, |owner| protected(&owner.root))?;
    let mut broker = effects::Broker::open(
        root,
        lease.root(),
        &denied,
        effects::RunBinding {
            run_id: run.id.clone(),
            attempt_id: binding.attempt_id.clone(),
            generation,
            auth_revision: profile.revision,
        },
    )?;
    for recovery in broker.recover()? {
        recovery.validate_committed(&run.id)?;
    }
    let shell = runtime::shell_profile(
        shells,
        run.shell_profile_id
            .as_deref()
            .ok_or("Choose a shell for the saved coding run.")?,
    )?;
    let resolved = runtime::resolve(shells, shell, &run.cwd, profile.cli)?;
    let program = codex::resolve_native(&resolved)?;
    let mut protocol = codex::CodingProtocol::new(
        model,
        Some(effort),
        &directory,
        &plan.prompt,
        &plan.client_id,
        journal.latest_saved()?,
        json!(effects::tool_specs()),
    )?;
    protocol.bind_executable(&program)?;
    runtime::verify_version(&program, profile.cli, &directory, shell, Some(model), None)?;
    let auth = crate::cli_usage::read_codex_profile_auth(&credential_directory)
        .map_err(|_| "Cannot bind the verified coding subscription.")?;
    if profile.quota_group_key.as_deref() != Some(auth.quota_group_key.as_str()) {
        return Err("The coding account identity changed. Verify it before continuing.".into());
    }
    let mut command = runtime::clean_command(
        &program,
        profile.cli,
        &directory,
        shell,
        &directory.to_string_lossy(),
    )?;
    command.env("HOME", &directory);
    codex::arguments(&mut command, &directory, model);
    let child = state.with_store(app, |owner| {
        runtime::storage_ready(state)?;
        let snapshot = owner.store.snapshot()?;
        let current_run = snapshot
            .runs
            .iter()
            .find(|current| current.id == run.id)
            .ok_or("Coding run no longer exists.")?;
        let current_router = snapshot
            .routers
            .iter()
            .find(|current| current.id == router.id)
            .ok_or("Coding router no longer exists.")?;
        let current_profile = snapshot
            .profiles
            .iter()
            .find(|current| current.id == profile.id)
            .ok_or("Coding account no longer exists.")?;
        if cancelled.load(Ordering::SeqCst)
            || state.closing.load(Ordering::SeqCst)
            || current_run.revision != run.revision
            || current_run.generation != run.generation
            || current_router.revision != router.revision
            || !current_router.enabled
            || current_profile.revision != profile.revision
            || !current_profile.enabled
            || super::profile_writer_busy(state, &profile.id)?
        {
            return Err("Coding ownership changed before process dispatch.".into());
        }
        let selection = policy::select(
            current_router,
            current_run,
            &snapshot.profiles,
            &snapshot.quota,
            now(),
        );
        if selection.profile_id.as_deref() != Some(profile.id.as_str()) {
            return Err("Coding account selection changed before dispatch.".into());
        }
        codex::ambient_policy()?;
        lease.check(&run.id, generation)?;
        let saved = owner.store.update(|snapshot| {
            let current = snapshot
                .runs
                .iter_mut()
                .find(|current| current.id == run.id)
                .ok_or("Coding run no longer exists.")?;
            current.generation = generation;
            current.revision = current
                .revision
                .checked_add(1)
                .ok_or("Coding run revision exhausted.")?;
            current.continuation_requested = false;
            current.state = RunState::Running;
            current.active_profile_id = Some(profile.id.clone());
            current.attempted_profile_ids.push(profile.id.clone());
            current.attempts.push(RunAttempt {
                id: binding.attempt_id.clone(),
                input_id: input.id.clone(),
                profile_id: profile.id.clone(),
                generation,
                state: AttemptState::DispatchIntent,
                reason: selection.reason.clone(),
            });
            current.turns.push(RunTurn {
                input_id: input.id.clone(),
                attempt_id: binding.attempt_id.clone(),
                profile_id: profile.id.clone(),
                generation,
                state: AttemptState::DispatchIntent,
                text: String::new(),
            });
            current.status_message = format!(
                "Opening the saved coding thread on {}. No native input has been released.",
                profile.label
            );
            Ok(())
        })?;
        state.changed(app, saved.revision);
        command.spawn().map_err(|_| {
            "Could not start coding CLI. Its saved process intent requires explicit review."
                .to_string()
        })
    })?;
    let mut supervisor = Owner {
        state,
        app,
        run_id: &run.id,
        cancelled,
        lease,
        binding,
        journal,
        broker,
        persisted: String::new(),
        dispatched: false,
    };
    let outcome = coding_transport::drive(
        runtime::OwnedChild::new(child),
        protocol,
        cancelled,
        auth,
        &mut supervisor,
    )?;
    // The exclusive project lease remains held through native process drain,
    // durable observations, and router settlement of this exact generation.
    runtime::settle_rpc(state, app, &run, &profile, generation, outcome)
}
