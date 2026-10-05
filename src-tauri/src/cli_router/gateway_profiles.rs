//! Fresh native coding profiles for an authenticated loopback API gateway.
//!
//! These are endpoint/configuration compatibility candidates, not the managed
//! text adapters. Native tools, history and permission prompts remain native.
//! The caller MUST clear the child environment, inject only its reviewed OS
//! variables and `Launch::environment`, and keep the authenticated gateway alive
//! until the complete native process group drains. Never merge real credentials
//! or user profile directories into these homes. Project/native plugin effects
//! remain governed by the native client's permission UX, not Lomi's tool broker.
//!
//! CCR profile schemas: released 3.1.1, gitHead
//! 471e715c20cfa855c681f6d31dc652164d4fa654. This module intentionally does not
//! reproduce CCR's symlink/import/global-profile writers or ToolHub injection.

use crate::cli_catalog::TitleCli;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) enum Protocol {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai_chat")]
    OpenAiChat,
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
    #[serde(rename = "gemini")]
    Gemini,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Compatibility {
    pub(crate) protocol: Protocol,
    pub(crate) source_url: &'static str,
    /// Required source/schema admission, not a claim about installed clients.
    pub(crate) gates: &'static str,
    /// Root must enforce this admission before gateway creation or credential access.
    pub(crate) required_admission: &'static str,
    pub(crate) native_family: &'static str,
    /// None means the CCR writer alone does not establish a native release pin.
    pub(crate) native_version: Option<&'static str>,
}

pub(crate) struct Launch {
    pub(crate) arguments: Vec<OsString>,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) private_home: PathBuf,
}

const CCR_SERVICE: &str = "https://github.com/musistudio/claude-code-router/blob/471e715c20cfa855c681f6d31dc652164d4fa654/packages/core/src/profiles/service.ts";
const PROVIDER: &str = "lomi-gateway";

pub(crate) fn support(cli: TitleCli) -> Option<Compatibility> {
    let (protocol, source_url, gates) = match cli {
        // Source/artifact pin: OpenClaw 2026.9.8 / fc23bc864e4553c2d215e479eeec47b67a0bf943
        TitleCli::Openclaw => (Protocol::OpenAiResponses,
            "https://github.com/openclaw/openclaw/tree/fc23bc864e4553c2d215e479eeec47b67a0bf943/src/tui",
            "Embedded tui --local only, never daemon attachment; models.mode replace suppresses provider discovery; sole custom Responses provider with main/utility/compaction/subagents pinned; owned config/workspace and fresh auth store; optional provider plugins/embedding services disabled; native coding tools and default permission checks retained; API-file/media workflows unsupported"),
        // Source/artifact pin: Qwen Code 0.24.7 / b12edec1401a28fc53cd9e714d5928b285071fc8
        TitleCli::Qwen => (Protocol::OpenAiChat,
            "https://github.com/QwenLM/qwen-code/tree/b12edec1401a28fc53cd9e714d5928b285071fc8/packages/cli/src/config",
            "Native OpenAI Chat backend; explicit main/fast/compaction/vision model and endpoint; private highest-priority system settings; project dotenv/settings absent; separate DashScope web-search disabled; API-file/image-generation/realtime workflows unsupported; native coding tools and permission prompts retained"),
        // Source/artifact pin: @anthropic-ai/claude-code 2.1.63 / npm sha1 fc4103d9e1041365c0a081273ea9af1ba53f979b
        TitleCli::Claude => (Protocol::Anthropic, "https://registry.npmjs.org/@anthropic-ai/claude-code/-/claude-code-2.1.63.tgz",
            "Native Claude 2.1.63/2.1.287 apiKeyHelper/CLAUDE_CONFIG_DIR schema; private settings and local token only; managed policy may override endpoint and must be admitted before launch."),
        // Source/artifact pin: Codex 0.160.0 / a956835d020762cb2b570053af06f643a11c0ecc; CCR 3.1.1
        TitleCli::Codex => (Protocol::OpenAiResponses, CCR_SERVICE,
            "Pinned Codex custom Responses provider and exact owned catalog model; ambient managed policy admission; normal native approvals and sandbox remain enabled."),
        // Source/artifact pin: Grok public source 2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8; CCR 3.1.1
        TitleCli::Grok => (Protocol::OpenAiResponses, CCR_SERVICE,
            "Router-owned Grok 1.0.45 built from pinned public source; SHA-256 artifact and source version checked before dispatch; native Responses tools and permissions retained."),
        TitleCli::Agy => (Protocol::Gemini,
            "https://www.antigravity.google/docs/cli/install/",
            "Antigravity agy 1.2.16 native Gemini API mode; private HOME and explicit modelProvider=gemini, GEMINI_API_KEY and GOOGLE_GEMINI_BASE_URL; native tools, history and permission prompts retained. Google account subscriptions use a separate authentication mode."),
        // Source/artifact pin: kimi-cli 1.52.0 / sdist sha256 765c520b95ee9831e72c335cd57e4f669489d2b871413b9948553cc7279562d5
        TitleCli::Kimi => (Protocol::OpenAiChat, "https://pypi.org/project/kimi-cli/1.52.0/#files",
            "Python kimi-cli 1.52.0 openai_legacy and npm @moonshot-ai/kimi-code 2.1.1 openai Chat schemas selected by an isolated version inspection. Private state, explicit main and secondary model; native tools and permissions retained."),
        // Source/artifact pin: @mariozechner/pi-coding-agent 0.73.1 / 781152fc24841dc54b22284514604048ebe5e2c9
        TitleCli::Pi => (Protocol::OpenAiResponses,
            "https://registry.npmjs.org/@mariozechner/pi-coding-agent/-/pi-coding-agent-0.73.1.tgz",
            "Pi 0.73.1 and Earendil Pi 1.0.1 models.json custom openai-responses provider; explicit provider/model/local API token and private auth/session namespace; native auxiliary requests use the selected provider."),
        // Source/artifact pin: CCR 3.1.1 / 471e715c20cfa855c681f6d31dc652164d4fa654
        TitleCli::Opencode => (Protocol::OpenAiChat,
            "https://github.com/musistudio/claude-code-router/blob/471e715c20cfa855c681f6d31dc652164d4fa654/packages/core/src/agents/opencode/profile-config.ts",
            "OpenCode 1.18.33/1.18.34 JSON provider.options.baseURL/apiKey and OPENCODE_CONFIG_CONTENT schema; root must admit managed overrides and native config precedence before launch."),
        // Source/artifact pin: @kilocode/cli 7.8.3 / 59f1428abb5fe782ee7bd4d258e72a08b74aadb4
        TitleCli::Kilo => (Protocol::OpenAiChat,
            "https://github.com/Kilo-Org/kilocode/blob/59f1428abb5fe782ee7bd4d258e72a08b74aadb4/packages/opencode/src/config/config.ts",
            "Kilo OpenCode-derived KILO_CONFIG_CONTENT schema; new private HOME/XDG state; attach to an existing daemon is forbidden."),
        // Source/artifact pin: Hermes 0.21.5 / f97608f178d1ffeca59860195ab7da295f7c8e5f
        TitleCli::Hermes => (Protocol::Anthropic,
            "https://github.com/NousResearch/hermes-agent/blob/f97608f178d1ffeca59860195ab7da295f7c8e5f/agent/anthropic_adapter.py",
            "Native Anthropic model.base_url/api_mode; HERMES_HOME and real HOME private; alternate auxiliary providers, memories, title upgrade and credential fallbacks absent."),
        // Source/artifact pin: Vibe 2.25.8 / 7c19608af06f6c61d63f8f7a5c3430da73fba2ab
        TitleCli::Vibe => (Protocol::OpenAiChat,
            "https://github.com/mistralai/mistral-vibe/blob/7c19608af06f6c61d63f8f7a5c3430da73fba2ab/vibe/core/config/vibe_schema.py",
            "Native generic OpenAI-compatible provider; MISTRAL_API_KEY absent prevents managed remote configuration; selected provider/model and title model fixed locally."),
        _ => return None,
    };
    Some(Compatibility {
        protocol,
        source_url,
        gates,
        required_admission: match cli {
            TitleCli::Openclaw => "npm openclaw 2026.9.8; prepare_with_project required; admit_project before dispatch and final owner fence; embedded native runtime only",
            TitleCli::Qwen => "npm @qwen-code/qwen-code 0.24.7; admit_project before dispatch and final owner fence; no OAuth/login provider declaration",

            TitleCli::Kimi => "Only source-pinned Python 1.52.0 or npm 2.1.1; family-specific configuration and flags",
            TitleCli::Grok => "Only the router-owned source-built artifact; PATH Grok installations are never admitted",
            TitleCli::Agy => "Native agy 1.2.16; machine/project provider overrides must be absent; API and subscription modes are distinct",
            _ => "Installed native executable must match the source-pinned endpoint/config schema; fresh HOME, cleared environment and ambient managed policy admitted",
        },
        native_family: match cli {
            TitleCli::Openclaw => "npm openclaw embedded native TUI", TitleCli::Qwen => "npm @qwen-code/qwen-code", TitleCli::Claude => "npm @anthropic-ai/claude-code", TitleCli::Codex => "Native OpenAI codex-cli", TitleCli::Pi => "npm @mariozechner/pi-coding-agent or @earendil-works/pi-coding-agent", TitleCli::Kilo => "npm @kilocode/cli native platform binary", TitleCli::Opencode => "npm opencode-ai native platform binary", TitleCli::Hermes => "Python hermes-agent", TitleCli::Vibe => "Python mistral-vibe", TitleCli::Kimi => "Python kimi-cli or npm @moonshot-ai/kimi-code", TitleCli::Grok => "Lomi-owned Grok source-built native Responses artifact", TitleCli::Agy => "Official Antigravity agy native CLI", _ => "Native CLI family documented by source URL and gates",
        },
        native_version: match cli {
            TitleCli::Openclaw => Some("2026.9.8"), TitleCli::Qwen => Some("0.24.7"), TitleCli::Claude => Some("2.1.63"), TitleCli::Kimi => Some("1.52.0"), TitleCli::Pi => Some("0.73.1"), TitleCli::Kilo => Some("7.8.3"), TitleCli::Codex => Some("0.160.0"), TitleCli::Opencode => Some("1.18.34"), TitleCli::Hermes => Some("0.21.5"), TitleCli::Vibe => Some("2.25.8"), TitleCli::Grok => Some("1.0.45"), TitleCli::Agy => Some("1.2.16"), _ => None,
        },
    })
}

