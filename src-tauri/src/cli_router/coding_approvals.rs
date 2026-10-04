//! Main-only one-use consent for an exact durable project-file intent.
use super::CliRouterService;
use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, State, Window};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EditorProof {
    pub(crate) path: String,
    pub(crate) documents: Vec<EditorDocumentProof>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EditorDocumentProof {
    pub(crate) document_id: String,
    pub(crate) buffer_revision: String,
    pub(crate) disk_revision: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EffectDecision {
    approval_id: String,
    args_digest: String,
    after_hash: String,
    result_digest: String,
    approve: bool,
    editor_proof: Option<EditorProof>,
}

#[cfg(unix)]
mod platform {
    use super::super::{effects, types::*};
    use super::*;
    use std::{
        collections::{HashMap, HashSet},
        path::Path,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Condvar, Mutex,
        },
        time::{Duration, Instant},
    };
    use tauri::Emitter;

    #[derive(Default, Clone)]
    pub(crate) struct Coordinator {
        inner: Arc<Mutex<HashMap<String, Arc<Pending>>>>,
    }
    enum Decision {
        Approved(Box<effects::Approval>),
        Denied,
    }
    #[derive(Default)]
    struct Progress {
        decision: Option<Decision>,
        finished: Option<Result<(), String>>,
    }
    struct Pending {
        id: String,
        preview: effects::Preview,
        target: String,
        project: String,
        progress: Mutex<Progress>,
        changed: Condvar,
    }
    pub(crate) struct Ticket {
        coordinator: Coordinator,
        pending: Arc<Pending>,
    }
    impl Ticket {
        pub(crate) fn wait(
            &self,
            cancelled: &Arc<AtomicBool>,
            healthy: &Arc<AtomicBool>,
        ) -> Result<Option<effects::Approval>, String> {
            let deadline = Instant::now() + Duration::from_secs(300);
            let mut progress = self
                .pending
                .progress
                .lock()
                .map_err(|_| "File approval is unavailable.")?;
            loop {
                // Cancellation wins even when a previously approved decision
                // has not yet reached the executing broker.
                if cancelled.load(Ordering::SeqCst)
                    || !healthy.load(Ordering::SeqCst)
                    || Instant::now() >= deadline
                {
                    return Ok(None);
                }
                if let Some(decision) = &progress.decision {
                    return Ok(match decision {
                        Decision::Approved(approval) => Some(approval.as_ref().clone()),
                        Decision::Denied => None,
                    });
                }
                progress = self
                    .pending
                    .changed
                    .wait_timeout(progress, Duration::from_millis(50))
                    .map_err(|_| "File approval is unavailable.")?
                    .0;
            }
        }
        pub(crate) fn finish(&self, result: Result<(), String>) {
            if let Ok(mut progress) = self.pending.progress.lock() {
                progress.finished = Some(result);
                self.pending.changed.notify_all();
            }
        }
    }
    impl Drop for Ticket {
        fn drop(&mut self) {
            if let Ok(mut progress) = self.pending.progress.lock() {
                if progress.finished.is_none() {
                    progress.finished = Some(Err("The file operation stopped before its durable result was confirmed. Review recovery before retrying.".into()));
                }
                self.pending.changed.notify_all();
            }
            if let Ok(mut pending) = self.coordinator.inner.lock() {
                pending.remove(&self.pending.id);
            }
        }
    }
    pub(crate) fn fence(
        state: &CliRouterService,
        app: &AppHandle,
        binding: &effects::CallBinding,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        state.with_store(app, |owner| fence_locked(state, owner, binding, cancelled))
    }
    pub(in crate::cli_router) fn fence_locked(
        state: &CliRouterService,
        owner: &super::super::Owner,
        binding: &effects::CallBinding,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        if cancelled.load(Ordering::SeqCst)
            || state.closing.load(Ordering::SeqCst)
            || state
                .storage_error
                .lock()
                .map_err(|_| "Router storage status unavailable.")?
                .is_some()
        {
            return Err("The coding owner stopped or lost durable storage.".into());
        }
        if !state
            .active
            .lock()
            .map_err(|_| "Router ownership unavailable.")?
            .get(&binding.run_id)
            .is_some_and(|current| Arc::ptr_eq(current, cancelled))
        {
            return Err("The coding owner changed.".into());
        }
        let snapshot = owner.store.snapshot()?;
        let run = snapshot
            .runs
            .iter()
            .find(|run| run.id == binding.run_id)
            .ok_or("Coding run no longer exists.")?;
        let attempt = run
            .attempts
            .last()
            .filter(|attempt| {
                attempt.id == binding.attempt_id
                    && attempt.generation == binding.generation
                    && attempt.state.is_active()
            })
            .ok_or("The coding attempt changed.")?;
        let router = snapshot
            .routers
            .iter()
            .find(|router| router.id == run.router_id)
            .ok_or("Coding router no longer exists.")?;
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == attempt.profile_id)
            .ok_or("Coding account no longer exists.")?;
        if run.execution_mode != RunExecutionMode::Coding
            || run.state != RunState::Running
            || run.generation != binding.generation
            || run.active_profile_id.as_deref() != Some(profile.id.as_str())
            || !router.enabled
            || !profile.enabled
            || profile.revision != binding.auth_revision
            || profile.auth_state != AuthState::Ready
            || !router.ordered_profile_ids.contains(&profile.id)
            || !run.allowed_profile_ids.contains(&profile.id)
            || super::super::profile_writer_busy(state, &profile.id)?
        {
            return Err(
                "Coding consent, account or configuration changed. The file operation was fenced."
                    .into(),
            );
        }
        Ok(())
    }
    impl Coordinator {
        pub(crate) fn request(
            &self,
            state: &CliRouterService,
            app: &AppHandle,
            project: &Path,
            preview: effects::Preview,
            cancelled: &Arc<AtomicBool>,
        ) -> Result<Ticket, String> {
            let id = super::super::new_id()?;
            let pending = Arc::new(Pending {
                id: id.clone(),
                target: project.join(&preview.path).to_string_lossy().into_owned(),
                project: project.to_string_lossy().into_owned(),
                preview,
                progress: Mutex::new(Progress::default()),
                changed: Condvar::new(),
            });
            state.with_store(app, |owner| {
                fence_locked(state, owner, &pending.preview.binding, cancelled)?;
                let mut entries = self
                    .inner
                    .lock()
                    .map_err(|_| "File approvals are unavailable.")?;
                if entries.len() >= 4
                    || entries.values().any(|existing| {
                        existing.preview.binding.run_id == pending.preview.binding.run_id
                    })
                {
                    return Err("A file operation is already awaiting a decision.".into());
                }
                entries.insert(id, pending.clone());
                Ok(())
            })?;
            let _ = app.emit_to(
                tauri::EventTarget::webview("main"),
                "cli-router-effect-approval",
                serde_json::json!({"runId":pending.preview.binding.run_id}),
            );
            Ok(Ticket {
                coordinator: self.clone(),
                pending,
            })
        }
        pub(crate) fn views(&self) -> Result<Vec<Value>, String> {
            let pending = self
                .inner
                .lock()
                .map_err(|_| "File approvals are unavailable.")?;
            pending.values().map(|pending| Ok(serde_json::json!({"approvalId":pending.id,"projectRoot":pending.project,"targetPath":pending.target,"preview":pending.preview}))).collect()
        }
        fn decide(
            &self,
            state: &CliRouterService,
            app: &AppHandle,
            request: EffectDecision,
        ) -> Result<(), String> {
            // Lock order is native owner -> coordinator -> pending progress.
            let pending = state.with_store(app, |owner| {
                let entries = self.inner.lock().map_err(|_| "File approvals are unavailable.")?;
                let pending = entries.get(&request.approval_id).cloned().ok_or("This file approval expired.")?;
                let cancelled = state.active.lock().map_err(|_| "Router ownership unavailable.")?.get(&pending.preview.binding.run_id).cloned().ok_or("The coding run stopped.")?;
                fence_locked(state, owner, &pending.preview.binding, &cancelled)?;
                if request.args_digest != pending.preview.args_digest || request.after_hash != pending.preview.after_hash || request.result_digest != pending.preview.result_digest { return Err("This decision does not match the reviewed file intent.".into()); }
                let mut progress = pending.progress.lock().map_err(|_| "File approval is unavailable.")?;
                if progress.decision.is_some() || progress.finished.is_some() { return Err("This one-use file approval already received a decision.".into()); }
                progress.decision = Some(if request.approve {
                    let proof = request.editor_proof.as_ref().ok_or("Freeze matching Main editors before approving a write.")?;
                    if proof.path != pending.target || proof.documents.len() > 128 { return Err("Editor evidence does not match this file intent.".into()); }
                    let mut ids = HashSet::new();
                    for document in &proof.documents {
                        if document.document_id.is_empty() || document.document_id.len() > 160 || document.buffer_revision.is_empty() || document.buffer_revision.len() > 160 || !ids.insert(&document.document_id) || pending.preview.before_hash.as_deref() != Some(document.disk_revision.as_str()) { return Err("A matching editor changed or has no approved disk version. Save or close it before reviewing a new operation.".into()); }
                        // Buffer revisions identify the exact frozen Main
                        // document; disk hashes bind them to native before data.
                        let _ = &document.buffer_revision;
                    }
                    Decision::Approved(Box::new(effects::Approval { binding: pending.preview.binding.clone(), nonce: super::super::new_id()?, args_digest: pending.preview.args_digest.clone(), before_hash: pending.preview.before_hash.clone(), after_hash: pending.preview.after_hash.clone(), result_digest: pending.preview.result_digest.clone() }))
                } else { Decision::Denied });
                pending.changed.notify_all();
                drop(progress);
                Ok(pending)
            })?;
            // Keep the frontend freeze until the worker confirms completion or
            // uncertain cleanup. A timeout must never release an approved lock.
            let mut progress = pending
                .progress
                .lock()
                .map_err(|_| "File approval is unavailable.")?;
            loop {
                if let Some(result) = &progress.finished {
                    return result.clone();
                }
                progress = pending
                    .changed
                    .wait(progress)
                    .map_err(|_| "File approval is unavailable.")?;
            }
        }
    }
    pub(super) fn decide(
        state: &CliRouterService,
        app: &AppHandle,
        request: EffectDecision,
    ) -> Result<(), String> {
        state.approvals.decide(state, app, request)
    }
}
#[cfg(unix)]
pub(super) use platform::fence_locked;
#[cfg(unix)]
pub(crate) use platform::{fence, Coordinator};

#[tauri::command]
pub(crate) async fn cli_run_effect_approvals(
    window: Window,
    state: State<'_, CliRouterService>,
) -> Result<Vec<Value>, String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        state.approvals.views()
    }
    #[cfg(not(unix))]
    {
        let _ = state;
        Ok(vec![])
    }
}
#[tauri::command]
pub(crate) async fn cli_run_decide_effect(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: EffectDecision,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let state = state.inner().clone();
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || platform::decide(&state, &app, request))
            .await
            .map_err(|_| "File approval task failed.")?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, app, request);
        Err("Managed file effects are unavailable on this platform.".into())
    }
}
