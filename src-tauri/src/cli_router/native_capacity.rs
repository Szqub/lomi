//! Native CLI observations only. A fresh observation is not a routing grant:
//! these source schemas do not establish router-wide or model-specific scope.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_TEXT: usize = 512;
const MAX_GROUPS: usize = 64;
const MAX_WINDOWS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Identity {
    pub(crate) source: String,
    pub(crate) provider: String,
    pub(crate) principal_id: String,
    pub(crate) organization_id: Option<String>,
    pub(crate) display_label: String,
}

impl Identity {
    /// Length-delimited encoding preserves principal and selected organization
    /// boundaries without using email, credential material, or display labels.
    pub(crate) fn fingerprint(&self) -> String {
        let mut result = String::new();
        for part in [
            self.source.as_str(),
            self.provider.as_str(),
            self.principal_id.as_str(),
        ] {
            result.push_str(&format!("{}:{part}", part.len()));
        }
        match self.organization_id.as_deref() {
            Some(org) => result.push_str(&format!("s{}:{org}", org.len())),
            None => result.push('n'),
        }
        result
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Window {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) remaining_percent: Option<f64>,
    pub(crate) remaining_amount: Option<f64>,
    pub(crate) unit: Option<String>,
    /// Unix milliseconds, matching the router's existing quota windows.
    pub(crate) reset_at: Option<i64>,
    pub(crate) disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) native_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) native_window: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CapacityStatus {
    Unknown,
    Fresh,
    Exhausted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Observation {
    pub(crate) identity: Option<Identity>,
    pub(crate) windows: Vec<Window>,
    pub(crate) source: String,
    pub(crate) status: CapacityStatus,
    pub(crate) scope: Option<String>,
}

fn observation(source: &str) -> Observation {
    Observation {
        identity: None,
        windows: Vec::new(),
        source: source.into(),
        status: CapacityStatus::Unknown,
        scope: None,
    }
}

fn window(id: &str, name: &str) -> Window {
    Window {
        id: id.into(),
        name: name.into(),
        remaining_percent: None,
        remaining_amount: None,
        unit: None,
        reset_at: None,
        disabled: false,
        native_id: None,
        native_window: None,
    }
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= MAX_TEXT && !s.chars().any(char::is_control))
        .map(str::to_owned)
}

fn number(value: &Value, max: f64) -> Option<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && (0.0..=max).contains(n))
}

fn amount(value: &Value) -> Option<f64> {
    number(value, 9_007_199_254_740_991.0)
}

fn used_percent(value: &Value) -> Option<f64> {
    number(value, 100.0).map(|n| 100.0 - n)
}

fn used_ratio(value: &Value) -> Option<f64> {
    number(value, 1.0).map(|n| (1.0 - n) * 100.0)
}

fn rfc3339(value: &Value) -> Option<i64> {
    let value = value.as_str().filter(|s| s.len() <= 64)?;
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.timestamp_millis())
        .filter(|n| *n >= 0)
}

#[cfg(test)]
fn seconds(value: &Value) -> Option<i64> {
    value.as_i64().filter(|n| *n >= 0)?.checked_mul(1000)
}

fn dimension_status(windows: &[Window]) -> CapacityStatus {
    let enabled: Vec<_> = windows.iter().filter(|w| !w.disabled).collect();
    if enabled.is_empty() {
        return CapacityStatus::Unknown;
    }
    if enabled
        .iter()
        .any(|w| w.remaining_percent == Some(0.0) || w.remaining_amount == Some(0.0))
    {
        return CapacityStatus::Exhausted;
    }
    if enabled
        .iter()
        .all(|w| w.remaining_percent.is_some() || w.remaining_amount.is_some())
    {
        CapacityStatus::Fresh
    } else {
        CapacityStatus::Unknown
    }
}

/// Auth status supplies labels and organization but no stable principal ID.
/// The collector must bind it to an independently verified native identity.
pub(crate) fn parse_claude_auth(_status: &Value) -> Observation {
    observation("claude:auth-status")
}

// Native statusline quotas require a live owned run; no independent reader is
// qualified for production account refresh. Keep source fixtures for qualification.
#[cfg(test)]
pub(crate) fn parse_claude_statusline(value: &Value) -> Observation {
    let mut result = observation("claude:statusline");
    for (key, name) in [("five_hour", "5 hour"), ("seven_day", "7 day")] {
        let mut item = window(&format!("claude:{key}"), name);
        let raw = &value["rate_limits"][key];
        item.remaining_percent = used_percent(&raw["used_percentage"]);
        item.reset_at = seconds(&raw["resets_at"]);
        result.windows.push(item);
    }
    result.status = dimension_status(&result.windows);
    result
}

