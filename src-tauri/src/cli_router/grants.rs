use super::{adapters, types::*};
use crate::cli_catalog::TitleCli;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProfileRevision {
    pub(crate) profile_id: String,
    pub(crate) expected_revision: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpdateDataGrant {
    pub(crate) request_id: String,
    pub(crate) run_id: String,
    pub(crate) expected_revision: u64,
    pub(crate) allowed_profile_ids: Vec<String>,
    pub(crate) profiles: Vec<ProfileRevision>,
}

/// Consent names an exact saved-run revision and the reviewed account revisions.
/// This changes eligibility only; it neither dispatches nor resumes a task.
pub(crate) fn apply(
    snapshot: &mut Snapshot,
    request: &UpdateDataGrant,
    mut writer_busy: impl FnMut(&str) -> Result<bool, String>,
) -> Result<(), String> {
    if request.allowed_profile_ids.len() > 64
        || request.profiles.len() > 64
        || request
            .allowed_profile_ids
            .iter()
            .collect::<HashSet<_>>()
            .len()
            != request.allowed_profile_ids.len()
        || request
            .profiles
            .iter()
            .map(|p| &p.profile_id)
            .collect::<HashSet<_>>()
            .len()
            != request.profiles.len()
    {
        return Err("Choose a bounded, unique list of accounts.".into());
    }
    let index = snapshot
        .runs
        .iter()
        .position(|r| r.id == request.run_id)
        .ok_or("Run no longer exists.")?;
    let run = &snapshot.runs[index];
    if run.revision != request.expected_revision {
        return Err("The saved task changed. Review its history and account access again.".into());
    }
    if matches!(
        run.state,
        RunState::Starting | RunState::Running | RunState::Switching
    ) || run.attempts.iter().any(|attempt| attempt.state.is_active())
    {
        return Err(
            "Stop this run and wait for its CLI to finish before changing account access.".into(),
        );
    }
    let router = snapshot.routers.iter().find(|r| r.id == run.router_id);
    for binding in &request.profiles {
        if !request.allowed_profile_ids.contains(&binding.profile_id)
            || snapshot
                .profiles
                .iter()
                .find(|p| p.id == binding.profile_id)
                .is_none_or(|p| p.revision != binding.expected_revision)
        {
            return Err("An account changed. Review its current identity and access again.".into());
        }
    }
    for id in &request.allowed_profile_ids {
        let existing = run.allowed_profile_ids.contains(id);
        let profile = snapshot.profiles.iter().find(|p| p.id == *id);
        if let Some(profile) = profile {
            if !request
                .profiles
                .iter()
                .any(|p| p.profile_id == *id && p.expected_revision == profile.revision)
            {
                return Err("Review the current revision of every selected account.".into());
            }
        } else if !existing {
            return Err("An account is no longer available.".into());
        }
        if existing {
            // Retired or disabled grants may be retained or removed. They do
            // not become eligible without the independent dispatch checks.
            continue;
        }
        let router = router
            .filter(|r| r.enabled)
            .ok_or("Enable this router before sharing history with another account.")?;
        let profile = profile.ok_or("An account is no longer available.")?;
        if profile.cli != router.cli
            || !router.ordered_profile_ids.contains(id)
            || !(if run.execution_mode == RunExecutionMode::Gateway {
                adapters::adapter(router.cli).capability.gateway_terminal
            } else if run.execution_mode == RunExecutionMode::Native {
                adapters::adapter(router.cli).capability.native_turns
            } else {
                adapters::adapter(router.cli).capability.managed_turns
            })
            || !profile.enabled
            || !(profile.auth_state == AuthState::Ready
                || run.execution_mode == RunExecutionMode::Native
                    && profile.auth_state == AuthState::Unverified)
            || writer_busy(id)?
        {
            return Err("This account is not eligible to receive the saved history.".into());
        }
        if run.execution_mode == RunExecutionMode::Gateway {
            let protocol = super::gateway_profiles::support(router.cli)
                .ok_or("This CLI has no gateway protocol.")?
                .protocol;
            if !super::gateway_config::eligible(profile, router.cli, protocol) {
                return Err(
                    "Configure this account's compatible API destination and key first.".into(),
                );
            }
        } else if profile.gateway_provider.is_some() {
            return Err("API gateway accounts are used only by native gateway terminals.".into());
        } else if run.execution_mode == RunExecutionMode::Native {
            if profile.storage_mode == StorageMode::ApiKey {
                return Err(
                    "Native subscription runs require CLI-managed account credentials.".into(),
                );
            }
        } else if router.cli == TitleCli::Codex {
            if profile.storage_mode == StorageMode::ApiKey
                || profile.quota_group_key.as_deref().is_none_or(str::is_empty)
            {
                return Err(
                    "Verify this subscription account and refresh its identity first.".into(),
                );
            }
        } else if profile.storage_mode != StorageMode::ApiKey || profile.credential_ref.is_none() {
            return Err("Configure and verify this account's API key first.".into());
        }
    }
    let run = &mut snapshot.runs[index];
    let revision = run
        .revision
        .checked_add(1)
        .ok_or("Run revision exhausted.")?;
    run.allowed_profile_ids = request.allowed_profile_ids.clone();
    if run
        .pinned_profile_id
        .as_ref()
        .is_some_and(|id| !run.allowed_profile_ids.contains(id))
    {
        run.pinned_profile_id = None;
    }
    run.revision = revision;
    run.status_message =
        "Account access updated. Send or Continue explicitly to start another attempt.".into();
    Ok(())
}
