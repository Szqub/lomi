use super::{adapters::*, policy::*, store::Store, types::*};
use crate::cli_catalog::TitleCli;

fn profile(id: &str) -> Profile {
    Profile {
        id: id.into(),
        cli: TitleCli::Codex,
        label: id.into(),
        enabled: true,
        revision: 1,
        auth_state: AuthState::Ready,
        storage_mode: StorageMode::Keyring,
        credential_ref: None,
        quota_group_key: None,
        gateway_provider: None,
    }
}
fn router(balance: bool) -> Router {
    Router {
        id: "router".into(),
        cli: TitleCli::Codex,
        label: "Router".into(),
        enabled: true,
        ordered_profile_ids: vec!["a".into(), "b".into()],
        balance_remaining_quota: balance,
        revision: 1,
    }
}
fn run() -> Run {
    Run {
        id: "run".into(),
        router_id: "router".into(),
        cwd: "/project".into(),
        shell_profile_id: None,
        title: "Task".into(),
        state: RunState::Idle,
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
        output: String::new(),
        turns: vec![],
        legacy_output: None,
        status_message: String::new(),
        attempted_profile_ids: vec![],
    }
}
fn quota(id: &str, values: &[f64]) -> Quota {
    Quota {
        profile_id: id.into(),
        status: QuotaStatus::Fresh,
        windows: values
            .iter()
            .enumerate()
            .map(|(i, value)| QuotaWindow {
                id: i.to_string(),
                remaining_percent: Some(*value),
                reset_at: Some(500),
            })
            .collect(),
        observed_at: 1000,
        expires_at: 31_000,
        epoch: 1,
        block_revision: 0,
    }
}
fn fixture() -> Snapshot {
    Snapshot {
        revision: 0,
        profiles: vec![profile("a"), profile("b")],
        routers: vec![router(false)],
        quota: vec![quota("a", &[100.0]), quota("b", &[80.0])],
        runs: vec![run()],
    }
}

fn grant_request(snapshot: &Snapshot, ids: &[&str]) -> super::grants::UpdateDataGrant {
    super::grants::UpdateDataGrant {
        request_id: "grant-request".into(),
        run_id: "run".into(),
        expected_revision: snapshot.runs[0].revision,
        allowed_profile_ids: ids.iter().map(|id| (*id).into()).collect(),
        profiles: snapshot
            .profiles
            .iter()
            .filter(|p| ids.contains(&p.id.as_str()))
            .map(|p| super::grants::ProfileRevision {
                profile_id: p.id.clone(),
                expected_revision: p.revision,
            })
            .collect(),
    }
}

#[test]
fn explicit_grant_addition_binds_history_and_identity_without_starting_an_attempt() {
    let mut snapshot = fixture();
    let mut third = profile("c");
    third.quota_group_key = Some("authenticated-c".into());
    snapshot.profiles.push(third);
    snapshot.routers[0].ordered_profile_ids.push("c".into());
    snapshot.runs[0].state = RunState::RecoveryRequired;
    snapshot.runs[0].inputs.push(RunInput {
        id: "input".into(),
        text: "private history".into(),
    });
    assert_eq!(snapshot.runs[0].allowed_profile_ids, ["a", "b"]);
    let request = grant_request(&snapshot, &["a", "c"]);
    let before = snapshot.clone();
    let mut changed = snapshot.clone();
    changed.profiles[2].revision += 1;
    let unchanged = changed.clone();
    assert!(super::grants::apply(&mut changed, &request, |_| Ok(false)).is_err());
    assert_eq!(changed, unchanged);
    changed = snapshot.clone();
    changed.runs[0].revision += 1;
    let unchanged = changed.clone();
    assert!(super::grants::apply(&mut changed, &request, |_| Ok(false)).is_err());
    assert_eq!(changed, unchanged);
    assert!(super::grants::apply(&mut snapshot, &request, |_| Ok(true)).is_err());
    assert_eq!(snapshot, before);
    super::grants::apply(&mut snapshot, &request, |_| Ok(false)).unwrap();
    assert_eq!(snapshot.runs[0].allowed_profile_ids, ["a", "c"]);
    assert_eq!(snapshot.runs[0].state, RunState::RecoveryRequired);
    assert_eq!(snapshot.runs[0].generation, before.runs[0].generation);
    assert_eq!(snapshot.runs[0].inputs, before.runs[0].inputs);
    assert_eq!(snapshot.runs[0].attempts, before.runs[0].attempts);
    assert_eq!(snapshot.runs[0].revision, before.runs[0].revision + 1);
}

#[test]
fn revoking_grants_can_remove_retired_accounts_and_clears_their_pin() {
    let mut snapshot = fixture();
    snapshot.runs[0].allowed_profile_ids.push("retired".into());
    snapshot.runs[0].pinned_profile_id = Some("retired".into());
    snapshot.routers.clear();
    let request = grant_request(&snapshot, &["a"]);
    super::grants::apply(&mut snapshot, &request, |_| panic!("No new accounts")).unwrap();
    assert_eq!(snapshot.runs[0].allowed_profile_ids, ["a"]);
    assert_eq!(snapshot.runs[0].pinned_profile_id, None);
    for state in [RunState::Starting, RunState::Running, RunState::Switching] {
        snapshot.runs[0].state = state;
        let request = grant_request(&snapshot, &[]);
        let before = snapshot.clone();
        assert!(super::grants::apply(&mut snapshot, &request, |_| Ok(false)).is_err());
        assert_eq!(snapshot, before);
    }
}