pub(crate) fn parse_grok(auth: &Value, billing: &Value) -> Observation {
    let mut result = observation("grok:acp-billing");
    if let (Some(principal), Some(kind)) =
        (text(&auth["principalId"]), text(&auth["principalType"]))
    {
        // Principal kind is part of the namespace; user and team IDs can overlap.
        result.identity = Some(Identity {
            source: "grok:acp-auth-info".into(),
            provider: "xai".into(),
            principal_id: format!("{}:{kind}{principal}", kind.len()),
            organization_id: text(&auth["organizationId"]).or_else(|| text(&auth["teamId"])),
            display_label: text(&auth["email"]).unwrap_or(principal),
        });
    }
    let raw = &billing["config"];
    let mut included = window("grok:included-credits", "Included credits");
    included.remaining_percent = used_percent(&raw["creditUsagePercent"]);
    included.reset_at = rfc3339(&raw["currentPeriod"]["end"]);
    result.windows.push(included);
    if raw.get("prepaidBalance").is_some() {
        let mut prepaid = window("grok:prepaid", "Prepaid balance");
        // Do not apply proto defaults to an absent/null scalar ourselves.
        prepaid.remaining_amount = amount(&raw["prepaidBalance"]["val"]);
        prepaid.unit = Some("USD cents".into());
        result.windows.push(prepaid);
    }
    // Included, prepaid and on-demand pools are alternatives. This schema does
    // not prove which can service the selected model, so keep aggregate unknown.
    result
}

pub(crate) fn parse_kimi(userinfo_envelope: &Value, usage_envelope: &Value) -> Observation {
    let mut result = observation("kimi:native-oauth-usage");
    let user = &userinfo_envelope["data"];
    if user["kind"].as_str() == Some("ok") {
        let info = &user["userInfo"];
        if let Some(principal) = text(&info["globalId"]).or_else(|| text(&info["userId"])) {
            result.identity = Some(Identity {
                source: "kimi:native-oauth-userinfo".into(),
                provider: "kimi".into(),
                principal_id: principal.clone(),
                organization_id: None,
                display_label: text(&info["email"])
                    .or_else(|| text(&info["nickname"]))
                    .unwrap_or(principal),
            });
        }
    }
    let usage = &usage_envelope["data"];
    if usage["kind"].as_str() != Some("ok") {
        return result;
    }
    for (key, name) in [
        ("limit5h", "5 hour"),
        ("limit7d", "7 day"),
        ("monthTotal", "Monthly total"),
        ("monthCode", "Monthly coding"),
    ] {
        let Some(raw) = usage["quota"]["usages"].get(key) else {
            continue;
        };
        let mut item = window(&format!("kimi:{key}"), name);
        item.remaining_percent = used_ratio(&raw["usedRatio"]);
        item.reset_at = rfc3339(&raw["resetAt"]);
        result.windows.push(item);
    }
    // monthCode is the coding breakdown of monthTotal, not a separate limit.
    let limits: Vec<_> = result
        .windows
        .iter()
        .filter(|window| window.id != "kimi:monthCode")
        .cloned()
        .collect();
    result.status = dimension_status(&limits);
    // The extra wallet can permit use beyond the plan quota. Its billing scope
    // is not a universal blocking window and is deliberately not inferred.
    if usage["quota"]
        .get("extraUsage")
        .is_some_and(|v| !v.is_null())
    {
        result.status = CapacityStatus::Unknown;
    }
    result
}

pub(crate) fn parse_kilo(value: &Value) -> Observation {
    let mut result = observation("kilo:native-profile");
    // The profile has email and selected organization, but no principal ID.
    let mut balance = window("kilo:balance", "Credit balance");
    balance.remaining_amount = amount(&value["balance"]["balance"]);
    balance.unit = Some("USD".into());
    result.windows.push(balance);
    if value.get("kiloPass").is_some_and(|v| !v.is_null()) {
        let raw = &value["kiloPass"];
        let mut pass = window("kilo:pass", "Kilo Pass credits");
        pass.remaining_amount = amount(&raw["currentPeriodBaseCreditsUsd"])
            .zip(amount(&raw["currentPeriodBonusCreditsUsd"]))
            .zip(amount(&raw["currentPeriodUsageUsd"]))
            .and_then(|((base, bonus), used)| {
                let remaining = base + bonus - used;
                (remaining.is_finite() && (0.0..=9_007_199_254_740_991.0).contains(&remaining))
                    .then_some(remaining)
            });
        pass.unit = Some("USD".into());
        pass.reset_at = rfc3339(&raw["nextBillingAt"]);
        result.windows.push(pass);
    }
    // Balance and pass credits are not established as independent blocking
    // dimensions. Preserve amounts without guessing aggregate availability.
    result
}

