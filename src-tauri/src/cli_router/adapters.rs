use crate::cli_catalog::TitleCli;
use serde::Serialize;

pub(crate) const ALL_CLIS: [TitleCli; 31] = [
    TitleCli::Codex,
    TitleCli::Agy,
    TitleCli::Cursor,
    TitleCli::Claude,
    TitleCli::Gemini,
    TitleCli::Copilot,
    TitleCli::Opencode,
    TitleCli::Openclaw,
    TitleCli::Hermes,
    TitleCli::Pi,
    TitleCli::Aider,
    TitleCli::Goose,
    TitleCli::Cline,
    TitleCli::Kilo,
    TitleCli::Qwen,
    TitleCli::Kiro,
    TitleCli::Droid,
    TitleCli::Openhands,
    TitleCli::Continue,
    TitleCli::Amp,
    TitleCli::Auggie,
    TitleCli::Crush,
    TitleCli::Vibe,
    TitleCli::Kimi,
    TitleCli::Interpreter,
    TitleCli::Grok,
    TitleCli::Junie,
    TitleCli::Deepagents,
    TitleCli::Freebuff,
    TitleCli::Trae,
    TitleCli::Sweagent,
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelChoice {
    pub(crate) id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_efforts: Option<Vec<String>>,
}

fn model_choices(cli: TitleCli) -> Option<Vec<ModelChoice>> {
    if cli == TitleCli::Codex {
        return Some(
            super::codex::model_choices()
                .into_iter()
                .map(|(id, efforts)| ModelChoice {
                    id,
                    reasoning_efforts: Some(efforts),
                })
                .collect(),
        );
    }
    let models = match cli {
        TitleCli::Pi => super::runtime::pi_supported_models(),
        TitleCli::Openclaw => super::openclaw::supported_models(),
        TitleCli::Opencode => super::opencode::supported_models(),
        TitleCli::Vibe => super::vibe::supported_models(),
        TitleCli::Qwen => super::qwen::supported_models(),
        TitleCli::Kimi => super::kimi::supported_models(),
        TitleCli::Hermes => super::hermes::supported_models(),
        TitleCli::Crush => super::crush::supported_models(),
        TitleCli::Grok => super::grok::supported_models(),
        _ => return None,
    };
    Some(
        models
            .iter()
            .map(|id| ModelChoice {
                id: (*id).into(),
                reasoning_efforts: None,
            })
            .collect(),
    )
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Capability {
    pub(crate) cli: TitleCli,
    pub(crate) name: String,
    pub(crate) can_create_profile: bool,
    pub(crate) can_verify_login: bool,
    pub(crate) api_key_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) models: Option<Vec<ModelChoice>>,
    pub(crate) can_start: bool,
    pub(crate) profile_terminal: bool,
    pub(crate) managed_turns: bool,
    pub(crate) coding_turns: bool,
    pub(crate) native_turns: bool,
    pub(crate) native_account_terminal: bool,
    pub(crate) gateway_terminal: bool,
    pub(crate) native_history_resume: bool,
    pub(crate) gateway_protocol: Option<super::gateway_profiles::Protocol>,
    pub(crate) gateway_source: Option<String>,
    pub(crate) gateway_gates: Option<String>,
    pub(crate) same_account_resume: bool,
    pub(crate) cross_account_resume: bool,
    pub(crate) quota_read: bool,
    pub(crate) balance: bool,
    pub(crate) read_only: bool,
    pub(crate) reason: String,
}

pub(crate) struct Adapter {
    pub(crate) capability: Capability,
    pub(crate) namespace_env: Option<&'static str>,
}

/// Namespace support is independent of token isolation or tool-effect fencing.
/// No adapter promises replay/resume after an uncertain effect or account switch.
pub(crate) fn adapter(cli: TitleCli) -> Adapter {
    let gateway = super::gateway_profiles::support(cli);
    let gateway_available = gateway.is_some()
        && cfg!(unix)
        && (cli != TitleCli::Grok || super::grok_artifact::compiled_available());
    let native_account = super::native_accounts::available(cli);
    let namespace_env = match cli {
        TitleCli::Grok if native_account => Some("GROK_HOME"),
        TitleCli::Agy if native_account => Some("HOME"),
        TitleCli::Kilo if native_account => Some("XDG_DATA_HOME"),
        TitleCli::Codex => Some("CODEX_HOME"),
        TitleCli::Claude => Some("CLAUDE_CONFIG_DIR"),
        TitleCli::Gemini => Some("GEMINI_CLI_HOME"),
        TitleCli::Pi => Some("PI_CODING_AGENT_DIR"),
        TitleCli::Goose => Some("GOOSE_PATH_ROOT"),
        TitleCli::Openclaw => Some("OPENCLAW_HOME"),
        TitleCli::Vibe => Some("VIBE_HOME"),
        TitleCli::Opencode => Some("OPENCODE_TEST_HOME"),
        TitleCli::Qwen => Some("QWEN_HOME"),
        TitleCli::Kimi => Some("KIMI_CODE_HOME"),
        TitleCli::Hermes => Some("HERMES_HOME"),
        TitleCli::Crush if cfg!(target_os = "macos") => Some("CRUSH_GLOBAL_CONFIG"),
        _ => None,
    };
    let qualified = namespace_env.is_some();
    let managed_turns = matches!(
        cli,
        TitleCli::Codex
            | TitleCli::Claude
            | TitleCli::Gemini
            | TitleCli::Pi
            | TitleCli::Goose
            | TitleCli::Openclaw
            | TitleCli::Vibe
            | TitleCli::Opencode
            | TitleCli::Qwen
            | TitleCli::Kimi
            | TitleCli::Hermes
    ) || (cli == TitleCli::Crush && cfg!(target_os = "macos"));
    let reason = match cli {
        TitleCli::Codex => "Source-based candidate for Codex 0.160.0: verified subscription profiles, exact owned model catalog and explicit reasoning effort. Text turns disable environment, filesystem and tools. Mediated Coding retains a native thread and offers four bounded project file tools; writes require one-use Main approval and an editor freeze. Typed quota rejection may switch only after durable native checkpoint and process drain; unknown acceptance or effects block replay. Cross-account opaque history remains subject to real account qualification. Installed clients and real accounts have not been exercised.",
        TitleCli::Claude => "Claude Code 2.1.63 and 2.1.287 support native Anthropic API Gateway coding, private native subscription login and saved workspace history. Native tools and permissions remain available. Managed text mode is separate and requires an API key with tools disabled. Subscription quota and automatic subscription-account switching are unavailable.",
        TitleCli::Gemini => "Managed API-key turns use a private working directory with tools, extensions, MCP and hooks disabled. Project files are unavailable. OAuth profile verification, quota and cross-account resume are not qualified.",
        TitleCli::Agy => "Native agy 1.2.16 Gemini API Gateway preserves coding tools and permission prompts. Subscription accounts use private HOME and the exact verified macOS ARM64 artifact's native file-backed OAuth login. The native remote login flow avoids its shared keyring. Native history resume requires a valid owned conversation database and original account binding. Subscription quota and automatic cross-account continuation are not qualified.",
        TitleCli::Cursor => "API-key headless execution is documented. Subscription storage, complete tool restrictions and final failure events still need an adapter.",
        TitleCli::Copilot => "A separate COPILOT_HOME and JSONL output are documented. Credential precedence and complete tool restrictions still need an adapter.",
        TitleCli::Opencode => "OpenCode 1.18.33 and 1.18.34 support native API Gateway coding, private account login and saved workspace history. Native tools and permissions remain available. Managed text mode is separate and pinned to 1.18.34 with tools and plugins disabled. Subscription quota and automatic subscription-account switching are unavailable.",
        TitleCli::Openclaw => "Source-based candidate for OpenClaw 2026.9.8: OpenAI API keys only, an approved exact text model, fresh private state, all tools, plugins, hooks and skills disabled, and one authoritative JSON result. No profile terminal, quota, balance, native resume or automatic failover. Native overflow recovery can compact up to three times, and timeout recovery up to twice; multiple provider requests remain possible. Installed clients and real accounts have not been exercised.",
        TitleCli::Hermes => "Source-based candidate for Hermes Agent 0.21.5, stable v2026.9.24: native Anthropic API keys only and exact claude-sonnet-4-6 text model. Fresh owned state, role-denied tools and disabled plugins, MCP, hooks, skills, compression and auxiliary calls. Admission requires the exact official installer shim, pinned checkout and its venv Python plus Anthropic SDK 0.87.0. An owned isolated bootstrap bypasses shell wrappers, editable .pth and startup customizers; bytecode shadows, installation overrides and repair markers are refused. Context including the owned non-path prefix is limited to 60 KiB. Completion requires correlated final text and owned process drain. No OAuth profile login, quota, balance, resume or automatic failover. Installed clients and real accounts have not been exercised.",
        TitleCli::Pi => "Pi 0.73.1 and 1.0.1 support native Responses API Gateway coding, private provider login and saved workspace history. Native tools and permissions remain available; subscriptions use native /login. Managed text mode is separate and disables tools. Subscription quota and automatic subscription-account switching are unavailable.",
        TitleCli::Aider => "Separate configuration and history paths are documented. Ask/dry-run modes do not establish full isolation or structured completion; managed routing needs a dedicated adapter.",
        TitleCli::Goose => "Source-based candidate for Goose 1.53.0: OpenAI API keys only, an exact model ID, fresh private state, no extensions or hooks, and chat mode. Shared system configuration must be absent under trusted root-owned directories. No ordinary profile terminal, quota, balancing, resume or automatic failover. Native provider and initial-stream retries each default to three; empty replies can make up to four ordinary turns despite max-turns 1, and context overflow can invoke one auxiliary compaction request. Five-minute and one-MiB output limits apply. Installed clients and real accounts have not been exercised.",
        TitleCli::Cline => "JSONL and separate configuration/data roots are documented. Headless approval defaults and complete tool restrictions still need an adapter.",
        TitleCli::Kilo => "Native Kilo 7.8.3 API Gateway and persistent private HOME/XDG account login preserve native tools and permissions. KILO_NO_DAEMON prevents existing-daemon attachment. Saved Gateway history resumes by an existing session ID. Native subscription/provider credentials remain with Kilo; Lomi does not claim subscription quota or automatic subscription-account continuation.",
        TitleCli::Qwen => "Source-based candidate for Qwen Code 0.24.7: OpenAI API keys and exact approved text model, fresh private state, and the pinned private Hosted Harness HTTP/SSE protocol. An owned durable store checks writer leases, immutable journal/resources and final model provenance. Tools and startup discovery are disabled by the hosted profile. Prompt context is limited to 60 KiB; inline resources to 64 KiB. No profile terminal, quota, balance, native resume or automatic failover. Installed clients and real accounts have not been exercised.",
        TitleCli::Kiro => "API headless execution is documented. JSON depends on the selected engine; account roots, MCP and complete tool restrictions still need an adapter.",
        TitleCli::Droid => "JSON execution is documented. Credential roots, skill discovery and complete tool restrictions still need a dedicated adapter.",
        TitleCli::Openhands => "CLI integration is documented. Account isolation, sandbox ownership and the headless event protocol need qualification together.",
        TitleCli::Continue => "Print JSON is documented. Readonly/auto modes ignore tool exclusions, and readonly permits Bash; private auth and MCP startup need separate restrictions.",
        TitleCli::Amp => "Tool-denial globs are documented. Project settings can override local settings; private state, MCP startup and completion need a dedicated adapter.",
        TitleCli::Auggie => "Print execution and named tool removal are documented. Credential roots, automatic skills/indexing and complete tool restrictions still need an adapter.",
        TitleCli::Crush => "Source-based macOS candidate for Crush 0.97.1: native Anthropic API keys and exact claude-sonnet-4-6, fresh private state and an owned foreground Unix-socket HTTP/SSE server. Builtin tools, MCP, hooks, skills and configuration commands are fenced. Completion requires correlated RunComplete, canonical messages, idle state, client retirement, validated stream EOF and owned process drain. Native retries, same-key authentication retry, a title helper and a secret-free GitHub release check remain possible. No profile terminal, quota, balance, resume or automatic failover. Linux and installed clients have not been qualified.",
        TitleCli::Vibe => "Source-based candidate for Vibe 2.25.8: the public legacy app-server, OpenAI API keys and exact approved chat model, fresh private state, tools and startup discovery disabled. Mistral credentials are absent, preventing remote managed configuration. No profile terminal, quota, balance or native resume. Generic transport can retry within a 300-second budget. Installed clients and real accounts have not been exercised.",
        TitleCli::Kimi => "Kimi npm 2.1.1 supports native API Gateway coding, private subscription login and saved workspace history. Python 1.52.0 supports API Gateway and native continuation only. Native tools and permissions remain available. Managed text mode is separate and requires international Moonshot Open Platform API keys. Subscription quota and automatic subscription-account switching are unavailable.",
        TitleCli::Interpreter => "The current native client has separate state and JSON execution. Published feature switches do not establish complete tool and startup restrictions.",
        TitleCli::Grok => "Native Grok 1.0.45 is built from pinned public source and admitted by its compiled SHA-256 manifest. Responses API Gateway and isolated native subscription login retain native coding tools, permissions and history. Published PATH binaries are not used. Native resume remains with the account that created opaque context. Subscription quota and automatic subscription-account continuation are not qualified.",
        TitleCli::Junie => "JSON task execution and private JUNIE_HOME are documented. Headless project trust, shared credentials and complete builtin-tool restrictions still need qualification.",
        TitleCli::Deepagents => "The documented filesystem allow-list requires read_file and leaves other tools outside its scope. Current text output lacks the required managed-turn protocol.",
        TitleCli::Freebuff => "Current free-account CLI has no headless JSON turn interface. Service terms require provider permission for unattended wrappers; automatic account routing is unavailable.",
        TitleCli::Trae => "The task runner completes through a task_done tool and emits console text. It needs a contained task-runner adapter with its own completion contract.",
        TitleCli::Sweagent => "The issue runner performs environment and shell operations outside model tools. It needs a contained issue-runner adapter and trajectory contract.",
    };
    Adapter {
        capability: Capability {
            cli,
            name: cli.name().into(),
            can_create_profile: qualified || gateway_available,
            can_verify_login: matches!(cli, TitleCli::Codex | TitleCli::Claude),
            api_key_label: match cli {
                TitleCli::Claude | TitleCli::Hermes => Some("Anthropic API key".into()),
                TitleCli::Crush if qualified => Some("Anthropic API key".into()),
                TitleCli::Gemini => Some("Gemini API key".into()),
                TitleCli::Kimi => Some("Moonshot international Open Platform API key".into()),
                TitleCli::Pi
                | TitleCli::Goose
                | TitleCli::Openclaw
                | TitleCli::Vibe
                | TitleCli::Opencode
                | TitleCli::Qwen => Some("OpenAI API key".into()),
                _ => None,
            },
            models: model_choices(cli),
            can_start: managed_turns,
            profile_terminal: native_account || cli == TitleCli::Gemini,
            managed_turns,
            coding_turns: cli == TitleCli::Codex && cfg!(unix),
            native_turns: native_account && cli != TitleCli::Agy,
            native_account_terminal: native_account,
            gateway_terminal: gateway_available,
            native_history_resume: gateway_available && super::gateway_profiles::can_resume(cli),
            gateway_protocol: gateway.as_ref().map(|support| support.protocol),
            gateway_source: gateway
                .as_ref()
                .map(|support| support.source_url.to_owned()),
            gateway_gates: gateway
                .as_ref()
                .map(|support| format!("{} {}", support.gates, support.required_admission)),
            same_account_resume: false,
            cross_account_resume: false,
            quota_read: cli == TitleCli::Codex,
            balance: cli == TitleCli::Codex,
            read_only: !qualified && !gateway_available,
            reason: reason.into(),
        },
        namespace_env,
    }
}

pub(crate) fn registry() -> Vec<Capability> {
    ALL_CLIS
        .into_iter()
        .map(|cli| adapter(cli).capability)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermes_is_api_text_only_and_grok_requires_its_compiled_owned_artifact() {
        let hermes = adapter(TitleCli::Hermes);
        assert_eq!(hermes.namespace_env, Some("HERMES_HOME"));
        let capability = hermes.capability;
        assert!(capability.can_create_profile && capability.can_start && capability.managed_turns);
        assert_eq!(
            capability.api_key_label.as_deref(),
            Some("Anthropic API key")
        );
        assert!(
            !capability.profile_terminal
                && !capability.can_verify_login
                && !capability.quota_read
                && !capability.balance
                && !capability.same_account_resume
                && !capability.cross_account_resume
        );
        let grok = adapter(TitleCli::Grok).capability;
        assert_eq!(
            grok.can_create_profile,
            super::super::grok_artifact::compiled_available()
        );
        assert_eq!(
            grok.gateway_terminal,
            super::super::grok_artifact::compiled_available()
        );
        assert!(!grok.can_start && !grok.managed_turns);
        assert!(grok.api_key_label.is_none());
    }
    #[test]
    fn published_model_choices_are_admitted_by_dispatch() {
        for cli in [
            TitleCli::Pi,
            TitleCli::Openclaw,
            TitleCli::Opencode,
            TitleCli::Vibe,
            TitleCli::Qwen,
            TitleCli::Kimi,
            TitleCli::Hermes,
            TitleCli::Codex,
        ] {
            let choices = adapter(cli).capability.models.unwrap();
            assert!(!choices.is_empty());
            for choice in choices {
                let id = Some(choice.id.as_str());
                let accepted = match cli {
                    TitleCli::Pi => {
                        super::super::runtime::pi_supported_models().contains(&choice.id.as_str())
                    }
                    TitleCli::Openclaw => super::super::openclaw::model(id).is_ok(),
                    TitleCli::Opencode => super::super::opencode::model(id).is_ok(),
                    TitleCli::Vibe => super::super::vibe::model(id).is_ok(),
                    TitleCli::Qwen => super::super::qwen::model(id).is_ok(),
                    TitleCli::Kimi => super::super::kimi::model(id).is_ok(),
                    TitleCli::Hermes => super::super::hermes::model(id).is_ok(),
                    TitleCli::Codex => super::super::codex::model(id).is_ok(),
                    _ => unreachable!(),
                };
                assert!(
                    accepted,
                    "advertised model must be executable: {}",
                    choice.id
                );
                if cli == TitleCli::Codex {
                    let efforts = choice.reasoning_efforts.unwrap();
                    assert!(!efforts.is_empty());
                    for effort in efforts {
                        assert!(super::super::codex::effort(&choice.id, Some(&effort)).is_ok());
                    }
                    assert!(super::super::codex::effort(&choice.id, None).is_err());
                    assert!(
                        super::super::codex::effort(&choice.id, Some("unknown-effort")).is_err()
                    );
                }
            }
        }
        for cli in [TitleCli::Claude, TitleCli::Gemini, TitleCli::Goose] {
            assert!(adapter(cli).capability.models.is_none());
        }
        assert!(adapter(TitleCli::Codex).capability.api_key_label.is_none());
    }
}

// Native-owned namespace variables whose configured locations require protection.
pub(crate) fn namespace_variables() -> Vec<&'static str> {
    ALL_CLIS
        .iter()
        .filter_map(|cli| adapter(*cli).namespace_env)
        .collect()
}
