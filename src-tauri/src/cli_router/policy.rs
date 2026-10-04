use super::types::*;
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Selection {
    pub(crate) profile_id: Option<String>,
    pub(crate) reason: String,
    pub(crate) balance_degraded: bool,
}

pub(crate) fn remaining(quota: &Quota, now: i64) -> Option<f64> {
    if quota.status != QuotaStatus::Fresh
        || quota.observed_at > now
        || quota.expires_at <= now
        || now.saturating_sub(quota.observed_at) > 30_000
        || quota.windows.is_empty()
    {
        return None;
    }
    quota.windows.iter().try_fold(100.0_f64, |minimum, window| {
        let value = window.remaining_percent?;
        (value.is_finite() && (0.0..=100.0).contains(&value)).then(|| minimum.min(value))
    })
}

fn blocked(quota: &Quota) -> bool {
    // A wall-clock reset or stale reader never clears authoritative exhaustion.
    quota.status == QuotaStatus::Exhausted
        || (matches!(quota.status, QuotaStatus::Fresh | QuotaStatus::Stale)
            && quota
                .windows
                .iter()
                .any(|window| window.remaining_percent == Some(0.0)))
}

pub(crate) fn shares_quota_group(profile: &Profile, other: &Profile) -> bool {
    profile.id == other.id
        || (profile.cli == other.cli
            && profile
                .quota_group_key
                .as_deref()
                .filter(|key| !key.is_empty())
                .is_some_and(|key| other.quota_group_key.as_deref() == Some(key)))
}

/// Pure selection only: callers serialize this with reservation, revision and
/// auth/model/capability fences before dispatch. This is not a dispatch grant.
pub(crate) fn select(
    router: &Router,
    run: &Run,
    profiles: &[Profile],
    quota: &[Quota],
    now: i64,
) -> Selection {
    select_with_reservations(router, run, profiles, quota, now, &[])
}

pub(crate) fn select_with_reservations(
    router: &Router,
    run: &Run,
    profiles: &[Profile],
    quota: &[Quota],
    now: i64,
    reservations: &[(String, usize)],
) -> Selection {
    let result = |id: Option<&str>, reason: &str, degraded| Selection {
        profile_id: id.map(str::to_owned),
        reason: reason.into(),
        balance_degraded: degraded,
    };
    if !router.enabled || run.router_id != router.id {
        return result(None, "router_disabled", false);
    }
    let candidates = router
        .ordered_profile_ids
        .iter()
        .filter_map(|id| {
            let profile = profiles.iter().find(|profile| profile.id == *id)?;
            if !profile.enabled
                || profile.cli != router.cli
                || profile.auth_state != AuthState::Ready
                || profile.gateway_provider.is_some()
                || !run.allowed_profile_ids.contains(id)
                || profiles.iter().any(|other| {
                    shares_quota_group(profile, other)
                        && run.attempted_profile_ids.contains(&other.id)
                })
                || run
                    .pinned_profile_id
                    .as_ref()
                    .is_some_and(|pinned| pinned != id)
                || profiles.iter().any(|other| {
                    shares_quota_group(profile, other)
                        && quota
                            .iter()
                            .find(|q| q.profile_id == other.id)
                            .is_some_and(blocked)
                })
            {
                return None;
            }
            Some(profile)
        })
        .collect::<Vec<_>>();
    // Multiple namespaces for one authenticated quota group are one budget.
    // Keep the healthy current namespace as representative; otherwise priority.
    let mut groups: Vec<&Profile> = Vec::new();
    for profile in candidates {
        if let Some(index) = groups
            .iter()
            .position(|other| shares_quota_group(profile, other))
        {
            if run.active_profile_id.as_deref() == Some(profile.id.as_str()) {
                groups[index] = profile;
            }
        } else {
            groups.push(profile);
        }
    }
    let candidates = groups;
    if candidates.is_empty() {
        return result(None, "waiting_for_capacity", false);
    }
    let current = candidates
        .iter()
        .copied()
        .find(|profile| run.active_profile_id.as_deref() == Some(profile.id.as_str()));
    if router.balance_remaining_quota {
        let reports = candidates
            .iter()
            .map(|profile| {
                let report = quota.iter().find(|q| q.profile_id == profile.id)?;
                Some((profile, remaining(report, now)?, report.epoch))
            })
            .collect::<Option<Vec<_>>>();
        if let Some(reports) =
            reports.filter(|reports| reports.iter().all(|(_, _, epoch)| *epoch == reports[0].2))
        {
            let maximum = reports
                .iter()
                .map(|(_, value, _)| *value)
                .fold(0.0_f64, f64::max);
            let chosen = current
                .filter(|profile| {
                    reports
                        .iter()
                        .any(|(p, value, _)| p.id == profile.id && *value == maximum)
                })
                .unwrap_or_else(|| {
                    reports
                        .iter()
                        .filter(|(_, value, _)| *value == maximum)
                        .min_by_key(|(profile, _, _)| {
                            reservations
                                .iter()
                                .filter(|(id, _)| {
                                    profiles.iter().any(|other| {
                                        other.id == *id && shares_quota_group(profile, other)
                                    })
                                })
                                .fold(0usize, |total, (_, count)| total.saturating_add(*count))
                        })
                        .unwrap()
                        .0
                });
            return result(Some(&chosen.id), "highest_remaining_quota", false);
        }
        let chosen = current.unwrap_or(candidates[0]);
        return result(Some(&chosen.id), "balance_waiting_for_fresh_quota", true);
    }
    result(
        Some(&current.unwrap_or(candidates[0]).id),
        "priority_failover",
        false,
    )
}