#[test]
fn explicit_grant_commit_replays_without_reapplying_consent_to_newer_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("router.sqlite")).unwrap();
    store
        .update(|snapshot| {
            *snapshot = fixture();
            Ok(())
        })
        .unwrap();
    let request = grant_request(&store.snapshot().unwrap(), &["a"]);
    store
        .mutate("main:grant:grant-request", &request, |snapshot| {
            super::grants::apply(snapshot, &request, |_| Ok(false))
        })
        .unwrap();
    store
        .update(|snapshot| {
            snapshot.runs[0].revision += 1;
            Ok(())
        })
        .unwrap();
    let before = store.snapshot().unwrap();
    store
        .mutate("main:grant:grant-request", &request, |_| {
            panic!("Committed request must not replay")
        })
        .unwrap();
    assert_eq!(store.snapshot().unwrap(), before);
}
fn choose(router: &Router, run: &Run, reports: &[Quota]) -> Selection {
    select(router, run, &[profile("a"), profile("b")], reports, 1001)
}

#[test]
fn balance_tracks_remaining_at_safe_boundaries_and_keeps_current_ties() {
    let router = router(true);
    let mut run = run();
    for (a, b, expected) in [
        (100., 80., "a"),
        (90., 80., "a"),
        (80., 80., "a"),
        (75., 80., "b"),
        (75., 75., "b"),
        (75., 70., "a"),
        (79.9, 80., "b"),
    ] {
        let result = choose(&router, &run, &[quota("a", &[a]), quota("b", &[b])]);
        assert_eq!(result.profile_id.as_deref(), Some(expected));
        assert!(!result.balance_degraded);
        run.active_profile_id = result.profile_id;
    }
}

#[test]
fn balance_uses_tightest_applicable_window() {
    assert_eq!(
        choose(
            &router(true),
            &run(),
            &[quota("a", &[90., 10.]), quota("b", &[65., 60.])]
        )
        .profile_id
        .as_deref(),
        Some("b")
    );
}

#[test]
fn new_balance_ties_prefer_fewer_reservations_then_user_order() {
    let profiles = [profile("a"), profile("b")];
    let reports = [quota("a", &[80.]), quota("b", &[80.])];
    let reservations = [("a".into(), 2), ("b".into(), 1)];
    assert_eq!(
        select_with_reservations(
            &router(true),
            &run(),
            &profiles,
            &reports,
            1001,
            &reservations
        )
        .profile_id
        .as_deref(),
        Some("b")
    );
    let mut current = run();
    current.active_profile_id = Some("a".into());
    assert_eq!(
        select_with_reservations(
            &router(true),
            &current,
            &profiles,
            &reports,
            1001,
            &reservations
        )
        .profile_id
        .as_deref(),
        Some("a")
    );
    assert_eq!(
        select_with_reservations(&router(true), &run(), &profiles, &reports, 1001, &[])
            .profile_id
            .as_deref(),
        Some("a")
    );
}

#[test]
fn failover_keeps_healthy_current_and_new_runs_follow_user_order() {
    let mut run = run();
    let reports = [quota("a", &[100.]), quota("b", &[40.])];
    assert_eq!(
        choose(&router(false), &run, &reports).profile_id.as_deref(),
        Some("a")
    );
    run.active_profile_id = Some("b".into());
    assert_eq!(
        choose(&router(false), &run, &reports).profile_id.as_deref(),
        Some("b")
    );
}

#[test]
fn stale_unknown_or_incomparable_balance_falls_back_without_excluding_profiles() {
    let mut run = run();
    run.active_profile_id = Some("b".into());
    for status in [
        QuotaStatus::Unknown,
        QuotaStatus::Stale,
        QuotaStatus::ReaderError,
        QuotaStatus::Unsupported,
    ] {
        let mut a = quota("a", &[100.]);
        a.status = status;
        let result = choose(&router(true), &run, &[a, quota("b", &[80.])]);
        assert_eq!(result.profile_id.as_deref(), Some("b"));
        assert!(result.balance_degraded);
    }
    let mut b = quota("b", &[80.]);
    b.epoch = 2;
    assert!(choose(&router(true), &run, &[quota("a", &[100.]), b]).balance_degraded);
    assert_eq!(
        choose(&router(true), &fixture().runs[0], &[])
            .profile_id
            .as_deref(),
        Some("a")
    );
}

#[test]
fn blocks_survive_staleness_and_passed_reset_times() {
    let mut a = quota("a", &[0.]);
    a.status = QuotaStatus::Stale;
    assert_eq!(
        choose(&router(false), &run(), &[a, quota("b", &[80.])])
            .profile_id
            .as_deref(),
        Some("b")
    );
    let mut a = quota("a", &[60.]);
    a.status = QuotaStatus::Exhausted;
    assert_eq!(
        choose(&router(false), &run(), &[a, quota("b", &[80.])])
            .profile_id
            .as_deref(),
        Some("b")
    );
}

#[test]
fn pin_grant_auth_and_attempt_ledger_are_hard_exclusions() {
    let mut run = run();
    run.pinned_profile_id = Some("a".into());
    run.attempted_profile_ids.push("a".into());
    assert!(choose(&router(false), &run, &[]).profile_id.is_none());
    run.pinned_profile_id = None;
    assert_eq!(
        choose(&router(false), &run, &[]).profile_id.as_deref(),
        Some("b")
    );
    run.allowed_profile_ids = vec!["a".into()];
    assert!(choose(&router(false), &run, &[]).profile_id.is_none());
    let mut a = profile("a");
    a.auth_state = AuthState::Unverified;
    assert!(select(&router(false), &run, &[a], &[], 1001)
        .profile_id
        .is_none());
}