/// Pinned Agy 1.2.16 quotaData. Positions preserve duplicate native IDs without
/// treating display names as provider/model scope.
pub(crate) fn parse_agy_usage(value: &Value) -> Observation {
    let mut result = observation("antigravity:native-usage");
    let Some(groups) = value["groups"].as_array() else {
        return result;
    };
    let mut invalid = groups.len() > MAX_GROUPS;
    for (group_index, group) in groups.iter().take(MAX_GROUPS).enumerate() {
        let Some(buckets) = group["buckets"].as_array() else {
            invalid = true;
            continue;
        };
        if buckets.len() > MAX_WINDOWS.saturating_sub(result.windows.len()) {
            invalid = true;
        }
        for (bucket_index, raw) in buckets.iter().enumerate() {
            if result.windows.len() >= MAX_WINDOWS {
                break;
            }
            // Include group and array positions to retain duplicate native IDs.
            let id = format!("agy:{group_index}:{bucket_index}");
            let mut item = window(&id, &text(&raw["name"]).unwrap_or_else(|| id.clone()));
            item.native_id = text(&raw["id"]);
            item.native_window = text(&raw["window"]);
            match raw.get("disabled") {
                None => {}
                Some(Value::Bool(disabled)) => item.disabled = *disabled,
                Some(_) => invalid = true,
            }
            item.remaining_percent = number(&raw["remaining_fraction"], 1.0).map(|n| n * 100.0);
            item.remaining_amount = raw["remaining_amount"]
                .as_i64()
                .filter(|n| (0..=9_007_199_254_740_991).contains(n))
                .map(|n| n as f64);
            if !item.disabled
                && (raw
                    .get("remaining_fraction")
                    .is_some_and(|v| !v.is_null() && item.remaining_percent.is_none())
                    || raw
                        .get("remaining_amount")
                        .is_some_and(|v| !v.is_null() && item.remaining_amount.is_none()))
            {
                invalid = true;
            }
            item.reset_at = rfc3339(&raw["reset_time"]);
            result.windows.push(item);
        }
    }
    if !invalid {
        result.status = dimension_status(&result.windows);
    }
    result
}

/// Standalone /usage is handled before an agent turn. Require the reviewed
/// report envelope so inference output cannot masquerade as a quota report.
pub(crate) fn parse_agy_report(value: &Value) -> Result<Observation, String> {
    let invalid = || "Antigravity returned an unqualified native usage report.".to_owned();
    if value["status"] != "SUCCESS"
        || value["num_turns"] != 0
        || value["conversation_id"] != ""
        || value["command"]["name"] != "usage"
    {
        return Err(invalid());
    }
    let usage = value["usage"].as_object().ok_or_else(invalid)?;
    let counters = [
        "input_tokens",
        "output_tokens",
        "thinking_tokens",
        "cache_read_tokens",
        "total_tokens",
    ];
    if usage.len() != counters.len()
        || counters
            .iter()
            .any(|key| usage.get(*key).and_then(Value::as_u64) != Some(0))
    {
        return Err(invalid());
    }
    let data = &value["command"]["data"];
    let groups = data["groups"].as_array().ok_or_else(invalid)?;
    if groups.len() > MAX_GROUPS {
        return Err(invalid());
    }
    let mut total = 0usize;
    for group in groups {
        text(&group["name"]).ok_or_else(invalid)?;
        let buckets = group["buckets"].as_array().ok_or_else(invalid)?;
        total = total.checked_add(buckets.len()).ok_or_else(invalid)?;
        if total > MAX_WINDOWS {
            return Err(invalid());
        }
        for bucket in buckets {
            text(&bucket["name"]).ok_or_else(invalid)?;
            for key in ["id", "window"] {
                if bucket
                    .get(key)
                    .is_some_and(|value| !value.is_null() && text(value).is_none())
                {
                    return Err(invalid());
                }
            }
            if bucket
                .get("disabled")
                .is_some_and(|value| !value.is_boolean())
                || bucket
                    .get("remaining_fraction")
                    .is_some_and(|value| !value.is_null() && number(value, 1.0).is_none())
                || bucket.get("remaining_amount").is_some_and(|value| {
                    !value.is_null()
                        && value
                            .as_i64()
                            .is_none_or(|n| !(0..=9_007_199_254_740_991).contains(&n))
                })
                || bucket
                    .get("reset_time")
                    .is_some_and(|value| !value.is_null() && rfc3339(value).is_none())
            {
                return Err(invalid());
            }
        }
    }
    Ok(parse_agy_usage(data))
}

