//! Qwen Code 0.24.7, b12edec1401a28fc53cd9e714d5928b285071fc8.
//! Public `qwen serve`, exact PRIVATE Hosted Harness protocol 1. No-tool fresh
//! OpenAI API text turns only. Protocol/source changes require a new version gate.
//! Authoritative sources: packages/cli/src/serve/hosted-harness-{profile,model,
//! contract,session}.ts, packages/core/src/managed-runtime/http-managed-session-
//! store.ts and the production Java ManagedSessionStore. The launcher fixture
//! establishes argument spelling only; it is not the durable-store authority.
//! A controller owns its private durable store before daemon startup. The caller
//! clears the environment, injects only the selected OPENAI_API_KEY, and keeps
//! private daemon arguments and diagnostics out of the webview/logs.

use super::{qwen_store::OwnedStore, runtime::OwnedChild, transport::Outcome};
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE},
    Client, Method,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
use zeroize::Zeroizing;

pub(crate) const VERSION: &str = "0.24.7";
const PIN: &str = "b12edec1401a28fc53cd9e714d5928b285071fc8";
const MAX_JSON: usize = 256 * 1024;
const MAX_FRAME: usize = 512 * 1024;
const MAX_WIRE: usize = 16 * 1024 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const MODELS: &[&str] = &[
    "gpt-4.1",
    "gpt-4.1-mini",
    "gpt-4.1-nano",
    "gpt-4o",
    "gpt-4o-mini",
];

fn failure() -> String {
    "Qwen violated the owned Hosted Harness text-turn contract.".into()
}
pub(crate) fn supported_models() -> &'static [&'static str] {
    MODELS
}

pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    value.filter(|id| MODELS.contains(id)).ok_or_else(|| {
        "Qwen requires an exact approved OpenAI text model. No request was sent.".into()
    })
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn uuid() -> Result<String, String> {
    let mut bytes = super::new_id()?.into_bytes();
    if bytes.len() != 32 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err(failure());
    }
    bytes[12] = b'4';
    bytes[16] = b'8';
    let hex = std::str::from_utf8(&bytes).map_err(|_| failure())?;
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
fn canonical_uuid(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 36
        && (b'1'..=b'5').contains(&b[14])
        && b"89ab".contains(&b[19])
        && b.iter().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                *b == b'-'
            } else {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }
        })
}
fn keys(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|map| map.keys().all(|key| allowed.contains(&key.as_str())))
}
fn settings(id: &str) -> Value {
    json!({"security":{"auth":{"selectedType":"openai"}},"model":{"name":id},"telemetry":{"enabled":false},
        "modelProviders":{"openai":[{"id":id,"envKey":"OPENAI_API_KEY","baseUrl":"https://api.openai.com/v1","generationConfig":{"maxRetries":0}}]}})
}
pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
    let id = model(Some(id))?;
    let root = parent.join(format!("qwen-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| "Cannot create fresh Qwen storage.")?;
    crate::chat::storage::private(&root, true)?;
    let root = root
        .canonicalize()
        .map_err(|_| "Cannot resolve private Qwen storage.")?;
    for directory in [
        ".qwen", "runtime", "tmp", "config", "data", "cache", "state",
    ] {
        let path = root.join(directory);
        fs::create_dir(&path).map_err(|_| "Cannot isolate Qwen storage.")?;
        crate::chat::storage::private(&path, true)?;
    }
    crate::chat::storage::atomic(&root.join("system-settings.json"), b"{}\n")?;
    crate::chat::storage::atomic(&root.join("system-defaults.json"), b"{}\n")?;
    crate::chat::storage::atomic(
        &root.join(".qwen/settings.json"),
        &serde_json::to_vec(&settings(id)).map_err(|_| failure())?,
    )?;
    Ok(root)
}
pub(crate) fn environment(command: &mut Command, root: &Path, id: &str) {
    // cwd == HOME prevents findEnvFiles from traversing shared ancestors.
    command
        .current_dir(root)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("QWEN_HOME", root.join(".qwen"))
        .env("QWEN_RUNTIME_DIR", root.join("runtime"))
        .env(
            "QWEN_CODE_SYSTEM_SETTINGS_PATH",
            root.join("system-settings.json"),
        )
        .env(
            "QWEN_CODE_SYSTEM_DEFAULTS_PATH",
            root.join("system-defaults.json"),
        )
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        .env("OPENAI_BASE_URL", "https://api.openai.com/v1")
        .env("OPENAI_MODEL", id)
        .env("QWEN_MODEL", id)
        .env("QWEN_SANDBOX", "false")
        .env("NO_COLOR", "1")
        .env("OTEL_SDK_DISABLED", "true");
}
pub(crate) fn version_matches(output: &str) -> bool {
    output.trim() == VERSION
}

pub(super) struct Controller {
    root: PathBuf,
    model: String,
    session_id: String,
    workspace_id: String,
    tenant_id: String,
    prompt_id: String,
    prompt: Value,
    payload_digest: String,
    capability_digest: String,
    token: Zeroizing<String>,
    store: OwnedStore,
}
impl Controller {
    /// Construct before spawning the owned CLI. Its listener accepts no writer
    /// until capabilities supplies the exact process boot UUID.
    pub(super) fn new(root: &Path, id: &str, prompt: &str) -> Result<Self, String> {
        let id = model(Some(id))?;
        if prompt.trim().is_empty() {
            return Err("Qwen requires nonempty text.".into());
        }
        let prompt = json!([{"type":"text","text":prompt}]);
        let bytes = serde_json::to_vec(&prompt).map_err(|_| failure())?;
        // Managed resources have a 64KiB limit including the ChatRecord wrapper.
        if bytes.len() > 60 * 1024 {
            return Err(
                "Qwen text exceeds its owned inline-resource limit. No request was sent.".into(),
            );
        }
        let session_id = uuid()?;
        let workspace_id = uuid()?;
        let tenant_id = uuid()?;
        let prompt_id = uuid()?;
        let capability_digest = digest(
            &serde_json::to_vec(&json!({"protocol":1,"version":VERSION,"source":PIN,
            "profile":"hosted-harness","tools":false,"settings":settings(id)}))
            .map_err(|_| failure())?,
        );
        let token = Zeroizing::new(format!("{}{}", super::new_id()?, super::new_id()?));
        let store = OwnedStore::start(root, &session_id, &workspace_id, &tenant_id)?;
        Ok(Self {
            root: root.into(),
            model: id.into(),
            session_id,
            workspace_id,
            tenant_id,
            prompt_id,
            prompt,
            payload_digest: digest(&bytes),
            capability_digest,
            token,
            store,
        })
    }
}
pub(super) fn arguments(command: &mut Command, root: &Path, id: &str, controller: &Controller) {
    environment(command, root, id);
    command
        .args([
            "serve",
            "--profile",
            "hosted-harness",
            "--http-bridge",
            "--no-web",
            "--hostname",
            "127.0.0.1",
            "--port",
            "0",
            "--token",
        ])
        .arg(controller.token.as_str())
        .arg("--hosted-harness-capability-digest")
        .arg(&controller.capability_digest)
        .arg("--workspace")
        .arg(root);
}

#[derive(Clone)]
struct Http {
    client: Client,
    url: String,
    headers: HeaderMap,
    boot: Option<String>,
}
impl Http {
    fn new(port: u16, token: &str) -> Result<Self, String> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| failure())?;
        let mut headers = HeaderMap::new();
        let mut bearer =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| failure())?;
        bearer.set_sensitive(true);
        headers.insert(AUTHORIZATION, bearer);
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        Ok(Self {
            client,
            url: format!("http://127.0.0.1:{port}"),
            headers,
            boot: None,
        })
    }
    fn bind(&mut self, boot: &str, client: Option<&str>) -> Result<(), String> {
        if !canonical_uuid(boot) {
            return Err(failure());
        }
        self.headers.insert(
            "x-qwen-harness-protocol-version",
            HeaderValue::from_static("1"),
        );
        self.headers.insert(
            "x-qwen-harness-boot-id",
            HeaderValue::from_str(boot).map_err(|_| failure())?,
        );
        self.boot = Some(boot.into());
        if let Some(client) = client {
            if !canonical_uuid(client) {
                return Err(failure());
            }
            self.headers.insert(
                "x-qwen-client-id",
                HeaderValue::from_str(client).map_err(|_| failure())?,
            );
        }
        Ok(())
    }
    fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value), String> {
        let bytes = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| failure())?;
        if bytes.as_ref().is_some_and(|bytes| bytes.len() > MAX_JSON) {
            return Err(failure());
        }
        tauri::async_runtime::block_on(async {
            let mut request = self
                .client
                .request(method, format!("{}{path}", self.url))
                .headers(self.headers.clone());
            if let Some(bytes) = bytes {
                request = request.header(CONTENT_TYPE, "application/json").body(bytes);
            }
            let mut response = request.send().await.map_err(|_| failure())?;
            let status = response.status().as_u16();
            if response.status().is_redirection()
                || response
                    .content_length()
                    .is_some_and(|length| length > MAX_JSON as u64)
            {
                return Err(failure());
            }
            if let Some(boot) = &self.boot {
                if response
                    .headers()
                    .get("X-Qwen-Harness-Boot-Id")
                    .and_then(|value| value.to_str().ok())
                    != Some(boot.as_str())
                {
                    return Err(failure());
                }
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
                if bytes.len().saturating_add(chunk.len()) > MAX_JSON {
                    return Err(failure());
                }
                bytes.extend_from_slice(&chunk);
            }
            if status == 204 {
                return if bytes.is_empty() {
                    Ok((status, Value::Null))
                } else {
                    Err(failure())
                };
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split(';').next() == Some("application/json"))
            {
                return Err(failure());
            }
            Ok((
                status,
                serde_json::from_slice(&bytes).map_err(|_| failure())?,
            ))
        })
    }
}