#[test]
fn sibling_namespaces_share_exhaustion_and_recovery_attempt_budget() {
    let mut profiles = [profile("a"), profile("b")];
    for profile in &mut profiles {
        profile.quota_group_key = Some("authenticated-account-group".into());
    }
    let mut a = quota("a", &[0.0]);
    a.status = QuotaStatus::Exhausted;
    let reports = [a, quota("b", &[100.0])];
    let mut task = run();
    task.pinned_profile_id = Some("b".into());
    assert!(select(&router(false), &task, &profiles, &reports, 1001)
        .profile_id
        .is_none());
    task.pinned_profile_id = None;
    task.attempted_profile_ids.push("a".into());
    assert!(select(&router(false), &task, &profiles, &[], 1001)
        .profile_id
        .is_none());
    profiles[1].quota_group_key = Some("independent-account-group".into());
    assert_eq!(
        select(&router(false), &task, &profiles, &reports, 1001)
            .profile_id
            .as_deref(),
        Some("b")
    );
}

#[test]
fn duplicate_account_groups_do_not_balance_between_profile_ids() {
    let mut profiles = [profile("a"), profile("b")];
    for profile in &mut profiles {
        profile.quota_group_key = Some("authenticated-account-group".into());
    }
    let reports = [quota("a", &[100.0]), quota("b", &[80.0])];
    let mut task = run();
    task.active_profile_id = Some("b".into());
    assert_eq!(
        select(&router(true), &task, &profiles, &reports, 1001)
            .profile_id
            .as_deref(),
        Some("b")
    );
    task.active_profile_id = None;
    assert_eq!(
        select(&router(true), &task, &profiles, &reports, 1001)
            .profile_id
            .as_deref(),
        Some("a")
    );
    profiles[0].quota_group_key = Some(String::new());
    profiles[1].quota_group_key = Some(String::new());
    assert!(!shares_quota_group(&profiles[0], &profiles[1]));
}

#[test]
fn balance_ties_count_reservations_across_all_namespaces_in_a_group() {
    let mut profiles = [profile("a"), profile("b"), profile("c")];
    profiles[0].quota_group_key = Some("shared-group".into());
    profiles[1].quota_group_key = Some("shared-group".into());
    profiles[2].quota_group_key = Some("independent-group".into());
    let reports = [
        quota("a", &[80.0]),
        quota("b", &[80.0]),
        quota("c", &[80.0]),
    ];
    let mut policy = router(true);
    policy.ordered_profile_ids.push("c".into());
    let mut task = run();
    task.allowed_profile_ids.push("c".into());
    let reservations = [("a".into(), 0), ("b".into(), 3), ("c".into(), 1)];
    assert_eq!(
        select_with_reservations(&policy, &task, &profiles, &reports, 1001, &reservations)
            .profile_id
            .as_deref(),
        Some("c")
    );
    task.active_profile_id = Some("b".into());
    assert_eq!(
        select_with_reservations(&policy, &task, &profiles, &reports, 1001, &reservations)
            .profile_id
            .as_deref(),
        Some("b")
    );
}

#[test]
fn invalid_or_empty_quota_never_becomes_zero_or_full() {
    for value in [f64::NAN, f64::INFINITY, -1., 101.] {
        assert!(remaining(&quota("a", &[value]), 1001).is_none());
    }
    assert!(remaining(&quota("a", &[]), 1001).is_none());
    assert!(remaining(&quota("a", &[80.]), 32_000).is_none());
    assert!(remaining(&quota("a", &[80.]), 999).is_none());
}

#[test]
fn pre_block_or_older_reader_cannot_clear_exhaustion() {
    let mut old = quota("a", &[0.]);
    old.status = QuotaStatus::Exhausted;
    old.block_revision = 4;
    let mut report = quota("a", &[80.]);
    report.block_revision = 4;
    report.observed_at = 1100;
    assert!(!apply_quota(&mut old, report.clone(), 3));
    assert_eq!(old.status, QuotaStatus::Exhausted);
    report.epoch = 0;
    assert!(!apply_quota(&mut old, report.clone(), 4));
    report.epoch = 2;
    assert!(apply_quota(&mut old, report, 4));
    assert_eq!(old.status, QuotaStatus::Fresh);
}

#[test]
fn newer_authoritative_zero_updates_report_before_native_block_revision_advances() {
    let mut existing = quota("a", &[0.0]);
    existing.status = QuotaStatus::Exhausted;
    existing.block_revision = 3;
    let mut incoming = quota("a", &[0.0, 50.0]);
    incoming.status = QuotaStatus::Exhausted;
    incoming.block_revision = 3;
    incoming.epoch = 2;
    incoming.observed_at = 1001;
    assert!(apply_quota(&mut existing, incoming.clone(), 3));
    assert_eq!(existing.windows.len(), 2);
    assert_eq!(existing.epoch, 2);
    assert_eq!(existing.block_revision, 3);
    existing.block_revision += 1;
    let before = existing.clone();
    assert!(!apply_quota(&mut existing, incoming, 3));
    assert_eq!(existing, before);
}