/// Refuse native project files that can import credentials or replace routing.
/// Root must call before gateway/key creation and again at its final owner fence.
/// This is a source admission check, not an OS sandbox against later external edits.
pub(crate) fn admit_project(cli: TitleCli, cwd: &Path) -> Result<(), String> {
    if !cwd.is_absolute() {
        return Err("Native gateway project directory must be absolute.".into());
    }
    let canonical = cwd
        .canonicalize()
        .map_err(|_| "Cannot resolve gateway project directory.")?;
    if !canonical.is_dir() {
        return Err("Native gateway project scope must be a directory.".into());
    }
    if cli == TitleCli::Codex {
        super::codex::ambient_policy()?;
    }
    if cli == TitleCli::Agy {
        absent(Path::new("/etc/antigravity/admin_settings.json"))?;
        #[cfg(target_os = "macos")]
        absent(Path::new(
            "/Library/Application Support/Antigravity/admin_settings.json",
        ))?;
    }
    let blocked: &[&str] = match cli {
        TitleCli::Openclaw => &[".env"],
        TitleCli::Qwen => &[".env", ".qwen/.env", ".qwen/settings.json"],
        TitleCli::Agy => &[".gemini/antigravity-cli/settings.json"],
        _ => &[],
    };
    // Native config discovery may walk ancestors. Check lexical and
    // canonical chains, rejecting links
    // and unreadable candidates as well as regular files without reading secrets.
    for chain in [cwd, canonical.as_path()] {
        for ancestor in chain.ancestors() {
            for name in blocked {
                let candidate = ancestor.join(name);
                match fs::symlink_metadata(&candidate) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    _ => return Err(format!("Native gateway admission requires removing or isolating project credential/config override: {}", candidate.display())),
                }
            }
        }
    }
    if matches!(cli, TitleCli::Claude | TitleCli::Opencode | TitleCli::Kilo) {
        #[cfg(target_os = "macos")]
        absent(Path::new("/Library/Managed Preferences"))?;
        let namespace = match cli {
            TitleCli::Claude => "ClaudeCode",
            TitleCli::Kilo => "kilo",
            _ => "opencode",
        };
        #[cfg(target_os = "macos")]
        absent(&Path::new("/Library/Application Support").join(namespace))?;
        #[cfg(not(target_os = "macos"))]
        absent(&Path::new("/etc").join(if cli == TitleCli::Claude {
            "claude-code"
        } else {
            namespace
        }))?;
    }
    Ok(())
}

/// Pin the native coding workspace before creating any private configuration.
/// The caller must launch with this same canonical cwd and recheck admission at
/// its final owner fence; no workspace path or clean-project flag comes from CLI.
/// Only these native clients have source-qualified history continuation flags.
pub(crate) fn can_resume(cli: TitleCli) -> bool {
    matches!(
        cli,
        TitleCli::Claude
            | TitleCli::Codex
            | TitleCli::Grok
            | TitleCli::Kimi
            | TitleCli::Kilo
            | TitleCli::Opencode
            | TitleCli::Pi
            | TitleCli::Agy
    )
}

