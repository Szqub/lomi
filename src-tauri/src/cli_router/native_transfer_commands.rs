//! Main-only review and application. Neither operation dispatches native input.
use super::{native_runtime, native_transfer, types::*, view, CliRouterService, View};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, Window};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreviewRequest {
    pub(crate) run_id: String,
    pub(crate) expected_revision: u64,
    pub(crate) destination_profile_id: String,
    pub(crate) destination_profile_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HandoffReview {
    run_id: String,
    run_revision: u64,
    source_profile_id: String,
    source_profile_revision: u64,
    source_label: String,
    destination_profile_id: String,
    destination_profile_revision: u64,
    destination_label: String,
    generation: u64,
    input_id: String,
    model: String,
    version: String,
    history_digest: String,
    history_bytes: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyRequest {
    request_id: String,
    review: HandoffReview,
}
fn review(
    state: &CliRouterService,
    owner: &super::Owner,
    snapshot: &Snapshot,
    request: &PreviewRequest,
) -> Result<(HandoffReview, native_transfer::ForkRequest, bool), String> {
    review_at_root(state, &owner.root, snapshot, request)
}
#[tauri::command]
pub(crate) async fn cli_native_handoff_preview(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: PreviewRequest,
) -> Result<HandoffReview, String> {
    crate::files::main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_store(&app, |owner| {
            let snapshot = owner.store.snapshot()?;
            review(&state, owner, &snapshot, &request).map(|(review, _, _)| review)
        })
    })
    .await
    .map_err(|_| "Could not review native history transfer.")?
}
#[tauri::command]
pub(crate) async fn cli_native_handoff_apply(
    window: Window,
    app: AppHandle,
    state: State<'_, CliRouterService>,
    request: ApplyRequest,
) -> Result<View, String> {
    crate::files::main_window(&window)?;
    if !crate::chat::process::valid_id(&request.request_id) {
        return Err("Invalid history transfer request identity.".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let saved = state.with_store(&app, |owner| {
            // Split borrow: immutable native storage authority is independent of
            // the transactional snapshot passed to the idempotent mutation.
            let root = owner.root.clone();
            owner.store.mutate(&format!("main:native-transfer:{}", request.request_id), &request, |snapshot| {
                let preview_request = PreviewRequest {
                    run_id: request.review.run_id.clone(), expected_revision: request.review.run_revision,
                    destination_profile_id: request.review.destination_profile_id.clone(),
                    destination_profile_revision: request.review.destination_profile_revision,
                };
                let (current, fork, completed) = review_at_root(&state, &root, snapshot, &preview_request)?;
                if current != request.review {
                    return Err("Native history or accounts changed. Review the transfer again.".into());
                }
                let native_root = root.join("runs").join(&fork.run_id).join("native");
                native_transfer::prepare_reviewed(&native_root, &fork, &current.history_digest, current.history_bytes)?;
                let run = snapshot.runs.iter_mut().find(|run| run.id == fork.run_id)
                    .ok_or("The native run changed.")?;
                run.pinned_profile_id = Some(fork.destination_profile.clone());
                run.active_profile_id = None;
                run.attempted_profile_ids.clear();
                run.continuation_requested = false;
                run.state = if completed { RunState::Idle } else { RunState::RecoveryRequired };
                run.revision = run.revision.checked_add(1).ok_or("Run revision exhausted.")?;
                run.status_message = "Native Pi history was copied to the approved account. Send a new message or explicitly continue; no task was launched.".into();
                Ok(())
            })
        })?;
        state.changed(&app, saved.snapshot.revision);
        Ok(view(saved.snapshot, false))
    }).await.map_err(|_| "Could not apply native history transfer.")?
}

fn review_at_root(
    state: &CliRouterService,
    root: &std::path::Path,
    snapshot: &Snapshot,
    request: &PreviewRequest,
) -> Result<(HandoffReview, native_transfer::ForkRequest, bool), String> {
    // The runtime helper needs only the private root, not a second Store owner.
    let (fork, digest, bytes, completed) =
        native_runtime::handoff_review_at_root(state, root, snapshot, request)?;
    let source = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == fork.source_profile)
        .ok_or("Source profile changed.")?;
    let destination = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == fork.destination_profile)
        .ok_or("Destination profile changed.")?;
    Ok((
        HandoffReview {
            run_id: fork.run_id.clone(),
            run_revision: request.expected_revision,
            source_profile_id: source.id.clone(),
            source_profile_revision: source.revision,
            source_label: source.label.clone(),
            destination_profile_id: destination.id.clone(),
            destination_profile_revision: destination.revision,
            destination_label: destination.label.clone(),
            generation: fork.source_generation,
            input_id: fork.input_id.clone(),
            model: fork.model.clone(),
            version: fork.version.clone(),
            history_digest: digest,
            history_bytes: bytes,
        },
        fork,
        completed,
    ))
}
