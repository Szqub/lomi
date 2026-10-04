use super::types::QuotaWindow;
use crate::cli_usage::CliUsage;
use serde_json::Value;
use std::path::Path;

pub(crate) use crate::cli_usage::ProfileQuotaError as CodexQuotaError;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AuthenticatedCodexQuota {
    pub(crate) group_key: String,
    pub(crate) windows: Result<Vec<QuotaWindow>, CodexQuotaError>,
}

/// Only ordinary local Codex turns are qualified. Unknown model-specific
/// buckets degrade the whole report; labels are never used for scope matching.
pub(crate) async fn read_codex(
    state: &CliUsage,
    directory: &Path,
    model: Option<&str>,
) -> Result<AuthenticatedCodexQuota, CodexQuotaError> {
    let (report, group_key) = crate::cli_usage::fetch_codex_profile_quota(state, directory).await?;
    Ok(AuthenticatedCodexQuota {
        group_key,
        windows: parse_codex_report(&report, model),
    })
}

pub(crate) fn parse_codex_report(
    report: &Value,
    _model: Option<&str>,
) -> Result<Vec<QuotaWindow>, CodexQuotaError> {
    let report = report.as_object().ok_or(CodexQuotaError::InvalidReport)?;
    match report.get("plan_type").and_then(Value::as_str) {
        Some(
            "plus" | "pro" | "prolite" | "promax" | "team" | "business" | "enterprise" | "edu"
            | "education" | "edu_plus" | "edu_pro",
        ) => {}
        _ => return Err(CodexQuotaError::Unsupported),
    }
    // Additional scopes can target a normal_model_slug or a shared pool. Until
    // the installed model contract is qualified, none can be ignored or guessed.
    if report
        .get("additional_rate_limits")
        .is_some_and(|value| !value.is_null() && !value.as_array().is_some_and(Vec::is_empty))
        || report
            .get("spend_control")
            .is_some_and(|value| !value.is_null())
    {
        return Err(CodexQuotaError::UnknownScope);
    }
    let rate = report
        .get("rate_limit")
        .and_then(Value::as_object)
        .ok_or(CodexQuotaError::InvalidReport)?;
    let allowed = rate
        .get("allowed")
        .and_then(Value::as_bool)
        .ok_or(CodexQuotaError::InvalidReport)?;
    let limit_reached = rate
        .get("limit_reached")
        .and_then(Value::as_bool)
        .ok_or(CodexQuotaError::InvalidReport)?;
    let mut windows = Vec::new();
    for field in ["primary_window", "secondary_window"] {
        let Some(value) = rate.get(field).filter(|value| !value.is_null()) else {
            continue;
        };
        let window = value.as_object().ok_or(CodexQuotaError::InvalidReport)?;
        let used = window
            .get("used_percent")
            .and_then(Value::as_f64)
            .ok_or(CodexQuotaError::InvalidReport)?;
        if !used.is_finite() || !(0.0..=100.0).contains(&used) {
            return Err(CodexQuotaError::InvalidReport);
        }
        let reset_at = match window.get("reset_at") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let seconds = value
                    .as_i64()
                    .filter(|seconds| *seconds >= 0)
                    .ok_or(CodexQuotaError::InvalidReport)?;
                Some(
                    seconds
                        .checked_mul(1000)
                        .ok_or(CodexQuotaError::InvalidReport)?,
                )
            }
        };
        windows.push(QuotaWindow {
            id: format!("codex:{field}"),
            remaining_percent: Some(100.0 - used),
            reset_at,
        });
    }
    if windows.is_empty() {
        return Err(CodexQuotaError::InvalidReport);
    }
    let has_zero = windows
        .iter()
        .any(|window| window.remaining_percent == Some(0.0));
    if (!allowed || limit_reached) && !has_zero {
        // A non-window block (credits/workspace policy) is not a made-up 0%.
        return Err(CodexQuotaError::UnknownScope);
    }
    if allowed && !limit_reached && has_zero {
        return Err(CodexQuotaError::InvalidReport);
    }
    Ok(windows)
}