pub(crate) fn admitted_version(cli: TitleCli, output: &[u8]) -> Option<String> {
    if cli == TitleCli::Grok {
        return super::grok_artifact::version_matches(output).then(|| "1.0.45".into());
    }
    let text = std::str::from_utf8(output).ok()?;
    let expected = support(cli)?.native_version?;
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .map(|v| v.trim_start_matches('v'))
        .find(|v| {
            *v == expected
                || matches!(
                    (cli, *v),
                    (TitleCli::Claude, "2.1.287")
                        | (TitleCli::Pi, "1.0.1")
                        | (TitleCli::Kimi, "2.1.1")
                        // Config, auth, provider and session schemas match
                        // 1.18.34 at 51ef4be1d3c122f18fefb510dca8d778571f4f18.
                        | (TitleCli::Opencode, "1.18.33")
                )
        })
        .map(str::to_owned)
}

#[cfg(test)]
pub(crate) fn prepare_with_project(
    root: &Path,
    cli: TitleCli,
    base: &str,
    token: &str,
    model: &str,
    cwd: &Path,
) -> Result<Launch, String> {
    let version = support(cli)
        .and_then(|s| s.native_version)
        .ok_or("Unqualified native version.")?;
    prepare_native(root, cli, base, token, model, cwd, version, false)
}

// Keep the reviewed launch inputs explicit at the native owner boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_native(
    root: &Path,
    cli: TitleCli,
    base: &str,
    token: &str,
    model: &str,
    cwd: &Path,
    version: &str,
    resume: bool,
) -> Result<Launch, String> {
    admit_project(cli, cwd)?;
    if admitted_version(cli, version.as_bytes()).is_none() && cli != TitleCli::Grok {
        return Err("This native CLI version has no reviewed profile schema.".into());
    }
    if resume && !can_resume(cli) {
        return Err("This CLI has no qualified native continuation.".into());
    }
    let cwd = cwd
        .canonicalize()
        .map_err(|_| "Cannot pin native gateway workspace.")?;
    prepare_inner(root, cli, base, token, model, Some(&cwd), version, resume)
}

