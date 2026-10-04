//! Source-built Grok 1.0.45 candidate, public commit
//! 2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8 (SOURCE_REV
//! 559751fdcec02d413e4c57c8832ab275e4f44980). Published npm 1.0.45/46 has
//! different, unavailable source provenance and MUST NOT pass this admission.
//! The managed-engine artifact manifest below is intentionally EMPTY: this
//! engine remains a fixture candidate. Gateway source artifacts are admitted
//! separately by grok_artifact; their trust does not enable this engine.
//! Semver, user paths and user hashes cannot enable this candidate.
//! Artifact and policy admission must run before version probes
//! and again under the final dispatch owner fence.
//!
//! Authority: xai-org/grok-build at the public pin, crates/codegen/
//! xai-grok-{pager/src/headless/{cli,reducer/messages/{wire,partial,mod,usage}},
//! config/src/{paths,macos_managed},shell/src/{agent/{config,builder,agent_ops},
//! session/{summary,persistence,helpers/session_summary}}}, permission-rules/src/
//! repo.rs, and xai-grok-sampler/src/retry.rs. Native xAI grok-4.6 reasons;
//! thinking/signatures remain separate from final text. Tools are removed by
//! recognized allowlist + denylist, not by an unknown tool sentinel. A fresh
//! session's first-title helper may make one additional request, with a
//! session_title function definition and max_tokens=100, to the SAME fixed
//! model/key/endpoint with retries=0. This is not a zero-auxiliary-call claim.
//! Fresh logical context only; no native resume/continuity or typed quota wire.
//! The driver must retain reasoning separately and require complete EOF, input,
//! exit=0 and bounded process-group/readers drain after the terminal result.
//! Leading slash inputs are denied: session/slash_authority.rs and
//! session/acp_session_impl/turn.rs resolve native controls before sampling.
//! Headless metadata has no LocalWorkspaceIntent, so agent_ops.rs does not
//! launch a workspace_server. MCP startup uses its own default compatibility
//! settings; the empty HOME/cwd and ancestor/policy admission are required.

use std::{fs, path::Path};

const MODELS: &[&str] = &["grok-4.6"];
pub(crate) fn supported_models() -> &'static [&'static str] {
    MODELS
}
fn isolation() -> String {
    "Grok refuses shared managed policy or ancestor Git state. No request was sent.".into()
}
fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(isolation()),
    }
}
fn system_absent(path: &Path) -> Result<(), String> {
    absent(path)?;
    for ancestor in path.parent().into_iter().flat_map(Path::ancestors) {
        match fs::symlink_metadata(ancestor) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(link) => {
                let meta = fs::metadata(ancestor).map_err(|_| isolation())?;
                if !meta.is_dir() {
                    return Err(isolation());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if link.uid() != 0 || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
                        return Err(isolation());
                    }
                }
            }
            Err(_) => return Err(isolation()),
        }
    }
    Ok(())
}
pub(super) fn admission(root: &Path) -> Result<(), String> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err(isolation());
    }
    system_absent(Path::new("/etc/grok/managed_config.toml"))?;
    system_absent(Path::new("/etc/grok/requirements.toml"))?;
    #[cfg(target_os = "macos")]
    {
        system_absent(Path::new(
            "/Library/Application Support/ClaudeCode/managed-settings.json",
        ))?;
        forced_preferences()?;
    }
    #[cfg(target_os = "linux")]
    system_absent(Path::new("/etc/claude-code/managed-settings.json"))?;
    for ancestor in root.ancestors() {
        absent(&ancestor.join(".git"))?;
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn forced_preferences() -> Result<(), String> {
    use std::ffi::{c_char, c_void};
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(a: *const c_void, s: *const c_char, e: u32) -> *const c_void;
        fn CFPreferencesAppValueIsForced(key: *const c_void, application: *const c_void) -> u8;
        fn CFRelease(value: *const c_void);
    }
    unsafe {
        let domain = CFStringCreateWithCString(std::ptr::null(), c"ai.x.grok".as_ptr(), 0x08000100);
        if domain.is_null() {
            return Err(isolation());
        }
        let key = CFStringCreateWithCString(
            std::ptr::null(),
            c"requirements_toml_base64".as_ptr(),
            0x08000100,
        );
        if key.is_null() {
            CFRelease(domain);
            return Err(isolation());
        }
        let forced = CFPreferencesAppValueIsForced(key, domain) != 0;
        CFRelease(key);
        CFRelease(domain);
        if forced {
            return Err(isolation());
        }
    }
    Ok(())
}

