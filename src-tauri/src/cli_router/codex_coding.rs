//! Persistent native Codex coding candidate. Effects remain in the root broker.
//! Contract: openai/codex rust-v0.160.0, a956835d020762cb2b570053af06f643a11c0ecc.
use super::{empty_array, request, Phase, Protocol, Rejection, CATALOG, MAX_OUTPUT, VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Component, Path, PathBuf},
};

const MAX_HISTORY: usize = 32 * 1024 * 1024;
const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_ITEMS: usize = 8192;
const TOOL_NAMES: [&str; 4] = ["lomi_list", "lomi_read", "lomi_write", "lomi_apply_patch"];
const HYDRATION_NOTICE: &str = "Full-history hydration is deprecated for paginated threads; omit `includeTurns` or set it to `false`, then page with `thread/turns/list` and `thread/items/list`.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SavedThread {
    pub(crate) id: String,
    pub(crate) rollout_path: PathBuf,
    pub(crate) creator_account_id: String,
    pub(crate) history_sha256: String,
}
#[derive(Clone, Debug)]
pub(crate) struct ToolRequest {
    pub(crate) rpc_id: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) call_id: String,
    pub(crate) tool: String,
    pub(crate) arguments: Value,
}
struct ToolCall {
    request: Option<ToolRequest>,
    result: Option<Value>,
    receipt: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Admission,
    Thread,
    BeforeRead,
    Ready,
    Turn,
    Running,
    AfterRead,
    Done,
}

pub(crate) fn coding_home(parent: &Path, model: &str) -> Result<PathBuf, String> {
    super::model(Some(model))?;
    super::ambient_policy()?;
    let root = parent.join("codex-coding");
    let created = match fs::symlink_metadata(&root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&root).map_err(|_| "Cannot create private Codex coding storage.")?;
            crate::chat::storage::private(&root, true)?;
            true
        }
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => false,
        _ => return Err("Codex coding storage ownership is invalid.".into()),
    };
    let root = root
        .canonicalize()
        .map_err(|_| "Cannot resolve Codex coding storage.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::symlink_metadata(&root)
            .map_err(|_| "Cannot inspect Codex coding storage ownership.")?;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err("Codex coding storage is not private to its owner.".into());
        }
    }
    let config = super::restrictions(&root, model)
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| format!("{key} = {}\n", super::toml_value(value)))
        .collect::<String>();
    for (name, bytes) in [
        ("models.json", CATALOG.as_bytes()),
        ("config.toml", config.as_bytes()),
    ] {
        let path = root.join(name);
        if created {
            crate::chat::storage::atomic(&path, bytes)?;
        } else {
            let meta =
                fs::symlink_metadata(&path).map_err(|_| "Codex coding policy is missing.")?;
            if !meta.is_file()
                || meta.file_type().is_symlink()
                || meta.len() != bytes.len() as u64
                || fs::read(&path).map_err(|_| "Cannot inspect Codex coding policy.")? != bytes
            {
                return Err("Codex coding policy changed; resume is fenced.".into());
            }
        }
    }
    Ok(root)
}