/// Accept only a read started against the current block revision and collection
/// epoch. The caller must additionally fence identity/auth and qualified scope.
pub(crate) fn apply_quota(
    existing: &mut Quota,
    incoming: Quota,
    request_block_revision: u64,
) -> bool {
    if incoming.profile_id != existing.profile_id
        || request_block_revision != existing.block_revision
        || incoming.block_revision != existing.block_revision
        || incoming.epoch < existing.epoch
        || incoming.observed_at < existing.observed_at
        || (blocked(existing)
            && !(incoming.status == QuotaStatus::Exhausted
                && incoming
                    .windows
                    .iter()
                    .any(|window| window.remaining_percent == Some(0.0)))
            && remaining(&incoming, incoming.observed_at).is_none_or(|value| value <= 0.0))
    {
        return false;
    }
    *existing = incoming;
    true
}

/// Recover an already identified group from an authoritative positive report.
/// `baseline` must be captured before dispatching the read. Call inside the
/// snapshot mutation, before applying the source report or changing its group.
/// Returned IDs may renew the current recovery ledger; ordinary refreshes never
/// do so. This function does not authorize replay or restart a run.
pub(crate) fn apply_group_recovery(
    snapshot: &mut Snapshot,
    baseline: &Snapshot,
    source_profile_id: &str,
    authenticated_group_key: &str,
    incoming: &Quota,
    now: i64,
) -> Vec<String> {
    if authenticated_group_key.is_empty()
        || incoming.profile_id != source_profile_id
        || remaining(incoming, now).is_none_or(|value| value <= 0.0)
    {
        return vec![];
    }
    let Some(source) = snapshot.profiles.iter().find(|p| p.id == source_profile_id) else {
        return vec![];
    };
    if source.auth_state != AuthState::Ready
        || source.quota_group_key.as_deref() != Some(authenticated_group_key)
    {
        return vec![];
    }
    let members = snapshot
        .profiles
        .iter()
        .filter(|p| shares_quota_group(source, p))
        .collect::<Vec<_>>();
    let old_members = baseline
        .profiles
        .iter()
        .filter(|p| {
            p.cli == source.cli && p.quota_group_key.as_deref() == Some(authenticated_group_key)
        })
        .collect::<Vec<_>>();
    if members.len() != old_members.len()
        || !members.iter().any(|p| {
            snapshot
                .quota
                .iter()
                .find(|q| q.profile_id == p.id)
                .is_some_and(blocked)
        })
        || !old_members.iter().any(|p| {
            baseline
                .quota
                .iter()
                .find(|q| q.profile_id == p.id)
                .is_some_and(blocked)
        })
    {
        return vec![];
    }
    for member in &members {
        let Some(old) = old_members.iter().find(|p| p.id == member.id) else {
            return vec![];
        };
        if old.revision != member.revision {
            return vec![];
        }
        let previous = baseline.quota.iter().find(|q| q.profile_id == member.id);
        let current = snapshot.quota.iter().find(|q| q.profile_id == member.id);
        if previous.map(|q| q.block_revision) != current.map(|q| q.block_revision)
            || current
                .is_some_and(|q| incoming.epoch < q.epoch || incoming.observed_at < q.observed_at)
            || (member.id == source_profile_id
                && incoming.block_revision != current.map_or(0, |q| q.block_revision))
        {
            return vec![];
        }
    }
    let recovered_ids = members.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
    for id in &recovered_ids {
        let current = snapshot.quota.iter_mut().find(|q| q.profile_id == *id);
        let mut report = incoming.clone();
        report.profile_id = id.clone();
        report.block_revision = current.as_ref().map_or(0, |q| q.block_revision);
        if let Some(current) = current {
            *current = report;
        } else {
            snapshot.quota.push(report);
        }
    }
    recovered_ids
}
