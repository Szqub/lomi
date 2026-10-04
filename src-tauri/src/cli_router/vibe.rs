//! Mistral Vibe 2.25.8, source commit 7c19608af06f6c61d63f8f7a5c3430da73fba2ab.
//! Public legacy app-server protocol; fresh, isolated OpenAI text turns only.
//! The owner clears the environment and supplies only the selected API key.

use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const VERSION: &str = "2.25.8";
const ALIAS: &str = "lomi-router-owned";
const PROMPT: &str = "You are a text-only assistant. Answer using only the supplied conversation. Tools, skills, filesystem access and project context are unavailable.";

const MODELS: &[&str] = &[
    "gpt-4.1",
    "gpt-4.1-mini",
    "gpt-4.1-nano",
    "gpt-4.1-2025-04-14",
    "gpt-4.1-mini-2025-04-14",
    "gpt-4.1-nano-2025-04-14",
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4o-2024-05-13",
    "gpt-4o-2024-08-06",
    "gpt-4o-2024-11-20",
    "gpt-4o-mini-2024-07-18",
];
pub(crate) fn supported_models() -> &'static [&'static str] {
    MODELS
}

pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    // The pinned generic backend sends temperature and chat-completions fields.
    // Reasoning models and effort suffixes need a separate qualified contract.
    value.filter(|id| MODELS.contains(id)).ok_or_else(|| "Vibe requires an explicitly supported OpenAI chat-completions model without an effort suffix. No request was sent.".into())
}

fn config(model_id: &str) -> String {
    format!(
        r#"active_model = "{ALIAS}"
allowed_models = ["{ALIAS}"]
default_agent = "ask"
disabled_tools = ["*"]
disabled_skills = ["*"]
disabled_agents = ["*"]
enabled_agents = ["ask"]
tool_paths = []
skill_paths = []
agent_paths = []
mcp_servers = []
connectors = []
enable_connectors = false
experimental_enable_registry_skills = false
include_project_context = false
include_prompt_detail = false
include_model_info = false
managed_shell_tools_enabled = false
bypass_tool_permissions = false
auto_compact_threshold = 0
raise_on_compaction_failure = true
enable_telemetry = false
enable_otel = false
enable_update_checks = false
enable_auto_update = false
enable_notifications = false
experimental_enable_tab_status = false
file_watcher_for_autocomplete = false
show_greeting = false
autocopy_to_clipboard = false
voice_mode_enabled = false
narrator_enabled = false
api_retry_max_elapsed_time = 300.0

[experiments]
enable = false

[session_logging]
enabled = false
generate_titles = false

[[providers]]
name = "router-openai"
api_base = "https://api.openai.com/v1"
api_key_env_var = "OPENAI_API_KEY"
api_style = "openai"
backend = "generic"
emits_finish_reason = true

[[models]]
name = "{model_id}"
alias = "{ALIAS}"
provider = "router-openai"
thinking = "off"
supports_images = false
auto_compact_threshold = 0
"#
    )
}

