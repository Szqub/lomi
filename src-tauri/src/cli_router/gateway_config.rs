//! Native validation of explicit provider destinations and account-pool leases.
use super::{gateway_profiles, types::*, CliRouterService};
use crate::cli_catalog::TitleCli;

pub(crate) fn destination(cli: TitleCli, value: &ApiDestination) -> Result<ApiDestination, String> {
    let compatibility = gateway_profiles::support(cli)
        .ok_or("This CLI has no source-confirmed gateway endpoint override.")?;
    if !super::gateway_transform::native_cli_supports(cli, compatibility.protocol, value.protocol) {
        return Err("This provider protocol cannot preserve this native CLI's required request fields. Codex requires a compatible Responses endpoint.".into());
    }
    let base_url = super::gateway_url::canonical(&value.base_url)?;
    Ok(ApiDestination {
        protocol: value.protocol,
        base_url,
    })
}
/// A changed destination needs an explicit new key save. Retain the old
/// immutable Keychain entry solely for its normal rotation/cleanup transaction.
pub(crate) fn configure(
    profile: &mut Profile,
    provider: Option<&ApiDestination>,
) -> Result<(), String> {
    let revision = profile
        .revision
        .checked_add(1)
        .ok_or("Account revision exhausted.")?;
    let destination = provider
        .map(|provider| destination(profile.cli, provider))
        .transpose()?;
    if profile.gateway_provider != destination {
        profile.auth_state = AuthState::Disconnected;
        profile.quota_group_key = None;
    }
    profile.gateway_provider = destination;
    profile.revision = revision;
    Ok(())
}
pub(crate) fn profile_idle(snapshot: &Snapshot, id: &str) -> Result<(), String> {
    if snapshot.runs.iter().any(|run| {
        run.execution_mode == RunExecutionMode::Gateway
            && run.allowed_profile_ids.iter().any(|allowed| allowed == id)
            && matches!(
                run.state,
                RunState::Starting | RunState::Running | RunState::Switching
            )
    }) {
        return Err(
            "Stop the gateway CLI terminal before changing an approved account or its destination."
                .into(),
        );
    }
    Ok(())
}
pub(crate) fn router_idle(snapshot: &Snapshot, id: &str) -> Result<(), String> {
    if snapshot.runs.iter().any(|run| {
        run.router_id == id
            && run.execution_mode == RunExecutionMode::Gateway
            && matches!(
                run.state,
                RunState::Starting | RunState::Running | RunState::Switching
            )
    }) {
        return Err("Stop the gateway CLI terminal before changing its account pool.".into());
    }
    Ok(())
}
pub(crate) fn eligible(
    profile: &Profile,
    cli: TitleCli,
    protocol: gateway_profiles::Protocol,
) -> bool {
    profile.cli == cli
        && profile.enabled
        && profile.auth_state == AuthState::Ready
        && profile.storage_mode == StorageMode::ApiKey
        && profile.credential_ref.is_some()
        && profile
            .quota_group_key
            .as_deref()
            .is_some_and(|key| !key.is_empty())
        && profile.gateway_provider.as_ref().is_some_and(|provider| {
            super::gateway_transform::native_cli_supports(cli, protocol, provider.protocol)
        })
}
pub(crate) fn pool(
    snapshot: &Snapshot,
    router: &Router,
    state: &CliRouterService,
) -> Result<Vec<Profile>, String> {
    let protocol = gateway_profiles::support(router.cli)
        .ok_or("This CLI has no source-confirmed gateway profile.")?
        .protocol;
    let mut distinct = std::collections::HashSet::new();
    let mut profiles = Vec::new();
    for id in &router.ordered_profile_ids {
        if let Some(profile) = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == *id && eligible(profile, router.cli, protocol))
        {
            if super::profile_writer_busy(state, id)? {
                continue;
            }
            let provider = destination(router.cli, profile.gateway_provider.as_ref().unwrap())?;
            // Separate keys are credential bindings, not a claim of distinct
            // subscriptions or budgets. Deduplicate identical key+effective destination pools.
            let identity =
                super::gateway_url::dispatch_identity(&provider.base_url, provider.protocol)?;
            if distinct.insert((profile.quota_group_key.clone(), identity)) {
                profiles.push(profile.clone());
            }
        }
    }
    if profiles.len() < 2 {
        return Err("Configure at least two distinct API credential bindings with explicit compatible destinations for this gateway pool.".into());
    }
    Ok(profiles)
}