#[test]
fn all_catalog_clis_have_explicit_capabilities_without_resume_promises() {
    let entries = registry();
    assert_eq!(entries.len(), 31);
    assert_eq!(
        entries
            .iter()
            .map(|c| c.cli)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        31
    );
    for entry in entries {
        assert!(!entry.reason.is_empty());
        assert!(!entry.cross_account_resume);
        assert!(!entry.same_account_resume);
        assert_eq!(entry.balance, entry.cli == TitleCli::Codex);
        assert_eq!(entry.quota_read, entry.cli == TitleCli::Codex);
        if entry.can_create_profile {
            let managed_namespace = adapter(entry.cli).namespace_env.is_some();
            assert!(
                managed_namespace || entry.gateway_terminal,
                "{:?}",
                entry.cli
            );
            if !managed_namespace {
                assert!(!entry.profile_terminal);
                assert!(!entry.managed_turns);
                assert!(super::gateway_profiles::support(entry.cli)
                    .is_some_and(|support| support.native_version.is_some()));
            }
        }
    }
    assert_eq!(
        adapter(TitleCli::Claude)
            .capability
            .api_key_label
            .as_deref(),
        Some("Anthropic API key")
    );
}

fn raw_codex_usage(
    used_primary: serde_json::Value,
    used_secondary: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "plan_type": "plus",
        "rate_limit": {
            "allowed": true,
            "limit_reached": false,
            "primary_window": {"used_percent": used_primary, "reset_at": 1_800_000_000},
            "secondary_window": {"used_percent": used_secondary, "reset_at": 1_800_000_100}
        },
        "additional_rate_limits": []
    })
}

#[test]
fn profile_quota_parser_preserves_remaining_units_without_clamping() {
    use super::usage::{parse_codex_report, CodexQuotaError};
    use serde_json::json;
    let parsed =
        parse_codex_report(&raw_codex_usage(json!(0), json!(20)), Some("gpt-5.4")).unwrap();
    assert_eq!(parsed[0].remaining_percent, Some(100.0));
    assert_eq!(parsed[1].remaining_percent, Some(80.0));
    assert_eq!(parsed[0].reset_at, Some(1_800_000_000_000));
    for invalid in [
        json!(100.01),
        json!(101),
        json!(-1),
        json!("NaN"),
        json!("20"),
        serde_json::Value::Null,
    ] {
        assert_eq!(
            parse_codex_report(&raw_codex_usage(invalid, json!(20)), None),
            Err(CodexQuotaError::InvalidReport)
        );
    }
    assert!(serde_json::from_str::<serde_json::Value>("{\"used_percent\":NaN}").is_err());
}

#[test]
fn profile_quota_parser_does_not_guess_model_or_shared_scope() {
    use super::usage::{parse_codex_report, CodexQuotaError};
    use serde_json::json;
    let mut report = raw_codex_usage(json!(0), json!(20));
    report["additional_rate_limits"] = json!([{
        "limit_name": "looks like a review bucket",
        "normal_model_slug": "gpt-5.4",
        "rate_limit": {"primary_window": {"used_percent": 100}}
    }]);
    for model in [None, Some("gpt-5.4"), Some("gpt-other")] {
        assert_eq!(
            parse_codex_report(&report, model),
            Err(CodexQuotaError::UnknownScope)
        );
    }
    report["additional_rate_limits"] = json!({"new_schema": true});
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::UnknownScope)
    );
    report["additional_rate_limits"] = json!([]);
    report["spend_control"] = json!({"allowed": false});
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::UnknownScope)
    );
}

#[test]
fn profile_quota_parser_distinguishes_known_zero_from_unresolved_block() {
    use super::usage::{parse_codex_report, CodexQuotaError};
    use serde_json::json;
    let mut report = raw_codex_usage(json!(20), json!(40));
    report["rate_limit"]["allowed"] = json!(false);
    report["rate_limit"]["limit_reached"] = json!(true);
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::UnknownScope)
    );
    report["rate_limit"]["primary_window"]["used_percent"] = json!(100);
    assert_eq!(
        parse_codex_report(&report, None).unwrap()[0].remaining_percent,
        Some(0.0)
    );
    report["plan_type"] = json!("api_key");
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::Unsupported)
    );
    report["plan_type"] = json!("unrecognized_plan");
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::Unsupported)
    );
}

#[test]
fn profile_quota_parser_rejects_missing_windows_and_invalid_reset_values() {
    use super::usage::{parse_codex_report, CodexQuotaError};
    use serde_json::json;
    let mut report = raw_codex_usage(json!(0), json!(20));
    report["rate_limit"]["primary_window"]["reset_at"] = json!(i64::MAX);
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::InvalidReport)
    );
    report["rate_limit"]["primary_window"]["reset_at"] = json!(1.5);
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::InvalidReport)
    );
    report["rate_limit"]["primary_window"] = json!(null);
    report["rate_limit"]["secondary_window"] = json!(null);
    assert_eq!(
        parse_codex_report(&report, None),
        Err(CodexQuotaError::InvalidReport)
    );
}

#[test]
fn message_only_exec_errors_and_http_errors_are_not_quota_reports() {
    use super::usage::parse_codex_report;
    use serde_json::json;
    for report in [
        json!({"type": "error", "message": "You've hit your usage limit. Try again later."}),
        json!({"type": "turn.failed", "error": {"message": "Rate limit reached"}}),
        json!({"status": 429, "error": {"message": "Too many requests"}}),
    ] {
        assert!(parse_codex_report(&report, None).is_err());
    }
}