pub(crate) struct CodingProtocol {
    admission: Protocol,
    stage: Stage,
    tools: Value,
    client_id: String,
    saved: Option<SavedThread>,
    before_turns: Vec<Value>,
    native_history: Option<Value>,
    observed: HashMap<String, Value>,
    finished: HashSet<String>,
    order: Vec<String>,
    text_deltas: HashMap<String, String>,
    reasoning_deltas: HashMap<String, Vec<Value>>,
    tool_calls: HashMap<String, ToolCall>,
    pending: VecDeque<ToolRequest>,
    restored_usage_turn: Option<String>,
    owned_user_id: Option<String>,
    terminal_turn: Option<Value>,
    quota_error: bool,
    terminal_quota: bool,
    aggregate_bytes: usize,
    delta: Option<String>,
    pub(crate) output: String,
    pub(crate) completed: bool,
    pub(crate) terminal: bool,
    pub(crate) checkpointed: bool,
    pub(crate) exhausted: bool,
    pub(crate) effect: bool,
    pub(crate) rejection: Option<Rejection>,
}
impl CodingProtocol {
    pub(crate) fn new(
        model: &str,
        effort: Option<&str>,
        root: &Path,
        prompt: &str,
        client_user_message_id: &str,
        saved: Option<SavedThread>,
        dynamic_tools: Value,
    ) -> Result<Self, String> {
        validate_tools(&dynamic_tools).map_err(|_| "Invalid native coding tools.")?;
        if client_user_message_id.is_empty()
            || client_user_message_id.len() > 256
            || prompt.len() > MAX_OUTPUT
        {
            return Err("Invalid Codex coding input bound.".into());
        }
        if let Some(binding) = &saved {
            if binding.id.is_empty()
                || binding.creator_account_id.is_empty()
                || binding.history_sha256.len() != 64
                || !binding
                    .history_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("Invalid owned Codex thread binding.".into());
            }
            owned_path(root, &binding.rollout_path).map_err(|_| "Invalid owned Codex rollout.")?;
        }
        Ok(Self {
            admission: Protocol::new(model, effort, root, prompt)?,
            stage: Stage::Admission,
            tools: dynamic_tools,
            client_id: client_user_message_id.into(),
            saved,
            before_turns: vec![],
            native_history: None,
            observed: HashMap::new(),
            finished: HashSet::new(),
            order: vec![],
            text_deltas: HashMap::new(),
            reasoning_deltas: HashMap::new(),
            tool_calls: HashMap::new(),
            pending: VecDeque::new(),
            restored_usage_turn: None,
            owned_user_id: None,
            terminal_turn: None,
            quota_error: false,
            terminal_quota: false,
            aggregate_bytes: 0,
            delta: None,
            output: String::new(),
            completed: false,
            terminal: false,
            checkpointed: false,
            exhausted: false,
            effect: false,
            rejection: None,
        })
    }
    pub(crate) fn bind_executable(&mut self, path: &Path) -> Result<(), String> {
        self.admission.bind_executable(path)
    }
    pub(crate) fn initialize(&self) -> Value {
        self.admission.initialize()
    }
    pub(crate) fn needs_login(&self) -> bool {
        self.admission.needs_login()
    }
    pub(crate) fn login(&mut self, token: &str, account: &str) -> Result<Value, Rejection> {
        self.admission.login(token, account)
    }
    pub(crate) fn pre_turn_ready(&self) -> bool {
        self.stage == Stage::Ready
    }
    /// The root commits the native binding, input intent and final dispatch fences first.
    pub(crate) fn release_turn_start(&mut self, root_receipt: &str) -> Result<Value, Rejection> {
        if self.stage != Stage::Ready || root_receipt.is_empty() || root_receipt.len() > 256 {
            return Err(Rejection::Policy);
        }
        self.stage = Stage::Turn;
        Ok(request(
            7,
            "turn/start",
            json!({"threadId":self.admission.thread,
            "model":self.admission.model,"effort":self.admission.effort,"environments":[],"runtimeWorkspaceRoots":[],
            "clientUserMessageId":self.client_id,"input":self.input()}),
        ))
    }
    pub(crate) fn interrupt(&self) -> Option<Value> {
        self.admission.interrupt()
    }
    pub(crate) fn take_tool_request(&mut self) -> Option<ToolRequest> {
        self.pending.pop_front()
    }
    pub(crate) fn native_thread(&self) -> Option<&SavedThread> {
        self.saved.as_ref()
    }
    /// Full native history includes reasoning; UI output remains assistant text only.
    pub(crate) fn native_history(&self) -> Option<&Value> {
        self.native_history.as_ref()
    }
    /// Unfinished event text is retained as observations and never promoted to history.
    pub(crate) fn uncertain_observations(&self) -> Value {
        json!(self
            .order
            .iter()
            .map(|id| json!({"item":self.observed.get(id),"completed":self.finished.contains(id),"nativeReadCheckpoint":self.checkpointed,"textDelta":self.text_deltas.get(id), "reasoningEvents":self.reasoning_deltas.get(id)}))
            .collect::<Vec<_>>())
    }
    pub(crate) fn complete_tool(
        &mut self,
        call_id: &str,
        content_items: Value,
        success: bool,
        journal_receipt: &str,
    ) -> Result<Value, Rejection> {
        if !matches!(self.stage, Stage::Turn | Stage::Running)
            || journal_receipt.is_empty()
            || journal_receipt.len() > 256
        {
            return Err(Rejection::Policy);
        }
        let items = content_items.as_array().ok_or(Rejection::Malformed)?;
        if items.is_empty()
            || content_items.to_string().len() > MAX_FRAME - 65536
            || items.iter().any(|item| {
                item["type"] != "inputText"
                    || item["text"].as_str().is_none()
                    || item.as_object().is_none_or(|o| o.len() != 2)
            })
        {
            return Err(Rejection::Malformed);
        }
        let call = self
            .tool_calls
            .get_mut(call_id)
            .ok_or(Rejection::Malformed)?;
        if call.result.is_some() {
            return Err(Rejection::Malformed);
        }
        let req = call.request.as_ref().ok_or(Rejection::Malformed)?;
        let result = json!({"contentItems":content_items,"success":success});
        call.result = Some(result.clone());
        call.receipt = Some(journal_receipt.into());
        Ok(json!({"id":req.rpc_id,"result":result}))
    }
    pub(crate) fn consume(&mut self, event: &Value) -> Result<Vec<Value>, Rejection> {
        let result = self.accept(event);
        if let Err(error) = result {
            self.effect |= error == Rejection::Effect;
            self.rejection = Some(error);
        }
        result
    }
    fn accept(&mut self, event: &Value) -> Result<Vec<Value>, Rejection> {
        let bytes = event.to_string().len();
        self.aggregate_bytes = self
            .aggregate_bytes
            .checked_add(bytes)
            .ok_or(Rejection::Malformed)?;
        if self.rejection.is_some()
            || !event.is_object()
            || bytes > MAX_FRAME
            || self.aggregate_bytes > MAX_HISTORY
        {
            return Err(Rejection::Malformed);
        }
        if self.stage == Stage::Admission {
            let mut frames = self.admission.consume(event)?;
            if self.admission.phase == Phase::Thread {
                if frames.len() != 1 || frames[0]["method"] != "thread/start" {
                    return Err(Rejection::Malformed);
                }
                let p = &mut frames[0]["params"];
                if let Some(saved) = &self.saved {
                    frames[0] = request(
                        6,
                        "thread/resume",
                        json!({"threadId":saved.id,"model":self.admission.model,
                        "modelProvider":"openai","cwd":self.admission.root,"runtimeWorkspaceRoots":[],
                        "approvalPolicy":"never","approvalsReviewer":"user","sandbox":"read-only",
                        "config":{"model_reasoning_effort":self.admission.effort},"excludeTurns":true}),
                    );
                } else {
                    p["ephemeral"] = json!(false);
                    p["historyMode"] = json!("paginated");
                    p["dynamicTools"] = self.tools.clone();
                }
                self.stage = Stage::Thread;
            }
            return Ok(frames);
        }
        if let Some(id) = event["id"].as_u64() {
            if event["method"].is_string() {
                return self.tool_request(event, id);
            }
            if event.get("error").is_some() {
                return Err(Rejection::Failed);
            }
            let r = event.get("result").ok_or(Rejection::Malformed)?;
            return match (self.stage, id) {
                (Stage::Thread, 6) => {
                    // Reuse the exact text protocol's response admission; suppress its eager turn.
                    self.admission.response(r)?;
                    self.verify_thread(&r["thread"])?;
                    self.stage = Stage::BeforeRead;
                    Ok(vec![self.read_request(10)])
                }
                (Stage::BeforeRead, 10) => {
                    self.barrier(&r["thread"])?;
                    self.before_turns = r["thread"]["turns"]
                        .as_array()
                        .ok_or(Rejection::Malformed)?
                        .clone();
                    if self.before_turns.iter().any(|turn| {
                        !matches!(
                            turn["status"].as_str(),
                            Some("completed" | "failed" | "interrupted")
                        )
                    }) {
                        return Err(Rejection::Policy);
                    }
                    if self.restored_usage_turn.as_deref().is_some_and(|id| {
                        !self
                            .before_turns
                            .iter()
                            .any(|turn| turn["id"].as_str() == Some(id))
                    }) {
                        return Err(Rejection::Policy);
                    }
                    let digest = history_digest(&r["thread"]["turns"])?;
                    let binding = self.saved.as_mut().ok_or(Rejection::Policy)?;
                    if !binding.history_sha256.is_empty() && binding.history_sha256 != digest {
                        return Err(Rejection::Policy);
                    }
                    binding.history_sha256 = digest;
                    self.native_history = Some(r["thread"].clone());
                    self.stage = Stage::Ready;
                    Ok(vec![])
                }
                (Stage::Turn, 7) => {
                    self.admission.response(r)?;
                    self.stage = Stage::Running;
                    Ok(vec![])
                }
                (Stage::AfterRead, 11) => {
                    self.barrier(&r["thread"])?;
                    self.reconcile(&r["thread"])?;
                    self.saved.as_mut().ok_or(Rejection::Policy)?.history_sha256 =
                        history_digest(&r["thread"]["turns"])?;
                    self.native_history = Some(r["thread"].clone());
                    self.exhausted = self.terminal_quota;
                    self.checkpointed = true;
                    self.stage = Stage::Done;
                    Ok(vec![])
                }
                (_, 9) if matches!(self.stage, Stage::Running | Stage::AfterRead | Stage::Done) => {
                    Ok(vec![])
                }
                _ => Err(Rejection::Malformed),
            };
        }
        let method = event["method"].as_str().ok_or(Rejection::Malformed)?;
        let p = &event["params"];
        if !p.is_object() {
            return Err(Rejection::Malformed);
        }
        match method {
            "deprecationNotice" if matches!(self.stage, Stage::BeforeRead | Stage::AfterRead) => {
                if p["summary"] != HYDRATION_NOTICE
                    || !p["details"].is_null()
                    || p.as_object()
                        .is_none_or(|object| object.len() != 2 || !object.contains_key("details"))
                {
                    return Err(Rejection::Policy);
                }
            }
            "account/updated" => {
                if p["authMode"] != "chatgptAuthTokens" {
                    return Err(Rejection::Auth);
                }
            }
            "account/rateLimits/updated" => {}
            "thread/started" if self.stage == Stage::Thread => {
                let id = p["thread"]["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(Rejection::Malformed)?;
                if self.saved.as_ref().is_some_and(|s| s.id != id) {
                    return Err(Rejection::Policy);
                }
                self.admission.thread = Some(id.into());
            }
            "thread/status/changed" => {
                self.thread_correlation(p)?;
                match p["status"]["type"].as_str() {
                    Some("idle" | "notLoaded" | "systemError") => {}
                    Some("active") if empty_array(&p["status"]["activeFlags"]) => {}
                    _ => return Err(Rejection::Effect),
                }
            }
            "thread/settings/updated" => {
                self.thread_correlation(p)?;
                self.admission.settings(&p["threadSettings"])?;
            }
            "turn/started" if matches!(self.stage, Stage::Turn | Stage::Running) => {
                self.thread_correlation(p)?;
                let id = p["turn"]["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(Rejection::Malformed)?;
                if self.admission.turn.as_deref().is_some_and(|old| old != id) {
                    return Err(Rejection::Malformed);
                }
                self.admission.turn = Some(id.into());
            }
            "item/started" | "item/completed" => {
                self.turn_correlation(p)?;
                self.observe(&p["item"], method == "item/completed")?;
            }
            "item/agentMessage/delta" => {
                self.turn_correlation(p)?;
                let id = p["itemId"].as_str().ok_or(Rejection::Malformed)?;
                let known = self.observed.get(id).ok_or(Rejection::Malformed)?;
                if self.finished.contains(id) || known["type"] != "agentMessage" {
                    return Err(Rejection::Malformed);
                }
                let text = self.text_deltas.entry(id.into()).or_default();
                text.push_str(p["delta"].as_str().ok_or(Rejection::Malformed)?);
                if text.len() > MAX_OUTPUT {
                    return Err(Rejection::Malformed);
                }
                self.update_output()?;
            }
            "item/reasoning/summaryTextDelta"
            | "item/reasoning/textDelta"
            | "item/reasoning/summaryPartAdded" => {
                self.turn_correlation(p)?;
                let item = self
                    .observed
                    .get(p["itemId"].as_str().ok_or(Rejection::Malformed)?)
                    .ok_or(Rejection::Malformed)?;
                if item["type"] != "reasoning"
                    || self
                        .finished
                        .contains(p["itemId"].as_str().ok_or(Rejection::Malformed)?)
                {
                    return Err(Rejection::Malformed);
                }
                self.reasoning_deltas
                    .entry(p["itemId"].as_str().ok_or(Rejection::Malformed)?.into())
                    .or_default()
                    .push(json!({"method":method,"params":p}));
            }
            "thread/tokenUsage/updated" => {
                self.thread_correlation(p)?;
                validate_usage(&p["tokenUsage"])?;
                if self.admission.turn.is_some() {
                    self.turn_correlation(p)?;
                } else {
                    if !matches!(self.stage, Stage::BeforeRead | Stage::Ready | Stage::Turn)
                        || self.restored_usage_turn.is_some()
                    {
                        return Err(Rejection::Malformed);
                    }
                    let id = p["turnId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or(Rejection::Malformed)?;
                    if self.stage != Stage::BeforeRead
                        && !self
                            .before_turns
                            .iter()
                            .any(|turn| turn["id"].as_str() == Some(id))
                    {
                        return Err(Rejection::Policy);
                    }
                    self.restored_usage_turn = Some(id.into());
                }
            }
            "error" => {
                self.turn_correlation(p)?;
                if !p["willRetry"].is_boolean() {
                    return Err(Rejection::Malformed);
                }
                self.quota_error |=
                    p["willRetry"] == false && p["error"]["codexErrorInfo"] == "usageLimitExceeded";
            }
            "turn/completed" if self.stage == Stage::Running => {
                self.thread_correlation(p)?;
                if p["turn"]["id"].as_str() != self.admission.turn.as_deref() {
                    return Err(Rejection::Malformed);
                }
                let turn = &p["turn"];
                match turn["status"].as_str() {
                    Some("completed") if turn["error"].is_null() => {}
                    Some("failed" | "interrupted") => {}
                    _ => return Err(Rejection::Malformed),
                }
                self.terminal_quota = self.quota_error
                    && turn["status"] == "failed"
                    && turn["error"]["codexErrorInfo"] == "usageLimitExceeded";
                self.terminal = true;
                self.terminal_turn = Some(turn.clone());
                self.stage = Stage::AfterRead;
                return Ok(vec![self.read_request(11)]);
            }
            _ => return Err(Rejection::Effect),
        }
        Ok(vec![])
    }
    fn input(&self) -> Value {
        json!([{"type":"text","text":self.admission.prompt,"text_elements":[]}])
    }
    fn read_request(&self, id: u64) -> Value {
        request(
            id,
            "thread/read",
            json!({"threadId":self.admission.thread,"includeTurns":true}),
        )
    }
    fn thread_correlation(&self, p: &Value) -> Result<(), Rejection> {
        if self.admission.thread.is_none()
            || p["threadId"].as_str() != self.admission.thread.as_deref()
        {
            return Err(Rejection::Malformed);
        }
        Ok(())
    }
    fn turn_correlation(&self, p: &Value) -> Result<(), Rejection> {
        self.thread_correlation(p)?;
        if !matches!(self.stage, Stage::Turn | Stage::Running)
            || self.admission.turn.is_none()
            || p["turnId"].as_str() != self.admission.turn.as_deref()
        {
            return Err(Rejection::Malformed);
        }
        Ok(())
    }
    fn verify_thread(&self, thread: &Value) -> Result<(), Rejection> {
        if thread["id"].as_str() != self.admission.thread.as_deref()
            || thread["sessionId"] != thread["id"]
            || thread["ephemeral"] != false
            || thread["historyMode"] != "paginated"
            || thread["modelProvider"] != "openai"
            || thread["cwd"].as_str() != self.admission.root.to_str()
            || thread["cliVersion"] != VERSION
            || !thread["parentThreadId"].is_null()
            || !thread["forkedFromId"].is_null()
        {
            return Err(Rejection::Policy);
        }
        if let Some(saved) = &self.saved {
            if thread["id"] != saved.id || thread["path"].as_str() != saved.rollout_path.to_str() {
                return Err(Rejection::Policy);
            }
        }
        Ok(())
    }
    fn barrier(&mut self, thread: &Value) -> Result<(), Rejection> {
        self.verify_thread(thread)?;
        let path = PathBuf::from(thread["path"].as_str().ok_or(Rejection::Policy)?);
        let file = owned_path(&self.admission.root, &path)?;
        let mut line = String::new();
        let mut reader = BufReader::new(file);
        // take prevents an oversized first record from allocating without a bound.
        use std::io::Read;
        Read::by_ref(&mut reader)
            .take((MAX_FRAME + 1) as u64)
            .read_line(&mut line)
            .map_err(|_| Rejection::Policy)?;
        if line.len() > MAX_FRAME || !line.ends_with('\n') {
            return Err(Rejection::Policy);
        }
        let record: Value = serde_json::from_str(&line).map_err(|_| Rejection::Policy)?;
        let meta = &record["payload"];
        if record["type"] != "session_meta"
            || meta["id"] != thread["id"]
            || meta["session_id"] != thread["id"]
            || meta["cwd"].as_str() != self.admission.root.to_str()
            || meta["cli_version"] != VERSION
            || meta["model_provider"] != "openai"
            || meta["history_mode"] != "paginated"
            || meta["dynamic_tools"] != self.tools
            || !empty_array(&meta["runtime_workspace_roots"])
            || !(meta.get("selected_capability_roots").is_none()
                || empty_array(&meta["selected_capability_roots"]))
            || meta.get("history_base").is_some_and(|v| !v.is_null())
            || meta.get("parent_thread_id").is_some_and(|v| !v.is_null())
            || meta.get("forked_from_id").is_some_and(|v| !v.is_null())
        {
            return Err(Rejection::Policy);
        }
        let creator = meta["creator_account_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Rejection::Policy)?;
        if let Some(saved) = &self.saved {
            if creator != saved.creator_account_id {
                return Err(Rejection::Policy);
            }
        } else {
            if Some(creator) != self.admission.account.as_deref() {
                return Err(Rejection::Auth);
            }
            self.saved = Some(SavedThread {
                id: thread["id"].as_str().ok_or(Rejection::Malformed)?.into(),
                rollout_path: path.clone(),
                creator_account_id: creator.into(),
                history_sha256: String::new(),
            });
        }
        let turns = thread["turns"].as_array().ok_or(Rejection::Malformed)?;
        if turns.len() > MAX_ITEMS {
            return Err(Rejection::Malformed);
        }
        for turn in turns {
            if turn["itemsView"] != "full" {
                return Err(Rejection::Policy);
            }
            let items = turn["items"].as_array().ok_or(Rejection::Malformed)?;
            if items.len() > MAX_ITEMS {
                return Err(Rejection::Malformed);
            }
            for item in items {
                allowed_item(item)?;
            }
        }
        reader.get_ref().sync_all().map_err(|_| Rejection::Policy)?;
        for parent in path.parent().ok_or(Rejection::Policy)?.ancestors() {
            if !parent.starts_with(&self.admission.root) {
                break;
            }
            File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|_| Rejection::Policy)?;
        }
        Ok(())
    }
    fn observe(&mut self, item: &Value, complete: bool) -> Result<(), Rejection> {
        allowed_item(item)?;
        let id = item["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Rejection::Malformed)?
            .to_owned();
        if item["type"] == "userMessage"
            && (item["clientId"] != self.client_id
                || item["content"] != self.input()
                || item.as_object().is_none_or(|object| object.len() != 4)
                || self
                    .owned_user_id
                    .as_deref()
                    .is_some_and(|owned| !complete || owned != id))
        {
            return Err(Rejection::Policy);
        }
        if self.order.len() >= MAX_ITEMS {
            return Err(Rejection::Malformed);
        }
        if !complete {
            if self.observed.contains_key(&id) {
                return Err(Rejection::Malformed);
            }
            self.order.push(id.clone());
            if item["type"] == "agentMessage" {
                let initial = item["text"].as_str().ok_or(Rejection::Malformed)?;
                if initial.len() > MAX_OUTPUT {
                    return Err(Rejection::Malformed);
                }
                self.text_deltas.insert(id.clone(), initial.into());
            }
            if item["type"] == "dynamicToolCall" {
                if item["status"] != "inProgress"
                    || !item["contentItems"].is_null()
                    || !item["success"].is_null()
                    || !item["durationMs"].is_null()
                {
                    return Err(Rejection::Malformed);
                }
                self.tool_calls.insert(
                    id.clone(),
                    ToolCall {
                        request: None,
                        result: None,
                        receipt: None,
                    },
                );
            }
        } else {
            if self.finished.contains(&id) {
                return Err(Rejection::Malformed);
            }
            let started = self.observed.get(&id).ok_or(Rejection::Malformed)?;
            if started["type"] != item["type"] {
                return Err(Rejection::Malformed);
            }
            if item["type"] == "dynamicToolCall" {
                let call = self.tool_calls.get(&id).ok_or(Rejection::Malformed)?;
                let req = call.request.as_ref().ok_or(Rejection::Malformed)?;
                let result = call.result.as_ref().ok_or(Rejection::Policy)?;
                if call.receipt.is_none()
                    || item["tool"] != req.tool
                    || item["arguments"] != req.arguments
                    || item["durationMs"]
                        .as_i64()
                        .is_none_or(|duration| duration < 0)
                    || item["contentItems"] != result["contentItems"]
                    || item["success"] != result["success"]
                    || item["status"]
                        != (if result["success"] == true {
                            "completed"
                        } else {
                            "failed"
                        })
                {
                    return Err(Rejection::Policy);
                }
            }
            if item["type"] == "agentMessage" {
                let text = item["text"].as_str().ok_or(Rejection::Malformed)?;
                if self
                    .text_deltas
                    .get(&id)
                    .is_some_and(|observed| !text.starts_with(observed))
                {
                    return Err(Rejection::Malformed);
                }
                self.text_deltas.insert(id.clone(), text.into());
            }
        }
        self.observed.insert(id.clone(), item.clone());
        self.update_output()?;
        if item["type"] == "userMessage" {
            self.owned_user_id = Some(id.clone());
        }
        if complete {
            self.finished.insert(id);
        }
        Ok(())
    }
    fn update_output(&mut self) -> Result<(), Rejection> {
        let output = self
            .order
            .iter()
            .filter(|id| {
                self.observed
                    .get(*id)
                    .is_some_and(|i| i["type"] == "agentMessage")
            })
            .filter_map(|id| self.text_deltas.get(id))
            .filter(|t| !t.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n");
        if output.len() > MAX_OUTPUT || !output.starts_with(&self.output) {
            return Err(Rejection::Malformed);
        }
        if output.len() > self.output.len() {
            self.delta
                .get_or_insert_with(String::new)
                .push_str(&output[self.output.len()..]);
        }
        self.output = output;
        Ok(())
    }
    fn tool_request(&mut self, event: &Value, rpc_id: u64) -> Result<Vec<Value>, Rejection> {
        if event["method"] != "item/tool/call" {
            return Err(Rejection::Effect);
        }
        let p = &event["params"];
        if event.as_object().is_none_or(|o| o.len() != 3)
            || p.as_object().is_none_or(|o| o.len() != 6)
        {
            return Err(Rejection::Malformed);
        }
        self.turn_correlation(p)?;
        let id = p["callId"].as_str().ok_or(Rejection::Malformed)?;
        let item = self.observed.get(id).ok_or(Rejection::Malformed)?;
        if item["type"] != "dynamicToolCall"
            || !p["namespace"].is_null()
            || p["tool"] != item["tool"]
            || p["arguments"] != item["arguments"]
        {
            return Err(Rejection::Effect);
        }
        if !self
            .owned_user_id
            .as_ref()
            .is_some_and(|id| self.finished.contains(id))
        {
            return Err(Rejection::Policy);
        }
        if self
            .tool_calls
            .values()
            .any(|call| call.request.as_ref().is_some_and(|r| r.rpc_id == rpc_id))
        {
            return Err(Rejection::Malformed);
        }
        let call = self.tool_calls.get_mut(id).ok_or(Rejection::Malformed)?;
        if call.request.is_some() {
            return Err(Rejection::Malformed);
        }
        let req = ToolRequest {
            rpc_id,
            thread_id: self.admission.thread.clone().ok_or(Rejection::Malformed)?,
            turn_id: self.admission.turn.clone().ok_or(Rejection::Malformed)?,
            call_id: id.into(),
            tool: p["tool"].as_str().ok_or(Rejection::Malformed)?.into(),
            arguments: p["arguments"].clone(),
        };
        call.request = Some(req.clone());
        self.pending.push_back(req);
        Ok(vec![])
    }
    fn reconcile(&mut self, thread: &Value) -> Result<(), Rejection> {
        let turns = thread["turns"].as_array().ok_or(Rejection::Malformed)?;
        if turns.len() != self.before_turns.len() + 1
            || turns[..self.before_turns.len()] != self.before_turns
        {
            return Err(Rejection::Policy);
        }
        let turn = turns.last().ok_or(Rejection::Malformed)?;
        let terminal = self.terminal_turn.as_ref().ok_or(Rejection::Malformed)?;
        if turn["id"].as_str() != self.admission.turn.as_deref()
            || turn["status"] != terminal["status"]
            || turn["error"] != terminal["error"]
        {
            return Err(Rejection::Malformed);
        }
        let items = turn["items"].as_array().ok_or(Rejection::Malformed)?;
        let successful = terminal["status"] == "completed";
        let mut user = 0;
        let mut canonical_ids = Vec::new();
        for item in items {
            let id = item["id"].as_str().ok_or(Rejection::Malformed)?;
            if item["type"] == "userMessage" {
                user += 1;
                if item["clientId"] != self.client_id || item["content"] != self.input() {
                    return Err(Rejection::Policy);
                }
            }
            if let Some(observed) = self.observed.get(id) {
                if !self.finished.contains(id) || observed != item {
                    return Err(Rejection::Policy);
                }
                canonical_ids.push(id.to_owned());
            } else if item["type"] != "userMessage" {
                return Err(Rejection::Policy);
            }
        }
        let expected_ids = self
            .order
            .iter()
            .filter(|id| self.finished.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        if user != 1
            || canonical_ids != expected_ids
            || (successful && self.finished.len() != self.observed.len())
            || self.tool_calls.iter().any(|(id, c)| {
                c.request.is_some()
                    && (c.result.is_none() || c.receipt.is_none() || !self.finished.contains(id))
            })
        {
            return Err(Rejection::Policy);
        }
        // Summary notifications are not history. Confirm their entries against the full read.
        for summary in terminal["items"].as_array().ok_or(Rejection::Malformed)? {
            if !items.contains(summary) {
                return Err(Rejection::Policy);
            }
        }
        if successful
            && (terminal["itemsView"] != "summary"
                || self.output.trim().is_empty()
                || terminal["items"]
                    .as_array()
                    .is_none_or(|items| items.len() != 1)
                || terminal["items"][0]["type"] != "agentMessage"
                || terminal["items"][0]["phase"] == "commentary"
                || items.iter().rev().find(|i| {
                    i["type"] == "agentMessage"
                        && i["text"].as_str().is_some_and(|s| !s.trim().is_empty())
                }) != Some(&terminal["items"][0]))
        {
            return Err(Rejection::Malformed);
        }
        if !successful && (terminal["itemsView"] != "notLoaded" || !empty_array(&terminal["items"]))
        {
            return Err(Rejection::Malformed);
        }
        self.completed = successful;
        Ok(())
    }
}
fn validate_usage(value: &Value) -> Result<(), Rejection> {
    for name in ["total", "last"] {
        let object = value[name].as_object().ok_or(Rejection::Malformed)?;
        let keys = [
            "totalTokens",
            "inputTokens",
            "cachedInputTokens",
            "cacheWriteInputTokens",
            "outputTokens",
            "reasoningOutputTokens",
        ];
        if object.len() != keys.len()
            || keys.iter().any(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_i64)
                    .is_none_or(|count| count < 0)
            })
        {
            return Err(Rejection::Malformed);
        }
    }
    if value
        .as_object()
        .is_none_or(|object| object.len() != 3 || !object.contains_key("modelContextWindow"))
        || !(value["modelContextWindow"].is_null()
            || value["modelContextWindow"]
                .as_i64()
                .is_some_and(|count| count > 0))
    {
        return Err(Rejection::Malformed);
    }
    Ok(())
}

fn history_digest(turns: &Value) -> Result<String, Rejection> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(turns).map_err(|_| Rejection::Malformed)?;
    if !turns.is_array() || bytes.len() > MAX_HISTORY {
        return Err(Rejection::Malformed);
    }
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn allowed_item(item: &Value) -> Result<(), Rejection> {
    match item["type"].as_str() {
        Some("userMessage" | "agentMessage" | "reasoning") => Ok(()),
        Some("dynamicToolCall")
            if item.as_object().is_some_and(|o| o.len() == 9)
                && item["namespace"].is_null()
                && item["tool"]
                    .as_str()
                    .is_some_and(|name| TOOL_NAMES.contains(&name))
                && item["arguments"].is_object() =>
        {
            Ok(())
        }
        _ => Err(Rejection::Effect),
    }
}
fn validate_tools(tools: &Value) -> Result<(), Rejection> {
    let specs = tools.as_array().ok_or(Rejection::Policy)?;
    if specs.len() != TOOL_NAMES.len() || tools.to_string().len() > 65536 {
        return Err(Rejection::Policy);
    }
    for name in TOOL_NAMES {
        let matching = specs
            .iter()
            .filter(|s| s["name"] == name)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(Rejection::Policy);
        }
        let spec = matching[0];
        if spec.as_object().is_none_or(|o| {
            o.keys().any(|k| {
                !["type", "name", "description", "inputSchema", "deferLoading"]
                    .contains(&k.as_str())
            })
        }) || spec["type"] != "function"
            || spec["description"].as_str().is_none()
            || spec.get("deferLoading").is_some_and(|v| *v != false)
            || spec["inputSchema"]["type"] != "object"
            || spec["inputSchema"]["additionalProperties"] != false
        {
            return Err(Rejection::Policy);
        }
    }
    Ok(())
}
fn owned_path(root: &Path, path: &Path) -> Result<File, Rejection> {
    let suffix = path.strip_prefix(root).map_err(|_| Rejection::Policy)?;
    if suffix.as_os_str().is_empty() || !path.is_absolute() {
        return Err(Rejection::Policy);
    }
    let root_meta = fs::symlink_metadata(root).map_err(|_| Rejection::Policy)?;
    if !root_meta.is_dir() || root_meta.file_type().is_symlink() {
        return Err(Rejection::Policy);
    }
    let mut current = root.to_path_buf();
    for component in suffix.components() {
        let Component::Normal(part) = component else {
            return Err(Rejection::Policy);
        };
        current.push(part);
        let meta = fs::symlink_metadata(&current).map_err(|_| Rejection::Policy)?;
        if meta.file_type().is_symlink() || (current != path && !meta.is_dir()) {
            return Err(Rejection::Policy);
        }
    }
    let meta = fs::symlink_metadata(path).map_err(|_| Rejection::Policy)?;
    if !meta.is_file() || meta.len() > MAX_HISTORY as u64 {
        return Err(Rejection::Policy);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if root_meta.uid() != unsafe { libc::geteuid() }
            || root_meta.mode() & 0o077 != 0
            || meta.uid() != root_meta.uid()
            || meta.mode() & 0o022 != 0
        {
            return Err(Rejection::Policy);
        }
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| Rejection::Policy)?;
    let opened = file.metadata().map_err(|_| Rejection::Policy)?;
    if !opened.is_file() || opened.len() > MAX_HISTORY as u64 {
        return Err(Rejection::Policy);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != meta.dev() || opened.ino() != meta.ino() {
            return Err(Rejection::Policy);
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn running() -> CodingProtocol {
        let tools = json!(TOOL_NAMES
            .iter()
            .map(|name| json!({"type":"function","name":name,
            "description":"native tool","inputSchema":{"type":"object","additionalProperties":false,
            "properties":{"path":{"type":"string"}},"required":["path"]}}))
            .collect::<Vec<_>>());
        let mut p = CodingProtocol::new(
            "gpt-6.1-sol",
            Some("medium"),
            Path::new("/owned/private"),
            "inspect",
            "client_1",
            None,
            tools,
        )
        .unwrap();
        p.stage = Stage::Running;
        p.admission.phase = Phase::Running;
        p.admission.thread = Some("thread_1".into());
        p.admission.turn = Some("turn_1".into());
        p
    }
    fn notification(method: &str, item: Value) -> Value {
        json!({"method":method,"params":{"threadId":"thread_1","turnId":"turn_1","item":item}})
    }
    fn tool() -> Value {
        json!({"type":"dynamicToolCall","id":"call_1","namespace":null,"tool":"lomi_read",
            "arguments":{"path":"src/main.rs"},"status":"inProgress","contentItems":null,"success":null,"durationMs":null})
    }
    fn tool_rpc(call: &str) -> Value {
        json!({"id":80,"method":"item/tool/call","params":{"threadId":"thread_1","turnId":"turn_1",
            "callId":call,"namespace":null,"tool":"lomi_read","arguments":{"path":"src/main.rs"}}})
    }
    fn accept_user(p: &mut CodingProtocol) {
        let user =
            json!({"type":"userMessage","id":"user_1","clientId":"client_1","content":p.input()});
        p.consume(&notification("item/started", user.clone()))
            .unwrap();
        p.consume(&notification("item/completed", user)).unwrap();
    }
    #[test]
    fn native_input_dispatch_waits_for_one_use_root_commit_and_history_hash() {
        let mut p = running();
        assert_eq!(
            p.release_turn_start("committed").err(),
            Some(Rejection::Policy)
        );
        p.stage = Stage::Ready;
        assert!(p.pre_turn_ready());
        assert_eq!(p.release_turn_start("").err(), Some(Rejection::Policy));
        let start = p.release_turn_start("input_intent_1").unwrap();
        assert_eq!(start["params"]["clientUserMessageId"], "client_1");
        assert_eq!(start["params"]["environments"], json!([]));
        assert!(p.release_turn_start("input_intent_1").is_err());
        assert_eq!(history_digest(&json!([])).unwrap().len(), 64);
        assert_ne!(
            history_digest(&json!([])).unwrap(),
            history_digest(&json!([{"id":"changed"}])).unwrap()
        );
    }
    #[test]
    fn tool_result_requires_started_request_and_durable_root_receipt() {
        let mut p = running();
        assert_eq!(
            p.consume(&tool_rpc("call_1")).err(),
            Some(Rejection::Malformed)
        );
        let mut p = running();
        accept_user(&mut p);
        p.consume(&notification("item/started", tool())).unwrap();
        p.consume(&tool_rpc("call_1")).unwrap();
        let req = p.take_tool_request().unwrap();
        assert_eq!(
            (
                req.thread_id.as_str(),
                req.turn_id.as_str(),
                req.call_id.as_str()
            ),
            ("thread_1", "turn_1", "call_1")
        );
        let content = json!([{"type":"inputText","text":"{\"path\":\"src/main.rs\"}"}]);
        assert_eq!(
            p.complete_tool("call_1", content.clone(), true, "").err(),
            Some(Rejection::Policy)
        );
        let reply = p
            .complete_tool("call_1", content.clone(), true, "journal_1")
            .unwrap();
        assert_eq!(reply["id"], 80);
        assert!(p
            .complete_tool("call_1", content.clone(), true, "journal_1")
            .is_err());
        let mut completed = tool();
        completed["status"] = json!("completed");
        completed["durationMs"] = json!(12);
        completed["contentItems"] = content;
        completed["success"] = json!(true);
        p.consume(&notification("item/completed", completed))
            .unwrap();
    }

    #[test]
    fn foreign_or_unfinished_user_input_never_releases_a_project_tool() {
        let mut p = running();
        let foreign =
            json!({"type":"userMessage","id":"user_1","clientId":"foreign","content":p.input()});
        assert_eq!(
            p.consume(&notification("item/started", foreign)).err(),
            Some(Rejection::Policy)
        );
        assert!(p.take_tool_request().is_none());
        let mut p = running();
        let user =
            json!({"type":"userMessage","id":"user_1","clientId":"client_1","content":p.input()});
        p.consume(&notification("item/started", user)).unwrap();
        p.consume(&notification("item/started", tool())).unwrap();
        assert_eq!(
            p.consume(&tool_rpc("call_1")).err(),
            Some(Rejection::Policy)
        );
        assert!(p.take_tool_request().is_none());
    }

    #[test]
    fn seeded_agent_text_and_completed_observations_survive_interruption() {
        let mut p = running();
        let mut message = json!({"type":"agentMessage","id":"msg_1","text":"hello","phase":null,"memoryCitation":null});
        p.consume(&notification("item/started", message.clone()))
            .unwrap();
        assert_eq!(p.output, "hello");
        p.consume(&json!({"method":"item/agentMessage/delta","params":{"threadId":"thread_1","turnId":"turn_1","itemId":"msg_1","delta":" world"}})).unwrap();
        message["text"] = json!("hello world");
        p.consume(&notification("item/completed", message)).unwrap();
        assert_eq!(p.output, "hello world");
        assert_eq!(p.uncertain_observations()[0]["item"]["text"], "hello world");
        assert_eq!(p.uncertain_observations()[0]["completed"], true);
        let mut p = running();
        let message = json!({"type":"agentMessage","id":"msg_1","text":"hello","phase":null,"memoryCitation":null});
        p.consume(&notification("item/started", message.clone()))
            .unwrap();
        let mut malformed = message;
        malformed["text"] = json!("foreign");
        assert!(p
            .consume(&notification("item/completed", malformed))
            .is_err());
        assert_eq!(p.uncertain_observations()[0]["textDelta"], "hello");
        assert_eq!(p.uncertain_observations()[0]["completed"], false);
    }
    #[test]
    fn mismatched_tool_arguments_and_native_effects_are_fenced() {
        let mut p = running();
        p.consume(&notification("item/started", tool())).unwrap();
        let mut rpc = tool_rpc("call_1");
        rpc["params"]["arguments"]["path"] = json!("foreign");
        assert_eq!(p.consume(&rpc).err(), Some(Rejection::Effect));
        let mut p = running();
        assert_eq!(
            p.consume(&notification(
                "item/started",
                json!({"type":"commandExecution","id":"native"})
            ))
            .err(),
            Some(Rejection::Effect)
        );
    }
    #[test]
    fn exhaustion_requires_matching_nonretry_error_and_failed_terminal() {
        for (retry, terminal_info, expected) in [
            (true, "usageLimitExceeded", false),
            (false, "other", false),
            (false, "usageLimitExceeded", true),
        ] {
            let mut p = running();
            p.consume(&json!({"method":"error","params":{"threadId":"thread_1","turnId":"turn_1",
                "willRetry":retry,"error":{"codexErrorInfo":"usageLimitExceeded","message":"quota"}}})).unwrap();
            let frames = p.consume(&json!({"method":"turn/completed","params":{"threadId":"thread_1","turn":{
                "id":"turn_1","status":"failed","items":[],"itemsView":"notLoaded","error":{"codexErrorInfo":terminal_info}}}})).unwrap();
            assert_eq!(p.terminal_quota, expected);
            assert!(!p.exhausted);
            assert!(p.terminal);
            assert!(!p.checkpointed);
            assert_eq!(frames[0]["method"], "thread/read");
            assert_eq!(frames[0]["params"]["includeTurns"], true);
        }
    }
    #[test]
    fn unfinished_deltas_are_observations_and_not_native_history() {
        let mut p = running();
        p.consume(&notification(
            "item/started",
            json!({"type":"agentMessage","id":"msg_1","text":"","phase":"final_answer"}),
        ))
        .unwrap();
        p.consume(&json!({"method":"item/agentMessage/delta","params":{"threadId":"thread_1","turnId":"turn_1","itemId":"msg_1","delta":"partial"}})).unwrap();
        assert_eq!(p.output, "partial");
        assert!(p.native_history().is_none());
        assert_eq!(p.uncertain_observations()[0]["textDelta"], "partial");
        p.terminal_turn = Some(
            json!({"id":"turn_1","status":"failed","error":null,"items":[],"itemsView":"notLoaded"}),
        );
        p.reconcile(&json!({"turns":[{"id":"turn_1","status":"failed","error":null,"itemsView":"full","items":[{
            "type":"userMessage","id":"user_1","clientId":"client_1","content":p.input()}]}]})).unwrap();
        assert!(!p.completed);
        assert_eq!(p.output, "partial");
    }
    #[test]
    fn full_read_must_confirm_exact_client_input_and_observed_items() {
        let mut p = running();
        let message =
            json!({"type":"agentMessage","id":"msg_1","text":"done","phase":"final_answer"});
        p.consume(&notification(
            "item/started",
            json!({"type":"agentMessage","id":"msg_1","text":"","phase":"final_answer"}),
        ))
        .unwrap();
        p.consume(&notification("item/completed", message.clone()))
            .unwrap();
        p.terminal_turn = Some(
            json!({"id":"turn_1","status":"completed","error":null,"items":[message.clone()],"itemsView":"summary"}),
        );
        let history = json!({"turns":[{"id":"turn_1","status":"completed","error":null,"itemsView":"full","items":[
            {"type":"userMessage","id":"user_1","clientId":"client_1","content":p.input()},message]}]});
        let mut corrupted = history.clone();
        corrupted["turns"][0]["items"][0]["clientId"] = json!("foreign");
        assert_eq!(p.reconcile(&corrupted).err(), Some(Rejection::Policy));
        p.reconcile(&history).unwrap();
        assert!(p.completed);
    }
}
