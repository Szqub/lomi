//! Native @moonshot-ai/kimi-code 2.1.1, f67e6398fb3210ad8ace970e2dfd5bcc984ed61f.
//! Public foreground web launcher, PRIVATE pinned REST/WS protocol 2. The caller
//! clears the inherited environment and injects only LOMI_KIMI_API_KEY. Never
//! expose readiness diagnostics, private bearer tokens or provider errors.
//! Source authority: apps/kimi-code/src/cli/sub/web/run.ts; kap-server routes,
//! protocol/{events-zod,ws-control}.ts, ws/v1/{sessionEventBroadcaster,
//! inFlightTurnTracker}.ts; agent-core-v2 toolPolicy/evaluate.ts and config sections.
//! HTTP admission is 200/code=0 at this pin, never evidence of completion.
//! Native Moonshot international Open Platform only: kimi-k2.6 supports disabled
//! thinking. llm-kimi/trait.ts encodes off as {thinking:{type:"disabled"}};
//! requester/bases/openai/requester.ts sets SDK maxRetries=0. Capabilities=[]
//! does not erase inferred capabilities; tool policy and explicit thinking off
//! are the actual fences. The experimental SDK, compaction and private watcher
//! lifecycle are source-qualified, not execution-qualified.

use super::{runtime::OwnedChild, transport::Outcome};
use futures_util::{SinkExt, StreamExt};
use reqwest::{
    header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE},
    Client, Method,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest,
        protocol::{Message, WebSocketConfig},
    },
    MaybeTlsStream, WebSocketStream,
};
use zeroize::Zeroizing;

