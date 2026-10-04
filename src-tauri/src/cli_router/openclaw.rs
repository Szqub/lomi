//! Source-based OpenClaw 2026.9.8 candidate (fc23bc864e4553c2d215e479eeec47b67a0bf943).
//! Fresh OpenAI API text turns only. Native recovery can still compact/retry
//! after overflow (three attempts) or timeout (two); no single-request promise.

use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const VERSION: &str = "2026.9.8";
pub(crate) const MAX_OUTPUT: usize = 1024 * 1024;
const MODELS: &[&str] = &[
    "gpt-4.1",
    "gpt-4.1-mini",
    "gpt-4.1-nano",
    "gpt-4o",
    "gpt-4o-mini",
];
const NO_SKILL: &str = "lomi-router-no-bundled-skills-fc23bc864e4553c2";

pub(crate) fn supported_models() -> &'static [&'static str] {
    MODELS
}

pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    value.filter(|id| MODELS.contains(id)).ok_or_else(||
        "OpenClaw requires an exact approved OpenAI text model: gpt-4.1, gpt-4.1-mini, gpt-4.1-nano, gpt-4o or gpt-4o-mini. No request was sent.".into())
}

fn config(root: &Path, id: &str) -> Value {
    let reference = format!("openai/{id}");
    json!({
        "env": {"shellEnv": {"enabled": false}},
        "plugins": {"enabled": false},
        "hooks": {"enabled": false, "internal": {"enabled": false}},
        "mcp": {},
        "tools": {"deny": ["*"], "codeMode": false, "elevated": {"enabled": false}},
        "skills": {"allowBundled": [NO_SKILL], "load": {"extraDirs": [], "allowSymlinkTargets": [], "watch": false}},
        "models": {"mode": "replace", "providers": {"openai": {
            "baseUrl": "https://api.openai.com/v1", "api": "openai-responses",
            "apiKey": "${OPENAI_API_KEY}",
            "models": [{"id": id, "name": id, "reasoning": false, "input": ["text"], "agentRuntime": {"id": "openclaw"}}]
        }}},
        "agents": {
            "defaults": {
                "workspace": root, "skipBootstrap": true,
                "systemAgent": {"agentId": "main"},
                "model": {"primary": reference, "fallbacks": []},
                "models": {(reference.clone()): {"agentRuntime": {"id": "openclaw"}, "codeMode": false}},
                "thinkingDefault": "off", "reasoningDefault": "off",
                "compaction": {"enabled": false, "memoryFlush": {"enabled": false, "forceFlushTranscriptBytes": 0}, "maxActiveTranscriptBytes": 0}
            },
            "entries": {"main": {"id": "main", "workspace": root, "runtime": {"type": "embedded"}}}
        }
    })
}

pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
    let id = model(Some(id))?;
    let root = parent.join(format!("openclaw-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| "Cannot create fresh OpenClaw attempt storage.")?;
    crate::chat::storage::private(&root, true)?;
    let root = root
        .canonicalize()
        .map_err(|_| "Cannot resolve private OpenClaw attempt storage.")?;
    crate::chat::storage::atomic(
        &root.join("openclaw.json"),
        &serde_json::to_vec(&config(&root, id))
            .map_err(|_| "Cannot build OpenClaw restrictions.")?,
    )?;
    Ok(root)
}

pub(crate) fn environment(command: &mut Command, root: &Path) {
    // Set these before importing agent-exec: it captures original auth/plugin
    // roots before redirecting its runtime state. No ambient HOME is admitted.
    command
        .current_dir(root)
        .env("HOME", root)
        .env("OPENCLAW_HOME", root)
        .env("OPENCLAW_STATE_DIR", root)
        .env("OPENCLAW_CONFIG_PATH", root.join("openclaw.json"))
        .env("OPENCLAW_WORKSPACE_DIR", root);
}

pub(crate) fn arguments(command: &mut Command, root: &Path, id: &str) {
    environment(command, root);
    command
        .args([
            "agent",
            "exec",
            "--message-file",
            "-",
            "--json",
            "--thinking",
            "off",
            "--timeout",
            "300",
            "--code-mode",
            "direct",
        ])
        .arg("--cwd")
        .arg(root)
        .arg("--state-dir")
        .arg(root)
        .arg("--config")
        .arg(root.join("openclaw.json"))
        .arg("--model")
        .arg(format!("openai/{id}"));
    // --auth-env-only discards the explicit owned restrictions; the config's
    // credential placeholder and fresh captured roots establish API ownership.
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

pub(crate) struct Progress {
    model: String,
    pub(crate) output: String,
    pub(crate) completed: bool,
    pub(crate) rejection: Option<Rejection>,
    seen: bool,
}

fn session_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        })
}

