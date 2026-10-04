//! Source-qualified OpenCode 1.18.34, aec0b9a6d8898f68f923aaf08b7306d931fd9d76.
//! Fresh OpenAI API text turns only. The owner must clear the environment,
//! inject only its selected OPENAI_API_KEY, send the prompt on stdin, and require
//! successful process exit, EOF and complete input before committing output.
//! Source: anomalyco/opencode at the above commit: packages/opencode/src/
//! {config/{config,managed,paths},plugin/index,session/{prompt,processor,tools},
//! session/llm/request,permission/index,cli/cmd/run}.ts; packages/core/src/
//! {global,git,npm,models-dev}.ts; packages/schema/src/v1/session.ts.
//! --pure and disable-default-plugins jointly suppress external/internal hooks.
//! Startup's mandatory npm install is short-circuited by owned lockfile data.
//! macOS managed preferences and ancestor Git repositories are refused rather
//! than read. This is conditional text-mode admission, not an OS sandbox.

use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const VERSION: &str = "1.18.34";
const MAX_OUTPUT: usize = 1024 * 1024;
const AGENT: &str = "lomi-text";
const MODELS: &[&str] = &[
    "gpt-4.1",
    "gpt-4.1-mini",
    "gpt-4.1-nano",
    "gpt-4o",
    "gpt-4o-mini",
];
const ENVIRONMENT: &[(&str, &str)] = &[
    ("OPENCODE_AUTH_CONTENT", "{}"),
    ("OPENCODE_PURE", "true"),
    ("OPENCODE_DISABLE_DEFAULT_PLUGINS", "true"),
    ("OPENCODE_DISABLE_PROJECT_CONFIG", "true"),
    ("OPENCODE_DISABLE_CLAUDE_CODE", "true"),
    ("OPENCODE_DISABLE_EXTERNAL_SKILLS", "true"),
    ("OPENCODE_DISABLE_AUTOUPDATE", "true"),
    ("OPENCODE_DISABLE_MODELS_FETCH", "true"),
    ("OPENCODE_DISABLE_AUTOCOMPACT", "true"),
    ("OPENCODE_DISABLE_PRUNE", "true"),
    ("OPENCODE_DISABLE_LSP_DOWNLOAD", "true"),
    ("OPENCODE_DISABLE_TERMINAL_TITLE", "true"),
    ("OPENCODE_EXPERIMENTAL_DISABLE_FILEWATCHER", "true"),
    ("OPENCODE_DISABLE_FFF", "true"),
    ("OPENCODE_PERMISSION", "{\"*\":\"deny\"}"),
    ("OTEL_SDK_DISABLED", "true"),
];

pub(crate) fn supported_models() -> &'static [&'static str] {
    MODELS
}

pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    value.filter(|id| MODELS.contains(id)).ok_or_else(|| {
        "OpenCode requires an exact approved OpenAI text model. No request was sent.".into()
    })
}

fn config(id: &str) -> Value {
    json!({
        "$schema": "https://opencode.ai/config.json",
        "model": format!("openai/{id}"), "small_model": format!("openai/{id}"),
        "default_agent": AGENT, "enabled_providers": ["openai"],
        "permission": {"*": "deny"}, "plugin": [], "mcp": {}, "instructions": [],
        "snapshot": false, "share": "disabled", "lsp": false, "formatter": false,
        "compaction": {"auto": false, "prune": false},
        "agent": {
            (AGENT): {"mode": "primary", "model": format!("openai/{id}"), "steps": 1,
                "permission": {"*": "deny"},
                "prompt": "Answer using only the supplied conversation. Tools and project context are unavailable. Do not claim to inspect or modify files."},
            "build": {"disable": true}, "plan": {"disable": true},
            "general": {"disable": true}, "explore": {"disable": true},
            "title": {"disable": true}, "summary": {"disable": true}, "compaction": {"disable": true}
        },
        "provider": {"openai": {"npm": "@ai-sdk/openai", "env": ["OPENAI_API_KEY"],
            "whitelist": [id], "options": {"apiKey": "{env:OPENAI_API_KEY}", "baseURL": "https://api.openai.com/v1"},
            "models": {(id): {"id": id, "name": id, "reasoning": false, "attachment": false,
                "tool_call": false, "modalities": {"input": ["text"], "output": ["text"]},
                "limit": {"context": 128000, "output": 16384}, "options": {"store": false}}}
        }}
    })
}

fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err("OpenCode text mode refuses shared managed preferences or ancestor Git state. No request was sent.".into()),
        Err(_) => Err("Cannot establish OpenCode text-mode isolation. No request was sent.".into()),
    }
}

/// ConfigManaged has no switch for macOS MDM preferences. Refuse even the
/// preferences directory, including links, without inspecting its contents.
pub(super) fn admission(root: &Path) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        absent(Path::new("/Library/Managed Preferences"))?;
    }
    // core/Git.repo.discover walks upward independently of project config.
    for ancestor in root.ancestors() {
        absent(&ancestor.join(".git"))?;
    }
    Ok(())
}

pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
    let id = model(Some(id))?;
    let root = parent.join(format!("opencode-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| "Cannot create fresh OpenCode attempt storage.")?;
    crate::chat::storage::private(&root, true)?;
    let root = root
        .canonicalize()
        .map_err(|_| "Cannot resolve private OpenCode storage.")?;
    admission(&root)?;
    for name in ["config", "data", "state", "cache", "tmp", "work", "managed"] {
        let directory = root.join(name);
        fs::create_dir(&directory).map_err(|_| "Cannot isolate OpenCode storage.")?;
        crate::chat::storage::private(&directory, true)?;
    }
    let directory = root.join("config/opencode");
    fs::create_dir(&directory).map_err(|_| "Cannot isolate OpenCode configuration.")?;
    crate::chat::storage::private(&directory, true)?;
    let modules = directory.join("node_modules");
    fs::create_dir(&modules).map_err(|_| "Cannot suppress OpenCode dependency installation.")?;
    crate::chat::storage::private(&modules, true)?;
    // Npm.install checks only node_modules existence and root lockfile name
    // membership before returning. No plugin package is installed or imported.
    crate::chat::storage::atomic(&directory.join("package.json"), b"{}\n")?;
    crate::chat::storage::atomic(&directory.join("package-lock.json"),
        b"{\"lockfileVersion\":3,\"packages\":{\"\":{\"dependencies\":{\"@opencode-ai/plugin\":\"1.18.34\"}}}}\n")?;
    crate::chat::storage::atomic(
        &directory.join("opencode.json"),
        &serde_json::to_vec(&config(id)).map_err(|_| "Cannot build OpenCode restrictions.")?,
    )?;
    Ok(root)
}

pub(crate) fn environment(command: &mut Command, root: &Path) {
    command
        .current_dir(root.join("work"))
        .env("HOME", root)
        .env("OPENCODE_TEST_HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        .env("OPENCODE_TEST_MANAGED_CONFIG_DIR", root.join("managed"))
        .env("OPENCODE_DB", root.join("data/opencode.db"));
    for (name, value) in ENVIRONMENT {
        command.env(name, value);
    }
    // Do not set OPENCODE_CONFIG_DIR: it would add a second config directory
    // whose mandatory dependency install would also need owned lockfile data.
}

pub(crate) fn arguments(command: &mut Command, root: &Path, id: &str) {
    environment(command, root);
    command
        .args(["run", "--format", "json", "--model"])
        .arg(format!("openai/{id}"))
        .args(["--agent", AGENT, "--title", "Lomi text turn"]);
}

pub(crate) fn version_matches(output: &str) -> bool {
    output.trim() == VERSION
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    Malformed,
    Failed,
    Effect,
}

#[derive(Default)]
pub(crate) struct Progress {
    session: Option<String>,
    message: Option<String>,
    ids: HashSet<String>,
    started: bool,
    terminal: bool,
    rejected: bool,
    pub(crate) output: String,
}

fn keys(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
}
fn identifier(value: &Value) -> Option<&str> {
    value.as_str().filter(|value| {
        !value.is_empty()
            && value.len() <= 200
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    })
}
fn number(value: &Value) -> bool {
    value
        .as_f64()
        .is_some_and(|value| value.is_finite() && value >= 0.0)
}

impl Progress {
    pub(crate) fn new(_model: &str) -> Self {
        Self::default()
    }

    pub(crate) fn event(&mut self, value: &Value) -> Result<(Option<String>, bool), Rejection> {
        let result = self.accept(value);
        if result.is_err() {
            self.rejected = true;
            self.terminal = false;
        }
        result
    }

    fn accept(&mut self, value: &Value) -> Result<(Option<String>, bool), Rejection> {
        if self.rejected || self.terminal || self.ids.len() >= 1024 {
            return Err(Rejection::Malformed);
        }
        match value["type"].as_str() {
            Some("error") => return Err(Rejection::Failed),
            Some("tool_use" | "reasoning") => return Err(Rejection::Effect),
            Some("step_start" | "text" | "step_finish") => {}
            _ => return Err(Rejection::Malformed),
        }
        if !keys(value, &["type", "timestamp", "sessionID", "part"])
            || value["timestamp"].as_u64().is_none()
        {
            return Err(Rejection::Malformed);
        }
        let session = identifier(&value["sessionID"]).ok_or(Rejection::Malformed)?;
        let part = &value["part"];
        if part["sessionID"] != value["sessionID"] {
            return Err(Rejection::Malformed);
        }
        let message = identifier(&part["messageID"]).ok_or(Rejection::Malformed)?;
        let id = identifier(&part["id"]).ok_or(Rejection::Malformed)?;
        if self.session.as_deref().is_some_and(|old| old != session)
            || self.message.as_deref().is_some_and(|old| old != message)
            || !self.ids.insert(id.into())
        {
            return Err(Rejection::Malformed);
        }
        self.session = Some(session.into());
        self.message = Some(message.into());
        match value["type"].as_str() {
            Some("step_start") => {
                if self.started
                    || part["type"] != "step-start"
                    || !keys(part, &["id", "sessionID", "messageID", "type", "snapshot"])
                {
                    return Err(Rejection::Malformed);
                }
                if part.get("snapshot").is_some() {
                    return Err(Rejection::Effect);
                }
                self.started = true;
                Ok((None, false))
            }
            Some("text") => {
                if !self.started
                    || part["type"] != "text"
                    || !keys(
                        part,
                        &[
                            "id",
                            "sessionID",
                            "messageID",
                            "type",
                            "text",
                            "time",
                            "metadata",
                            "synthetic",
                            "ignored",
                        ],
                    )
                    || ["synthetic", "ignored"]
                        .iter()
                        .any(|key| part.get(*key).is_some_and(|flag| flag != false))
                {
                    return Err(Rejection::Malformed);
                }
                let time = &part["time"];
                let start = time["start"].as_u64().ok_or(Rejection::Malformed)?;
                let end = time["end"].as_u64().ok_or(Rejection::Malformed)?;
                if !keys(time, &["start", "end"]) || end < start {
                    return Err(Rejection::Malformed);
                }
                if let Some(meta) = part.get("metadata") {
                    // OpenAI Responses text metadata is an item identifier only.
                    if !keys(meta, &["openai"]) {
                        return Err(Rejection::Malformed);
                    }
                    if let Some(openai) = meta.get("openai") {
                        if !keys(openai, &["itemId"]) || identifier(&openai["itemId"]).is_none() {
                            return Err(Rejection::Malformed);
                        }
                    }
                }
                let text = part["text"].as_str().ok_or(Rejection::Malformed)?;
                if self.output.len().saturating_add(text.len()) > MAX_OUTPUT {
                    return Err(Rejection::Malformed);
                }
                self.output.push_str(text);
                Ok(((!text.is_empty()).then(|| text.into()), false))
            }
            Some("step_finish") => {
                if !self.started
                    || part["type"] != "step-finish"
                    || !keys(
                        part,
                        &[
                            "id",
                            "sessionID",
                            "messageID",
                            "type",
                            "reason",
                            "snapshot",
                            "tokens",
                            "cost",
                        ],
                    )
                    || !number(&part["cost"])
                {
                    return Err(Rejection::Malformed);
                }
                if part.get("snapshot").is_some() || part["reason"] == "tool-calls" {
                    return Err(Rejection::Effect);
                }
                if part["reason"] != "stop" || self.output.trim().is_empty() {
                    return Err(Rejection::Failed);
                }
                let tokens = &part["tokens"];
                let cache = &tokens["cache"];
                if !keys(tokens, &["total", "input", "output", "reasoning", "cache"])
                    || !["input", "output", "reasoning"]
                        .iter()
                        .all(|key| number(&tokens[*key]))
                    || tokens.get("total").is_some_and(|total| !number(total))
                    || !keys(cache, &["read", "write"])
                    || !["read", "write"].iter().all(|key| number(&cache[*key]))
                {
                    return Err(Rejection::Malformed);
                }
                if tokens["reasoning"].as_f64() != Some(0.0) {
                    return Err(Rejection::Effect);
                }
                self.terminal = true;
                Ok((None, true))
            }
            _ => Err(Rejection::Malformed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: &str, part: Value) -> Value {
        let mut part = part;
        part["sessionID"] = json!("ses_fixture");
        part["messageID"] = json!("msg_fixture");
        json!({"type":kind,"timestamp":1,"sessionID":"ses_fixture","part":part})
    }
    fn start() -> Value {
        event("step_start", json!({"id":"prt_start","type":"step-start"}))
    }
    fn text() -> Value {
        event(
            "text",
            json!({"id":"prt_text","type":"text","text":"answer","time":{"start":1,"end":2},"metadata":{"openai":{"itemId":"msg_answer"}}}),
        )
    }
    fn finish() -> Value {
        event(
            "step_finish",
            json!({"id":"prt_finish","type":"step-finish","reason":"stop","cost":0,"tokens":{"total":2,"input":1,"output":1,"reasoning":0,"cache":{"read":0,"write":0}}}),
        )
    }
    #[test]
    fn only_completed_nonempty_owned_text_can_finish() {
        let mut p = Progress::new("gpt-4.1");
        assert_eq!(p.event(&start()).unwrap(), (None, false));
        assert_eq!(p.event(&text()).unwrap(), (Some("answer".into()), false));
        assert_eq!(p.event(&finish()).unwrap(), (None, true));
        assert_eq!(p.output, "answer");
        assert!(p.event(&finish()).is_err());
        let mut empty = Progress::default();
        empty.event(&start()).unwrap();
        assert_eq!(empty.event(&finish()), Err(Rejection::Failed));
        assert!(Progress::default().event(&text()).is_err());
    }
    #[test]
    fn errors_effects_identity_and_unknown_fields_are_sticky() {
        for value in [
            json!({"type":"error"}),
            json!({"type":"tool_use"}),
            json!({"type":"reasoning"}),
            json!({"type":"future"}),
        ] {
            let mut p = Progress::default();
            assert!(p.event(&value).is_err());
            assert!(p.event(&start()).is_err());
        }
        for (pointer, value) in [
            ("/part/sessionID", json!("ses_other")),
            ("/part/messageID", json!("msg_other")),
            ("/part/time/end", json!(0)),
            ("/part/synthetic", json!(true)),
            ("/part/metadata", json!({"error":"failed"})),
            ("/part/text", json!(null)),
        ] {
            let mut p = Progress::default();
            p.event(&start()).unwrap();
            let mut t = text();
            if let Some(slot) = t.pointer_mut(pointer) {
                *slot = value;
            } else {
                t["part"]["synthetic"] = value;
            }
            assert!(p.event(&t).is_err(), "accepted {pointer}");
        }
        for value in ["length", "error", "tool-calls", "unknown"] {
            let mut p = Progress::default();
            p.event(&start()).unwrap();
            p.event(&text()).unwrap();
            let mut f = finish();
            f["part"]["reason"] = json!(value);
            assert!(p.event(&f).is_err());
        }
        let mut p = Progress::default();
        p.event(&start()).unwrap();
        p.event(&text()).unwrap();
        assert!(p.event(&text()).is_err());
    }
    #[test]
    fn configuration_and_arguments_have_no_resume_or_credential_import() {
        let c = config("gpt-4.1");
        assert_eq!(c["permission"]["*"], "deny");
        assert_eq!(c["agent"][AGENT]["permission"]["*"], "deny");
        assert_eq!(c["plugin"], json!([]));
        assert_eq!(c["mcp"], json!({}));
        assert_eq!(c["snapshot"], false);
        assert_eq!(c["share"], "disabled");
        assert_eq!(c["agent"]["title"]["disable"], true);
        assert_eq!(
            c["provider"]["openai"]["options"]["apiKey"],
            "{env:OPENAI_API_KEY}"
        );
        let mut command = Command::new("unused-fixture");
        command.env_clear();
        arguments(&mut command, Path::new("/private/attempt"), "gpt-4.1");
        let args: Vec<_> = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|pair| pair == ["--format", "json"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--model", "openai/gpt-4.1"]));
        assert!(!args.iter().any(|arg| [
            "--attach",
            "--continue",
            "--session",
            "--command",
            "--auto",
            "--share"
        ]
        .contains(&arg.as_str())));
        assert!(command
            .get_envs()
            .all(|(key, _)| key != "OPENAI_API_KEY" && key != "OPENCODE_CONFIG_DIR"));
        assert!(version_matches("1.18.34\n"));
        assert!(!version_matches("1.18.35"));
        for id in [
            None,
            Some("openai/gpt-4.1"),
            Some("gpt"),
            Some("gpt-4.1:high"),
        ] {
            assert!(model(id).is_err());
        }
    }
    #[test]
    fn admission_sees_existing_entries_and_dangling_links() {
        let parent = tempfile::tempdir().unwrap();
        assert!(absent(&parent.path().join("policy")).is_ok());
        fs::create_dir(parent.path().join("policy")).unwrap();
        assert!(absent(&parent.path().join("policy")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("missing", parent.path().join("link")).unwrap();
            assert!(absent(&parent.path().join("link")).is_err());
        }
    }
    #[test]
    fn whitespace_overflow_snapshots_and_missing_completion_are_rejected() {
        for replacement in [" ".into(), "x".repeat(MAX_OUTPUT + 1)] {
            let mut p = Progress::default();
            p.event(&start()).unwrap();
            let mut t = text();
            t["part"]["text"] = json!(replacement);
            if p.event(&t).is_ok() {
                assert!(p.event(&finish()).is_err());
            }
            assert!(!p.terminal);
        }
        let mut p = Progress::default();
        p.event(&start()).unwrap();
        p.event(&text()).unwrap();
        assert!(!p.terminal);
        let mut f = finish();
        f["part"]["snapshot"] = json!("shared-state");
        assert_eq!(p.event(&f), Err(Rejection::Effect));
        let mut s = start();
        s["part"]["snapshot"] = json!("shared-state");
        assert_eq!(Progress::default().event(&s), Err(Rejection::Effect));
    }
}