pub(crate) fn unsupported_capacity(provider: &str) -> Observation {
    match provider {
        "pi" => observation("pi:unsupported-native-capacity"),
        "opencode" => observation("opencode:unsupported-native-capacity"),
        _ => observation("unsupported-native-capacity"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_requires_real_percentages_and_ignores_cost() {
        let result = parse_claude_statusline(&json!({"cost":{"total_cost_usd":0},
            "rate_limits":{"five_hour":{"used_percentage":20,"resets_at":123},
            "seven_day":{"used_percentage":100}}}));
        assert_eq!(result.status, CapacityStatus::Exhausted);
        assert_eq!(result.windows[0].remaining_percent, Some(80.0));
        assert_eq!(result.windows[0].reset_at, Some(123000));
        for bad in [json!(null), json!("0"), json!(-1), json!(101), json!(true)] {
            let result = parse_claude_statusline(&json!({"rate_limits":{
                "five_hour":{"used_percentage":bad},"seven_day":{"used_percentage":10}}}));
            assert_eq!(result.status, CapacityStatus::Unknown);
        }
        assert!(
            parse_claude_auth(&json!({"loggedIn":true,"email":"a@b","orgId":"o"}))
                .identity
                .is_none()
        );
    }

    #[test]
    fn grok_parses_native_camel_case_without_treating_alternative_pools_as_limits() {
        let result = parse_grok(
            &json!({"principalId":"u1","principalType":"user",
            "organizationId":"o1","email":"a@b","token":"secret"}),
            &json!({"config":{"creditUsagePercent":100,"prepaidBalance":{"val":500},
                "currentPeriod":{"end":"2026-10-04T12:00:00Z"}}}),
        );
        assert_eq!(result.status, CapacityStatus::Unknown);
        assert_eq!(result.windows[0].remaining_percent, Some(0.0));
        assert_eq!(result.windows[1].remaining_amount, Some(500.0));
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("token"));
        assert!(parse_grok(&json!({"email":"a@b"}), &Value::Null)
            .identity
            .is_none());
    }

    #[test]
    fn fingerprints_bind_principal_and_organization_without_display_labels() {
        let mut identity = Identity {
            source: "s".into(),
            provider: "p".into(),
            principal_id: "user".into(),
            organization_id: Some("org1".into()),
            display_label: "a@b".into(),
        };
        let original = identity.fingerprint();
        identity.display_label = "changed@b".into();
        assert_eq!(original, identity.fingerprint());
        identity.organization_id = Some("org2".into());
        assert_ne!(original, identity.fingerprint());
    }

    #[test]
    fn kimi_retains_every_dimension_and_requires_success_envelopes() {
        let identity = json!({"data":{"kind":"ok","userInfo":{"userId":"u","email":"a@b"}}});
        let usage = json!({"data":{"kind":"ok","quota":{"extraUsage":null,"usages":{
            "limit5h":{"usedRatio":0.2},"limit7d":{"usedRatio":0.3},
            "monthTotal":{"usedRatio":1},"monthCode":{"usedRatio":0.4}}}}});
        let result = parse_kimi(&identity, &usage);
        assert_eq!(result.windows.len(), 4);
        assert_eq!(result.status, CapacityStatus::Exhausted);
        assert_eq!(result.identity.unwrap().principal_id, "u");
        assert_eq!(
            parse_kimi(
                &identity,
                &json!({"data":{"kind":"error","quota":usage["data"]["quota"]}})
            )
            .status,
            CapacityStatus::Unknown
        );
        let breakdown = json!({"data":{"kind":"ok","quota":{"usages":{
            "monthTotal":{"usedRatio":0.8},"monthCode":{"usedRatio":1}}}}});
        assert_eq!(
            parse_kimi(&identity, &breakdown).status,
            CapacityStatus::Fresh
        );
    }

    #[test]
    fn kilo_null_balance_is_unknown_and_never_zero() {
        let result = parse_kilo(&json!({"profile":{"email":"a@b"},"balance":null,"kiloPass":null}));
        assert_eq!(result.windows[0].remaining_amount, None);
        assert_eq!(result.status, CapacityStatus::Unknown);
        assert!(result.identity.is_none());
        let result = parse_kilo(&json!({"balance":{"balance":12},"kiloPass":{
            "currentPeriodBaseCreditsUsd":10,"currentPeriodBonusCreditsUsd":5,"currentPeriodUsageUsd":4}}));
        assert_eq!(result.windows[1].remaining_amount, Some(11.0));
        assert_eq!(result.windows[0].remaining_percent, None);
    }

    #[test]
    fn agy_keeps_disabled_unknown_and_all_enabled_buckets_distinct() {
        let result = parse_agy_usage(&json!({"groups":[{"name":"g","buckets":[
            {"id":"same","name":"a","remaining_fraction":0.7},
            {"id":"same","name":"b","remaining_fraction":0,"disabled":true},
            {"name":"c","remaining_amount":3}]}]}));
        assert_eq!(result.status, CapacityStatus::Fresh);
        assert_eq!(result.windows.len(), 3);
        assert_ne!(result.windows[0].id, result.windows[1].id);
        assert!(result.scope.is_none());
        let unknown =
            parse_agy_usage(&json!({"groups":[{"buckets":[{"remaining_fraction":null}]}]}));
        assert_eq!(unknown.status, CapacityStatus::Unknown);
        let invalid = parse_agy_usage(&json!({"groups":[{"buckets":[{
            "remaining_fraction":2,"remaining_amount":3}]}]}));
        assert_eq!(invalid.status, CapacityStatus::Unknown);
        let oversized = parse_agy_usage(
            &json!({"groups":[{"buckets":vec![json!({"remaining_fraction":1});MAX_WINDOWS + 1]}]}),
        );
        assert_eq!(oversized.windows.len(), MAX_WINDOWS);
        assert_eq!(oversized.status, CapacityStatus::Unknown);
    }

    #[test]
    fn standalone_agy_report_preserves_native_dimensions_without_inventing_scope() {
        let envelope = json!({"status":"SUCCESS","num_turns":0,"conversation_id":"",
        "usage":{"input_tokens":0,"output_tokens":0,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":0},"command":{"name":"usage","data":{
            "groups":[{"name":"Models","buckets":[
                {"id":"bucket","name":"Capacity","window":"daily","remaining_fraction":0,
                 "remaining_amount":0,"reset_time":"2026-10-05T00:00:00Z"},
                {"id":"bucket","name":"Unavailable","disabled":true,"remaining_fraction":null}
            ]}]}}});
        let report = parse_agy_report(&envelope).unwrap();
        assert_eq!(report.windows[0].remaining_percent, Some(0.0));
        assert_eq!(report.windows[0].remaining_amount, Some(0.0));
        assert_eq!(report.windows[0].native_id.as_deref(), Some("bucket"));
        assert_eq!(report.windows[0].native_window.as_deref(), Some("daily"));
        assert_eq!(report.windows[0].reset_at, Some(1_791_158_400_000));
        assert!(report.windows[0].unit.is_none());
        assert!(report.windows[1].disabled);
        assert!(report.windows[1].remaining_percent.is_none());
        assert_ne!(report.windows[0].id, report.windows[1].id);
        assert!(report.identity.is_none());
        assert!(report.scope.is_none());
        for (key, value) in [
            ("status", json!("ERROR")),
            ("num_turns", json!(1)),
            ("conversation_id", json!("agent-session")),
            ("usage", json!({"input_tokens":1})),
        ] {
            let mut changed = envelope.clone();
            changed[key] = value;
            assert!(parse_agy_report(&changed).is_err());
        }
        for bad in [json!(-0.1), json!(1.1), json!("0"), json!(true)] {
            let mut changed = envelope.clone();
            changed["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] = bad;
            assert!(parse_agy_report(&changed).is_err());
        }
        let mut changed = envelope;
        changed["command"]["data"]["groups"][0]["buckets"] =
            json!(vec![json!({"name":"b"}); MAX_WINDOWS + 1]);
        assert!(parse_agy_report(&changed).is_err());
    }

    #[test]
    fn reset_types_are_strict_and_unsupported_sources_stay_unknown() {
        assert_eq!(seconds(&json!("123")), None);
        assert_eq!(seconds(&json!(1.5)), None);
        assert_eq!(seconds(&json!(i64::MAX)), None);
        assert_eq!(rfc3339(&json!("tomorrow")), None);
        assert_eq!(rfc3339(&json!("2026-10-04")), None);
        for provider in ["pi", "opencode"] {
            let result = unsupported_capacity(provider);
            assert_eq!(result.status, CapacityStatus::Unknown);
            assert!(result.windows.is_empty());
            assert!(result.identity.is_none());
        }
    }
}