// The provenance-disabled managed engine is retained only as a source-qualified
// fixture candidate. Gateway catalog and ambient-policy admission remain live.
#[cfg(test)]
mod managed_candidate {
    use super::super::{runtime::OwnedChild, transport::Outcome};
    use super::{admission, isolation, MODELS};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{
        collections::HashSet,
        fs,
        io::Read,
        path::{Path, PathBuf},
        process::Command,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    pub(crate) const VERSION: &str = "1.0.45";
    pub(crate) const PIN: &str = "2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8";
    const SOURCE_REV: &str = "559751fdcec02d413e4c57c8832ab275e4f44980";
    const MAX_INPUT: usize = 60 * 1024;
    const MAX_OUTPUT: usize = 1024 * 1024;
    const MAX_REASONING: usize = 1024 * 1024;
    const MAX_SIGNATURE: usize = 65536;
    pub(crate) const MAX_FRAME: usize = 6 * (MAX_OUTPUT + MAX_REASONING + MAX_SIGNATURE) + 16384;
    const MAX_WIRE: usize = 64 * 1024 * 1024;
    const MAX_EVENTS: usize = 65536;
    const MAX_BINARY: u64 = 512 * 1024 * 1024;

    struct TrustedArtifact {
        os: &'static str,
        arch: &'static str,
        public_commit: &'static str,
        source_rev: &'static str,
        version: &'static str,
        sha256: &'static str,
    }
    // Only a reviewed source build may add an entry through a native source change.
    // Never deserialize this manifest from webview input, profile state or an env.
    const TRUSTED_ARTIFACTS: &[TrustedArtifact] = &[];
    pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
        value.filter(|value| MODELS.contains(value)).ok_or_else(|| {
            "Grok requires the approved native xAI model. No request was sent.".into()
        })
    }
    pub(crate) fn artifact_available() -> bool {
        TRUSTED_ARTIFACTS.iter().any(|a| {
            a.os == std::env::consts::OS
                && a.arch == std::env::consts::ARCH
                && a.public_commit == PIN
                && a.source_rev == SOURCE_REV
                && a.version == VERSION
                && a.sha256.len() == 64
        })
    }
    fn unavailable() -> String {
        "Grok requires a qualified source-built native artifact; published npm versions are unavailable for managed text execution.".into()
    }
    pub(crate) fn artifact_admission(executable: &Path) -> Result<(), String> {
        if !artifact_available() {
            return Err(unavailable());
        }
        let meta = fs::symlink_metadata(executable).map_err(|_| unavailable())?;
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.len() == 0
            || meta.len() > MAX_BINARY
        {
            return Err(unavailable());
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options.open(executable).map_err(|_| unavailable())?;
        let opened = file.metadata().map_err(|_| unavailable())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.dev() != opened.dev()
                || meta.ino() != opened.ino()
                || opened.mode() & 0o022 != 0
                || opened.mode() & 0o111 == 0
            {
                return Err(unavailable());
            }
        }
        if opened.len() != meta.len() {
            return Err(unavailable());
        }
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut bytes = [0u8; 65536];
        loop {
            let n = file.read(&mut bytes).map_err(|_| unavailable())?;
            if n == 0 {
                break;
            }
            total = total.saturating_add(n as u64);
            if total > MAX_BINARY {
                return Err(unavailable());
            }
            hash.update(&bytes[..n]);
        }
        let after = file.metadata().map_err(|_| unavailable())?;
        if total != opened.len()
            || after.len() != opened.len()
            || after.modified().ok() != opened.modified().ok()
        {
            return Err(unavailable());
        }
        let digest = format!("{:x}", hash.finalize());
        if !TRUSTED_ARTIFACTS.iter().any(|a| {
            a.os == std::env::consts::OS
                && a.arch == std::env::consts::ARCH
                && a.public_commit == PIN
                && a.source_rev == SOURCE_REV
                && a.version == VERSION
                && a.sha256 == digest
        }) {
            return Err(unavailable());
        }
        Ok(())
    }
    /// This helper cannot admit published semver alone: an owned artifact pin must
    /// exist and the caller must separately hash the resolved executable each time.
    pub(crate) fn version_matches(bytes: &[u8]) -> bool {
        if !artifact_available() {
            return false;
        }
        let Ok(value) = std::str::from_utf8(bytes) else {
            return false;
        };
        let value = value.trim();
        // pager-bin/build.rs stamps the first 12 chars of checkout HEAD. A
        // qualified source checkout must identify the public commit, not npm's HEAD.
        value == format!("grok {VERSION} ({})", &PIN[..12])
    }
    fn settings() -> &'static str {
        r#"disable_web_search = true
[auth]
preferred_method = "api_key"
[cli]
auto_update = false
[features]
remote_fetch = false
managed_config = false
telemetry = "off"
title_refresh = false
session_recap = false
session_search = false
lsp_tools = false
image_gen = false
file_acceleration = false
codebase_indexing = false
mcp_auto_restart = false
mcp_liveness_watchers = false
mcp_push_server_status = false
mcp_recursive_config_watch = false
[telemetry]
trace_upload = false
[memory]
enabled = false
[memory_v2]
enabled = false
file_writes_enabled = false
capture_enabled = false
automatic_dream_enabled = false
manual_dream_enabled = false
[session]
load_envrc = false
[plugins]
paths = []
enabled = []
[skills]
paths = []
[paths]
extra_rule_dirs = []
extra_skill_dirs = []
[compat.claude]
agents = false
hooks = false
mcps = false
rules = false
skills = false
[compat.codex]
hooks = false
skills = false
[compat.cursor]
agents = false
hooks = false
mcps = false
rules = false
skills = false
[models]
default = "grok-4.6"
allowed_models = ["grok-4.6"]
session_summary = "grok-4.6"
max_retries = 0
rate_limit_retry_threshold = 0
subagent_rate_limit_max_attempts = 0
[model."grok-4.6"]
model = "grok-4.6"
base_url = "https://api.x.ai/v1"
env_key = "LOMI_GROK_API_KEY"
api_backend = "chat_completions"
max_retries = 0
rate_limit_retry_threshold = 0
subagent_rate_limit_max_attempts = 0
max_completion_tokens = 8192
context_window = 500000
reasoning_effort = "low"
"#
    }
    pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
        model(Some(id))?;
        let root = parent.join(format!("grok-{}", super::super::new_id()?));
        fs::create_dir(&root).map_err(|_| isolation())?;
        crate::chat::storage::private(&root, true)?;
        let root = root.canonicalize().map_err(|_| isolation())?;
        admission(&root)?;
        for name in [".grok", "config", "data", "state", "cache", "tmp"] {
            let path = root.join(name);
            fs::create_dir(&path).map_err(|_| isolation())?;
            crate::chat::storage::private(&path, true)?;
        }
        crate::chat::storage::atomic(&root.join(".grok/config.toml"), settings().as_bytes())?;
        Ok(root)
    }
    pub(crate) fn environment(command: &mut Command, root: &Path) {
        command
            .current_dir(root)
            .env("HOME", root)
            .env("USERPROFILE", root)
            .env("GROK_HOME", root.join(".grok"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("TMPDIR", root.join("tmp"))
            .env("TMP", root.join("tmp"))
            .env("TEMP", root.join("tmp"))
            .env("GROK_MANAGED_CONFIG", "false")
            .env("GROK_MAX_RETRIES", "0")
            .env("GROK_TELEMETRY_ENABLED", "off")
            .env("GROK_TELEMETRY_TRACE_UPLOAD", "false");
    }
    pub(crate) struct Controller {
        root: PathBuf,
        session: String,
        prompt: String,
    }
    fn input_admission(prompt: &str) -> Result<(), String> {
        if prompt.is_empty() || prompt.len() > MAX_INPUT {
            return Err("Grok managed text context exceeds its 60 KiB bound.".into());
        }
        if prompt.trim_start().starts_with('/') {
            return Err(
                "Grok managed text cannot invoke native slash controls. No request was sent."
                    .into(),
            );
        }
        Ok(())
    }
    impl Controller {
        pub(crate) fn new(root: &Path, id: &str, prompt: &str) -> Result<Self, String> {
            model(Some(id))?;
            admission(root)?;
            input_admission(prompt)?;
            let mut hex = super::super::new_id()?.into_bytes();
            if hex.len() != 32 || !hex.iter().all(u8::is_ascii_hexdigit) {
                return Err(isolation());
            }
            hex[12] = b'4';
            hex[16] = b'8';
            let hex = std::str::from_utf8(&hex).map_err(|_| isolation())?;
            let session = format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            );
            Ok(Self {
                root: root.to_owned(),
                session,
                prompt: prompt.into(),
            })
        }
        pub(crate) fn session_id(&self) -> &str {
            &self.session
        }
    }
    pub(crate) fn arguments(command: &mut Command, controller: &Controller) -> Result<(), String> {
        artifact_admission(Path::new(command.get_program()))?;
        admission(&controller.root)?;
        command
            .arg("-p")
            .arg(&controller.prompt)
            .arg("--cwd")
            .arg(&controller.root)
            .arg("--session-id")
            .arg(&controller.session)
            .args([
                "--model",
                "grok-4.6",
                "--tools",
                "read_file",
                "--disallowed-tools",
                "read_file,search_tool,use_tool,Agent",
                "--max-turns",
                "1",
                "--no-memory",
                "--disable-web-search",
                "--no-auto-update",
                "--output-format",
                "streaming-messages-json",
                "--include-partial-messages",
            ]);
        Ok(())
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum Rejection {
        Malformed,
        Failed,
        Effect,
    }
    fn keys(value: &Value, allowed: &[&str]) -> bool {
        value
            .as_object()
            .is_some_and(|map| map.keys().all(|key| allowed.contains(&key.as_str())))
    }
    fn empty(value: &Value) -> bool {
        value.as_array().is_some_and(Vec::is_empty)
    }
    fn null(value: &Value, key: &str) -> bool {
        value.get(key).is_some_and(Value::is_null)
    }
    fn uuid(value: &str) -> bool {
        let b = value.as_bytes();
        b.len() == 36
            && b[14] == b'4'
            && b"89ab".contains(&b[19])
            && b.iter().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    *b == b'-'
                } else {
                    b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
                }
            })
    }
    fn usage(value: &Value) -> bool {
        keys(
            value,
            &[
                "input_tokens",
                "output_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
                "server_tool_use",
            ],
        ) && [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .iter()
        .all(|key| value[key].as_u64().is_some())
            && value
                .get("server_tool_use")
                .is_none_or(|v| keys(v, &["web_search_requests"]) && v["web_search_requests"] == 0)
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum ReasoningBlock {
        Text(String),
        Thinking { thinking: String, signature: String },
    }
    impl ReasoningBlock {
        fn value(&self) -> Value {
            match self {
                Self::Text(text) => json!({"type":"text","text":text}),
                Self::Thinking {
                    thinking,
                    signature,
                } => json!({"type":"thinking","thinking":thinking,"signature":signature}),
            }
        }
    }
    pub(crate) struct Progress {
        root: PathBuf,
        session: String,
        ids: HashSet<String>,
        init: bool,
        message: Option<String>,
        blocks: Vec<ReasoningBlock>,
        open: Option<usize>,
        delta: bool,
        message_stop: bool,
        assistant: bool,
        terminal: bool,
        rejected: bool,
        final_usage: Option<Value>,
        signature_bytes: usize,
        pub(crate) output: String,
        pub(crate) reasoning: String,
    }
    impl Progress {
        pub(crate) fn new(controller: &Controller) -> Self {
            Self {
                root: controller.root.clone(),
                session: controller.session.clone(),
                ids: HashSet::new(),
                init: false,
                message: None,
                blocks: Vec::new(),
                open: None,
                delta: false,
                message_stop: false,
                assistant: false,
                terminal: false,
                rejected: false,
                final_usage: None,
                signature_bytes: 0,
                output: String::new(),
                reasoning: String::new(),
            }
        }
        /// Ordered separate text/reasoning blocks and signatures, for durable native
        /// retention. They must never be flattened into the final answer.
        pub(crate) fn reasoning_blocks(&self) -> &[ReasoningBlock] {
            &self.blocks
        }
        pub(crate) fn completed(&self) -> bool {
            self.terminal && !self.rejected
        }
        pub(crate) fn event(&mut self, value: &Value) -> Result<(Option<String>, bool), Rejection> {
            let result = self.accept(value);
            if result.is_err() {
                self.rejected = true;
                self.terminal = false;
            }
            result
        }
        fn accept(&mut self, v: &Value) -> Result<(Option<String>, bool), Rejection> {
            if self.rejected
                || self.terminal
                || self.ids.len() >= MAX_EVENTS
                || v["session_id"] != self.session
            {
                return Err(Rejection::Malformed);
            }
            let id = v["uuid"]
                .as_str()
                .filter(|v| uuid(v))
                .ok_or(Rejection::Malformed)?;
            if !self.ids.insert(id.into()) {
                return Err(Rejection::Malformed);
            }
            match v["type"].as_str() {
                Some("system") if !self.init => {
                    if !keys(
                        v,
                        &[
                            "type",
                            "subtype",
                            "session_id",
                            "apiKeySource",
                            "model",
                            "cwd",
                            "permissionMode",
                            "tools",
                            "slash_commands",
                            "mcp_servers",
                            "skills",
                            "uuid",
                        ],
                    ) || v["subtype"] != "init"
                        || v["apiKeySource"] != "user"
                        || v["model"] != "grok-4.6"
                        || v["cwd"].as_str() != self.root.to_str()
                        || v["permissionMode"] != "default"
                        || !empty(&v["tools"])
                        || !empty(&v["mcp_servers"])
                        || !empty(&v["skills"])
                        || !v["slash_commands"].as_array().is_some_and(|items| {
                            items.len() <= 256
                                && items.iter().all(|v| {
                                    v.as_str().is_some_and(|s| {
                                        !s.is_empty()
                                            && s.len() <= 128
                                            && s.bytes().all(|b| {
                                                b.is_ascii_alphanumeric() || b"_-".contains(&b)
                                            })
                                    })
                                })
                        })
                    {
                        return Err(Rejection::Effect);
                    }
                    self.init = true;
                    Ok((None, false))
                }
                Some("stream_event") if self.init && !self.assistant => {
                    if !keys(
                        v,
                        &["type", "event", "session_id", "uuid", "parent_tool_use_id"],
                    ) || !null(v, "parent_tool_use_id")
                    {
                        return Err(Rejection::Malformed);
                    }
                    self.partial(&v["event"])
                }
                Some("assistant") if self.init && self.message_stop && !self.assistant => {
                    if !keys(
                        v,
                        &[
                            "type",
                            "message",
                            "parent_tool_use_id",
                            "session_id",
                            "uuid",
                        ],
                    ) || !null(v, "parent_tool_use_id")
                    {
                        return Err(Rejection::Malformed);
                    }
                    let m = &v["message"];
                    if !message(m)
                        || m["id"].as_str() != self.message.as_deref()
                        || m["stop_reason"] != "end_turn"
                        || !null(m, "stop_sequence")
                        || m["content"]
                            != Value::Array(self.blocks.iter().map(ReasoningBlock::value).collect())
                        || self.final_usage.as_ref() != m.get("usage")
                        || self.output.trim().is_empty()
                    {
                        return Err(Rejection::Failed);
                    }
                    self.assistant = true;
                    Ok((None, false))
                }
                Some("result") if self.assistant => {
                    if !keys(
                        v,
                        &[
                            "type",
                            "subtype",
                            "is_error",
                            "duration_ms",
                            "duration_api_ms",
                            "num_turns",
                            "result",
                            "stop_reason",
                            "total_cost_usd",
                            "usage",
                            "modelUsage",
                            "structured_output",
                            "errors",
                            "session_id",
                            "uuid",
                        ],
                    ) || v["subtype"] != "success"
                        || v["is_error"] != false
                        || v["stop_reason"] != "end_turn"
                        || v["result"] != self.output
                        || v.get("errors").is_some()
                        || v.get("structured_output").is_some()
                        || v["num_turns"] != 1
                        || !usage(&v["usage"])
                        || !["duration_ms", "duration_api_ms"]
                            .iter()
                            .all(|k| v[k].as_u64().is_some())
                        || !v["total_cost_usd"]
                            .as_f64()
                            .is_some_and(|v| v.is_finite() && v >= 0.0)
                    {
                        return Err(Rejection::Failed);
                    }
                    let models = v["modelUsage"].as_object().ok_or(Rejection::Malformed)?;
                    if models.len() != 1 || !models.contains_key("grok-4.6") {
                        return Err(Rejection::Malformed);
                    }
                    for row in models.values() {
                        if !keys(
                            row,
                            &[
                                "inputTokens",
                                "outputTokens",
                                "cacheReadInputTokens",
                                "cacheCreationInputTokens",
                                "webSearchRequests",
                                "costUSD",
                                "contextWindow",
                            ],
                        ) || ![
                            "inputTokens",
                            "outputTokens",
                            "cacheReadInputTokens",
                            "cacheCreationInputTokens",
                            "webSearchRequests",
                        ]
                        .iter()
                        .all(|k| row[k].as_u64().is_some())
                            || row["webSearchRequests"] != 0
                            || !row["costUSD"]
                                .as_f64()
                                .is_some_and(|v| v.is_finite() && v >= 0.0)
                            || row.get("contextWindow").is_some_and(|v| v != 500000)
                        {
                            return Err(Rejection::Malformed);
                        }
                    }
                    self.terminal = true;
                    Ok((None, true))
                }
                Some("result") => Err(Rejection::Failed),
                Some("user" | "system") => Err(Rejection::Effect),
                _ => Err(Rejection::Malformed),
            }
        }
        fn partial(&mut self, e: &Value) -> Result<(Option<String>, bool), Rejection> {
            match e["type"].as_str() {
                Some("message_start") if self.message.is_none() => {
                    if !keys(e, &["type", "message"])
                        || !message(&e["message"])
                        || !empty(&e["message"]["content"])
                        || !null(&e["message"], "stop_reason")
                        || !null(&e["message"], "stop_sequence")
                    {
                        return Err(Rejection::Malformed);
                    }
                    self.message = Some(
                        e["message"]["id"]
                            .as_str()
                            .ok_or(Rejection::Malformed)?
                            .into(),
                    );
                }
                Some("content_block_start")
                    if self.message.is_some()
                        && self.open.is_none()
                        && !self.delta
                        && !self.message_stop =>
                {
                    if !keys(e, &["type", "index", "content_block"])
                        || e["index"].as_u64() != Some(self.blocks.len() as u64)
                        || self.blocks.len() >= 128
                    {
                        return Err(Rejection::Malformed);
                    }
                    let b = &e["content_block"];
                    let block = match b["type"].as_str() {
                        Some("text") if keys(b, &["type", "text"]) && b["text"] == "" => {
                            ReasoningBlock::Text(String::new())
                        }
                        Some("thinking")
                            if keys(b, &["type", "thinking", "signature"])
                                && b["thinking"] == ""
                                && b["signature"] == "" =>
                        {
                            ReasoningBlock::Thinking {
                                thinking: String::new(),
                                signature: String::new(),
                            }
                        }
                        _ => return Err(Rejection::Effect),
                    };
                    self.open = Some(self.blocks.len());
                    self.blocks.push(block);
                }
                Some("content_block_delta") if self.open.is_some() && !self.delta => {
                    if !keys(e, &["type", "index", "delta"])
                        || e["index"].as_u64() != self.open.map(|v| v as u64)
                    {
                        return Err(Rejection::Malformed);
                    }
                    let d = &e["delta"];
                    let index = self.open.ok_or(Rejection::Malformed)?;
                    match (&mut self.blocks[index], d["type"].as_str()) {
                        (ReasoningBlock::Text(text), Some("text_delta"))
                            if keys(d, &["type", "text"]) =>
                        {
                            let delta = d["text"].as_str().ok_or(Rejection::Malformed)?;
                            if self.output.len().saturating_add(delta.len()) > MAX_OUTPUT {
                                return Err(Rejection::Malformed);
                            }
                            text.push_str(delta);
                            self.output.push_str(delta);
                            return Ok(((!delta.is_empty()).then(|| delta.into()), false));
                        }
                        (ReasoningBlock::Thinking { thinking, .. }, Some("thinking_delta"))
                            if keys(d, &["type", "thinking"]) =>
                        {
                            let delta = d["thinking"].as_str().ok_or(Rejection::Malformed)?;
                            if self.reasoning.len().saturating_add(delta.len()) > MAX_REASONING {
                                return Err(Rejection::Malformed);
                            }
                            thinking.push_str(delta);
                            self.reasoning.push_str(delta);
                        }
                        (ReasoningBlock::Thinking { signature, .. }, Some("signature_delta"))
                            if keys(d, &["type", "signature"]) =>
                        {
                            let delta = d["signature"].as_str().ok_or(Rejection::Malformed)?;
                            if !signature.is_empty()
                                || self.signature_bytes.saturating_add(delta.len()) > MAX_SIGNATURE
                            {
                                return Err(Rejection::Malformed);
                            }
                            signature.push_str(delta);
                            self.signature_bytes += delta.len();
                        }
                        _ => return Err(Rejection::Effect),
                    }
                }
                Some("content_block_stop") if self.open.is_some() && !self.delta => {
                    if !keys(e, &["type", "index"])
                        || e["index"].as_u64() != self.open.map(|v| v as u64)
                    {
                        return Err(Rejection::Malformed);
                    }
                    self.open = None;
                }
                Some("message_delta")
                    if self.message.is_some()
                        && self.open.is_none()
                        && !self.delta
                        && !self.message_stop =>
                {
                    if !keys(e, &["type", "delta", "usage"])
                        || !keys(&e["delta"], &["stop_reason", "stop_sequence"])
                        || e["delta"]["stop_reason"] != "end_turn"
                        || !null(&e["delta"], "stop_sequence")
                        || !usage(&e["usage"])
                    {
                        return Err(Rejection::Failed);
                    }
                    self.delta = true;
                    self.final_usage = Some(e["usage"].clone());
                }
                Some("message_stop") if self.delta && !self.message_stop => {
                    if !keys(e, &["type"]) {
                        return Err(Rejection::Malformed);
                    }
                    self.message_stop = true;
                }
                _ => return Err(Rejection::Malformed),
            }
            Ok((None, false))
        }
    }
    fn message(m: &Value) -> bool {
        // Chat-completions does not emit ResponseStarted; this source generation
        // uses the CLI/config alias as frame_model. Alias and upstream ID must be
        // the same fixed value so partial/canonical frames and modelUsage agree.
        keys(
            m,
            &[
                "id",
                "type",
                "role",
                "model",
                "content",
                "stop_reason",
                "stop_sequence",
                "usage",
            ],
        ) && m["type"] == "message"
            && m["role"] == "assistant"
            && m["model"] == "grok-4.6"
            && m["id"].as_str().is_some_and(|v| {
                !v.is_empty()
                    && v.len() <= 200
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            })
            && usage(&m["usage"])
    }

    #[cfg(unix)]
    fn prepare_pipe<R: std::os::fd::AsRawFd>(pipe: &R) -> Result<(), String> {
        let fd = pipe.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err("Cannot bound Grok reader shutdown.".into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    fn prepare_pipe<R>(_pipe: &R) -> Result<(), String> {
        Err(isolation())
    }
    fn publish_frame(
        sender: &mpsc::SyncSender<Vec<u8>>,
        mut frame: Vec<u8>,
        bad: &AtomicBool,
        stop: &AtomicBool,
    ) -> bool {
        loop {
            if stop.load(Ordering::SeqCst) || bad.load(Ordering::SeqCst) {
                return false;
            }
            match sender.try_send(frame) {
                Ok(()) => return true,
                Err(mpsc::TrySendError::Full(returned)) => {
                    frame = returned;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    bad.store(true, Ordering::SeqCst);
                    return false;
                }
            }
        }
    }
    fn stdout_reader<R: Read + Send + 'static>(
        mut pipe: R,
        sender: mpsc::SyncSender<Vec<u8>>,
        bad: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
    ) -> JoinHandle<bool> {
        thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            let mut pending = Vec::new();
            let mut total = 0usize;
            loop {
                match pipe.read(&mut bytes) {
                    Ok(0) => {
                        if !pending.is_empty() {
                            bad.store(true, Ordering::SeqCst);
                            return false;
                        }
                        return true;
                    }
                    Ok(n) => {
                        total = total.saturating_add(n);
                        if total > MAX_WIRE {
                            bad.store(true, Ordering::SeqCst);
                            return false;
                        }
                        for segment in bytes[..n].split_inclusive(|b| *b == b'\n') {
                            if pending.len().saturating_add(segment.len()) > MAX_FRAME {
                                bad.store(true, Ordering::SeqCst);
                                return false;
                            }
                            pending.extend_from_slice(segment);
                            if segment.last() == Some(&b'\n')
                                && !publish_frame(
                                    &sender,
                                    std::mem::take(&mut pending),
                                    &bad,
                                    &stop,
                                )
                            {
                                return false;
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if stop.load(Ordering::SeqCst) {
                            return false;
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        bad.store(true, Ordering::SeqCst);
                        return false;
                    }
                }
            }
        })
    }
    fn stderr_reader<R: Read + Send + 'static>(
        mut pipe: R,
        bad: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
    ) -> JoinHandle<bool> {
        thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            let mut total = 0usize;
            loop {
                match pipe.read(&mut bytes) {
                    Ok(0) => return true,
                    Ok(n) => {
                        total = total.saturating_add(n);
                        if total > MAX_WIRE {
                            bad.store(true, Ordering::SeqCst);
                            return false;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if stop.load(Ordering::SeqCst) {
                            return false;
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        bad.store(true, Ordering::SeqCst);
                        return false;
                    }
                }
            }
        })
    }
    /// The prompt is a private argv value. Close stdin immediately: do not run the
    /// generic input writer and duplicate it. Only final text enters the logical
    /// handoff. Reasoning remains in the owned native session history and this
    /// attempt's separate validation buffers; native reasoning continuity is not a
    /// supported managed-text feature.
    pub(crate) fn drive(
        mut child: OwnedChild,
        controller: Controller,
        cancel: &Arc<AtomicBool>,
        mut checkpoint: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<Outcome, String> {
        drop(child.stdin.take());
        let stdout = child
            .stdout
            .take()
            .ok_or("Grok did not provide owned stdout.")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Grok did not provide owned stderr.")?;
        let bad = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        prepare_pipe(&stdout)?;
        prepare_pipe(&stderr)?;
        let (sender, receiver) = mpsc::sync_channel(16);
        let stdout = stdout_reader(stdout, sender, bad.clone(), stop.clone());
        let stderr = stderr_reader(stderr, bad.clone(), stop.clone());
        let mut progress = Progress::new(&controller);
        let mut failed = false;
        let mut effect = false;
        let mut eof = false;
        let mut natural = false;
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if cancel.load(Ordering::SeqCst)
                || bad.load(Ordering::SeqCst)
                || Instant::now() >= deadline
            {
                failed = true;
                break;
            }
            if eof {
                match super::super::runtime::exit_pending(&child) {
                    Ok(true) => {
                        natural = true;
                        break;
                    }
                    Ok(false) => thread::sleep(Duration::from_millis(25)),
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
                continue;
            }
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(line) => {
                    let value: Value = match serde_json::from_slice(&line) {
                        Ok(value) => value,
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    };
                    match progress.event(&value) {
                        Ok((Some(_), _)) => {
                            if checkpoint(&progress.output).is_err() {
                                failed = true;
                                break;
                            }
                        }
                        Ok((None, _)) => {}
                        Err(rejection) => {
                            effect = rejection == Rejection::Effect;
                            failed = true;
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => eof = true,
            }
        }
        // WNOWAIT above observes natural exit. Terminate the process group before
        // the only leader reap on success, errors, EOF, timeout and callback failure.
        let status = child.stop_and_wait();
        drop(child);
        stop.store(true, Ordering::SeqCst);
        let stdout = stdout.join().unwrap_or(false);
        let stderr = stderr.join().unwrap_or(false);
        let drained = status.is_ok() && stdout && stderr;
        let completed = !failed
            && natural
            && eof
            && drained
            && status.is_ok_and(|s| s.success())
            && progress.completed()
            && !bad.load(Ordering::SeqCst)
            && !cancel.load(Ordering::SeqCst);
        Ok(Outcome {
            completed,
            exhausted: false,
            auth_failed: false,
            effects: effect,
            stopped: cancel.load(Ordering::SeqCst),
            drained,
            valid: completed,
            output: progress.output,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn bounded_reader_preserves_bursts_and_cancelled_backpressure_terminates() {
            let lines = (0..128)
                .map(|i| format!("{{\"n\":{i}}}\n"))
                .collect::<String>();
            let (sender, receiver) = mpsc::sync_channel(1);
            let bad = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let reader = stdout_reader(
                std::io::Cursor::new(lines.as_bytes().to_vec()),
                sender,
                bad.clone(),
                stop,
            );
            let mut received = Vec::new();
            while let Ok(frame) = receiver.recv() {
                received.extend(frame);
            }
            assert!(reader.join().unwrap());
            assert_eq!(received, lines.as_bytes());
            assert!(!bad.load(Ordering::SeqCst));
            let (sender, _receiver) = mpsc::sync_channel(1);
            sender.send(b"first".to_vec()).unwrap();
            let stop = AtomicBool::new(true);
            assert!(!publish_frame(&sender, b"second".to_vec(), &bad, &stop));
            assert!(!bad.load(Ordering::SeqCst));
        }
        fn parser() -> Progress {
            Progress::new(&Controller {
                root: PathBuf::from("/owned"),
                session: "11111111-1111-4111-8111-111111111111".into(),
                prompt: "p".into(),
            })
        }
        fn line(p: &Progress, n: u32, event: Value) -> Value {
            json!({"type":"stream_event","session_id":p.session,"uuid":format!("00000000-0000-4000-8000-{n:012x}"),"parent_tool_use_id":null,"event":event})
        }
        #[test]
        fn published_semver_cannot_enable_unqualified_native_binary() {
            assert!(!artifact_available());
            assert!(!version_matches(b"grok 1.0.45\n"));
            assert!(!version_matches(b"grok 1.0.46\n"));
        }
        #[test]
        fn native_config_alias_matches_partial_canonical_and_usage_model() {
            let config = settings().parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(config["models"]["default"].as_str(), Some("grok-4.6"));
            assert_eq!(
                config["models"]["session_summary"].as_str(),
                Some("grok-4.6")
            );
            assert_eq!(
                config["model"]["grok-4.6"]["model"].as_str(),
                Some("grok-4.6")
            );
            assert_eq!(config["model"].as_table().unwrap().len(), 1);
        }
        #[test]
        fn managed_prompt_cannot_resolve_native_slash_controls() {
            assert!(input_admission(" \n/compact").is_err());
            assert!(input_admission("/loop explain").is_err());
            assert!(input_admission("Explain /compact as text").is_ok());
            assert!(input_admission(&"x".repeat(MAX_INPUT + 1)).is_err());
        }
        #[test]
        fn reasoning_is_bounded_and_never_answer_text() {
            let mut p = parser();
            p.init = true;
            p.message = Some("msg_1".into());
            let start = line(
                &p,
                1,
                json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            );
            assert_eq!(p.event(&start), Ok((None, false)));
            let delta = line(
                &p,
                2,
                json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reason"}}),
            );
            assert_eq!(p.event(&delta), Ok((None, false)));
            assert_eq!(p.reasoning, "reason");
            assert_eq!(p.output, "");
            assert!(p.event(&delta).is_err());
        }
        #[test]
        fn tool_and_nondefinitive_stop_are_rejected() {
            let mut p = parser();
            p.init = true;
            p.message = Some("m".into());
            let tool = line(
                &p,
                1,
                json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"Shell","input":{}}}),
            );
            assert_eq!(p.event(&tool), Err(Rejection::Effect));
            let mut p = parser();
            p.init = true;
            p.message = Some("m".into());
            let truncated = line(
                &p,
                1,
                json!({"type":"message_delta","delta":{"stop_reason":"max_tokens","stop_sequence":null},"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}),
            );
            assert_eq!(p.event(&truncated), Err(Rejection::Failed));
        }
        #[test]
        fn canonical_reasoning_text_and_terminal_result_must_agree() {
            let mut p = parser();
            p.init = true;
            let usage = json!({"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0});
            let message = json!({"id":"msg_1","type":"message","role":"assistant","model":"grok-4.6","content":[],"stop_reason":null,"stop_sequence":null,"usage":usage});
            let events = [
                json!({"type":"message_start","message":message}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reason"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"answer"}}),
                json!({"type":"content_block_stop","index":1}),
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":usage}),
                json!({"type":"message_stop"}),
            ];
            for (i, event) in events.into_iter().enumerate() {
                let v = line(&p, i as u32 + 1, event);
                assert!(p.event(&v).is_ok());
            }
            assert!(!p.completed());
            let canonical = json!({"type":"assistant","session_id":p.session,"uuid":"00000000-0000-4000-8000-00000000000a","parent_tool_use_id":null,
            "message":{"id":"msg_1","type":"message","role":"assistant","model":"grok-4.6","content":[{"type":"thinking","thinking":"reason","signature":""},{"type":"text","text":"answer"}],"stop_reason":"end_turn","stop_sequence":null,"usage":usage}});
            assert_eq!(p.event(&canonical), Ok((None, false)));
            assert!(!p.completed());
            let result = json!({"type":"result","subtype":"success","is_error":false,"duration_ms":1,"duration_api_ms":1,"num_turns":1,"result":"answer","stop_reason":"end_turn","total_cost_usd":0.0,"usage":usage,
            "modelUsage":{"grok-4.6":{"inputTokens":1,"outputTokens":2,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.0}},"session_id":p.session,"uuid":"00000000-0000-4000-8000-00000000000b"});
            assert_eq!(p.event(&result), Ok((None, true)));
            assert!(p.completed());
            assert_eq!(p.output, "answer");
            assert_eq!(p.reasoning, "reason");
            assert!(p.event(&canonical).is_err());
            assert!(!p.completed());
        }
    }
}