impl Progress {
    pub(crate) fn new(model: &str) -> Self {
        Self {
            model: model.into(),
            output: String::new(),
            completed: false,
            rejection: None,
            seen: false,
        }
    }

    /// Call once with the complete stdout bytes after EOF; pretty JSON is not
    /// a JSONL event stream. serde_json rejects extra/trailing envelopes.
    #[cfg(test)]
    pub(crate) fn parse(&mut self, bytes: &[u8]) -> Result<(Option<String>, bool), Rejection> {
        let result = if bytes.len() > MAX_OUTPUT {
            Err(Rejection::Malformed)
        } else {
            serde_json::from_slice::<Value>(bytes)
                .map_err(|_| Rejection::Malformed)
                .and_then(|value| self.envelope(&value))
        };
        if let Err(error) = result {
            self.completed = false;
            self.rejection = Some(error);
        }
        result
    }

    pub(crate) fn envelope(&mut self, value: &Value) -> Result<(Option<String>, bool), Rejection> {
        let result = self.accept(value);
        if let Err(error) = result {
            self.completed = false;
            self.rejection = Some(error);
        }
        result
    }

    fn accept(&mut self, value: &Value) -> Result<(Option<String>, bool), Rejection> {
        if self.seen || self.rejection.is_some() {
            return Err(Rejection::Malformed);
        }
        self.seen = true;
        let object = value.as_object().ok_or(Rejection::Malformed)?;
        if object.keys().any(|key| {
            ![
                "ok",
                "status",
                "final",
                "payloads",
                "usage",
                "costUsd",
                "codeModeEngaged",
                "assistantTurns",
                "bridgeCalls",
                "toolSummary",
                "model",
                "provider",
                "sessionId",
                "error",
            ]
            .contains(&key.as_str())
        }) {
            return Err(Rejection::Malformed);
        }
        if value["ok"] != true || value["status"] != "ok" || value.get("error").is_some() {
            return Err(Rejection::Failed);
        }
        if value["provider"] != "openai"
            || value["model"].as_str() != Some(self.model.as_str())
            || !value["sessionId"].as_str().is_some_and(session_id)
        {
            return Err(Rejection::Malformed);
        }
        if value
            .get("codeModeEngaged")
            .is_some_and(|value| value != false)
        {
            return Err(Rejection::Effect);
        }
        if value
            .get("assistantTurns")
            .is_some_and(|value| value.as_u64() != Some(1))
        {
            return Err(Rejection::Failed);
        }
        if let Some(bridge) = value.get("bridgeCalls") {
            if !bridge.as_object().is_some_and(|calls| {
                calls.len() == 3
                    && ["search", "describe", "call"].iter().all(|key| {
                        calls
                            .get(*key)
                            .is_some_and(|count| count.as_u64() == Some(0))
                    })
            }) {
                return Err(Rejection::Effect);
            }
        }
        if let Some(summary) = value.get("toolSummary") {
            let summary = summary.as_object().ok_or(Rejection::Malformed)?;
            if summary.keys().any(|key| {
                !["calls", "tools", "failures", "totalToolTimeMs"].contains(&key.as_str())
            }) || summary.get("calls").and_then(Value::as_u64) != Some(0)
                || !summary
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                || summary
                    .get("failures")
                    .is_some_and(|count| count.as_u64() != Some(0))
                || summary
                    .get("totalToolTimeMs")
                    .is_some_and(|count| count.as_u64() != Some(0))
            {
                return Err(Rejection::Effect);
            }
        }
        let payloads = value["payloads"].as_array().ok_or(Rejection::Malformed)?;
        let mut text = Vec::new();
        for payload in payloads {
            let payload = payload.as_object().ok_or(Rejection::Malformed)?;
            if payload.keys().any(|key| {
                ![
                    "text",
                    "mediaUrl",
                    "mediaUrls",
                    "isError",
                    "isReasoning",
                    "isCommentary",
                ]
                .contains(&key.as_str())
            }) {
                return Err(Rejection::Malformed);
            }
            if ["isError", "isReasoning", "isCommentary"]
                .iter()
                .any(|key| payload.get(*key).is_some_and(|value| value != false))
            {
                return Err(Rejection::Failed);
            }
            if payload
                .get("mediaUrl")
                .is_some_and(|value| !value.is_null())
                || payload
                    .get("mediaUrls")
                    .is_some_and(|value| !value.as_array().is_some_and(Vec::is_empty))
            {
                return Err(Rejection::Effect);
            }
            let chunk = payload
                .get("text")
                .and_then(Value::as_str)
                .ok_or(Rejection::Malformed)?;
            if !chunk.trim().is_empty() {
                text.push(chunk.trim_end());
            }
        }
        let final_text = value["final"]
            .as_str()
            .filter(|text| !text.trim().is_empty() && text.len() <= MAX_OUTPUT)
            .ok_or(Rejection::Failed)?;
        if !text.is_empty() && text.join("\n") != final_text {
            return Err(Rejection::Malformed);
        }
        self.output = final_text.into();
        self.completed = true;
        Ok((Some(self.output.clone()), true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn answer() -> Value {
        json!({"ok":true,"status":"ok","final":"answer","payloads":[{"text":"answer"}],"provider":"openai","model":"gpt-4.1","sessionId":"12345678-1234-4123-8123-123456789abc","codeModeEngaged":false,"assistantTurns":1,"toolSummary":{"calls":0,"tools":[]}})
    }
    #[test]
    fn complete_pretty_json_is_authoritative_and_single() {
        let bytes = serde_json::to_vec_pretty(&answer()).unwrap();
        let mut progress = Progress::new("gpt-4.1");
        assert_eq!(
            progress.parse(&bytes).unwrap(),
            (Some("answer".into()), true)
        );
        assert_eq!(progress.output, "answer");
        assert!(progress.parse(&bytes).is_err());
        assert!(!progress.completed);
        let mut duplicate = bytes.clone();
        duplicate.extend_from_slice(&bytes);
        assert!(Progress::new("gpt-4.1").parse(&duplicate).is_err());
        assert!(Progress::new("gpt-4.1")
            .parse(&vec![b' '; MAX_OUTPUT + 1])
            .is_err());
    }
    #[test]
    fn failure_identity_effects_and_empty_final_are_rejected() {
        for (pointer, bad) in [
            ("/ok", json!(false)),
            ("/status", json!("timeout")),
            ("/provider", json!("anthropic")),
            ("/model", json!("gpt")),
            ("/sessionId", json!("old")),
            ("/final", json!("")),
            ("/final", json!("mismatched")),
            ("/codeModeEngaged", json!(true)),
            ("/assistantTurns", json!(2)),
            ("/toolSummary/calls", json!(1)),
            ("/toolSummary/tools", json!(["bash"])),
            ("/payloads/0", json!({"text":"answer","isError":true})),
            (
                "/payloads/0",
                json!({"text":"answer","mediaUrl":"file:///private/secret"}),
            ),
        ] {
            let mut value = answer();
            *value.pointer_mut(pointer).unwrap() = bad;
            let mut progress = Progress::new("gpt-4.1");
            assert!(progress.envelope(&value).is_err(), "accepted {pointer}");
            assert!(!progress.completed);
        }
        for key in ["error", "futureResult"] {
            let mut value = answer();
            value[key] = json!({});
            assert!(Progress::new("gpt-4.1").envelope(&value).is_err());
        }
    }
    #[test]
    fn owned_config_and_arguments_cannot_import_credentials_or_tools() {
        let root = Path::new("/private/openclaw");
        let config = config(root, "gpt-4.1");
        assert_eq!(config["env"]["shellEnv"]["enabled"], false);
        assert_eq!(config["plugins"]["enabled"], false);
        assert_eq!(config["hooks"]["internal"]["enabled"], false);
        assert_eq!(config["tools"]["deny"], json!(["*"]));
        assert_eq!(config["skills"]["allowBundled"], json!([NO_SKILL]));
        assert_eq!(
            config["models"]["providers"]["openai"]["apiKey"],
            "${OPENAI_API_KEY}"
        );
        assert_eq!(
            config["agents"]["defaults"]["models"]["openai/gpt-4.1"]["agentRuntime"]["id"],
            "openclaw"
        );
        let mut command = Command::new("unused-fixture");
        command.env_clear();
        arguments(&mut command, root, "gpt-4.1");
        let args: Vec<_> = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--model", "openai/gpt-4.1"]));
        assert!(args.windows(2).any(|pair| pair == ["--message-file", "-"]));
        assert!(!args
            .iter()
            .any(|value| value == "--auth-env-only" || value == "--fallback"));
        assert!(command.get_envs().all(|(key, _)| key != "OPENAI_API_KEY"));
        for id in [
            None,
            Some("gpt"),
            Some("openai/gpt-4.1"),
            Some("gpt-5-pro"),
            Some("gpt-4.1:high"),
        ] {
            assert!(model(id).is_err());
        }
        assert!(version_matches("2026.9.8\n"));
        assert!(!version_matches("2026.9.9"));
    }
    #[test]
    fn attempts_never_reuse_auth_or_state() {
        let parent = tempfile::tempdir().unwrap();
        let first = home(parent.path(), "gpt-4.1").unwrap();
        fs::write(first.join("auth-profiles.json"), "old credential fixture").unwrap();
        let second = home(parent.path(), "gpt-4.1").unwrap();
        assert_ne!(first, second);
        assert!(!second.join("auth-profiles.json").exists());
        assert!(second.join("openclaw.json").is_file());
    }
}