/// A private, credential-free namespace used only for bounded --version admission.
pub(crate) fn prepare_probe(root: &Path) -> Result<Launch, String> {
    private_parent(root.parent().ok_or("Missing version inspection parent.")?)?;
    directory(root)?;
    let mut launch = Launch {
        private_home: root.into(),
        arguments: vec![],
        environment: vec![],
    };
    for key in [
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "TMPDIR",
        "CLAUDE_CONFIG_DIR",
        "CODEX_HOME",
        "GROK_HOME",
        "KIMI_CODE_HOME",
        "KIMI_SHARE_DIR",
        "PI_CODING_AGENT_DIR",
    ] {
        env(&mut launch, key, root.as_os_str());
    }
    env(&mut launch, "DISABLE_AUTOUPDATER", "1");
    env(&mut launch, "AGY_CLI_DISABLE_AUTO_UPDATE", "1");
    env(&mut launch, "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
    env(&mut launch, "KILO_NO_DAEMON", "1");
    env(&mut launch, "KILO_DISABLE_AUTOUPDATE", "1");
    env(&mut launch, "OPENCODE_DISABLE_AUTOUPDATE", "1");
    env(&mut launch, "KIMI_CODE_NO_AUTO_UPDATE", "1");
    env(&mut launch, "KIMI_CLI_NO_AUTO_UPDATE", "1");
    Ok(launch)
}

#[allow(clippy::too_many_arguments)]
fn prepare_inner(
    root: &Path,
    cli: TitleCli,
    local_base_url: &str,
    ephemeral_client_token: &str,
    native_model: &str,
    cwd: Option<&Path>,
    native_version: &str,
    resume: bool,
) -> Result<Launch, String> {
    if cli == TitleCli::Openclaw && cwd.is_none() {
        return Err("OpenClaw embedded TUI requires a pinned project workspace.".into());
    }
    support(cli).ok_or("This native client has no source-pinned gateway profile.")?;
    let base = loopback_base(local_base_url)?;
    if !(32..=256).contains(&ephemeral_client_token.len())
        || !ephemeral_client_token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || native_model.is_empty()
        || native_model.len() > 200
        || !native_model.as_bytes()[0].is_ascii_alphanumeric()
        || !native_model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
    {
        return Err("Invalid owned gateway token or exact model identifier.".into());
    }
    if cli == TitleCli::Codex {
        super::codex::model(Some(native_model))?;
        super::codex::ambient_policy()?;
    }
    if cli == TitleCli::Grok {
        // Do not allow system/cloud requirements to silently replace routing.
        super::grok::admission(root)?;
    }
    if cli == TitleCli::Opencode {
        super::opencode::admission(root)?;
    }
    if !root.is_absolute() || root.file_name().is_none() {
        return Err("Gateway profile storage must be an absolute new private directory.".into());
    }
    root.to_str()
        .ok_or("Native gateway profile paths must be UTF-8.")?;
    let parent = root
        .parent()
        .ok_or("Invalid private gateway profile parent.")?
        .canonicalize()
        .map_err(|_| "Cannot resolve private gateway profile parent.")?;
    private_parent(&parent)?;
    let root = parent.join(
        root.file_name()
            .ok_or("Invalid private gateway profile name.")?,
    );
    let io = ProfileIo {
        root: &root,
        cli,
        resume,
    };
    io.directory(&root)?;
    for name in ["config", "data", "state", "cache", "tmp", "sessions"] {
        io.directory(&root.join(name))?;
    }
    let mut launch = Launch {
        arguments: Vec::new(),
        environment: vec![
            ("HOME".into(), root.clone().into_os_string()),
            ("USERPROFILE".into(), root.clone().into_os_string()),
            (
                "XDG_CONFIG_HOME".into(),
                root.join("config").into_os_string(),
            ),
            ("XDG_DATA_HOME".into(), root.join("data").into_os_string()),
            ("XDG_STATE_HOME".into(), root.join("state").into_os_string()),
            ("XDG_CACHE_HOME".into(), root.join("cache").into_os_string()),
            ("TMPDIR".into(), root.join("tmp").into_os_string()),
            ("TMP".into(), root.join("tmp").into_os_string()),
            ("TEMP".into(), root.join("tmp").into_os_string()),
            ("NO_PROXY".into(), "127.0.0.1,localhost,::1".into()),
            ("no_proxy".into(), "127.0.0.1,localhost,::1".into()),
            ("OTEL_SDK_DISABLED".into(), "true".into()),
        ],
        private_home: root.clone(),
    };
    let api = format!("{base}/v1");
    match cli {
        TitleCli::Openclaw => {
            let workspace = cwd.ok_or("OpenClaw requires a pinned project workspace.")?;
            let config = root.join("openclaw.json");
            io.json_file(&config, &openclaw_profile(workspace, native_model, &api))?;
            env(&mut launch, "OPENCLAW_HOME", root.as_os_str());
            env(&mut launch, "OPENCLAW_STATE_DIR", root.as_os_str());
            env(&mut launch, "OPENCLAW_CONFIG_PATH", config.as_os_str());
            env(&mut launch, "OPENCLAW_WORKSPACE_DIR", workspace.as_os_str());
            env(&mut launch, "OPENAI_API_KEY", ephemeral_client_token);
            args(&mut launch, &["tui", "--local"]);
        }
        TitleCli::Qwen => {
            let home = root.join("qwen");
            io.directory(&home)?;
            let profile = qwen_profile(native_model, &api);
            io.json_file(&home.join("settings.json"), &profile)?;
            let settings = root.join("qwen-system-settings.json");
            let defaults = root.join("qwen-system-defaults.json");
            io.json_file(&settings, &profile)?;
            io.json_file(&defaults, &json!({}))?;
            env(&mut launch, "QWEN_HOME", home.as_os_str());
            env(
                &mut launch,
                "QWEN_RUNTIME_DIR",
                root.join("state").as_os_str(),
            );
            env(
                &mut launch,
                "QWEN_CODE_SYSTEM_SETTINGS_PATH",
                settings.as_os_str(),
            );
            env(
                &mut launch,
                "QWEN_CODE_SYSTEM_DEFAULTS_PATH",
                defaults.as_os_str(),
            );
            env(&mut launch, "OPENAI_API_KEY", ephemeral_client_token);
            env(&mut launch, "OPENAI_BASE_URL", &api);
            env(&mut launch, "OPENAI_MODEL", native_model);
            env(&mut launch, "QWEN_MODEL", native_model);
            env(&mut launch, "ENABLE_WEB_SEARCH", "false");
            args(
                &mut launch,
                &[
                    "--auth-type",
                    "openai",
                    "--openai-base-url",
                    &api,
                    "--model",
                    native_model,
                ],
            );
        }
        TitleCli::Claude => {
            let home = root.join("claude");
            io.directory(&home)?;
            // A fixed private helper reads the local token file: no real key,
            // inherited provider environment or shell interpolation of data.
            io.write(
                &home.join("gateway-token"),
                ephemeral_client_token.as_bytes(),
            )?;
            let helper = home.join("api-key-helper");
            io.write_executable(
                &helper,
                format!(
                    "#!/bin/sh\nexec /bin/cat {}\n",
                    shell_quote(&home.join("gateway-token"))
                )
                .as_bytes(),
            )?;
            io.json_file(
                &home.join("settings.json"),
                &json!({
                    "apiKeyHelper": shell_quote(&helper),
                    "env": {"ANTHROPIC_BASE_URL":base, "ANTHROPIC_API_BASE_URL":base,
                        "CLAUDE_AGENT_API_BASE_URL":base, "ANTHROPIC_MODEL":native_model,
                        "ANTHROPIC_DEFAULT_OPUS_MODEL":native_model, "ANTHROPIC_DEFAULT_SONNET_MODEL":native_model,
                        "ANTHROPIC_DEFAULT_HAIKU_MODEL":native_model, "ANTHROPIC_SMALL_FAST_MODEL":native_model}
                }),
            )?;
            env(&mut launch, "CLAUDE_CONFIG_DIR", home.as_os_str());
            for key in [
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_API_BASE_URL",
                "CLAUDE_AGENT_API_BASE_URL",
            ] {
                env(&mut launch, key, &base);
            }
            env(&mut launch, "ANTHROPIC_MODEL", native_model);
            env(&mut launch, "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
            env(&mut launch, "DISABLE_AUTOUPDATER", "1");
            args(&mut launch, &["--settings"]);
            launch
                .arguments
                .push(home.join("settings.json").into_os_string());
            args(
                &mut launch,
                &["--setting-sources", "user", "--model", native_model],
            );
        }
        TitleCli::Codex => {
            let home = root.join("codex");
            io.directory(&home)?;
            let mut catalog: Value =
                serde_json::from_str(include_str!("codex-models-0.160.0.json"))
                    .map_err(|_| "Cannot read the owned Codex catalog.")?;
            let entries = catalog["models"]
                .as_array_mut()
                .ok_or("Invalid owned Codex catalog.")?;
            entries.retain(|entry| entry["slug"].as_str() == Some(native_model));
            // The native gateway transports HTTP/SSE, not WebSockets.
            for entry in entries.iter_mut() {
                entry["prefer_websockets"] = json!(false);
            }
            if entries.len() != 1 {
                return Err(
                    "The exact gateway model is absent from the owned Codex catalog.".into(),
                );
            }
            io.json_file(&home.join("models.json"), &catalog)?;
            io.write(&home.join("config.toml"), format!(
                "model = {}\nmodel_provider = {}\nmodel_catalog_json = {}\ncli_auth_credentials_store = \"ephemeral\"\ncheck_for_update_on_startup = false\n[model_providers.{}]\nname = \"Lomi authenticated loopback gateway\"\nbase_url = {}\nexperimental_bearer_token = {}\nwire_api = \"responses\"\nsupports_websockets = false\n",
                quoted(native_model)?, quoted(PROVIDER)?, quoted_path(&home.join("models.json"))?, PROVIDER,
                quoted(&api)?, quoted(ephemeral_client_token)?).as_bytes())?;
            env(&mut launch, "CODEX_HOME", home.as_os_str());
            args(&mut launch, &["--no-daemon", "--model", native_model]);
        }
        TitleCli::Grok => {
            let home = root.join("grok");
            io.directory(&home)?;
            // The explicit native model block also pins synthesis/title model
            // destinations. Source-family admission is required by the driver.
            io.write(&home.join("config.toml"), format!(
                "[auth]\npreferred_method = \"api_key\"\n[cli]\nauto_update = false\n[models]\ndefault = {}\nallowed_models = [{}]\nweb_search = {}\nimage_description = {}\nsession_summary = {}\n[model.{}]\nmodel = {}\nbase_url = {}\nenv_key = \"XAI_API_KEY\"\napi_backend = \"responses\"\n",
                quoted(native_model)?, quoted(native_model)?, quoted(native_model)?, quoted(native_model)?, quoted(native_model)?,
                quoted(native_model)?, quoted(native_model)?, quoted(&api)?).as_bytes())?;
            env(&mut launch, "GROK_HOME", home.as_os_str());
            env(&mut launch, "GROK_MODELS_BASE_URL", &api);
            env(&mut launch, "GROK_MODELS_LIST_URL", format!("{api}/models"));
            env(&mut launch, "GROK_DEFAULT_MODEL", native_model);
            env(&mut launch, "XAI_API_KEY", ephemeral_client_token);
        }
        TitleCli::Kimi => {
            let home = root.join("kimi");
            io.directory(&home)?;
            if native_version == "2.1.1" {
                io.write(&home.join("config.toml"), format!(
                    "default_model = \"lomi-model\"\n[providers.{PROVIDER}]\ntype = \"openai\"\nmodel_source = \"static\"\nbase_url = {}\napi_key = {}\n[models.lomi-model]\nprovider = {}\nmodel = {}\nmax_context_size = 131072\ncapabilities = []\n[secondary_model]\nmodel = \"lomi-model\"\nforce = true\n",
                    quoted(&api)?, quoted(ephemeral_client_token)?, quoted(PROVIDER)?, quoted(native_model)?).as_bytes())?;
                env(&mut launch, "KIMI_CODE_HOME", home.as_os_str());
                args(&mut launch, &["--model", "lomi-model"]);
            } else {
                io.write(&home.join("config.toml"), format!(
                    "default_model = {}\n[providers.{}]\ntype = \"openai_legacy\"\nbase_url = {}\napi_key = {}\n[models.{}]\nprovider = {}\nmodel = {}\nmax_context_size = 131072\ncapabilities = []\n",
                    quoted(native_model)?, PROVIDER, quoted(&api)?, quoted(ephemeral_client_token)?, quoted(native_model)?,
                    quoted(PROVIDER)?, quoted(native_model)?).as_bytes())?;
                env(&mut launch, "KIMI_SHARE_DIR", home.as_os_str());
                args(&mut launch, &["--config-file"]);
                launch
                    .arguments
                    .push(home.join("config.toml").into_os_string());
                args(&mut launch, &["--model", native_model]);
            }
            env(&mut launch, "OPENAI_BASE_URL", &api);
            env(&mut launch, "OPENAI_API_KEY", ephemeral_client_token);
            env(&mut launch, "KIMI_CODE_NO_AUTO_UPDATE", "1");
            env(&mut launch, "KIMI_CLI_NO_AUTO_UPDATE", "1");
        }
        TitleCli::Agy => {
            let gemini = root.join(".gemini");
            io.directory(&gemini)?;
            let home = gemini.join("antigravity-cli");
            io.directory(&home)?;
            io.json_file(
                &home.join("settings.json"),
                &json!({
                    "modelProvider":"gemini", "enableTelemetry":false, "useG1Credits":false
                }),
            )?;
            env(&mut launch, "GEMINI_API_KEY", ephemeral_client_token);
            env(&mut launch, "GOOGLE_GEMINI_BASE_URL", &base);
            env(&mut launch, "AGY_CLI_DISABLE_AUTO_UPDATE", "1");
            args(&mut launch, &["--model", native_model]);
        }
        TitleCli::Pi => {
            let home = root.join("pi");
            io.directory(&home)?;
            io.directory(&home.join("sessions"))?;
            io.json_file(
                &home.join("models.json"),
                &json!({"providers":{(PROVIDER):{
                "api":"openai-responses","apiKey":ephemeral_client_token,"authHeader":true,
                "baseUrl":api,"models":[{"id":native_model,"name":native_model}]}}}),
            )?;
            if !resume {
                io.json_file(&home.join("auth.json"), &json!({}))?;
            }
            env(&mut launch, "PI_CODING_AGENT_DIR", home.as_os_str());
            env(
                &mut launch,
                "PI_CODING_AGENT_SESSION_DIR",
                home.join("sessions").as_os_str(),
            );
            env(&mut launch, "PI_SKIP_VERSION_CHECK", "1");
            args(
                &mut launch,
                &[
                    "--provider",
                    PROVIDER,
                    "--model",
                    native_model,
                    "--api-key",
                    ephemeral_client_token,
                ],
            );
        }
        TitleCli::Opencode | TitleCli::Kilo => {
            if cli == TitleCli::Kilo {
                env(&mut launch, "KILO_NO_DAEMON", "1");
            }
            let name = if cli == TitleCli::Kilo {
                "kilo"
            } else {
                "opencode"
            };
            let home = root.join("config").join(name);
            io.directory(&home)?;
            let reference = format!("{PROVIDER}/{native_model}");
            let content = opencode_profile(&reference, native_model, &api, ephemeral_client_token);
            let config = home.join(format!("{name}.json"));
            io.json_file(&config, &content)?;
            let prefix = if cli == TitleCli::Kilo {
                "KILO"
            } else {
                "OPENCODE"
            };
            env(&mut launch, &format!("{prefix}_DISABLE_AUTOUPDATE"), "1");
            env(&mut launch, &format!("{prefix}_CONFIG"), config.as_os_str());
            env(
                &mut launch,
                &format!("{prefix}_CONFIG_CONTENT"),
                serde_json::to_string(&content).map_err(|_| "Cannot encode gateway provider.")?,
            );
        }
        TitleCli::Hermes => {
            let home = root.join(".hermes");
            io.directory(&home)?;
            io.json_file(
                &home.join("config.yaml"),
                &json!({"_config_version":46,
                "model":{"provider":"anthropic","default":native_model,"base_url":base,"api_mode":"anthropic_messages"},
                "fallback_providers":[],"providers":{},"security":{"allow_lazy_installs":false},
                "auxiliary":{"title_generation":{"enabled":false,"model_upgrade_enabled":false},"background_review":{"enabled":false}},
                "memory":{"memory_enabled":false,"user_profile_enabled":false,"provider":""},
                "curator":{"enabled":false,"consolidate":false},"model_catalog":{"enabled":false},"updates":{"check":false}}),
            )?;
            env(&mut launch, "HERMES_HOME", home.as_os_str());
            env(&mut launch, "HERMES_REAL_HOME", root.as_os_str());
            env(&mut launch, "HERMES_DISABLE_LAZY_INSTALLS", "1");
            env(&mut launch, "ANTHROPIC_API_KEY", ephemeral_client_token);
            args(
                &mut launch,
                &["--provider", "anthropic", "--model", native_model],
            );
        }
        TitleCli::Vibe => {
            let home = root.join(".vibe");
            io.directory(&home)?;
            io.write(&home.join("config.toml"), format!(
                "active_model = \"lomi\"\nallowed_models = [\"lomi\"]\nenable_telemetry = false\nenable_otel = false\nenable_update_checks = false\nenable_auto_update = false\nvoice_mode_enabled = false\nnarrator_enabled = false\n[session_logging]\ngenerate_titles = false\n[[providers]]\nname = \"lomi-gateway\"\napi_base = {}\napi_key_env_var = \"OPENAI_API_KEY\"\napi_style = \"openai\"\nbackend = \"generic\"\nemits_finish_reason = true\n[[models]]\nname = {}\nalias = \"lomi\"\nprovider = \"lomi-gateway\"\n",
                quoted(&api)?, quoted(native_model)?).as_bytes())?;
            env(&mut launch, "VIBE_HOME", home.as_os_str());
            env(&mut launch, "VIBE_TEST_DISABLE_KEYRING", "1");
            env(&mut launch, "OPENAI_API_KEY", ephemeral_client_token);
        }
        _ => return Err("This native client has no gateway profile writer.".into()),
    }
    File::open(&root)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Cannot persist private gateway profile directory.")?;
    Ok(launch)
}

// Source-pinned embedded runtime uses the configured workspace, not argv cwd.
// Replace mode bypasses implicit provider discovery. Fresh HOME owns auth and
// generated model stores. Core tools retain their native permission policy;
// optional plugin services cannot import a second inference/embedding route.
fn openclaw_profile(workspace: &Path, model: &str, api: &str) -> Value {
    let reference = format!("{PROVIDER}/{model}");
    let selected = json!({"primary":reference,"fallbacks":[]});
    json!({"env":{"shellEnv":{"enabled":false}},"plugins":{"enabled":false},
        "models":{"mode":"replace","providers":{(PROVIDER):{"baseUrl":api,"api":"openai-responses","apiKey":"${OPENAI_API_KEY}","models":[{"id":model,"name":model,"reasoning":false,"input":["text"],"contextWindow":128000,"maxTokens":8192,"agentRuntime":{"id":"openclaw"}}]}}},
        "agents":{"defaults":{"workspace":workspace,"cwd":workspace,"model":selected,"utilityModel":reference,"decisionModel":"","imageModel":selected,"pdfModel":selected,
            "models":{(reference.clone()):{"agentRuntime":{"id":"openclaw"}}},"modelPolicy":{"allow":[reference]},
            "subagents":{"model":selected},"compaction":{"model":reference,"postIndexSync":"off","memoryFlush":{"enabled":false}},
            "heartbeat":{"every":"0m","model":reference}},"entries":{"main":{"id":"main","workspace":workspace,"runtime":{"type":"embedded"}}}}})
}

// Qwen settingsSchema.ts modelProviders uses REPLACE, not merge. The owned
// system scope outranks workspace settings; helper routes cannot pick a second
// provider/model. No upstream key or OAuth credential is present in fresh HOME.
fn qwen_profile(model: &str, api: &str) -> Value {
    json!({"security":{"auth":{"selectedType":"openai"}},"model":{"name":model},
        "modelProviders":{"openai":[{"id":model,"envKey":"OPENAI_API_KEY","baseUrl":api,"wireApi":"chat-completions","generationConfig":{"maxRetries":0}}]},
        "fastModel":model,"compactionModel":model,"visionModel":model,"advisorModel":"",
        "tools":{"webSearch":{"enabled":false}},"telemetry":{"enabled":false}})
}

fn loopback_base(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid local gateway URL.")?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || url.port().is_none_or(|port| port == 0)
    {
        return Err("Gateway profiles require an explicit authenticated HTTP loopback port with no URL credentials or path overrides.".into());
    }
    Ok(value.trim_end_matches('/').to_owned())
}
fn env(launch: &mut Launch, name: &str, value: impl Into<OsString>) {
    launch.environment.push((name.into(), value.into()));
}
fn args(launch: &mut Launch, values: &[&str]) {
    launch
        .arguments
        .extend(values.iter().map(|value| OsString::from(*value)));
}
fn quoted(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| "Cannot encode owned native configuration.".into())
}
fn quoted_path(path: &Path) -> Result<String, String> {
    quoted(
        path.to_str()
            .ok_or("Native configuration paths must be UTF-8.")?,
    )
}
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
/// Resume rotates only router-generated endpoint material. Native credentials,
/// conversations, settings outside this allow-list and tool history are retained.
struct ProfileIo<'a> {
    root: &'a Path,
    cli: TitleCli,
    resume: bool,
}
impl ProfileIo<'_> {
    fn directory(&self, path: &Path) -> Result<(), String> {
        if self.resume {
            private_parent(path)
        } else {
            directory(path)
        }
    }
    fn json_file(&self, path: &Path, value: &Value) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|_| "Cannot encode native gateway configuration.")?;
        self.write(path, &bytes)
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.write_mode(path, bytes, false)
    }
    fn write_executable(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.write_mode(path, bytes, true)
    }
    fn write_mode(&self, path: &Path, bytes: &[u8], executable: bool) -> Result<(), String> {
        if !self.resume {
            return if executable {
                write_executable(path, bytes)
            } else {
                write(path, bytes)
            };
        }
        let relative = path
            .strip_prefix(self.root)
            .map_err(|_| "Native configuration escaped its private home.")?;
        let allowed: &[&str] = match self.cli {
            TitleCli::Claude => &[
                "claude/gateway-token",
                "claude/api-key-helper",
                "claude/settings.json",
            ],
            TitleCli::Codex => &["codex/config.toml", "codex/models.json"],
            TitleCli::Grok => &["grok/config.toml"],
            TitleCli::Kimi => &["kimi/config.toml"],
            TitleCli::Pi => &["pi/models.json"],
            TitleCli::Opencode => &["config/opencode/opencode.json"],
            TitleCli::Kilo => &["config/kilo/kilo.json"],
            TitleCli::Agy => &[".gemini/antigravity-cli/settings.json"],
            _ => &[],
        };
        if !allowed.iter().any(|name| relative == Path::new(name)) {
            return Err("Native resume attempted to replace account or conversation state.".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
            let parent = path
                .parent()
                .ok_or("Missing native configuration parent.")?;
            private_parent(parent)?;
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| "Native routing configuration is missing.")?;
            let mode = if executable { 0o700 } else { 0o600 };
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
                || metadata.mode() & 0o777 != mode
                || metadata.len() > 2 * 1024 * 1024
            {
                return Err("Native routing configuration is no longer privately owned.".into());
            }
            let temporary = parent.join(format!(".lomi-routing-{}", super::new_id()?));
            let result = (|| -> Result<(), String> {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&temporary)
                    .map_err(|_| "Cannot stage native routing configuration.")?;
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "Cannot persist native routing configuration.")?;
                let current = fs::symlink_metadata(path)
                    .map_err(|_| "Native routing configuration changed.")?;
                if current.dev() != metadata.dev()
                    || current.ino() != metadata.ino()
                    || current.len() != metadata.len()
                    || current.mtime() != metadata.mtime()
                    || current.mtime_nsec() != metadata.mtime_nsec()
                    || current.mode() != metadata.mode()
                    || current.uid() != metadata.uid()
                    || current.nlink() != 1
                {
                    return Err("Native routing configuration changed during rotation.".into());
                }
                fs::rename(&temporary, path)
                    .map_err(|_| "Cannot rotate native routing configuration.")?;
                File::open(parent)
                    .and_then(|f| f.sync_all())
                    .map_err(|_| "Cannot persist native routing directory.".to_string())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result
        }
        #[cfg(not(unix))]
        {
            Err("Native gateway resume requires Unix private-file ownership.".into())
        }
    }
}
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Cannot create a fresh private gateway configuration file.")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Cannot persist private gateway configuration.")?;
    File::open(path.parent().ok_or("Invalid gateway file parent.")?)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| "Cannot persist gateway configuration parent.")?;
    Ok(())
}
fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot restrict the native gateway helper.")?;
    }
    #[cfg(not(unix))]
    return Err("Native gateway helpers require qualified Unix permissions.".into());
    #[cfg(unix)]
    Ok(())
}
fn directory(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| {
        "Gateway profiles require a fresh owned directory; existing state cannot be reused.".into()
    })
}
fn private_parent(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Cannot inspect private gateway storage.")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Invalid private gateway storage.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o700 {
            return Err(
                "Gateway storage parent must belong to the current native owner with mode 0700."
                    .into(),
            );
        }
    }
    #[cfg(not(unix))]
    return Err("Private native gateway profiles are unqualified on this platform.".into());
    #[cfg(unix)]
    Ok(())
}
#[cfg(unix)]
pub(crate) fn check_private_directory(path: &Path) -> Result<(), String> {
    private_parent(path)
}
fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err("Native gateway profile refuses higher shared provider configuration.".into()),
    }
}