pub(crate) const VERSION: &str = "2.1.1";
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_FRAME: usize = 6 * MAX_OUTPUT + 8192;
const MAX_WIRE: usize = 32 * 1024 * 1024;
const MODELS: &[&str] = &["kimi-k2.6"];
pub(crate) fn supported_models() -> &'static [&'static str] {
    MODELS
}
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
fn failure() -> String {
    "Kimi violated the owned native text-turn contract.".into()
}
pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    value.filter(|value| MODELS.contains(value)).ok_or_else(|| {
        "Kimi requires the approved native Moonshot text model. No request was sent.".into()
    })
}
pub(crate) fn version_matches(value: &[u8]) -> bool {
    std::str::from_utf8(value)
        .ok()
        .is_some_and(|value| value.trim() == VERSION)
}
fn keys(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|value| value.keys().all(|key| allowed.contains(&key.as_str())))
}
fn empty(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn text(parts: &Value) -> Result<String, String> {
    let mut result = String::new();
    for part in parts.as_array().ok_or_else(failure)? {
        if !keys(part, &["type", "text"]) || part["type"] != "text" {
            return Err(failure());
        }
        result.push_str(part["text"].as_str().ok_or_else(failure)?);
        if result.len() > MAX_OUTPUT {
            return Err(failure());
        }
    }
    Ok(result)
}
fn settings(id: &str) -> String {
    format!(
        r#"default_model = "lomi-text"
default_provider = "lomi"
telemetry = false
builtin_product_skills = false
merge_all_available_skills = false
extra_skill_dirs = []
extra_agent_dirs = []
hooks = []
default_permission_mode = "manual"
default_plan_mode = false
[providers.lomi]
type = "kimi"
model_source = "static"
base_url = "https://api.moonshot.ai/v1"
api_key_env = "LOMI_KIMI_API_KEY"
[models.lomi-text]
provider = "lomi"
model = "{id}"
protocol = "openai"
max_context_size = 262144
capabilities = []
[thinking]
enabled = false
[tools]
enabled = ["*"]
[loop_control]
max_steps_per_turn = 1
max_attempts_per_step = 1
compaction_max_attempts = 1
[model_catalog]
refresh_on_start = false
refresh_interval_ms = 0
[watch]
enabled = false
"#
    )
}
pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
    let id = model(Some(id))?;
    let root = parent.join(format!("kimi-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| failure())?;
    crate::chat::storage::private(&root, true)?;
    let root = root.canonicalize().map_err(|_| failure())?;
    for name in [
        ".git",
        ".kimi-code",
        ".kimi-code/plugins",
        ".kimi-code/skills",
        "config",
        "data",
        "state",
        "cache",
        "tmp",
    ] {
        let path = root.join(name);
        fs::create_dir(&path).map_err(|_| failure())?;
        crate::chat::storage::private(&path, true)?;
    }
    crate::chat::storage::atomic(
        &root.join(".kimi-code/config.toml"),
        settings(id).as_bytes(),
    )?;
    crate::chat::storage::atomic(
        &root.join(".kimi-code/plugins/installed.json"),
        b"{\"version\":1,\"plugins\":[]}\n",
    )?;
    Ok(root)
}
pub(crate) fn environment(command: &mut Command, root: &Path, _id: &str) {
    command
        .current_dir(root)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("KIMI_CODE_HOME", root.join(".kimi-code"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        .env("KIMI_CODE_BUILTIN_PRODUCT_SKILLS", "false")
        .env("KIMI_DISABLE_TELEMETRY", "1")
        .env("KIMI_CODE_MODEL_CATALOG_REFRESH_ON_START", "false")
        .env("KIMI_CODE_MODEL_CATALOG_REFRESH_INTERVAL_MS", "0");
}
pub(crate) struct Controller {
    root: PathBuf,
    id: String,
    prompt: String,
    prompt_id: String,
    client_id: String,
    token: Zeroizing<String>,
}
impl Controller {
    pub(crate) fn new(root: &Path, id: &str, prompt: &str) -> Result<Self, String> {
        let id = model(Some(id))?;
        if prompt.is_empty()
            || prompt.len() > MAX_OUTPUT
            || fs::symlink_metadata(root.join(".kimi-code/server.token")).is_ok()
        {
            return Err(failure());
        }
        let token = Zeroizing::new(format!("{}{}", super::new_id()?, super::new_id()?));
        crate::chat::storage::atomic(&root.join(".kimi-code/server.token"), token.as_bytes())?;
        Ok(Self {
            root: root.to_owned(),
            id: id.into(),
            prompt: prompt.into(),
            prompt_id: super::new_id()?,
            client_id: super::new_id()?,
            token,
        })
    }
}
pub(crate) fn arguments(command: &mut Command, _root: &Path, _id: &str, _controller: &Controller) {
    command.args([
        "web",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--no-open",
        "--log-level",
        "error",
    ]);
}
fn ready(line: &[u8], token: &str) -> Option<u16> {
    let line = std::str::from_utf8(line)
        .ok()?
        .strip_suffix('\n')?
        .trim_end_matches('\r');
    let rest = line.strip_prefix("Kimi server: http://127.0.0.1:")?;
    let (port, actual) = rest.split_once("/#token=")?;
    if actual != token || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    port.parse().ok().filter(|port| *port != 0)
}
fn diagnostics<R: Read + Send + 'static>(
    pipe: R,
    token: Zeroizing<String>,
    sender: mpsc::SyncSender<u16>,
    bad: Arc<AtomicBool>,
    stdout: bool,
) -> JoinHandle<bool> {
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut total = 0usize;
        let mut seen = false;
        loop {
            let mut line = Vec::new();
            match (&mut reader).take(65537).read_until(b'\n', &mut line) {
                Ok(0) => return !stdout || seen,
                Ok(_) => {
                    total = total.saturating_add(line.len());
                    if line.len() > 65536 || total > MAX_WIRE {
                        bad.store(true, Ordering::SeqCst);
                        return false;
                    }
                    if stdout {
                        if let Some(port) = ready(&line, &token) {
                            if seen || sender.try_send(port).is_err() {
                                bad.store(true, Ordering::SeqCst);
                                return false;
                            }
                            seen = true;
                        } else if !line.iter().all(u8::is_ascii_whitespace) {
                            bad.store(true, Ordering::SeqCst);
                            return false;
                        }
                    }
                }
                Err(_) => {
                    bad.store(true, Ordering::SeqCst);
                    return false;
                }
            }
        }
    })
}
struct Http {
    client: Client,
    base: String,
}
impl Http {
    fn new(port: u16, token: &str) -> Result<Self, String> {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| failure())?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            CACHE_CONTROL,
            reqwest::header::HeaderValue::from_static("no-store"),
        );
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| failure())?;
        Ok(Self {
            client,
            base: format!("http://127.0.0.1:{port}/api/v1"),
        })
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut req = self.client.request(method, format!("{}{path}", self.base));
        if let Some(body) = body {
            req = req
                .header(CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(&body).map_err(|_| failure())?);
        }
        let mut response = req.send().await.map_err(|_| failure())?;
        if response.status().as_u16() != 200
            || response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                != Some("application/json")
            || response
                .content_length()
                .is_some_and(|n| n > MAX_FRAME as u64)
        {
            return Err(failure());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
            if bytes.len().saturating_add(chunk.len()) > MAX_FRAME {
                return Err(failure());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| failure())?;
        if !keys(&value, &["code", "msg", "data", "request_id"])
            || value["code"] != 0
            || value["msg"].as_str().is_none()
            || value["request_id"].as_str().is_none()
            || value["data"].is_null()
        {
            return Err(failure());
        }
        Ok(value["data"].clone())
    }
    async fn get(&self, path: &str) -> Result<Value, String> {
        self.request(Method::GET, path, None).await
    }
    async fn post(&self, path: &str, value: Value) -> Result<Value, String> {
        self.request(Method::POST, path, Some(value)).await
    }
}
fn config(value: &Value, id: &str) -> bool {
    value["default_model"] == "lomi-text"
        && value["default_provider"] == "lomi"
        && value["telemetry"] == false
        && value["builtin_product_skills"] == false
        && value["merge_all_available_skills"] == false
        && empty(&value["extra_skill_dirs"])
        && empty(&value["hooks"])
        && value["tools"]["enabled"] == json!(["*"])
        && value["loop_control"]["maxStepsPerTurn"] == 1
        && value["loop_control"]["maxAttemptsPerStep"] == 1
        && value["loop_control"]["compactionMaxAttempts"] == 1
        && value["model_catalog"]["refreshOnStart"] == false
        && value["model_catalog"]["refreshIntervalMs"] == 0
        && value["watch"]["enabled"] == false
        && keys(&value["providers"], &["lomi"])
        && value["providers"]["lomi"]["type"] == "kimi"
        && value["providers"]["lomi"]["base_url"] == "https://api.moonshot.ai/v1"
        && value["providers"]["lomi"]["api_key_env"] == "LOMI_KIMI_API_KEY"
        && value["providers"]["lomi"]["has_api_key"] == true
        && keys(&value["models"], &["lomi-text"])
        && value["models"]["lomi-text"]["provider"] == "lomi"
        && value["models"]["lomi-text"]["model"] == id
        && value["models"]["lomi-text"]["protocol"] == "openai"
        && value["models"]["lomi-text"]["maxContextSize"] == 262144
        && empty(&value["models"]["lomi-text"]["capabilities"])
        && value["thinking"]["enabled"] == false
}
fn identity(meta: &Value) -> bool {
    meta["server_version"] == VERSION
        && meta["backend"] == "v2"
        && meta["dangerous_bypass_auth"] == false
        && meta["server_id"].as_str().is_some_and(identifier)
        && meta["started_at"].as_str().is_some_and(|v| !v.is_empty())
}
fn quiescent(snapshot: &Value, session: &str, root: &Path) -> bool {
    let state = &snapshot["session"];
    state["id"] == session
        && state["metadata"]["cwd"].as_str() == root.to_str()
        && state["agent_config"]["model"] == "lomi-text"
        && state["busy"] == false
        && state["main_turn_active"] == false
        && state["pending_interaction"] == "none"
        && state["current_prompt_id"].is_null()
        && snapshot["in_flight_turn"].is_null()
        && empty(&snapshot["subagents"])
        && empty(&snapshot["pending_approvals"])
        && empty(&snapshot["pending_questions"])
        && snapshot["messages"]["has_more"] == false
}
async fn fences(http: &Http, session: &str) -> Result<(), String> {
    if !empty(&http.get("/plugins").await?["plugins"])
        || !empty(&http.get("/mcp/servers").await?["servers"])
        || !empty(&http.get(&format!("/sessions/{session}/tasks")).await?["items"])
    {
        return Err(failure());
    }
    let tools = http.get(&format!("/tools?session_id={session}")).await?;
    if !tools["tools"]
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item["active"] == false))
    {
        return Err(failure());
    }
    Ok(())
}
#[derive(Default)]
struct Progress {
    seq: u64,
    epoch: String,
    session: String,
    prompt_id: String,
    prompt: String,
    turn: Option<u64>,
    started: bool,
    step_done: bool,
    ended: bool,
    submitted: bool,
    prompt_started: bool,
    settled: bool,
    context_count: usize,
    output: String,
    utf16: usize,
}
impl Progress {
    fn event(&mut self, value: &Value) -> Result<bool, String> {
        if !keys(
            value,
            &[
                "type",
                "seq",
                "epoch",
                "volatile",
                "offset",
                "session_id",
                "timestamp",
                "payload",
            ],
        ) || value["session_id"] != self.session
            || value["epoch"] != self.epoch
            || value["timestamp"].as_str().is_none()
        {
            return Err(failure());
        }
        let typ = value["type"].as_str().ok_or_else(failure)?;
        let seq = value["seq"].as_u64().ok_or_else(failure)?;
        let volatile = value["volatile"] == true;
        if volatile {
            if seq != self.seq || !matches!(typ, "assistant.delta" | "agent.status.updated") {
                return Err(failure());
            }
        } else {
            if seq != self.seq.checked_add(1).ok_or_else(failure)? || value.get("offset").is_some()
            {
                return Err(failure());
            }
            self.seq = seq;
        }
        let p = &value["payload"];
        if p["type"] != typ || p["sessionId"] != self.session || p["agentId"] != "main" {
            return Err(failure());
        }
        let fields: &[&str] = match typ {
            "prompt.submitted" => &[
                "promptId",
                "userMessageId",
                "status",
                "content",
                "createdAt",
                "clientMetadata",
            ],
            "prompt.started" => &["promptId"],
            "prompt.completed" => &["promptId", "finishedAt", "reason"],
            "turn.started" => &["turnId", "origin", "prompt", "promptId"],
            "turn.step.started" => &["turnId", "step", "stepId"],
            "assistant.delta" => &["turnId", "delta"],
            "turn.step.completed" => &[
                "turnId",
                "step",
                "stepId",
                "usage",
                "finishReason",
                "llmFirstTokenLatencyMs",
                "llmStreamDurationMs",
                "llmRequestBuildMs",
                "llmServerFirstTokenMs",
                "llmServerDecodeMs",
                "llmClientConsumeMs",
                "llmClientBlockedMs",
                "providerFinishReason",
                "rawFinishReason",
            ],
            "turn.ended" => &[
                "turnId",
                "reason",
                "error",
                "durationMs",
                "interruptReason",
                "stopReason",
                "traceId",
            ],
            "context.spliced" => &["start", "deleteCount", "messages", "tokens"],
            "session.meta.updated" => &["title", "patch"],
            "agent.status.updated" => &[
                "model",
                "thinkingEffort",
                "contextTokens",
                "maxContextTokens",
                "contextUsage",
                "planMode",
                "swarmMode",
                "towerMode",
                "permission",
                "usage",
                "phase",
            ],
            "event.session.work_changed" => &[
                "busy",
                "main_turn_active",
                "pending_interaction",
                "last_turn_reason",
            ],
            "event.session.status_changed" => &["status", "previous_status", "current_prompt_id"],
            _ => return Err(failure()),
        };
        if !p.as_object().is_some_and(|map| {
            map.keys().all(|key| {
                ["type", "agentId", "sessionId", "time"].contains(&key.as_str())
                    || fields.contains(&key.as_str())
            })
        }) {
            return Err(failure());
        }
        let own_turn = self.turn.is_some() && p["turnId"].as_u64() == self.turn;
        match typ {
            "prompt.submitted" if !volatile && !self.submitted && !self.ended => {
                if p["promptId"] != self.prompt_id
                    || p["userMessageId"] != self.prompt_id
                    || p["status"] != "running"
                    || text(&p["content"])? != self.prompt
                {
                    return Err(failure());
                }
                self.submitted = true;
            }
            "prompt.started"
                if !volatile && self.submitted && !self.prompt_started && !self.ended =>
            {
                if p["promptId"] != self.prompt_id {
                    return Err(failure());
                }
                self.prompt_started = true;
            }
            "turn.started" if !volatile && self.submitted && self.turn.is_none() => {
                if p["promptId"] != self.prompt_id
                    || p["prompt"] != self.prompt
                    || p["origin"]["kind"] != "user"
                    || !keys(
                        &p["origin"],
                        &["kind", "attachments", "clientMetadata", "skillActivations"],
                    )
                    || p["origin"]
                        .get("skillActivations")
                        .is_some_and(|v| !empty(v))
                {
                    return Err(failure());
                }
                self.turn = Some(p["turnId"].as_u64().ok_or_else(failure)?);
            }
            "turn.step.started" if !volatile && own_turn && !self.started && !self.ended => {
                if p["step"] != 1 {
                    return Err(failure());
                }
                self.started = true;
            }
            "assistant.delta"
                if volatile && own_turn && self.started && !self.step_done && !self.ended =>
            {
                if value["offset"].as_u64() != Some(self.utf16 as u64) {
                    return Err(failure());
                }
                let delta = p["delta"].as_str().ok_or_else(failure)?;
                if self.output.len().saturating_add(delta.len()) > MAX_OUTPUT {
                    return Err(failure());
                }
                self.output.push_str(delta);
                self.utf16 += delta.encode_utf16().count();
                return Ok(!delta.is_empty());
            }
            "turn.step.completed"
                if !volatile && own_turn && self.started && !self.step_done && !self.ended =>
            {
                if p["providerFinishReason"] != "completed" || p["step"] != 1 {
                    return Err(failure());
                }
                self.step_done = true;
            }
            "turn.ended" if !volatile && own_turn && self.step_done && !self.ended => {
                if p["reason"] != "completed"
                    || p.get("error").is_some()
                    || p.get("interruptReason").is_some()
                    || p.get("stopReason").is_some()
                    || self.output.trim().is_empty()
                {
                    return Err(failure());
                }
                self.ended = true;
            }
            "prompt.completed" if !volatile && self.ended && !self.settled => {
                if p["promptId"] != self.prompt_id
                    || p["reason"] != "completed"
                    || p["finishedAt"].as_str().is_none()
                {
                    return Err(failure());
                }
                self.settled = true;
            }
            "session.meta.updated" if !volatile && !self.ended => {
                if !keys(
                    p,
                    &["type", "title", "patch", "agentId", "sessionId", "time"],
                ) || !keys(&p["patch"], &["title", "isCustomTitle", "lastPrompt"])
                    || p["patch"]["lastPrompt"] != self.prompt
                    || p.get("title").is_some()
                    || p["patch"].get("title").is_some()
                    || p["patch"].get("isCustomTitle").is_some()
                {
                    return Err(failure());
                }
            }
            "context.spliced" if !volatile && !self.ended => {
                if p["start"].as_u64() != Some(self.context_count as u64) || p["deleteCount"] != 0 {
                    return Err(failure());
                }
                for msg in p["messages"].as_array().ok_or_else(failure)? {
                    let role = if self.context_count == 0 {
                        "user"
                    } else {
                        "assistant"
                    };
                    let expected = if self.context_count == 0 {
                        &self.prompt
                    } else {
                        &self.output
                    };
                    if self.context_count >= 2
                        || msg["role"] != role
                        || text(&msg["content"])? != *expected
                        || !empty(&msg["toolCalls"])
                    {
                        return Err(failure());
                    }
                    self.context_count += 1;
                }
            }
            "agent.status.updated" if volatile => {
                if p.get("model").is_some_and(|v| v != "lomi-text")
                    || p.get("planMode").is_some_and(|v| v != false)
                    || p.get("swarmMode").is_some_and(|v| v != false)
                    || p.get("towerMode").is_some_and(|v| v != false)
                    || p.get("thinkingEffort")
                        .is_some_and(|v| v != "off" && v != "none" && v != "")
                {
                    return Err(failure());
                }
                if let Some(phase) = p.get("phase") {
                    let kind = phase["kind"].as_str().ok_or_else(failure)?;
                    let phase_turn = self.turn.is_some() && phase["turnId"].as_u64() == self.turn;
                    if !matches!(kind, "idle" | "running" | "ended")
                        || (kind == "running" && !phase_turn)
                        || (kind == "ended" && (!phase_turn || phase["reason"] != "completed"))
                    {
                        return Err(failure());
                    }
                }
            }
            "event.session.work_changed" if !volatile => {
                if p.get("pending_interaction").is_some_and(|v| v != "none")
                    || p.get("last_turn_reason").is_some_and(|v| v != "completed")
                {
                    return Err(failure());
                }
            }
            "event.session.status_changed" if !volatile => {
                if !matches!(p["status"].as_str(), Some("idle" | "running"))
                    || p.get("current_prompt_id")
                        .is_some_and(|v| v != &self.prompt_id)
                {
                    return Err(failure());
                }
            }
            _ => return Err(failure()),
        }
        Ok(false)
    }
}
// wsConnectionV1.onHeartbeat sends JSON buildPing(ulid()), rather than a
// WebSocket control frame. Pong has no ack; failure to send it is a transport
// failure, and it never advances the subscribed session's event cursor.
fn heartbeat(value: &Value) -> Result<Option<Value>, String> {
    if value["type"] != "ping" {
        return Ok(None);
    }
    let timestamp = value["timestamp"].as_str().ok_or_else(failure)?;
    let nonce = value["payload"]["nonce"].as_str().ok_or_else(failure)?;
    if !keys(value, &["type", "timestamp", "payload"])
        || !keys(&value["payload"], &["nonce"])
        || timestamp.len() != 24
        || !timestamp.ends_with('Z')
        || chrono::DateTime::parse_from_rfc3339(timestamp).is_err()
        || nonce.len() != 26
        || !matches!(nonce.as_bytes().first(), Some(b'0'..=b'7'))
        || !nonce
            .bytes()
            .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
    {
        return Err(failure());
    }
    Ok(Some(json!({"type":"pong","payload":{"nonce":nonce}})))
}
async fn frame(
    socket: &mut Socket,
    total: &mut usize,
    cancel: &Arc<AtomicBool>,
    bad: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        if cancel.load(Ordering::SeqCst) || bad.load(Ordering::SeqCst) || Instant::now() >= deadline
        {
            return Err(failure());
        }
        let next = tokio::time::timeout(Duration::from_millis(100), socket.next()).await;
        let message = match next {
            Err(_) => continue,
            Ok(Some(Ok(message))) => message,
            _ => return Err(failure()),
        };
        match message {
            Message::Text(bytes) => {
                *total = total.saturating_add(bytes.len());
                if bytes.len() > MAX_FRAME || *total > MAX_WIRE {
                    return Err(failure());
                }
                let value: Value = serde_json::from_str(bytes.as_str()).map_err(|_| failure())?;
                if let Some(pong) = heartbeat(&value)? {
                    send(socket, pong).await?;
                    continue;
                }
                return Ok(value);
            }
            Message::Ping(payload) if payload.len() <= 125 => {
                *total = total.saturating_add(payload.len() + 2);
                if *total > MAX_WIRE {
                    return Err(failure());
                }
                tokio::time::timeout(Duration::from_secs(2), socket.send(Message::Pong(payload)))
                    .await
                    .map_err(|_| failure())?
                    .map_err(|_| failure())?;
            }
            Message::Pong(payload) if payload.len() <= 125 => {
                *total = total.saturating_add(payload.len() + 2);
                if *total > MAX_WIRE {
                    return Err(failure());
                }
            }
            _ => return Err(failure()),
        }
    }
}
async fn send(socket: &mut Socket, value: Value) -> Result<(), String> {
    tokio::time::timeout(
        Duration::from_secs(2),
        socket.send(Message::Text(value.to_string().into())),
    )
    .await
    .map_err(|_| failure())?
    .map_err(|_| failure())
}
fn ack(value: &Value, id: &str) -> bool {
    keys(value, &["type", "id", "code", "msg", "payload"])
        && value["type"] == "ack"
        && value["id"] == id
        && value["code"] == 0
        && value["msg"].as_str().is_some()
}
async fn turn(
    http: &Http,
    controller: &Controller,
    session: &str,
    progress: &mut Progress,
    cancel: &Arc<AtomicBool>,
    bad: &Arc<AtomicBool>,
    checkpoint: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let mut request = http
        .base
        .replace("http://", "ws://")
        .replace("/api/v1", "/api/v1/ws")
        .into_client_request()
        .map_err(|_| failure())?;
    let mut auth = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&format!(
        "Bearer {}",
        *controller.token
    ))
    .map_err(|_| failure())?;
    auth.set_sensitive(true);
    request.headers_mut().insert("authorization", auth);
    let wsconfig = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (mut socket, response) = tokio::time::timeout(
        Duration::from_secs(5),
        connect_async_with_config(request, Some(wsconfig), true),
    )
    .await
    .map_err(|_| failure())?
    .map_err(|_| failure())?;
    if response.status().as_u16() != 101 {
        return Err(failure());
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut total = 0usize;
    let result = async {
        let hello = frame(&mut socket, &mut total, cancel, bad, deadline).await?;
        if !keys(&hello, &["type", "timestamp", "payload"]) || hello["type"] != "server_hello" || hello["payload"]["protocol_version"] != 2
            || hello["payload"]["heartbeat_ms"] != 10000
            || hello["payload"]["max_event_buffer_size"] != 1000 || hello["payload"]["capabilities"] != json!({"event_batching":false,"compression":false}) { return Err(failure()); }
        let hello_id = super::new_id()?;
        send(&mut socket,json!({"type":"client_hello","id":hello_id,"payload":{"client_id":controller.client_id}})).await?;
        let response = frame(&mut socket,&mut total,cancel,bad,deadline).await?;
        if !ack(&response,&hello_id) || !empty(&response["payload"]["accepted_subscriptions"]) || !empty(&response["payload"]["resync_required"]) { return Err(failure()); }
        let subscribe_id = super::new_id()?;
        let mut cursors = serde_json::Map::new(); cursors.insert(session.into(),json!({"seq":progress.seq,"epoch":progress.epoch}));
        send(&mut socket,json!({"type":"subscribe","id":subscribe_id,"payload":{"session_ids":[session],"cursors":cursors}})).await?;
        let response = frame(&mut socket,&mut total,cancel,bad,deadline).await?;
        if !ack(&response,&subscribe_id) || response["payload"]["accepted"] != json!([session]) || !empty(&response["payload"]["not_found"]) || !empty(&response["payload"]["resync_required"]) { return Err(failure()); }
        if cancel.load(Ordering::SeqCst) || bad.load(Ordering::SeqCst) { return Err(failure()); }
        let admitted = http.post(&format!("/sessions/{session}/prompts"),json!({"prompt_id":controller.prompt_id,"content":[{"type":"text","text":controller.prompt}],"model":"lomi-text","thinking":"off","plan_mode":false,"swarm_mode":false,"permission_mode":"manual"})).await?;
        if admitted["prompt_id"] != controller.prompt_id || admitted["user_message_id"] != controller.prompt_id || admitted["status"] != "running" || text(&admitted["content"])? != controller.prompt { return Err(failure()); }
        while !progress.settled {
            let event = frame(&mut socket,&mut total,cancel,bad,deadline).await?;
            if progress.event(&event)? { checkpoint(&progress.output)?; }
        }
        Ok(())
    }.await;
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
    result
}
fn reconcile(snapshot: &Value, progress: &Progress, controller: &Controller) -> Result<(), String> {
    if !quiescent(snapshot, &progress.session, &controller.root)
        || snapshot["epoch"] != progress.epoch
        || snapshot["as_of_seq"]
            .as_u64()
            .is_none_or(|v| v < progress.seq)
        || snapshot["session"]["last_turn_reason"] != "completed"
    {
        return Err(failure());
    }
    let items = snapshot["messages"]["items"]
        .as_array()
        .ok_or_else(failure)?;
    if items.len() != 2
        || items[0]["role"] != "user"
        || items[0]["id"] != controller.prompt_id
        || items[1]["role"] != "assistant"
        || items[0]["session_id"] != progress.session
        || items[1]["session_id"] != progress.session
        || text(&items[0]["content"])? != controller.prompt
        || text(&items[1]["content"])? != progress.output
        || items[0]["metadata"]["origin"]["kind"] != "user"
        || items[0]["metadata"]["origin"]
            .get("skillActivations")
            .is_some_and(|v| !empty(v))
        || items[1].get("metadata").is_some()
    {
        return Err(failure());
    }
    Ok(())
}
pub(crate) fn drive(
    mut child: OwnedChild,
    controller: Controller,
    cancel: &Arc<AtomicBool>,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<Outcome, String> {
    let bad = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::sync_channel(1);
    let stdout = child.stdout.take().ok_or_else(failure)?;
    let stderr = child.stderr.take().ok_or_else(failure)?;
    let stdout = diagnostics(
        stdout,
        Zeroizing::new(controller.token.to_string()),
        sender.clone(),
        bad.clone(),
        true,
    );
    let stderr = diagnostics(
        stderr,
        Zeroizing::new(String::new()),
        sender,
        bad.clone(),
        false,
    );
    let mut progress = Progress {
        prompt_id: controller.prompt_id.clone(),
        prompt: controller.prompt.clone(),
        ..Default::default()
    };
    let mut http = None;
    let mut confirmed = false;
    let mut shutdown = false;
    let result = tauri::async_runtime::block_on(async {
        let deadline = Instant::now() + Duration::from_secs(20);
        let port = loop {
            if cancel.load(Ordering::SeqCst)
                || bad.load(Ordering::SeqCst)
                || Instant::now() >= deadline
                || super::runtime::exit_pending(&child)?
            {
                return Err(failure());
            }
            if let Ok(port) = receiver.try_recv() {
                break port;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        http = Some(Http::new(port, &controller.token)?);
        let h = http.as_ref().ok_or_else(failure)?;
        let meta = h.get("/meta").await?;
        let initial_config = h.get("/config").await?;
        if !identity(&meta) || !config(&initial_config, &controller.id) {
            return Err(failure());
        }
        let created = h
            .post(
                "/sessions",
                json!({"title":"Lomi managed text","metadata":{"cwd":controller.root}}),
            )
            .await?;
        let session = created["id"]
            .as_str()
            .filter(|id| identifier(id))
            .ok_or_else(failure)?
            .to_owned();
        progress.session = session.clone();
        let initial = h.get(&format!("/sessions/{session}/snapshot")).await?;
        if !quiescent(&initial, &session, &controller.root) || !empty(&initial["messages"]["items"])
        {
            return Err(failure());
        }
        progress.seq = initial["as_of_seq"].as_u64().ok_or_else(failure)?;
        progress.epoch = initial["epoch"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(failure)?
            .into();
        fences(h, &session).await?;
        turn(
            h,
            &controller,
            &session,
            &mut progress,
            cancel,
            &bad,
            &mut checkpoint,
        )
        .await?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = h.get(&format!("/sessions/{session}/snapshot")).await?;
            if quiescent(&snapshot, &session, &controller.root) {
                reconcile(&snapshot, &progress, &controller)?;
                break;
            }
            if Instant::now() >= deadline {
                return Err(failure());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        fences(h, &session).await?;
        let prompts = h.get(&format!("/sessions/{session}/prompts")).await?;
        if !prompts["active"].is_null() || !empty(&prompts["queued"]) {
            return Err(failure());
        }
        let final_meta = h.get("/meta").await?;
        if !identity(&final_meta)
            || final_meta["server_id"] != meta["server_id"]
            || final_meta["started_at"] != meta["started_at"]
            || h.get("/config").await? != initial_config
        {
            return Err(failure());
        }
        confirmed = true;
        Ok(())
    });
    tauri::async_runtime::block_on(async {
        if let Some(h) = &http {
            if !confirmed && !progress.session.is_empty() {
                let _ = h
                    .post(&format!("/sessions/{}:abort", progress.session), json!({}))
                    .await;
                let deadline = Instant::now() + Duration::from_secs(3);
                while Instant::now() < deadline {
                    match h
                        .get(&format!("/sessions/{}/snapshot", progress.session))
                        .await
                    {
                        Ok(value) if quiescent(&value, &progress.session, &controller.root) => {
                            break
                        }
                        _ => tokio::time::sleep(Duration::from_millis(25)).await,
                    }
                }
            }
            shutdown = h
                .post("/shutdown", json!({}))
                .await
                .is_ok_and(|v| v["ok"] == true);
        }
        if shutdown {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline && !super::runtime::exit_pending(&child).unwrap_or(true)
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    });
    // Group termination precedes the only reap, including normal server exit.
    let reaped = child.stop_and_wait().is_ok();
    drop(child);
    let stdout = stdout.join().unwrap_or(false);
    let stderr = stderr.join().unwrap_or(false);
    let drained = reaped && stdout && stderr;
    let completed = result.is_ok()
        && confirmed
        && shutdown
        && drained
        && !bad.load(Ordering::SeqCst)
        && !cancel.load(Ordering::SeqCst);
    Ok(Outcome {
        completed,
        exhausted: false,
        auth_failed: false,
        effects: false,
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
    fn readiness_requires_private_token_and_loopback() {
        assert_eq!(
            ready(
                b"Kimi server: http://127.0.0.1:1234/#token=private\n",
                "private"
            ),
            Some(1234)
        );
        assert_eq!(
            ready(
                b"Kimi server: http://127.0.0.1:1234/#token=other\n",
                "private"
            ),
            None
        );
        assert_eq!(
            ready(
                b"Kimi server: http://localhost:1234/#token=private\n",
                "private"
            ),
            None
        );
    }
    #[test]
    fn delta_offsets_count_utf16_and_reject_gaps() {
        let mut p = Progress {
            seq: 3,
            epoch: "e".into(),
            session: "s".into(),
            turn: Some(1),
            started: true,
            output: "😀".into(),
            utf16: 2,
            ..Default::default()
        };
        let event = json!({"type":"assistant.delta","seq":3,"epoch":"e","session_id":"s","volatile":true,"offset":2,"timestamp":"t","payload":{"type":"assistant.delta","sessionId":"s","agentId":"main","turnId":1,"delta":"x"}});
        assert_eq!(p.event(&event), Ok(true));
        assert_eq!(p.output, "😀x");
        assert!(p.event(&event).is_err());
    }
    #[test]
    fn tool_content_and_generic_failures_are_rejected() {
        assert!(text(&json!([{"type":"tool_use","tool_name":"Shell","input":{}}])).is_err());
        let mut p = Progress {
            seq: 0,
            epoch: "e".into(),
            session: "s".into(),
            ..Default::default()
        };
        assert!(p.event(&json!({"type":"turn.ended","seq":1,"epoch":"e","session_id":"s","timestamp":"t","payload":{"type":"turn.ended","sessionId":"s","agentId":"main","turnId":1,"reason":"failed"}})).is_err());
    }
    #[test]
    fn native_json_heartbeat_replies_without_session_cursor() {
        let ping = json!({"type":"ping","timestamp":"2026-10-04T12:00:00.000Z","payload":{"nonce":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}});
        assert_eq!(
            heartbeat(&ping),
            Ok(Some(
                json!({"type":"pong","payload":{"nonce":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}})
            ))
        );
        let mut malformed = ping.clone();
        malformed["seq"] = json!(1);
        assert!(heartbeat(&malformed).is_err());
        let mut malformed = ping.clone();
        malformed["payload"]["session_id"] = json!("foreign");
        assert!(heartbeat(&malformed).is_err());
        assert_eq!(heartbeat(&json!({"type":"assistant.delta"})), Ok(None));
    }
}