#[test]
fn zero_under_unqualified_quota_status_is_not_exhaustion_evidence() {
    let mut task = run();
    task.active_profile_id = Some("a".into());
    for status in [
        QuotaStatus::Unknown,
        QuotaStatus::Unsupported,
        QuotaStatus::ReaderError,
        QuotaStatus::ReaderThrottled,
    ] {
        let mut unqualified = quota("a", &[0.0]);
        unqualified.status = status;
        let reports = [unqualified, quota("b", &[80.0])];
        let selected = choose(&router(true), &task, &reports);
        assert_eq!(selected.profile_id.as_deref(), Some("a"));
        assert!(selected.balance_degraded);
        assert_eq!(
            choose(&router(false), &task, &reports)
                .profile_id
                .as_deref(),
            Some("a")
        );
    }
}

#[test]
fn expired_or_old_positive_reports_degrade_balance_without_migrating_current() {
    let mut task = run();
    task.active_profile_id = Some("b".into());
    let mut expired = quota("a", &[100.0]);
    expired.expires_at = 1001;
    let reports = [expired, quota("b", &[80.0])];
    let selected = choose(&router(true), &task, &reports);
    assert_eq!(selected.profile_id.as_deref(), Some("b"));
    assert!(selected.balance_degraded);
    let mut old = quota("a", &[100.0]);
    old.observed_at = -30_000;
    old.expires_at = 100_000;
    let selected = choose(&router(true), &task, &[old, quota("b", &[80.0])]);
    assert_eq!(selected.profile_id.as_deref(), Some("b"));
    assert!(selected.balance_degraded);
}

#[test]
fn expired_known_zero_and_exhausted_reports_still_block_sibling_groups() {
    let mut profiles = [profile("a"), profile("b")];
    for profile in &mut profiles {
        profile.quota_group_key = Some("shared-group".into());
    }
    for status in [
        QuotaStatus::Fresh,
        QuotaStatus::Stale,
        QuotaStatus::Exhausted,
    ] {
        let mut known_zero = quota("a", &[0.0]);
        known_zero.status = status;
        known_zero.expires_at = 1000;
        known_zero.windows[0].reset_at = Some(999);
        let reports = [known_zero, quota("b", &[100.0])];
        assert!(select(&router(true), &run(), &profiles, &reports, 1001)
            .profile_id
            .is_none());
    }
}

#[test]
fn unqualified_refresh_never_erases_a_confirmed_capacity_block() {
    for status in [
        QuotaStatus::Unknown,
        QuotaStatus::Unsupported,
        QuotaStatus::ReaderError,
        QuotaStatus::ReaderThrottled,
        QuotaStatus::Stale,
    ] {
        let mut existing = quota("a", &[0.0]);
        existing.status = QuotaStatus::Exhausted;
        existing.block_revision = 2;
        let original = existing.clone();
        let mut incoming = quota("a", &[100.0]);
        incoming.status = status;
        incoming.block_revision = 2;
        incoming.epoch = 2;
        incoming.observed_at = 1001;
        assert!(!apply_quota(&mut existing, incoming, 2));
        assert_eq!(existing, original);
    }
}

fn blocked_group_fixture() -> Snapshot {
    let mut snapshot = fixture();
    for profile in &mut snapshot.profiles {
        profile.quota_group_key = Some("shared-group".into());
    }
    snapshot.profiles[1].enabled = false;
    snapshot.quota[0].block_revision = 3;
    snapshot.quota[1] = quota("b", &[0.0]);
    snapshot.quota[1].status = QuotaStatus::Exhausted;
    snapshot.quota[1].block_revision = 7;
    snapshot
}

fn positive_group_report() -> Quota {
    let mut report = quota("a", &[80.0]);
    report.epoch = 2;
    report.observed_at = 1001;
    report.block_revision = 3;
    report
}

#[test]
fn authoritative_positive_read_recovers_exhausted_disabled_sibling_atomically() {
    let baseline = blocked_group_fixture();
    let mut current = baseline.clone();
    assert!(select(
        &current.routers[0],
        &current.runs[0],
        &current.profiles,
        &current.quota,
        1001
    )
    .profile_id
    .is_none());
    let report = positive_group_report();
    let recovered =
        apply_group_recovery(&mut current, &baseline, "a", "shared-group", &report, 1001);
    assert_eq!(recovered, ["a", "b"]);
    assert_eq!(current.quota[1].status, QuotaStatus::Fresh);
    assert_eq!(current.quota[1].windows, report.windows);
    assert_eq!(current.quota[1].epoch, report.epoch);
    assert_eq!(current.quota[1].block_revision, 7);
    assert_eq!(current.quota[0].block_revision, 3);
    let recovered = current.clone();
    assert!(
        apply_group_recovery(&mut current, &baseline, "a", "shared-group", &report, 1001)
            .is_empty()
    );
    assert_eq!(current, recovered);
    assert_eq!(
        select(
            &current.routers[0],
            &current.runs[0],
            &current.profiles,
            &current.quota,
            1001
        )
        .profile_id
        .as_deref(),
        Some("a")
    );
}

#[test]
fn group_recovery_rejects_any_member_block_identity_or_membership_race() {
    let baseline = blocked_group_fixture();
    for race in 0..5 {
        let mut current = baseline.clone();
        match race {
            0 => current.quota[1].block_revision += 1,
            1 => current.profiles[1].revision += 1,
            2 => current.profiles[1].quota_group_key = Some("other-group".into()),
            3 => current.profiles.retain(|p| p.id != "b"),
            _ => current.quota[1].epoch = 3,
        }
        let before = current.clone();
        assert!(apply_group_recovery(
            &mut current,
            &baseline,
            "a",
            "shared-group",
            &positive_group_report(),
            1001
        )
        .is_empty());
        assert_eq!(current, before);
    }
}

