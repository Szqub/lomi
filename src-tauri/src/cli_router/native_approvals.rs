//! One-use Main decisions for permission requests owned by a native CLI.
//! The CLI continues to execute its own tools and enforce its own policies.
use super::{types::*, CliRouterService};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Arc, Mutex},
};
use tauri::{AppHandle, State, Window};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Choice {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) allow: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) generation: u64,
    pub(crate) native_request_id: String,
    pub(crate) tool: String,
    pub(crate) input: Value,
    pub(crate) choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text_input: Option<TextInput>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TextInput {
    pub(crate) label: String,
    pub(crate) initial: String,
    pub(crate) multiline: bool,
}
#[derive(Clone)]
pub(crate) struct Answer {
    pub(crate) choice_id: String,
    pub(crate) text: Option<String>,
}
struct Pending {
    preview: Preview,
    profile_id: String,
    profile_revision: u64,
    decision: Mutex<Option<Answer>>,
}
#[derive(Clone, Default)]
pub(crate) struct Coordinator {
    pending: Arc<Mutex<HashMap<String, Arc<Pending>>>>,
}
pub(crate) struct Ticket {
    coordinator: Coordinator,
    pending: Arc<Pending>,
}
impl Coordinator {
    pub(crate) fn request(
        &self,
        mut preview: Preview,
        profile: &Profile,
    ) -> Result<Ticket, String> {
        if preview.native_request_id.is_empty()
            || preview.native_request_id.len() > 512
            || preview.tool.len() > 512
            || preview.choices.is_empty()
            || preview.choices.len() > 32
            || serde_json::to_vec(&preview.input)
                .map_err(|_| "Invalid native tool request.")?
                .len()
                > 64 * 1024
        {
            return Err("Native permission exceeds the reviewed bounds.".into());
        }
        if preview
            .text_input
            .as_ref()
            .is_some_and(|input| input.label.len() > 512 || input.initial.len() > 64 * 1024)
        {
            return Err("Native text interaction exceeds its bounds.".into());
        }
        preview.id = super::new_id()?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "Native approvals unavailable.")?;
        if pending.len() >= 128
            || pending.values().any(|old| {
                old.preview.run_id == preview.run_id
                    && old.preview.native_request_id == preview.native_request_id
            })
        {
            return Err("Native permission is duplicated or approval capacity is full.".into());
        }
        let item = Arc::new(Pending {
            preview,
            profile_id: profile.id.clone(),
            profile_revision: profile.revision,
            decision: Mutex::new(None),
        });
        pending.insert(item.preview.id.clone(), item.clone());
        Ok(Ticket {
            coordinator: self.clone(),
            pending: item,
        })
    }
    fn for_run(&self, id: &str) -> Result<Vec<Preview>, String> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| "Native approvals unavailable.")?;
        Ok(pending
            .values()
            .filter(|item| item.preview.run_id == id)
            .map(|item| item.preview.clone())
            .collect())
    }
}
impl Ticket {
    pub(crate) fn decision(&self) -> Result<Option<Answer>, String> {
        Ok(self
            .pending
            .decision
            .lock()
            .map_err(|_| "Native approval unavailable.")?
            .clone())
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.coordinator.pending.lock() {
            pending.remove(&self.pending.preview.id);
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Decision {
    approval_id: String,
    run_id: String,
    generation: u64,
    choice_id: String,
    #[serde(default)]
    text: Option<String>,
}
#[tauri::command]
pub(crate) fn cli_native_permissions(
    window: Window,
    state: State<'_, CliRouterService>,
    run_id: String,
) -> Result<Vec<Preview>, String> {
    crate::files::main_window(&window)?;
    state.native_approvals.for_run(&run_id)
}
#[tauri::command]
pub(crate) fn cli_native_permission_reply(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: Decision,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let pending = state
        .native_approvals
        .pending
        .lock()
        .map_err(|_| "Native approvals unavailable.")?
        .get(&request.approval_id)
        .cloned()
        .ok_or("This native permission is no longer pending.")?;
    if pending.preview.run_id != request.run_id
        || pending.preview.generation != request.generation
        || !pending
            .preview
            .choices
            .iter()
            .any(|choice| choice.id == request.choice_id)
    {
        return Err("The native permission changed. Review its current request.".into());
    }
    if request.choice_id == "submit-text" {
        if pending.preview.text_input.is_none()
            || request
                .text
                .as_ref()
                .is_none_or(|text| text.len() > 64 * 1024)
        {
            return Err("Enter native text up to 64 KiB for this exact request.".into());
        }
    } else if request.text.is_some() {
        return Err("This native choice does not accept text.".into());
    }
    state.with_store(&app, |owner| {
        if state.closing.load(Ordering::SeqCst) {
            return Err("The native CLI is stopping.".into());
        }
        let snapshot = owner.store.snapshot()?;
        let run = snapshot
            .runs
            .iter()
            .find(|run| run.id == request.run_id)
            .ok_or("Native run no longer exists.")?;
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == pending.profile_id)
            .ok_or("Native account no longer exists.")?;
        let router = snapshot
            .routers
            .iter()
            .find(|router| router.id == run.router_id)
            .ok_or("Native router no longer exists.")?;
        if run.execution_mode != RunExecutionMode::Native
            || run.generation != request.generation
            || run.state != RunState::Running
            || run.active_profile_id.as_deref() != Some(profile.id.as_str())
            || !run.allowed_profile_ids.contains(&profile.id)
            || !router.enabled
            || !router.ordered_profile_ids.contains(&profile.id)
            || !profile.enabled
            || !matches!(profile.auth_state, AuthState::Ready | AuthState::Unverified)
            || profile.revision != pending.profile_revision
            || !state
                .active
                .lock()
                .map_err(|_| "Native ownership unavailable.")?
                .get(&run.id)
                .is_some_and(|cancelled| !cancelled.load(Ordering::SeqCst))
        {
            return Err("Native account, approval or execution ownership changed.".into());
        }
        let mut decision = pending
            .decision
            .lock()
            .map_err(|_| "Native approval unavailable.")?;
        if decision.is_some() {
            return Err("This native permission already has a decision.".into());
        }
        *decision = Some(Answer {
            choice_id: request.choice_id.clone(),
            text: request.text.clone(),
        });
        Ok(())
    })
}