// Released CCR OpenCode/Kilo profile writers use these explicit limits. Native
// provider defaults of zero would suppress compaction for our private model.
fn opencode_profile(reference: &str, model: &str, api: &str, token: &str) -> Value {
    json!({"model":reference,"small_model":reference,"autoupdate":false,
        "enabled_providers":[PROVIDER],"provider":{(PROVIDER):{"name":"Lomi gateway",
        "npm":"@ai-sdk/openai-compatible","models":{(model):{"name":model,"limit":{"context":128000,"output":8192}}},
        "options":{"apiKey":token,"baseURL":api}}}})
}

#[cfg(test)]
mod gateway_profile_fixtures {
    use super::*;

    #[test]
    fn current_native_families_use_distinct_reviewed_profiles() {
        assert_eq!(
            admitted_version(TitleCli::Kimi, b"kimi 2.1.1\n"),
            Some("2.1.1".into())
        );
        assert_eq!(
            admitted_version(TitleCli::Pi, b"1.0.1\n"),
            Some("1.0.1".into())
        );
        assert_eq!(
            admitted_version(TitleCli::Claude, b"2.1.287 (Claude Code)\n"),
            Some("2.1.287".into())
        );
        assert!(admitted_version(TitleCli::Kimi, b"2.1.2\n").is_none());
        assert_eq!(
            admitted_version(TitleCli::Opencode, b"1.18.33\n"),
            Some("1.18.33".into())
        );
        assert_eq!(
            admitted_version(TitleCli::Opencode, b"1.18.34\n"),
            Some("1.18.34".into())
        );
        assert!(admitted_version(TitleCli::Opencode, b"1.18.35\n").is_none());
        let storage = tempfile::tempdir().unwrap();
        crate::chat::storage::private(storage.path(), true).unwrap();
        let token = "a".repeat(64);
        let modern = prepare_inner(
            &storage.path().join("modern"),
            TitleCli::Kimi,
            "http://127.0.0.1:12345",
            &token,
            "kimi-native",
            Some(storage.path()),
            "2.1.1",
            false,
        )
        .unwrap();
        assert!(!modern.arguments.iter().any(|v| v == "--config-file"));
        assert!(modern
            .environment
            .iter()
            .any(|(k, _)| k == "KIMI_CODE_HOME"));
        let config = fs::read_to_string(modern.private_home.join("kimi/config.toml")).unwrap();
        assert!(config.contains("type = \"openai\"") && config.contains("force = true"));
        let legacy = prepare_with_project(
            &storage.path().join("legacy"),
            TitleCli::Kimi,
            "http://127.0.0.1:12345",
            &token,
            "kimi-native",
            storage.path(),
        )
        .unwrap();
        assert!(legacy.arguments.iter().any(|v| v == "--config-file"));
        assert!(
            fs::read_to_string(legacy.private_home.join("kimi/config.toml"))
                .unwrap()
                .contains("openai_legacy")
        );
    }

