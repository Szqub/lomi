use crate::cli_catalog::TitleCli;
use serde::{Deserialize, Serialize};

macro_rules! state {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum $name { $($variant),+ }
    };
}

state!(AuthState {
    Disconnected,
    Connecting,
    Verifying,
    Unverified,
    Ready,
    Refreshing,
    ReauthRequired,
    Disabled,
    PendingRemove,
    IdentityMismatch,
    Error
});
state!(StorageMode {
    Keyring,
    SessionOnly,
    CliManaged,
    ApiKey
});
state!(QuotaStatus {
    Unknown,
    Fresh,
    Stale,
    Exhausted,
    ReaderThrottled,
    ReaderError,
    Unsupported
});
state!(RunState {
    Idle,
    Starting,
    Running,
    Switching,
    WaitingForCapacity,
    Paused,
    Completed,
    Stopped,
    Failed,
    RecoveryRequired
});
state!(AttemptState {
    Selected,
    DispatchIntent,
    Running,
    Completed,
    Rejected,
    Failed,
    Stopped,
    RecoveryRequired
});

impl AttemptState {
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Selected | Self::DispatchIntent | Self::Running)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Profile {
    pub(crate) id: String,
    pub(crate) cli: TitleCli,
    pub(crate) label: String,
    pub(crate) enabled: bool,
    pub(crate) revision: u64,
    pub(crate) auth_state: AuthState,
    pub(crate) storage_mode: StorageMode,
    #[serde(default)]
    pub(crate) credential_ref: Option<String>,
    #[serde(default)]
    pub(crate) quota_group_key: Option<String>,
    /// Explicit native API destination used only by gateway terminals. Existing
    /// subscription and managed-text bindings keep their original contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) gateway_provider: Option<ApiDestination>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApiDestination {
    pub(crate) protocol: super::gateway_profiles::Protocol,
    pub(crate) base_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Router {
    pub(crate) id: String,
    pub(crate) cli: TitleCli,
    pub(crate) label: String,
    pub(crate) enabled: bool,
    pub(crate) ordered_profile_ids: Vec<String>,
    pub(crate) balance_remaining_quota: bool,
    pub(crate) revision: u64,
}

/// Windows must already be qualified by the native reader as blocking the run's
/// model and execution mode. Unknown or incomplete scope is not a fresh report.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QuotaWindow {
    pub(crate) id: String,
    pub(crate) remaining_percent: Option<f64>,
    pub(crate) reset_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Quota {
    pub(crate) profile_id: String,
    pub(crate) status: QuotaStatus,
    pub(crate) windows: Vec<QuotaWindow>,
    pub(crate) observed_at: i64,
    pub(crate) expires_at: i64,
    pub(crate) epoch: u64,
    pub(crate) block_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunInput {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunAttempt {
    pub(crate) id: String,
    pub(crate) input_id: String,
    pub(crate) profile_id: String,
    pub(crate) generation: u64,
    pub(crate) state: AttemptState,
    pub(crate) reason: String,
}

/// Text observed from one exact native attempt. Completed is authoritative only
/// after the provider terminal event, clean EOF and successful process drain.
/// Other terminal states preserve uncertain partial output without replay.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunTurn {
    pub(crate) input_id: String,
    pub(crate) attempt_id: String,
    pub(crate) profile_id: String,
    pub(crate) generation: u64,
    pub(crate) state: AttemptState,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunExecutionMode {
    #[default]
    Text,
    Coding,
    Gateway,
    Native,
}

impl RunExecutionMode {
    pub(crate) fn is_text(&self) -> bool {
        *self == Self::Text
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Run {
    pub(crate) id: String,
    pub(crate) router_id: String,
    pub(crate) cwd: String,
    #[serde(default)]
    pub(crate) shell_profile_id: Option<String>,
    pub(crate) title: String,
    pub(crate) state: RunState,
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(default)]
    pub(crate) execution_mode: RunExecutionMode,
    /// Main explicitly requested continuation of a fully checkpointed native
    /// interrupted turn. Consumed by dispatch; never inferred from status text.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) continuation_requested: bool,
    pub(crate) pinned_profile_id: Option<String>,
    pub(crate) allowed_profile_ids: Vec<String>,
    pub(crate) active_profile_id: Option<String>,
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) inputs: Vec<RunInput>,
    pub(crate) attempts: Vec<RunAttempt>,
    pub(crate) output: String,
    #[serde(default)]
    pub(crate) turns: Vec<RunTurn>,
    /// Pre-ledger output cannot be associated with an input or completion.
    /// Preserve it verbatim once, separate from newly recorded native turns.
    #[serde(default)]
    pub(crate) legacy_output: Option<String>,
    pub(crate) status_message: String,
    /// Recovery ledger for the current logical input, cleared only after a
    /// completed turn or an authoritative capacity/auth repair.
    pub(crate) attempted_profile_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub(crate) revision: u64,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) routers: Vec<Router>,
    pub(crate) quota: Vec<Quota>,
    pub(crate) runs: Vec<Run>,
}