#[test]
fn expired_failed_zero_or_wrong_account_read_never_recovers_a_group() {
    let baseline = blocked_group_fixture();
    for failure in 0..5 {
        let mut current = baseline.clone();
        let mut report = positive_group_report();
        match failure {
            0 => report.expires_at = 1001,
            1 => report.status = QuotaStatus::ReaderError,
            2 => report.windows[0].remaining_percent = Some(0.0),
            3 => report.block_revision = 2,
            _ => report.profile_id = "b".into(),
        }
        assert!(
            apply_group_recovery(&mut current, &baseline, "a", "shared-group", &report, 1001)
                .is_empty()
        );
        assert_eq!(current, baseline);
    }
    let mut current = baseline.clone();
    assert!(apply_group_recovery(
        &mut current,
        &baseline,
        "a",
        "wrong-group",
        &positive_group_report(),
        1001
    )
    .is_empty());
    assert_eq!(current, baseline);
}

#[test]
fn ordinary_positive_refresh_does_not_renew_recovery_budget() {
    let mut baseline = blocked_group_fixture();
    baseline.quota[1] = quota("b", &[50.0]);
    let mut current = baseline.clone();
    assert!(apply_group_recovery(
        &mut current,
        &baseline,
        "a",
        "shared-group",
        &positive_group_report(),
        1001
    )
    .is_empty());
    assert_eq!(current, baseline);
}

#[test]
fn mutation_and_idempotent_result_commit_together_and_payload_changes_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    let first = store
        .mutate("main:start:1", &serde_json::json!({"a":1,"b":2}), |s| {
            *s = fixture();
            Ok(())
        })
        .unwrap();
    assert!(!first.replayed);
    let replay = store
        .mutate("main:start:1", &serde_json::json!({"b":2,"a":1}), |_| {
            panic!("duplicate mutation")
        })
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.snapshot, replay.snapshot);
    assert!(store
        .mutate(
            "main:start:1",
            &serde_json::json!({"a":2,"b":2}),
            |_| Ok(())
        )
        .is_err());
    assert!(store
        .mutate("main:start:failed", &(), |s| {
            s.profiles.clear();
            Err("rejected".into())
        })
        .is_err());
    assert_eq!(store.snapshot().unwrap(), first.snapshot);
    assert!(
        !store
            .mutate("main:start:failed", &(), |_| Ok(()))
            .unwrap()
            .replayed
    );
}

#[test]
fn deleting_run_history_removes_transcripts_from_rows_and_replays_current_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    let input = "private input marker";
    let output = "private output marker";
    store
        .mutate("main:send:1", &input, |snapshot| {
            *snapshot = fixture();
            snapshot.runs[0].inputs.push(RunInput {
                id: "input".into(),
                text: input.into(),
            });
            snapshot.runs[0].output = output.into();
            snapshot.runs[0].state = RunState::Completed;
            Ok(())
        })
        .unwrap();
    let removed = store
        .mutate("main:remove:1", &"run", |snapshot| {
            snapshot.runs.clear();
            Ok(())
        })
        .unwrap()
        .snapshot;
    let replay = store
        .mutate("main:send:1", &input, |_| {
            panic!("deleted input must not be replayed")
        })
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.snapshot, removed);
    assert!(replay.snapshot.runs.is_empty());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let columns = connection
        .prepare("PRAGMA table_info(requests)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(columns, ["id", "digest", "committed_revision"]);
    let requests = connection
        .prepare("SELECT id || digest || committed_revision FROM requests")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let current: String = connection
        .query_row("SELECT data FROM snapshot", [], |row| row.get(0))
        .unwrap();
    for row in requests.iter().chain(std::iter::once(&current)) {
        assert!(!row.contains(input));
        assert!(!row.contains(output));
    }
    drop(connection);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(store
        .mutate("main:send:1", &input, |_| panic!(
            "restart must retain the tombstone"
        ))
        .unwrap()
        .snapshot
        .runs
        .is_empty());
}