    #[test]
    fn resume_rotates_only_endpoint_material_and_preserves_native_pi_history_and_auth() {
        let storage = tempfile::tempdir().unwrap();
        crate::chat::storage::private(storage.path(), true).unwrap();
        let root = storage.path().join("home");
        let first = prepare_inner(
            &root,
            TitleCli::Pi,
            "http://127.0.0.1:12345",
            &"a".repeat(64),
            "chosen-model",
            Some(storage.path()),
            "1.0.1",
            false,
        )
        .unwrap();
        fs::write(
            first.private_home.join("pi/auth.json"),
            b"native-auth-retained",
        )
        .unwrap();
        fs::write(
            first.private_home.join("pi/sessions/owned.jsonl"),
            b"native-history-retained",
        )
        .unwrap();
        let next = prepare_inner(
            &root,
            TitleCli::Pi,
            "http://127.0.0.1:23456",
            &"b".repeat(64),
            "chosen-model",
            Some(storage.path()),
            "1.0.1",
            true,
        )
        .unwrap();
        assert_eq!(first.private_home, next.private_home);
        assert_eq!(
            fs::read(root.join("pi/auth.json")).unwrap(),
            b"native-auth-retained"
        );
        assert_eq!(
            fs::read(root.join("pi/sessions/owned.jsonl")).unwrap(),
            b"native-history-retained"
        );
        let config: Value =
            serde_json::from_slice(&fs::read(root.join("pi/models.json")).unwrap()).unwrap();
        assert_eq!(
            config["providers"][PROVIDER]["baseUrl"],
            "http://127.0.0.1:23456/v1"
        );
        assert_eq!(config["providers"][PROVIDER]["apiKey"], "b".repeat(64));
        assert!(next.arguments.iter().any(|v| v == "--api-key"));
        assert!(ProfileIo {
            root: &root,
            cli: TitleCli::Pi,
            resume: true
        }
        .write(&root.join("pi/auth.json"), b"replace")
        .is_err());
    }