fn port(line: &[u8]) -> Option<u16> {
    let line = std::str::from_utf8(line)
        .ok()?
        .trim_end_matches(['\r', '\n']);
    let rest = line.strip_prefix("qwen serve listening on http://127.0.0.1:")?;
    let count = rest.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 || count > 5 || !rest[count..].starts_with(" (mode=") {
        return None;
    }
    rest[..count].parse::<u16>().ok().filter(|port| *port != 0)
}
fn diagnostics<R: Read + Send + 'static>(
    pipe: R,
    sender: mpsc::SyncSender<u16>,
    bad: Arc<AtomicBool>,
) -> JoinHandle<bool> {
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut total = 0usize;
        loop {
            let mut line = Vec::new();
            match (&mut reader).take(65537).read_until(b'\n', &mut line) {
                Ok(0) => return true,
                Ok(_) => {
                    total = total.saturating_add(line.len());
                    if line.len() > 65536 || total > MAX_WIRE {
                        bad.store(true, Ordering::SeqCst);
                        return false;
                    }
                    if let Some(port) = port(&line) {
                        if sender.try_send(port).is_err() {
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

#[derive(Default)]
struct Sse {
    bytes: Vec<u8>,
    total: usize,
}
impl Sse {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        self.total = self.total.saturating_add(bytes.len());
        if self.total > MAX_WIRE {
            return Err(failure());
        }
        self.bytes.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.bytes.windows(2).position(|value| value == b"\n\n") {
            if end > MAX_FRAME {
                return Err(failure());
            }
            let frame: Vec<u8> = self.bytes.drain(..end + 2).collect();
            events.push(Self::frame(&frame[..end])?);
        }
        if self.bytes.len() > MAX_FRAME {
            return Err(failure());
        }
        Ok(events)
    }
    fn frame(frame: &[u8]) -> Result<Value, String> {
        let frame = std::str::from_utf8(frame).map_err(|_| failure())?;
        let lines: Vec<_> = frame.split('\n').collect();
        if lines.len() != 3 {
            return Err(failure());
        }
        let id = lines[0]
            .strip_prefix("id: ")
            .and_then(|id| id.parse::<u64>().ok())
            .ok_or_else(failure)?;
        let event = lines[1].strip_prefix("event: ").ok_or_else(failure)?;
        let value: Value =
            serde_json::from_str(lines[2].strip_prefix("data: ").ok_or_else(failure)?)
                .map_err(|_| failure())?;
        if value["id"].as_u64() != Some(id) || value["type"].as_str() != Some(event) {
            return Err(failure());
        }
        Ok(value)
    }
}

struct Progress {
    session: String,
    prompt: String,
    cursor: u64,
    output: String,
    complete: bool,
    stopped: bool,
    effects: bool,
}
impl Progress {
    fn event(&mut self, value: &Value) -> Result<bool, String> {
        if (self.complete && value["type"] != "managed_journal_event")
            || self.stopped
            || !keys(value, &["v", "id", "type", "data", "promptId"])
            || value["v"] != 1
            || value["id"].as_u64() != self.cursor.checked_add(1)
            || value["data"]["sessionId"].as_str() != Some(self.session.as_str())
        {
            return Err(failure());
        }
        let data = &value["data"];
        self.cursor += 1;
        match value["type"].as_str() {
            Some("managed_journal_event") => {
                if data.get("record").is_some() {
                    self.effects = true;
                    return Err(failure());
                }
                if !keys(data, &["sessionId"]) || value.get("promptId").is_some() {
                    return Err(failure());
                }
                Ok(false)
            }
            Some("session_update") => {
                if value["promptId"].as_str() != Some(self.prompt.as_str())
                    || !keys(data, &["sessionId", "update"])
                {
                    return Err(failure());
                }
                let update = &data["update"];
                if update["sessionUpdate"] != "agent_message_chunk"
                    || !keys(update, &["sessionUpdate", "content"])
                    || update["content"]["type"] != "text"
                    || !keys(&update["content"], &["type", "text"])
                {
                    self.effects = true;
                    return Err(failure());
                }
                let text = update["content"]["text"].as_str().ok_or_else(failure)?;
                if self.output.len().saturating_add(text.len()) > MAX_OUTPUT {
                    return Err(failure());
                }
                self.output.push_str(text);
                Ok(!text.is_empty())
            }
            Some("turn_complete") => {
                if value["promptId"].as_str() != Some(self.prompt.as_str())
                    || data["promptId"].as_str() != Some(self.prompt.as_str())
                    || !keys(data, &["sessionId", "promptId", "stopReason"])
                {
                    return Err(failure());
                }
                match data["stopReason"].as_str() {
                    Some("end_turn") if !self.output.trim().is_empty() => self.complete = true,
                    Some("cancelled") => self.stopped = true,
                    _ => return Err(failure()),
                }
                Ok(false)
            }
            Some("turn_error") => Err(failure()), // flattened errors NEVER authorize account rotation
            _ => Err(failure()),
        }
    }
}

fn events(
    http: Http,
    path: String,
    epoch: String,
    cursor: u64,
    stop: Arc<AtomicBool>,
    sender: mpsc::SyncSender<Value>,
) -> JoinHandle<bool> {
    thread::spawn(move || {
        tauri::async_runtime::block_on(async {
            let read = async {
                let mut response = http
                    .client
                    .get(format!("{}{path}", http.url))
                    .headers(http.headers.clone())
                    .header("Last-Event-ID", cursor.to_string())
                    .header("X-Qwen-Event-Epoch", &epoch)
                    .timeout(Duration::from_secs(330))
                    .send()
                    .await
                    .map_err(|_| ())?;
                if response.status().as_u16() != 200
                    || response
                        .headers()
                        .get(CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        != Some("text/event-stream")
                    || response
                        .headers()
                        .get(CACHE_CONTROL)
                        .and_then(|v| v.to_str().ok())
                        != Some("no-store")
                    || response
                        .headers()
                        .get("X-Qwen-Event-Epoch")
                        .and_then(|v| v.to_str().ok())
                        != Some(epoch.as_str())
                    || response
                        .headers()
                        .get("X-Qwen-Harness-Boot-Id")
                        .and_then(|v| v.to_str().ok())
                        != http.boot.as_deref()
                {
                    return Err(());
                }
                let mut parser = Sse::default();
                while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
                    for mut event in parser.push(&chunk).map_err(|_| ())? {
                        loop {
                            match sender.try_send(event) {
                                Ok(()) => break,
                                Err(mpsc::TrySendError::Full(value)) => {
                                    event = value;
                                    tokio::time::sleep(Duration::from_millis(5)).await;
                                }
                                Err(mpsc::TrySendError::Disconnected(_)) => return Err(()),
                            }
                        }
                    }
                }
                // DELETE closes the source stream before its HTTP receipt can
                // reach the parent. EOF is transport completion only: drive
                // independently requires terminal text, canonical storage,
                // quiescence and a sealed DELETE receipt. Early EOF disconnects
                // the consumer before its terminal event and fails that gate.
                if parser.bytes.is_empty() {
                    Ok::<(), ()>(())
                } else {
                    Err(())
                }
            };
            tokio::select! {
                result = read => result.is_ok(),
                // Forced cancellation establishes ownership drain, never a
                // cleanly parsed stream completion.
                _ = async { while !stop.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(25)).await; } } => false,
            }
        })
    })
}

/// Checkpoint receives the complete accumulated text. Every exit path cancels
/// or settles, seals if quiescent, closes SSE, reaps the daemon group and joins
/// the diagnostic readers and private durable-store listener. No replay/failover.
pub(super) fn drive(
    mut child: OwnedChild,
    mut controller: Controller,
    cancelled: &Arc<AtomicBool>,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<Outcome, String> {
    drop(child.stdin.take());
    let stdout = child.stdout.take().ok_or_else(failure)?;
    let stderr = child.stderr.take().ok_or_else(failure)?;
    let (ports, port_receiver) = mpsc::sync_channel(4);
    let bad = Arc::new(AtomicBool::new(false));
    let stdout_reader = diagnostics(stdout, ports.clone(), bad.clone());
    let stderr_reader = diagnostics(stderr, ports, bad.clone());
    let mut http: Option<Http> = None;
    let mut created = false;
    let mut progress = Progress {
        session: controller.session_id.clone(),
        prompt: controller.prompt_id.clone(),
        cursor: 0,
        output: String::new(),
        complete: false,
        stopped: false,
        effects: false,
    };
    let sse_stop = Arc::new(AtomicBool::new(false));
    let mut sse_reader: Option<JoinHandle<bool>> = None;
    // Retain the receiver through cancel/status/delete so the reader cannot
    // mistake intentional settlement cleanup for a disconnected consumer.
    let mut sse_receiver: Option<mpsc::Receiver<Value>> = None;
    let turn = (|| -> Result<(), String> {
        let start = Instant::now();
        let selected_port = loop {
            if cancelled.load(Ordering::SeqCst)
                || bad.load(Ordering::SeqCst)
                || super::runtime::exit_pending(&child)?
                || start.elapsed() > Duration::from_secs(30)
            {
                return Err(failure());
            }
            match port_receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(port) => break port,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(failure()),
            }
        };
        http = Some(Http::new(selected_port, &controller.token)?);
        let connection = http.as_mut().ok_or_else(failure)?;
        let capabilities = loop {
            if cancelled.load(Ordering::SeqCst)
                || bad.load(Ordering::SeqCst)
                || super::runtime::exit_pending(&child)?
                || start.elapsed() > Duration::from_secs(40)
            {
                return Err(failure());
            }
            let (status, value) = connection.request(Method::GET, "/capabilities", None)?;
            if status == 200 {
                break value;
            }
            if status != 503 {
                return Err(failure());
            }
            thread::sleep(Duration::from_millis(100));
        };
        let harness = &capabilities["hostedHarness"];
        let boot = harness["bootId"]
            .as_str()
            .filter(|id| canonical_uuid(id))
            .ok_or_else(failure)?;
        if capabilities["v"] != 1
            || capabilities["qwenCodeVersion"].as_str() != Some(VERSION)
            || capabilities["features"] != json!(["hosted_harness_private_v1"])
            || capabilities["mode"] != "http-bridge"
            || !keys(harness, &["protocolVersions", "bootId", "capabilityDigest"])
            || harness["capabilityDigest"].as_str() != Some(controller.capability_digest.as_str())
            || harness["protocolVersions"]["current"] != 1
            || harness["protocolVersions"]["supported"] != json!([1])
            || !keys(&harness["protocolVersions"], &["current", "supported"])
        {
            return Err(failure());
        }
        controller.store.bind_boot(boot)?;
        connection.bind(boot, None)?;
        let body = json!({"sessionId":controller.session_id,"sessionScope":"thread","managedSessionStore":{
            "baseUrl":controller.store.base_url(),"tenantId":controller.tenant_id,"workspaceId":controller.workspace_id,
            "writerId":boot,"leaseDurationMs":60000}});
        // A partial/malformed admission response may still have created state:
        // cleanup always attempts cancel/status/delete after POST was issued.
        created = true;
        let (status, session) = connection.request(Method::POST, "/session", Some(&body))?;
        if status != 200
            || !keys(
                &session,
                &[
                    "sessionId",
                    "clientId",
                    "workspaceCwd",
                    "lastEventId",
                    "eventEpoch",
                ],
            )
            || session["sessionId"].as_str() != Some(controller.session_id.as_str())
            || session["workspaceCwd"].as_str() != controller.root.to_str()
        {
            return Err(failure());
        }
        let client = session["clientId"]
            .as_str()
            .filter(|id| canonical_uuid(id))
            .ok_or_else(failure)?;
        // The source defines the event epoch as this boot UUID with '_' in
        // place of '-'; it is not itself a UUID.
        let epoch = boot.replace('-', "_");
        if session["eventEpoch"].as_str() != Some(epoch.as_str()) {
            return Err(failure());
        }
        progress.cursor = session["lastEventId"].as_u64().ok_or_else(failure)?;
        connection.bind(boot, Some(client))?;
        if cancelled.load(Ordering::SeqCst) || !controller.store.healthy() {
            return Err(failure());
        }
        let body = json!({"promptId":controller.prompt_id,"prompt":controller.prompt,"payloadDigest":controller.payload_digest,"deadlineMs":300000});
        let path = format!("/session/{}/prompt", controller.session_id);
        let (status, admission) = connection.request(Method::POST, &path, Some(&body))?;
        if status != 202
            || !keys(&admission, &["promptId", "lastEventId", "eventEpoch"])
            || admission["promptId"].as_str() != Some(controller.prompt_id.as_str())
            || admission["eventEpoch"].as_str() != Some(epoch.as_str())
            || !admission["lastEventId"]
                .as_u64()
                .is_some_and(|id| id >= progress.cursor)
        {
            return Err(failure());
        }
        let (sender, receiver) = mpsc::sync_channel(16);
        sse_receiver = Some(receiver);
        sse_reader = Some(events(
            connection.clone(),
            format!("/session/{}/events", controller.session_id),
            epoch,
            progress.cursor,
            sse_stop.clone(),
            sender,
        ));
        let deadline = Instant::now();
        while !progress.complete && !progress.stopped {
            if cancelled.load(Ordering::SeqCst)
                || bad.load(Ordering::SeqCst)
                || !controller.store.healthy()
                || super::runtime::exit_pending(&child)?
                || deadline.elapsed() > Duration::from_secs(310)
            {
                return Err(failure());
            }
            match sse_receiver
                .as_ref()
                .ok_or_else(failure)?
                .recv_timeout(Duration::from_millis(50))
            {
                Ok(event) => {
                    if progress.event(&event)? {
                        checkpoint(&progress.output)?;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(failure()),
            }
        }
        if progress.stopped {
            return Err(failure());
        }
        let final_text = controller
            .store
            .final_assistant(&controller.prompt_id, &controller.model)?;
        if final_text != progress.output || final_text.trim().is_empty() {
            return Err(failure());
        }
        Ok(())
    })();

    let mut sealed = !created;
    if created {
        if let Some(connection) = &http {
            let base = format!("/session/{}", controller.session_id);
            if turn.is_err() || cancelled.load(Ordering::SeqCst) {
                let _ = connection.request(Method::POST, &format!("{base}/cancel"), None);
            }
            let deadline = Instant::now();
            let mut quiescent = false;
            while deadline.elapsed() < Duration::from_secs(15) {
                match connection.request(Method::GET, &format!("{base}/status"), None) {
                    Ok((200, status))
                        if keys(
                            &status,
                            &["sessionId", "hasActivePrompt", "recoveryBlocked"],
                        ) && status["sessionId"].as_str()
                            == Some(controller.session_id.as_str()) =>
                    {
                        if status["recoveryBlocked"] != false {
                            break;
                        }
                        if status["hasActivePrompt"] == false {
                            quiescent = true;
                            break;
                        }
                        if status["hasActivePrompt"] != true {
                            break;
                        }
                    }
                    _ => break,
                }
                thread::sleep(Duration::from_millis(50));
            }
            if quiescent {
                sealed = connection
                    .request(Method::DELETE, &base, None)
                    .is_ok_and(|(status, _)| status == 204)
                    && controller.store.sealed();
            }
        }
    }
    // DELETE closes the pinned server's streams before returning 204. A
    // successful turn must consume the administrative tail and reach natural
    // EOF; forced cancellation cannot conceal malformed trailing data.
    let mut tail_valid = true;
    if turn.is_ok() && sealed && !cancelled.load(Ordering::SeqCst) {
        if let Some(receiver) = &sse_receiver {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(event) => {
                        if progress.event(&event).is_err() {
                            tail_valid = false;
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if Instant::now() >= deadline {
                            tail_valid = false;
                            break;
                        }
                    }
                }
            }
        }
    } else {
        tail_valid = false;
    }
    if !tail_valid {
        sse_stop.store(true, Ordering::SeqCst);
    }
    let (sse_joined, sse_valid) = match sse_reader.take().map(JoinHandle::join) {
        None => (true, true),
        Some(Ok(valid)) => (true, valid),
        Some(Err(_)) => (false, false),
    };
    let reaped = child.stop_and_wait().is_ok();
    // Retry owned termination through Drop before joining pipe readers if the
    // reap failed; readers must not outlive the process ownership boundary.
    drop(child);
    let stdout_result = stdout_reader.join();
    let stderr_result = stderr_reader.join();
    let stdout_joined = stdout_result.is_ok();
    let stderr_joined = stderr_result.is_ok();
    let diagnostics_valid = stdout_result.unwrap_or(false) && stderr_result.unwrap_or(false);
    let healthy = controller.store.healthy();
    let effects = progress.effects || controller.store.effect_denied();
    let store_joined = controller.store.stop_and_join().is_ok();
    let drained = reaped && stdout_joined && stderr_joined && sse_joined && store_joined;
    let stopped = cancelled.load(Ordering::SeqCst) || progress.stopped;
    let valid = turn.is_ok()
        && sealed
        && healthy
        && drained
        && sse_valid
        && tail_valid
        && diagnostics_valid
        && !bad.load(Ordering::SeqCst)
        && !effects
        && !stopped;
    Ok(Outcome {
        completed: valid && progress.complete,
        exhausted: false,
        auth_failed: false,
        effects,
        stopped,
        drained,
        valid,
        output: progress.output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn progress() -> Progress {
        Progress {
            session: "session".into(),
            prompt: "prompt".into(),
            cursor: 5,
            output: String::new(),
            complete: false,
            stopped: false,
            effects: false,
        }
    }
    fn text() -> Value {
        json!({"v":1,"id":6,"type":"session_update","promptId":"prompt","data":{"sessionId":"session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}}})
    }
    fn complete() -> Value {
        json!({"v":1,"id":7,"type":"turn_complete","promptId":"prompt","data":{"sessionId":"session","promptId":"prompt","stopReason":"end_turn"}})
    }
    #[test]
    fn fragmented_sse_preserves_exact_correlated_committed_text() {
        let wire = format!("id: 6\nevent: session_update\ndata: {}\n\n", text());
        let mut parser = Sse::default();
        let mut all = Vec::new();
        for byte in wire.bytes() {
            all.extend(parser.push(&[byte]).unwrap());
        }
        assert_eq!(all, vec![text()]);
        let mut p = progress();
        assert!(p.event(&all[0]).unwrap());
        assert_eq!(p.output, "answer");
        assert!(!p.complete);
        p.event(&complete()).unwrap();
        assert!(p.complete);
        assert!(p.event(&complete()).is_err());
    }
    #[test]
    fn completion_only_allows_contiguous_empty_administrative_tail() {
        let mut settled = progress();
        settled.event(&text()).unwrap();
        settled.event(&complete()).unwrap();
        let tail =
            json!({"v":1,"id":8,"type":"managed_journal_event","data":{"sessionId":"session"}});
        assert!(!settled.event(&tail).unwrap());
        assert_eq!(settled.output, "answer");
        for bad in [
            json!({"v":1,"id":9,"type":"managed_journal_event","promptId":"prompt","data":{"sessionId":"session"}}),
            json!({"v":1,"id":9,"type":"managed_journal_event","data":{"sessionId":"other"}}),
            json!({"v":1,"id":9,"type":"managed_journal_event","data":{"sessionId":"session","record":{"type":"tool_result"}}}),
            json!({"v":1,"id":10,"type":"managed_journal_event","data":{"sessionId":"session"}}),
            json!({"v":1,"id":9,"type":"turn_complete","promptId":"prompt","data":{"sessionId":"session","promptId":"prompt","stopReason":"end_turn"}}),
        ] {
            let mut p = progress();
            p.event(&text()).unwrap();
            p.event(&complete()).unwrap();
            p.event(&tail).unwrap();
            assert!(p.event(&bad).is_err());
        }
        let mut parser = Sse::default();
        parser
            .push(b"id: 9\nevent: managed_journal_event\ndata: {")
            .unwrap();
        assert!(!parser.bytes.is_empty());
    }
    #[test]
    fn mismatched_cursor_prompt_session_unknown_fields_and_tools_fail() {
        for (pointer, bad) in [
            ("/id", json!(7)),
            ("/v", json!(2)),
            ("/promptId", json!("other")),
            ("/data/sessionId", json!("other")),
            ("/data/update/sessionUpdate", json!("tool_call")),
            ("/data/update/content/type", json!("image")),
        ] {
            let mut value = text();
            *value.pointer_mut(pointer).unwrap() = bad;
            assert!(progress().event(&value).is_err());
        }
        let mut value = text();
        value["extra"] = json!(true);
        assert!(progress().event(&value).is_err());
        let mut p = progress();
        let event = json!({"v":1,"id":6,"type":"managed_journal_event","data":{"sessionId":"session","record":{"type":"tool_result"}}});
        assert!(p.event(&event).is_err());
        assert!(p.effects);
        assert!(progress().event(&complete()).is_err());
    }
    #[test]
    fn wire_ids_headers_and_frames_cannot_be_ambiguously_reinterpreted() {
        assert!(
            Sse::frame(b"id: 6\nevent: error\ndata: {\"id\":6,\"type\":\"session_update\"}")
                .is_err()
        );
        assert!(Sse::frame(b"data: {}\ndata: {}").is_err());
        assert!(Sse::default().push(&vec![b'x'; MAX_FRAME + 1]).is_err());
        assert_eq!(
            port(b"qwen serve listening on http://127.0.0.1:12345 (mode=code)\n"),
            Some(12345)
        );
        assert_eq!(
            port(b"qwen serve listening on http://example.com:12345 (mode=code)\n"),
            None
        );
        assert_eq!(
            port(b"qwen serve listening on http://127.0.0.1:0 (mode=code)\n"),
            None
        );
        assert!(canonical_uuid("12345678-1234-4123-8123-123456789abc"));
        assert!(!canonical_uuid("12345678-1234-4123-8123-123456789ABC"));
    }
    #[test]
    fn exact_owned_provider_settings_never_store_credentials() {
        let c = settings("gpt-4.1");
        assert_eq!(c["telemetry"]["enabled"], false);
        assert_eq!(c["modelProviders"]["openai"][0]["envKey"], "OPENAI_API_KEY");
        assert_eq!(
            c["modelProviders"]["openai"][0]["generationConfig"]["maxRetries"],
            0
        );
        let mut command = Command::new("unused-fixture");
        command.env_clear();
        environment(&mut command, Path::new("/private/attempt"), "gpt-4.1");
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new("/private/attempt"))
        );
        assert!(command.get_envs().all(|(key, _)| key != "OPENAI_API_KEY"));
        assert!(version_matches("0.24.7\n"));
        assert!(!version_matches("0.24.8"));
        for id in [
            None,
            Some("gpt"),
            Some("openai/gpt-4.1"),
            Some("gpt-4.1:high"),
        ] {
            assert!(model(id).is_err());
        }
    }
}