fn legacy_router_database(path: &std::path::Path, current: &Snapshot, result: &str) {
    use sha2::{Digest, Sha256};
    let connection = rusqlite::Connection::open(path).unwrap();
    connection.execute_batch("CREATE TABLE snapshot (id INTEGER PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE requests (id TEXT PRIMARY KEY, digest TEXT NOT NULL, result TEXT NOT NULL);").unwrap();
    connection
        .execute(
            "INSERT INTO snapshot VALUES (1, ?1)",
            [serde_json::to_string(current).unwrap()],
        )
        .unwrap();
    let digest = format!("{:x}", Sha256::digest(b"\"input\""));
    connection
        .execute(
            "INSERT INTO requests VALUES ('main:send:legacy', ?1, ?2)",
            rusqlite::params![digest, result],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
}

#[test]
fn schema_one_migration_preserves_request_tombstone_without_archived_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut old = fixture();
    old.revision = 1;
    old.runs[0].inputs.push(RunInput {
        id: "legacy-input".into(),
        text: "deleted legacy input".into(),
    });
    old.runs[0].output = "deleted legacy output".into();
    let mut current = old.clone();
    current.revision = 2;
    current.runs.clear();
    legacy_router_database(&path, &current, &serde_json::to_string(&old).unwrap());
    let mut store = Store::open(&path).unwrap();
    let replay = store
        .mutate("main:send:legacy", &"input", |_| {
            panic!("legacy input must not execute")
        })
        .unwrap();
    assert!(replay.replayed);
    assert!(replay.snapshot.runs.is_empty());
    assert!(store
        .mutate("main:send:legacy", &"changed", |_| Ok(()))
        .is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
    let saved: String = connection
        .query_row(
            "SELECT committed_revision FROM requests WHERE id = 'main:send:legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, "1");
    assert!(connection.prepare("SELECT result FROM requests").is_err());
    let data: String = connection
        .query_row("SELECT data FROM snapshot", [], |row| row.get(0))
        .unwrap();
    assert!(!data.contains("deleted legacy"));
    assert!(store.credential_journal().unwrap().is_empty());
}

#[test]
fn corrupt_legacy_request_result_is_preserved_before_migration_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    legacy_router_database(&path, &fixture(), "corrupt archived request");
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
    let result: String = connection
        .query_row("SELECT result FROM requests", [], |row| row.get(0))
        .unwrap();
    assert_eq!(result, "corrupt archived request");
}

#[test]
fn corrupt_current_schema_tombstone_is_preserved_before_recovery_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.mutate("main:noop:1", &(), |_| Ok(())).unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE requests SET committed_revision = '2'", [])
        .unwrap();
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn schema_two_history_migrates_without_inventing_response_associations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .update(|snapshot| {
            *snapshot = fixture();
            snapshot.runs[0].inputs.push(RunInput {
                id: "old-input".into(),
                text: "legacy request".into(),
            });
            snapshot.runs[0].output = "unassociated legacy response".into();
            Ok(())
        })
        .unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let source: String = connection
        .query_row("SELECT data FROM snapshot", [], |row| row.get(0))
        .unwrap();
    let mut old: serde_json::Value = serde_json::from_str(&source).unwrap();
    for run in old["runs"].as_array_mut().unwrap() {
        run.as_object_mut().unwrap().remove("turns");
        run.as_object_mut().unwrap().remove("legacyOutput");
    }
    connection
        .execute(
            "UPDATE snapshot SET data = ?1",
            [serde_json::to_string(&old).unwrap()],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    let restored = store.snapshot().unwrap();
    assert_eq!(restored.runs[0].output, "unassociated legacy response");
    assert!(restored.runs[0].turns.is_empty());
    assert_eq!(restored.runs[0].legacy_output, None);
    assert_eq!(restored.runs[0].inputs[0].text, "legacy request");
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
}

#[test]
fn turn_records_cannot_forge_completion_or_reassign_saved_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("router.sqlite")).unwrap();
    store
        .update(|snapshot| {
            *snapshot = fixture();
            let run = &mut snapshot.runs[0];
            run.generation = 1;
            run.inputs.push(RunInput {
                id: "input".into(),
                text: "request".into(),
            });
            run.attempts.push(RunAttempt {
                id: "attempt".into(),
                input_id: "input".into(),
                profile_id: "a".into(),
                generation: 1,
                state: AttemptState::Completed,
                reason: "".into(),
            });
            run.turns.push(RunTurn {
                input_id: "input".into(),
                attempt_id: "attempt".into(),
                profile_id: "a".into(),
                generation: 1,
                state: AttemptState::Completed,
                text: "answer".into(),
            });
            run.output = "answer".into();
            Ok(())
        })
        .unwrap();
    let before = store.snapshot().unwrap();
    assert!(store
        .update(|snapshot| {
            snapshot.runs[0].turns[0].profile_id = "other".into();
            Ok(())
        })
        .is_err());
    assert!(store
        .update(|snapshot| {
            snapshot.runs[0].turns[0].state = AttemptState::Stopped;
            Ok(())
        })
        .is_err());
    assert!(store
        .update(|snapshot| {
            snapshot.runs[0].turns[0].text = "different".into();
            Ok(())
        })
        .is_err());
    assert_eq!(store.snapshot().unwrap(), before);
}

#[test]
fn schema_three_runs_remain_text_without_grant_expansion_or_inference() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .update(|snapshot| {
            *snapshot = fixture();
            for quota in &mut snapshot.quota {
                quota.status = QuotaStatus::Stale;
            }
            Ok(())
        })
        .unwrap();
    let before = store.snapshot().unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let mut legacy = serde_json::to_value(&before).unwrap();
    for run in legacy["runs"].as_array_mut().unwrap() {
        run.as_object_mut().unwrap().remove("executionMode");
    }
    connection
        .execute(
            "UPDATE snapshot SET data = ?1",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap(), before);
    assert_eq!(
        store.snapshot().unwrap().runs[0].execution_mode,
        RunExecutionMode::Text
    );
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
    let source: String = connection
        .query_row("SELECT data FROM snapshot", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&source).unwrap()["runs"][0]["executionMode"],
        "text"
    );
}

#[test]
fn default_text_creation_keeps_legacy_request_digest_shape() {
    let payload = serde_json::json!({
        "requestId":"c85b4737-11e4-4a44-8c99-0b00eb09cdb1",
        "routerId":"router", "cwd":"/project", "shellProfileId":"local:bash",
        "model":"gpt-6-sol", "reasoningEffort":"high", "title":"Task"
    });
    let request: super::commands::StartRun = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), payload);
    let mut explicit = payload.clone();
    explicit["executionMode"] = serde_json::json!("text");
    let request: super::commands::StartRun = serde_json::from_value(explicit.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), payload);
    explicit["executionMode"] = serde_json::json!("coding");
    let request: super::commands::StartRun = serde_json::from_value(explicit.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), explicit);
}