pub(crate) fn home(parent: &Path, model_id: &str) -> Result<PathBuf, String> {
    model(Some(model_id))?;
    let root = parent.join(format!("vibe-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| "Cannot create private Vibe attempt storage.")?;
    crate::chat::storage::private(&root, true)?;
    for name in [".vibe", "work"] {
        let directory = root.join(name);
        fs::create_dir(&directory).map_err(|_| "Cannot isolate Vibe attempt storage.")?;
        crate::chat::storage::private(&directory, true)?;
    }
    crate::chat::storage::atomic(&root.join(".vibe/config.toml"), config(model_id).as_bytes())?;
    // A fresh HOME excludes .agents, credentials, dotenv, plugins and hooks.
    // ProjectConfigLayer stops BEFORE VIBE_HOME.parent (root), so work cannot
    // discover a config in an ancestor of the private attempt directory.
    root.canonicalize()
        .map_err(|_| "Cannot resolve private Vibe storage.".into())
}

pub(crate) fn environment(command: &mut Command, root: &Path) {
    command
        .env("HOME", root)
        .env("VIBE_HOME", root.join(".vibe"))
        .env("VIBE_TEST_DISABLE_KEYRING", "1")
        .env("OTEL_SDK_DISABLED", "true")
        .env_remove("MISTRAL_API_KEY");
    // Default Mistral entries remain after union/deep config merging. With no
    // Mistral key, fetch_admin_toml returns NO_API_KEY BEFORE issuing HTTP.
}

pub(crate) fn arguments(command: &mut Command, root: &Path) {
    environment(command, root);
    command
        .current_dir(root.join("work"))
        .arg("--legacy-harness");
}

/// The installation publishes vibe-app-server beside vibe (pyproject scripts).
/// Preserve symlink installation location; resolving vibe first can lose the
/// environment's console-script sibling. The owner verifies this executable.
pub(crate) fn app_server(executable: &Path) -> Result<PathBuf, String> {
    let parent = executable
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("Vibe requires an absolute, resolved installation path.")?;
    if !executable.is_absolute() {
        return Err("Vibe requires an absolute installation path.".into());
    }
    Ok(parent.join(if cfg!(windows) {
        "vibe-app-server.exe"
    } else {
        "vibe-app-server"
    }))
}

/// app-server has no --version argument; initialize supplies the exact version.
pub(crate) fn version_matches(value: &str) -> bool {
    value == VERSION
}

#[derive(Debug, Default)]
pub(crate) struct Progress {
    pub(crate) outbound: Vec<Value>,
    pub(crate) text: Option<String>,
    pub(crate) request_started: bool,
    pub(crate) complete: bool,
    pub(crate) closed: bool,
    pub(crate) effect_denied: bool,
}

#[derive(Clone, Copy)]
enum Request {
    Initialize,
    Start,
    Ready,
    Runtime,
    Turn,
    Read,
    FinalRuntime,
    Stop,
    Interrupt,
    Deny,
}

pub(crate) struct Protocol {
    root: PathBuf,
    model: String,
    prompt: String,
    request_id: String,
    next: u64,
    pending: BTreeMap<u64, Request>,
    session: Option<String>,
    turn: Option<String>,
    completed: bool,
    stopping: bool,
    final_text: Option<String>,
    admitted: bool,
}

fn fail() -> String {
    "Vibe app-server violated the owned text-turn protocol.".into()
}
fn empty(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty)
}
fn null(value: &Value, key: &str) -> bool {
    value.get(key).is_some_and(Value::is_null)
}

impl Protocol {
    pub(crate) fn new(root: &Path, model_id: &str, prompt: &str, request_id: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            model: model_id.into(),
            prompt: prompt.into(),
            request_id: request_id.into(),
            next: 1,
            pending: BTreeMap::new(),
            session: None,
            turn: None,
            completed: false,
            stopping: false,
            final_text: None,
            admitted: false,
        }
    }

    fn request(&mut self, kind: Request, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.pending.insert(id, kind);
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    }

    pub(crate) fn initialize(&mut self) -> Value {
        self.request(
            Request::Initialize,
            "initialize",
            json!({
                "clientInfo":{"name":"lomi-router","version":"1","entrypoint":"programmatic"},
                "capabilities":{"callbackKinds":["approval","user_input"],"clientTools":[]}
            }),
        )
    }

    fn session_params(&self) -> Value {
        json!({"sessionId": self.session})
    }

    fn stop(&mut self) -> Value {
        self.stopping = true;
        self.request(Request::Stop, "session/stop", self.session_params())
    }

    /// Owner writes these frames in order, then bounds graceful exit/reaps the
    /// same owned child. There is no public shutdown RPC.
    pub(crate) fn cancel(&mut self) -> Vec<Value> {
        if self.stopping || self.session.is_none() {
            return Vec::new();
        }
        self.stopping = true;
        if let Some(turn) = self.turn.clone() {
            vec![self.request(
                Request::Interrupt,
                "turn/interrupt",
                json!({"sessionId":self.session,"expectedTurnId":turn}),
            )]
        } else {
            vec![self.stop()]
        }
    }

    fn runtime(&self, runtime: &Value) -> Result<(), String> {
        let config = &runtime["config"];
        let active = &config["activeModel"];
        if active["name"] != self.model
            || active["alias"] != ALIAS
            || active["thinking"] != "off"
            || config["activeModelPinned"] != true
            || config["awaitingExperimentModel"] != false
            || !empty(&runtime["tools"])
            || !empty(&runtime["skills"])
            || !empty(&runtime["issues"])
            || runtime["hooksCount"] != 0
            || runtime["experimentalHarness"] != false
            || runtime["bypassToolPermissions"] != false
            || runtime["contextWindow"] != 0
            || runtime["connectors"]["connected"] != 0
            || !(null(&runtime["connectors"], "total")
                || runtime["connectors"]["total"].as_u64() == Some(0))
            || !empty(&runtime["mcp"]["sources"])
            || !runtime["mcp"]["discoveryErrors"]
                .as_object()
                .is_some_and(|m| m.is_empty())
            || !null(&runtime["mcp"], "connectorError")
        {
            return Err(fail());
        }
        for flag in [
            "enableTelemetry",
            "enableUpdateChecks",
            "enableNotifications",
            "experimentalEnableRegistrySkills",
            "experimentalEnableTabStatus",
            "fileWatcherForAutocomplete",
            "voiceModeEnabled",
            "narratorEnabled",
        ] {
            if config[flag] != false {
                return Err(fail());
            }
        }
        Ok(())
    }

    fn state<'a>(&self, state: &'a Value, idle: bool) -> Result<&'a str, String> {
        let session = &state["session"];
        let id = session["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(fail)?;
        if state["format"] != "vibe.public-session-state/v1"
            || state["eventId"].as_u64().is_none()
            || self.session.as_deref().is_some_and(|owned| owned != id)
            || session["rootSessionId"].as_str() != Some(id)
            // _state.build_public_state leaves harness at its null default.
            // RuntimeSnapshot.experimentalHarness is the public legacy fence.
            || !null(session, "parentSessionId") || !null(session, "harness")
            || session["model"] != ALIAS || !null(session, "reasoningEffort")
            || session["cwd"].as_str() != self.root.join("work").to_str()
            || !empty(&session["workspaceRoots"])
            || !empty(&state["activeCallbacks"]) || !empty(&state["childSessions"])
            || !empty(&state["turnQueue"]["items"])
            || (idle && (state["isQuiescent"] != true || session["status"]["type"] != "idle"))
        {
            return Err(fail());
        }
        Ok(id)
    }

    fn turn_ok(&self, turn: &Value, completed: bool) -> Result<(), String> {
        if turn["sessionId"].as_str() != self.session.as_deref()
            || self
                .turn
                .as_deref()
                .is_some_and(|id| turn["id"].as_str() != Some(id))
            || turn["id"].as_str().filter(|id| !id.is_empty()).is_none()
            || !null(turn, "error")
            || !null(turn, "stopReason")
            || turn["status"]
                != if completed {
                    "completed"
                } else {
                    "in_progress"
                }
            || (completed && turn["completedAt"].as_u64().is_none())
        {
            return Err(fail());
        }
        Ok(())
    }

    pub(crate) fn consume(&mut self, event: &Value) -> Result<Progress, String> {
        if event["jsonrpc"] != "2.0" {
            return Err(fail());
        }
        let mut progress = Progress::default();
        if let Some(method) = event["method"].as_str() {
            if event.get("id").is_some() {
                // Acknowledge delivery but deny the callback via the actual
                // callback/result contract. Never approve any unexpected work.
                if method == "callback/call" {
                    let callback = &event["params"]["callback"];
                    let id = callback["callbackId"].as_str().ok_or_else(fail)?;
                    if callback["sessionId"].as_str() != self.session.as_deref() {
                        return Err(fail());
                    }
                    progress
                        .outbound
                        .push(json!({"jsonrpc":"2.0","id":event["id"],
                        "result":{"callbackId":id,"accepted":true}}));
                    progress.outbound.push(self.request(Request::Deny, "callback/result", json!({
                        "sessionId":self.session,"result":{"callbackId":id,"output":null,
                        "error":{"message":"Managed text turns deny callbacks","code":"FORBIDDEN"}}
                    })));
                } else {
                    progress.outbound.push(json!({"jsonrpc":"2.0","id":event["id"],
                        "error":{"code":"FORBIDDEN","message":"Managed text turns deny client tools"}}));
                }
                progress.effect_denied = true;
                return Ok(progress);
            }
            let params = &event["params"];
            if self.session.is_some()
                && params.get("sessionId").is_some()
                && params["sessionId"].as_str() != self.session.as_deref()
            {
                return Err(fail());
            }
            match method {
                "runtime/updated" => self.runtime(&params["runtime"])?,
                "session/snapshot" => {
                    self.state(&params["state"], false)?;
                }
                "turn/completed" => {
                    if self.stopping {
                        return Ok(progress);
                    }
                    self.turn_ok(&params["turn"], true)?;
                    if self.turn.is_none() || self.completed {
                        return Err(fail());
                    }
                    self.completed = true;
                    progress.outbound.push(self.request(
                        Request::Read,
                        "session/read",
                        json!({
                            "sessionId":self.session,"history":{"limit":500},"turns":{"limit":500}
                        }),
                    ));
                }
                "turn/started" => {
                    if !self.admitted {
                        return Err(fail());
                    }
                    self.turn_ok(&params["turn"], false)?;
                    if self.turn.is_none() {
                        self.turn = params["turn"]["id"].as_str().map(str::to_owned);
                    }
                }
                "history/entryAdded" => {
                    let entry = &params["entry"];
                    if entry["type"] != "message"
                        || !matches!(entry["role"].as_str(), Some("user" | "assistant"))
                    {
                        return Err(fail());
                    }
                    if entry["sessionId"].as_str() != self.session.as_deref() {
                        return Err(fail());
                    }
                }
                // Canonical final session/read validates content and identity;
                // patches are never interpreted as output or tool permissions.
                "history/entryUpdated"
                | "session/statsUpdated"
                | "session/updated"
                | "turn/retrying" => {}
                "turn/queueUpdated" if empty(&params["queue"]["items"]) => {}
                _ => return Err(fail()),
            }
            return Ok(progress);
        }
        let id = event["id"].as_u64().ok_or_else(fail)?;
        let request = self.pending.remove(&id).ok_or_else(fail)?;
        if self.stopping && matches!(request, Request::Interrupt) {
            // An already completed turn can reject interruption as stale. Stop
            // still closes this exact session and server, even in that race.
            progress.outbound.push(self.stop());
            return Ok(progress);
        }
        if self.stopping && !matches!(request, Request::Stop | Request::Deny) {
            return Ok(progress);
        }
        if event.get("error").is_some() {
            return Err("Vibe app-server rejected the owned request.".into());
        }
        let result = event
            .get("result")
            .filter(|r| r.is_object())
            .ok_or_else(fail)?;
        match request {
            Request::Initialize => {
                if result["serverInfo"]["name"] != "vibe-app-server"
                    || !result["serverInfo"]["version"]
                        .as_str()
                        .is_some_and(version_matches)
                {
                    return Err(fail());
                }
                progress
                    .outbound
                    .push(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
                progress.outbound.push(self.request(Request::Start, "session/start", json!({
                    "idempotencyKey":format!("{}-session",self.request_id),"kind":"normal",
                    "agentConfig":{"cwd":self.root.join("work"),"workspaceRoots":[],
                        "agent":"ask","autoApprove":false,"headless":true,"trustWorkspace":false,
                        "instructions":PROMPT,"tools":[],"hooks":[],"disabledTools":["*"],"mcpServers":[]}
                })));
            }
            Request::Start => {
                self.session = Some(self.state(&result["state"], true)?.to_owned());
                if !empty(&result["state"]["history"]) || !empty(&result["state"]["turns"]) {
                    return Err(fail());
                }
                progress.outbound.push(self.request(
                    Request::Ready,
                    "session/ready/wait",
                    self.session_params(),
                ));
            }
            Request::Ready => {
                if result["ready"] != true {
                    return Err(fail());
                }
                progress.outbound.push(self.request(
                    Request::Runtime,
                    "runtime/read",
                    self.session_params(),
                ));
            }
            Request::Runtime => {
                if result["ready"] != true {
                    return Err(fail());
                }
                self.runtime(&result["runtime"])?;
                self.admitted = true;
                progress.outbound.push(self.request(Request::Turn, "turn/start", json!({
                    "sessionId":self.session,"idempotencyKey":format!("{}-turn",self.request_id),
                    "clientUserMessageId":self.request_id,"message":[{"type":"text","text":self.prompt}]
                })));
                progress.request_started = true;
            }
            Request::Turn => {
                self.turn_ok(&result["turn"], false)?;
                self.turn = result["turn"]["id"].as_str().map(str::to_owned);
            }
            Request::Read => {
                self.state(&result["state"], true)?;
                let turns = result["state"]["turns"].as_array().ok_or_else(fail)?;
                if !self.completed || turns.len() != 1 {
                    return Err(fail());
                }
                self.turn_ok(&turns[0], true)?;
                let history = result["state"]["history"].as_array().ok_or_else(fail)?;
                let mut text = Vec::new();
                let mut user_seen = false;
                for entry in history {
                    if entry["sessionId"].as_str() != self.session.as_deref()
                        || entry["turnId"].as_str() != self.turn.as_deref()
                        || entry["type"] != "message"
                        || entry["generationStatus"] != "completed"
                    {
                        return Err(fail());
                    }
                    let content = entry["content"].as_array().ok_or_else(fail)?;
                    let mut blocks = Vec::new();
                    for block in content {
                        if block["type"] != "text" {
                            return Err(fail());
                        }
                        blocks.push(block["text"].as_str().ok_or_else(fail)?);
                    }
                    match entry["role"].as_str() {
                        Some("user") if !user_seen && text.is_empty() => {
                            if entry["id"] != self.request_id || blocks.join("\n\n") != self.prompt
                            {
                                return Err(fail());
                            }
                            user_seen = true;
                        }
                        Some("assistant") if user_seen => {
                            text.push(blocks.join("\n\n"));
                        }
                        _ => return Err(fail()),
                    }
                }
                let text = text.join("\n\n");
                if !user_seen || text.trim().is_empty() {
                    return Err(fail());
                }
                self.final_text = Some(text);
                progress.outbound.push(self.request(
                    Request::FinalRuntime,
                    "runtime/read",
                    self.session_params(),
                ));
            }
            Request::FinalRuntime => {
                if result["ready"] != true || !self.admitted {
                    return Err(fail());
                }
                self.runtime(&result["runtime"])?;
                progress.text = self.final_text.take();
                progress.complete = true;
                progress.outbound.push(self.stop());
            }
            Request::Stop => {
                if result["closed"] != true {
                    return Err(fail());
                }
                progress.closed = true;
            }
            Request::Interrupt => {
                progress.outbound.push(self.stop());
            }
            Request::Deny => {
                progress.effect_denied = true;
            }
        }
        Ok(progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> Value {
        json!({"config":{"activeModel":{"name":"gpt-4.1-mini","alias":ALIAS,"thinking":"off"},
            "activeModelPinned":true,"awaitingExperimentModel":false,
            "enableTelemetry":false,"enableUpdateChecks":false,"enableNotifications":false,
            "experimentalEnableRegistrySkills":false,"experimentalEnableTabStatus":false,
            "fileWatcherForAutocomplete":false,"voiceModeEnabled":false,"narratorEnabled":false},
            "tools":[],"skills":[],"issues":[],"hooksCount":0,"experimentalHarness":false,
            "bypassToolPermissions":false,"contextWindow":0,"connectors":{"connected":0,"total":null},
            "mcp":{"sources":[],"discoveryErrors":{},"connectorError":null}})
    }

    fn state(history: Value, turns: Value) -> Value {
        json!({"format":"vibe.public-session-state/v1","eventId":1,
            "session":{"id":"session-owned","rootSessionId":"session-owned","parentSessionId":null,
                "harness":null,"model":ALIAS,"reasoningEffort":null,"cwd":"/private/vibe/work",
                "workspaceRoots":[],"status":{"type":"idle"}},
            "isQuiescent":true,"activeCallbacks":[],"childSessions":[],"turnQueue":{"items":[]},
            "history":history,"turns":turns})
    }

    fn turn(status: &str) -> Value {
        json!({"id":"turn-owned","sessionId":"session-owned","status":status,
            "error":null,"stopReason":null,"completedAt":if status == "completed" { json!(5) } else { Value::Null }})
    }

    fn response(protocol: &mut Protocol, id: u64, result: Value) -> Progress {
        protocol
            .consume(&json!({"jsonrpc":"2.0","id":id,"result":result}))
            .unwrap()
    }

    #[test]
    fn complete_requires_correlated_final_state_runtime_and_server_close() {
        let mut protocol =
            Protocol::new(Path::new("/private/vibe"), "gpt-4.1-mini", "hello", "owned");
        protocol.initialize();
        response(
            &mut protocol,
            1,
            json!({"serverInfo":{"name":"vibe-app-server","version":VERSION}}),
        );
        response(
            &mut protocol,
            2,
            json!({"state":state(json!([]),json!([]))}),
        );
        response(&mut protocol, 3, json!({"ready":true}));
        let admitted = response(&mut protocol, 4, json!({"ready":true,"runtime":runtime()}));
        assert!(admitted.request_started);
        assert_eq!(
            admitted.outbound[0]["params"]["message"][0]["text"],
            "hello"
        );
        response(&mut protocol, 5, json!({"turn":turn("in_progress")}));
        let completed = protocol
            .consume(&json!({"jsonrpc":"2.0","method":"turn/completed",
            "params":{"sessionId":"session-owned","turn":turn("completed")}}))
            .unwrap();
        assert!(!completed.complete);
        assert_eq!(completed.outbound[0]["method"], "session/read");
        let history = json!([
            {"id":"owned","sessionId":"session-owned","turnId":"turn-owned","type":"message",
                "role":"user","generationStatus":"completed","content":[{"type":"text","text":"hello"}]},
            {"id":"answer","sessionId":"session-owned","turnId":"turn-owned","type":"message",
                "role":"assistant","generationStatus":"completed","content":[{"type":"text","text":"answer"}]}
        ]);
        let read = response(
            &mut protocol,
            6,
            json!({"state":state(history,json!([turn("completed")]))}),
        );
        assert!(!read.complete);
        assert!(read.text.is_none());
        let final_runtime = response(&mut protocol, 7, json!({"ready":true,"runtime":runtime()}));
        assert!(final_runtime.complete);
        assert!(!final_runtime.closed);
        assert_eq!(final_runtime.text.as_deref(), Some("answer"));
        assert_eq!(final_runtime.outbound[0]["method"], "session/stop");
        assert!(response(&mut protocol, 8, json!({"closed":true})).closed);
    }

    #[test]
    fn admission_rejects_tools_model_changes_and_nonquiescent_state() {
        let mut protocol =
            Protocol::new(Path::new("/private/vibe"), "gpt-4.1-mini", "hello", "owned");
        let mut changed = runtime();
        changed["tools"] = json!([{"name":"bash"}]);
        assert!(protocol.runtime(&changed).is_err());
        changed = runtime();
        changed["config"]["activeModel"]["name"] = json!("gpt-4o");
        assert!(protocol.runtime(&changed).is_err());
        let mut changed = state(json!([]), json!([]));
        changed["isQuiescent"] = json!(false);
        assert!(protocol.state(&changed, true).is_err());
        protocol.session = Some("session-owned".into());
        protocol.turn = Some("turn-owned".into());
        let mut stopped = turn("completed");
        stopped["stopReason"] = json!("limit");
        assert!(protocol.turn_ok(&stopped, true).is_err());
        stopped = turn("completed");
        stopped["id"] = json!("foreign-turn");
        assert!(protocol.turn_ok(&stopped, true).is_err());
    }

    #[test]
    fn owned_config_has_no_mistral_secret_or_secondary_work() {
        let text = config("gpt-4.1-mini");
        assert!(text.contains("api_base = \"https://api.openai.com/v1\""));
        assert!(text.contains("api_key_env_var = \"OPENAI_API_KEY\""));
        assert!(text.contains("disabled_tools = [\"*\"]"));
        assert!(text.contains("raise_on_compaction_failure = true"));
        assert!(!text.contains("MISTRAL_API_KEY"));
        assert!(model(Some("gpt-4.1-mini")).is_ok());
        assert!(model(Some("gpt-4.1-mini-high")).is_err());
        assert!(model(Some("o3")).is_err());
        assert!(model(None).is_err());
    }

    #[test]
    fn version_handshake_precedes_session_and_inference() {
        let mut protocol =
            Protocol::new(Path::new("/private/vibe"), "gpt-4.1-mini", "hello", "owned");
        let frame = protocol.initialize();
        assert_eq!(frame["method"], "initialize");
        let update = protocol
            .consume(&json!({"jsonrpc":"2.0","id":1,"result":{
            "serverInfo":{"name":"vibe-app-server","version":VERSION}}}))
            .unwrap();
        assert_eq!(update.outbound[0]["method"], "initialized");
        assert_eq!(update.outbound[1]["method"], "session/start");
        assert!(!update.request_started);
    }

    #[test]
    fn wrong_version_unknown_responses_and_tools_fail_closed() {
        let mut protocol =
            Protocol::new(Path::new("/private/vibe"), "gpt-4.1-mini", "hello", "owned");
        protocol.initialize();
        assert!(protocol
            .consume(&json!({"jsonrpc":"2.0","id":1,"result":{
            "serverInfo":{"name":"vibe-app-server","version":"2.25.9"}}}))
            .is_err());
        assert!(protocol
            .consume(&json!({"jsonrpc":"2.0","id":900,"result":{}}))
            .is_err());
        let update = protocol
            .consume(&json!({"jsonrpc":"2.0","id":"server-tool",
            "method":"clientTool/terminal/create","params":{}}))
            .unwrap();
        assert!(update.effect_denied);
        assert_eq!(update.outbound[0]["error"]["code"], "FORBIDDEN");
    }

    #[test]
    fn callbacks_are_denied_via_callback_result_and_cancel_interrupts() {
        let mut protocol =
            Protocol::new(Path::new("/private/vibe"), "gpt-4.1-mini", "hello", "owned");
        protocol.session = Some("session-owned".into());
        protocol.turn = Some("turn-owned".into());
        let update = protocol
            .consume(&json!({"jsonrpc":"2.0","id":"callback-request",
            "method":"callback/call","params":{"callback":{
                "callbackId":"callback-owned","sessionId":"session-owned"}}}))
            .unwrap();
        assert!(update.effect_denied);
        assert_eq!(update.outbound[1]["method"], "callback/result");
        assert_eq!(
            update.outbound[1]["params"]["result"]["error"]["code"],
            "FORBIDDEN"
        );
        let cancel = protocol.cancel();
        assert_eq!(cancel[0]["method"], "turn/interrupt");
        assert_eq!(cancel[0]["params"]["expectedTurnId"], "turn-owned");
    }
}