    #[test]
    fn native_daemon_ownership_and_antigravity_api_provider_are_explicit() {
        let storage = tempfile::tempdir().unwrap();
        crate::chat::storage::private(storage.path(), true).unwrap();
        let token = "a".repeat(64);
        let kilo = prepare_inner(
            &storage.path().join("kilo-home"),
            TitleCli::Kilo,
            "http://127.0.0.1:12345",
            &token,
            "native-model",
            Some(storage.path()),
            "7.8.3",
            false,
        )
        .unwrap();
        assert!(kilo
            .environment
            .iter()
            .any(|(k, v)| k == "KILO_NO_DAEMON" && v == "1"));
        let opencode = prepare_inner(
            &storage.path().join("opencode-home"),
            TitleCli::Opencode,
            "http://127.0.0.1:12345",
            &token,
            "native-model",
            Some(storage.path()),
            "1.18.33",
            false,
        )
        .unwrap();
        let probe = prepare_probe(&storage.path().join("probe")).unwrap();
        for (launch, prefix) in [(&kilo, "KILO"), (&opencode, "OPENCODE")] {
            let updater = format!("{prefix}_DISABLE_AUTOUPDATE");
            for environment in [&launch.environment, &probe.environment] {
                assert!(environment
                    .iter()
                    .any(|(key, value)| key == updater.as_str() && value == "1"));
            }
            let config = launch
                .environment
                .iter()
                .find(|(key, _)| key == format!("{prefix}_CONFIG_CONTENT").as_str())
                .unwrap();
            let config: Value = serde_json::from_str(config.1.to_str().unwrap()).unwrap();
            assert_eq!(config["autoupdate"], false);
        }
        let agy = prepare_inner(
            &storage.path().join("agy-home"),
            TitleCli::Agy,
            "http://127.0.0.1:12345",
            &token,
            "gemini-model",
            Some(storage.path()),
            "1.2.16",
            false,
        )
        .unwrap();
        let config: Value = serde_json::from_slice(
            &fs::read(
                agy.private_home
                    .join(".gemini/antigravity-cli/settings.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(config["modelProvider"], "gemini");
        assert!(agy
            .environment
            .iter()
            .any(|(k, v)| k == "GEMINI_API_KEY" && v == token.as_str()));
        assert!(agy
            .environment
            .iter()
            .any(|(k, v)| k == "GOOGLE_GEMINI_BASE_URL" && v == "http://127.0.0.1:12345"));
        assert!(!agy.environment.iter().any(|(k, _)| k == "GOOGLE_API_KEY"));
        let model = super::super::codex::model_choices()[0].0.clone();
        let codex = prepare_inner(
            &storage.path().join("codex-home"),
            TitleCli::Codex,
            "http://127.0.0.1:12345",
            &token,
            &model,
            Some(storage.path()),
            "0.160.0",
            false,
        )
        .unwrap();
        assert!(codex.arguments.iter().any(|v| v == "--no-daemon"));
    }

    #[test]
    fn native_compaction_limits_and_aux_model_are_preserved() {
        let profile = opencode_profile(
            "lomi-gateway/native-model",
            "native-model",
            "http://127.0.0.1:12345/v1",
            "local-token",
        );
        assert_eq!(profile["model"], profile["small_model"]);
        assert_eq!(
            profile["provider"][PROVIDER]["models"]["native-model"]["limit"]["context"],
            128000
        );
        assert_eq!(
            profile["provider"][PROVIDER]["models"]["native-model"]["limit"]["output"],
            8192
        );
        assert_eq!(profile["enabled_providers"], json!([PROVIDER]));
    }

    #[test]
    fn embedded_openclaw_owns_workspace_and_allows_only_one_native_model() {
        let profile = openclaw_profile(
            Path::new("/owned/project"),
            "chosen-model",
            "http://127.0.0.1:12345/v1",
        );
        assert_eq!(profile["agents"]["defaults"]["workspace"], "/owned/project");
        assert_eq!(profile["models"]["mode"], "replace");
        assert_eq!(profile["models"]["providers"].as_object().unwrap().len(), 1);
        let defaults = &profile["agents"]["defaults"];
        for role in ["utilityModel", "compaction"] {
            let value = if role == "compaction" {
                &defaults[role]["model"]
            } else {
                &defaults[role]
            };
            assert_eq!(value, &defaults["model"]["primary"]);
        }
        assert_eq!(defaults["model"]["fallbacks"], json!([]));
        assert_eq!(
            defaults["modelPolicy"]["allow"],
            json!([defaults["model"]["primary"]])
        );
        assert!(profile.get("tools").is_none());
    }

    #[test]
    fn native_gateway_auxiliary_models_share_the_only_provider() {
        let qwen = qwen_profile("chosen-model", "http://127.0.0.1:12345/v1");
        for role in ["fastModel", "compactionModel", "visionModel"] {
            assert_eq!(qwen[role], qwen["model"]["name"]);
        }
        assert_eq!(
            qwen["modelProviders"]["openai"][0]["wireApi"],
            "chat-completions"
        );
        assert_eq!(qwen["modelProviders"].as_object().unwrap().len(), 1);
        assert_eq!(qwen["tools"]["webSearch"]["enabled"], false);
    }

    #[test]
    fn native_config_discovery_cannot_replace_owned_gateway_routing() {
        for (cli, name) in [
            (TitleCli::Qwen, ".qwen/settings.json"),
            (TitleCli::Openclaw, ".env"),
        ] {
            let project = tempfile::tempdir().unwrap();
            let candidate = project.path().join(name);
            fs::create_dir_all(candidate.parent().unwrap()).unwrap();
            fs::write(&candidate, "not parsed by admission").unwrap();
            let error = admit_project(cli, project.path()).unwrap_err();
            assert!(error.contains(name));
        }
    }

    #[test]
    fn project_override_is_rejected_without_reading_its_contents() {
        let project = tempfile::tempdir().unwrap();
        let child = project.path().join("child");
        fs::create_dir(&child).unwrap();
        fs::create_dir(project.path().join(".qwen")).unwrap();
        fs::write(
            project.path().join(".qwen/settings.json"),
            "not parsed by admission",
        )
        .unwrap();
        let error = admit_project(TitleCli::Qwen, &child).unwrap_err();
        assert!(error.contains(".qwen/settings.json"));
        assert!(admit_project(TitleCli::Qwen, Path::new("relative-project")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn project_alias_cannot_bypass_dotenv_admission() {
        let storage = tempfile::tempdir().unwrap();
        let project = storage.path().join("project");
        fs::create_dir(&project).unwrap();
        fs::write(project.join(".env"), "not parsed by admission").unwrap();
        let alias = storage.path().join("alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        for cli in [TitleCli::Qwen, TitleCli::Openclaw] {
            assert!(admit_project(cli, &alias).unwrap_err().contains(".env"));
        }
    }

    #[test]
    fn protocol_wire_names_do_not_depend_on_rust_casing() {
        assert_eq!(
            serde_json::to_value(Protocol::OpenAiChat).unwrap(),
            "openai_chat"
        );
        assert_eq!(
            serde_json::to_value(Protocol::OpenAiResponses).unwrap(),
            "openai_responses"
        );
    }
}