#[test]
fn full_request_journal_blocks_new_effects_without_expiring_duplicate_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    let original = store
        .mutate("main:noop:1", &(), |_| Ok(()))
        .unwrap()
        .snapshot;
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("WITH RECURSIVE counter(n) AS (VALUES(1) UNION ALL SELECT n + 1 FROM counter WHERE n < 99999) INSERT INTO requests SELECT 'fixture:' || n, printf('%064d', 0), '0' FROM counter;").unwrap();
    drop(connection);
    let failed = store.mutate("main:new:1", &(), |_| {
        panic!("full journal must fence effects")
    });
    assert!(failed.err().unwrap().contains("capacity"));
    let replay = store
        .mutate("main:noop:1", &(), |_| panic!("tombstone must not expire"))
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.snapshot, original);
    assert!(store
        .mutate("main:noop:1", &"different", |_| Ok(()))
        .is_err());
}

#[test]
fn restored_writes_reconcile_interrupted_intents_before_recovery_is_declared() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("router.sqlite")).unwrap();
    let original = store
        .mutate("main:send:failed-save", &"exact input", |snapshot| {
            *snapshot = fixture();
            let run = &mut snapshot.runs[0];
            run.state = RunState::Running;
            run.inputs.push(RunInput {
                id: "input".into(),
                text: "exact input".into(),
            });
            run.attempted_profile_ids.push("a".into());
            run.attempts.push(RunAttempt {
                id: "attempt".into(),
                input_id: "input".into(),
                profile_id: "a".into(),
                generation: 1,
                state: AttemptState::DispatchIntent,
                reason: "committed before dispatch".into(),
            });
            Ok(())
        })
        .unwrap()
        .snapshot;
    store.reject_credential_test_commit();
    assert!(store
        .update(|snapshot| {
            snapshot.runs[0].state = RunState::Completed;
            snapshot.runs[0].attempts[0].state = AttemptState::Completed;
            Ok(())
        })
        .is_err());
    assert_eq!(store.snapshot().unwrap(), original);
    assert!(store.reconcile_interrupted().is_err());
    assert_eq!(store.snapshot().unwrap(), original);
    store.restore_test_writes();
    let recovered = store.reconcile_interrupted().unwrap();
    assert_eq!(recovered.revision, original.revision + 1);
    assert_eq!(recovered.runs[0].state, RunState::RecoveryRequired);
    assert_eq!(
        recovered.runs[0].attempts[0].state,
        AttemptState::RecoveryRequired
    );
    assert_eq!(
        recovered.runs[0].generation,
        original.runs[0].generation + 1
    );
    assert_eq!(recovered.runs[0].revision, original.runs[0].revision + 1);
    assert_eq!(recovered.runs[0].inputs, original.runs[0].inputs);
    assert_eq!(
        recovered.runs[0].attempted_profile_ids,
        original.runs[0].attempted_profile_ids
    );
    let repeated = store.reconcile_interrupted().unwrap();
    assert_eq!(repeated.runs[0], recovered.runs[0]);
    assert!(
        store
            .mutate("main:send:failed-save", &"exact input", |_| panic!(
                "recovery never dispatches the saved input"
            ))
            .unwrap()
            .replayed
    );
}

#[test]
fn restart_preserves_exact_input_ledger_and_requires_explicit_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .mutate("main:send:1", &"input", |s| {
            *s = fixture();
            let run = &mut s.runs[0];
            run.state = RunState::Running;
            run.inputs.push(RunInput {
                id: "input".into(),
                text: "exact\ninput\r\n".into(),
            });
            run.attempted_profile_ids.push("a".into());
            run.attempts.push(RunAttempt {
                id: "attempt".into(),
                input_id: "input".into(),
                profile_id: "a".into(),
                generation: 1,
                state: AttemptState::DispatchIntent,
                reason: "selected".into(),
            });
            Ok(())
        })
        .unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let snapshot = store.snapshot().unwrap();
    let run = &snapshot.runs[0];
    assert_eq!(run.state, RunState::RecoveryRequired);
    assert_eq!(run.generation, 2);
    assert_eq!(run.attempts[0].state, AttemptState::RecoveryRequired);
    assert_eq!(run.inputs[0].text, "exact\ninput\r\n");
    assert_eq!(run.attempted_profile_ids, ["a"]);
    assert_eq!(snapshot.quota[0].status, QuotaStatus::Stale);
    assert!(
        store
            .mutate("main:send:1", &"input", |_| panic!("must not replay"))
            .unwrap()
            .replayed
    );
}

#[test]
fn invalid_attempt_ledger_rolls_back_and_history_survives_router_removal() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("router.sqlite")).unwrap();
    store
        .update(|s| {
            *s = fixture();
            Ok(())
        })
        .unwrap();
    assert!(store
        .update(|s| {
            s.runs[0].attempts.push(RunAttempt {
                id: "invalid".into(),
                input_id: "missing".into(),
                profile_id: "a".into(),
                generation: 1,
                state: AttemptState::Running,
                reason: String::new(),
            });
            Ok(())
        })
        .is_err());
    assert!(store.snapshot().unwrap().runs[0].attempts.is_empty());
    store
        .update(|s| {
            s.routers.clear();
            Ok(())
        })
        .unwrap();
    assert_eq!(store.snapshot().unwrap().runs.len(), 1);
}

#[test]
fn corrupt_and_newer_databases_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let corrupt = b"not a database\0preserve me";
    std::fs::write(&path, corrupt).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    std::fs::remove_file(&path).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE future (payload TEXT); INSERT INTO future VALUES ('preserve');",
        )
        .unwrap();
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn private_storage_rejects_symlinks_and_a_second_owner() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router.sqlite");
    let store = Store::open(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    drop(store);
    let alias = dir.path().join("alias.sqlite");
    symlink(&path, &alias).unwrap();
    assert!(Store::open(&alias).is_err());
}
