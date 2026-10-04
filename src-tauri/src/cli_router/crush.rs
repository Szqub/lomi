//! Native Crush 0.97.1, e0baf255be53f932042b377630e233d4c33c3b4b.
//! PRIVATE pinned HTTP/SSE v1 over an owned Unix socket; never the attaching
//! `run` frontend. Source: internal/{cmd/server,config/{load,config,provider},
//! agent/{coordinator,agent},backend/{backend,agent},server/{proto,events},
//! proto/{proto,message},pubsub/broker}. Fantasy 0.45.1 is also pinned by go.mod.
//! Exact builtin denial is applied before tool registration. Empty HOME/cwd,
//! PATH pointing to an empty owned directory, absent system JSON and crushrc
//! files fence shell config, Git helpers, instructions, hooks, MCP and skills.
//!
//! Native behavior: App.New makes one secret-free GitHub latest-release GET
//! (30s deadline; native redirects; only literal User-Agent/Accept headers).
//! It never sends configuration, credentials, prompt or history to that URL.
//! The detached first-title helper tries small then large (same fixed model),
//! each with <=1024 output tokens and Fantasy's 3 retries. Main inference also
//! has 3 native retries; an env-key 401 permits one same-key re-resolution and
//! another retry pass. No router replay, quota inference or zero-auxiliary claim.
//! Title goroutines remain inside the owned foreground process and are killed
//! at drain. Only final answer text crosses the fresh logical-context handoff.
//! macOS only: clipboard dependency v0.9.0 initializes system AppKit symbols,
//! then initialize() returns nil; it does not read/write clipboard contents.

use super::{runtime::OwnedChild, transport::Outcome};
use reqwest::{
    header::{CACHE_CONTROL, CONTENT_TYPE},
    Client, Method, Response,
};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) const VERSION: &str = "0.97.1";
pub(crate) const PIN: &str = "e0baf255be53f932042b377630e233d4c33c3b4b";
const MODEL: &str = "claude-sonnet-4-6";
const MODELS: &[&str] = &[MODEL];
const MAX_INPUT: usize = 60 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_JSON: usize = 6 * MAX_OUTPUT + 65536;
const MAX_WIRE: usize = 64 * 1024 * 1024;
const TOOLS: &[&str] = &[
    "agent",
    "bash",
    "crush_info",
    "crush_logs",
    "job_output",
    "job_kill",
    "download",
    "edit",
    "multiedit",
    "lsp_diagnostics",
    "lsp_references",
    "lsp_restart",
    "lsp_symbols",
    "lsp_definition",
    "lsp_call_hierarchy",
    "lsp_rename",
    "lsp_replace_symbol",
    "fetch",
    "agentic_fetch",
    "glob",
    "grep",
    "ls",
    "question",
    "sourcegraph",
    "todos",
    "view",
    "write",
    "list_mcp_resources",
    "read_mcp_resource",
];
const SKILLS: &[&str] = &["crush-config", "crush-hooks", "jq"];
fn failure() -> String {
    "Crush violated the owned native text-turn contract.".into()
}
pub(crate) fn supported_models() -> &'static [&'static str] {
    MODELS
}
pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    value.filter(|v| MODELS.contains(v)).ok_or_else(|| {
        "Crush requires the approved native Anthropic API model. No request was sent.".into()
    })
}
pub(crate) fn version_matches(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .is_some_and(|v| matches!(v.trim(), "crush version 0.97.1" | "crush version v0.97.1"))
}
/// Resolve the executable natively before this check; shell/npm wrappers are
/// outside this adapter. The final dispatch fence must also compare the
/// resolver's executable fingerprint. Public release ldflags attest VERSION
/// only, so `commit=unknown` is not advertised as a source-build attestation.
pub(crate) fn executable_admission(path: &Path) -> Result<(), String> {
    let before = fs::symlink_metadata(path).map_err(|_| failure())?;
    if !before.is_file()
        || before.file_type().is_symlink()
        || before.len() < 32
        || before.len() > 512 * 1024 * 1024
    {
        return Err(failure());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if before.mode() & 0o022 != 0
            || before.mode() & 0o111 == 0
            || ![0, unsafe { libc::geteuid() }].contains(&before.uid())
        {
            return Err(failure());
        }
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(path).map_err(|_| failure())?;
    let opened = file.metadata().map_err(|_| failure())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err(failure());
        }
    }
    let mut header = [0u8; 16];
    file.read_exact(&mut header).map_err(|_| failure())?;
    let cpu = if cfg!(target_arch = "aarch64") {
        0x0100000cu32
    } else {
        0x01000007u32
    };
    if header[..4] != [0xcf, 0xfa, 0xed, 0xfe]
        || u32::from_le_bytes(header[4..8].try_into().map_err(|_| failure())?) != cpu
        || u32::from_le_bytes(header[12..16].try_into().map_err(|_| failure())?) != 2
    {
        return Err(failure());
    }
    Ok(())
}
/// Captured before any version probe and compared under the dispatch owner
/// fence. This detects replacement/mutation; it is not a release attestation.
pub(crate) fn executable_fingerprint(path: &Path) -> Result<Vec<u64>, String> {
    executable_admission(path)?;
    let meta = fs::symlink_metadata(path).map_err(|_| failure())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(vec![
            meta.dev(),
            meta.ino(),
            meta.len(),
            meta.mtime() as u64,
            meta.mtime_nsec() as u64,
            meta.ctime() as u64,
            meta.ctime_nsec() as u64,
            meta.mode() as u64,
            meta.uid() as u64,
        ])
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Err(failure())
    }
}
fn empty(value: &Value) -> bool {
    value.is_null()
        || value.as_object().is_some_and(|v| v.is_empty())
        || value.as_array().is_some_and(Vec::is_empty)
}
fn keys(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|v| v.keys().all(|k| allowed.contains(&k.as_str())))
}
fn identifier(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn uuid() -> Result<String, String> {
    let mut bytes = super::new_id()?.into_bytes();
    if bytes.len() != 32 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err(failure());
    }
    bytes[12] = b'4';
    bytes[16] = b'8';
    let v = std::str::from_utf8(&bytes).map_err(|_| failure())?;
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &v[..8],
        &v[8..12],
        &v[12..16],
        &v[16..20],
        &v[20..]
    ))
}
fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(failure()),
    }
}
pub(super) fn admission(root: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err(
            "Crush native text qualification currently covers macOS only. No request was sent."
                .into(),
        );
    }
    let policy = Path::new("/etc/crush/crush.json");
    absent(policy)?;
    for parent in policy.parent().into_iter().flat_map(Path::ancestors) {
        match fs::symlink_metadata(parent) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(link) => {
                let meta = fs::metadata(parent).map_err(|_| failure())?;
                if !meta.is_dir() {
                    return Err(failure());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if link.uid() != 0 || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
                        return Err(failure());
                    }
                }
            }
            _ => return Err(failure()),
        }
    }
    for ancestor in root.ancestors() {
        absent(&ancestor.join(".git"))?;
    }
    for dir in [
        root.to_owned(),
        root.join("config"),
        root.join("global-data"),
        root.join("global-state"),
        root.join("workspace-state"),
    ] {
        for name in ["crushrc", ".crushrc"] {
            absent(&dir.join(name))?;
        }
    }
    Ok(())
}
fn settings(root: &Path) -> Value {
    let selection = json!({"provider":"lomi","model":MODEL,"max_tokens":8192,"think":false,"provider_options":{"extra_body":{"thinking":{"type":"disabled"}}}});
    json!({"providers":{"lomi":{"id":"lomi","name":"Lomi Anthropic","type":"anthropic","base_url":"https://api.anthropic.com","api_key":"$LOMI_CRUSH_API_KEY","discover_models":false,"models":[{"id":MODEL,"name":MODEL,"context_window":200000,"default_max_tokens":1024,"can_reason":true,"supports_attachments":false}]}},
        "models":{"large":selection,"small":selection},"mcp":{},"lsp":{},"hooks":{},"env":{},
        "permissions":{"allowed_tools":[]},"options":{"data_directory":root.join("workspace-state"),"disabled_tools":TOOLS,"disabled_skills":SKILLS,"disable_auto_summarize":true,"disable_provider_auto_update":true,"disable_default_providers":true,"disable_metrics":true,"auto_lsp":false,"notifications":"disabled","progress":false,"request_timeout":60,"skills_paths":[root.join("empty-skills")],"global_context_paths":[root.join("empty-context")]}})
}
pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
    model(Some(id))?;
    let root = parent.join(format!("crush-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| failure())?;
    crate::chat::storage::private(&root, true)?;
    let root = root.canonicalize().map_err(|_| failure())?;
    for name in [
        "config",
        "global-data",
        "global-state",
        "workspace-state",
        "data",
        "state",
        "cache",
        "tmp",
        "runtime",
        "empty-bin",
        "empty-skills",
        "empty-context",
    ] {
        let dir = root.join(name);
        fs::create_dir(&dir).map_err(|_| failure())?;
        crate::chat::storage::private(&dir, true)?;
    }
    admission(&root)?;
    crate::chat::storage::atomic(
        &root.join("config/crush.json"),
        &serde_json::to_vec(&settings(&root)).map_err(|_| failure())?,
    )?;
    Ok(root)
}
/// Caller MUST env_clear() first and inject only LOMI_CRUSH_API_KEY. The
/// selected credential is resolved at server startup; no model call precedes
/// the private-socket identity/config checks. Do not preserve PATH/proxy vars.
pub(crate) fn environment(command: &mut Command, root: &Path) {
    command
        .current_dir(root)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("PATH", root.join("empty-bin"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        .env("CRUSH_GLOBAL_CONFIG", root.join("config"))
        .env("CRUSH_GLOBAL_DATA", root.join("global-data"))
        .env("CRUSH_CACHE_DIR", root.join("cache"))
        .env("CRUSH_SKILLS_DIR", root.join("empty-skills"))
        .env("CRUSH_DISABLE_METRICS", "1")
        .env("DO_NOT_TRACK", "1")
        .env("CRUSH_DISABLE_PROVIDER_AUTO_UPDATE", "1")
        .env("CRUSH_DISABLE_DEFAULT_PROVIDERS", "1");
}
pub(crate) struct Controller {
    root: PathBuf,
    socket_dir: PathBuf,
    socket: PathBuf,
    client: Client,
    client_id: String,
    run_id: String,
    prompt: String,
    workspace: Option<String>,
    session: Option<String>,
    config_snapshot: RefCell<Option<Value>>,
    identity: Option<Value>,
}
impl Drop for Controller {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_dir(&self.socket_dir);
    }
}
impl Controller {
    pub(crate) fn new(root: &Path, id: &str, prompt: &str) -> Result<Self, String> {
        model(Some(id))?;
        admission(root)?;
        if prompt.trim().is_empty() || prompt.len() > MAX_INPUT {
            return Err(failure());
        }
        let client_id = uuid()?;
        let run_id = uuid()?;
        let socket_dir = Path::new("/tmp").join(format!("lomi-crush-{}", super::new_id()?));
        fs::create_dir(&socket_dir).map_err(|_| failure())?;
        let result = (|| {
            crate::chat::storage::private(&socket_dir, true)?;
            let socket_dir = socket_dir.canonicalize().map_err(|_| failure())?;
            let socket = socket_dir.join("c.sock");
            if socket.as_os_str().len() >= 104 {
                let _ = fs::remove_dir(&socket_dir);
                return Err(failure());
            }
            let builder = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .http1_only()
                .connect_timeout(Duration::from_secs(2));
            #[cfg(unix)]
            let builder = builder.unix_socket(socket.as_path());
            let client = builder.build().map_err(|_| failure())?;
            Ok(Self {
                root: root.to_owned(),
                socket_dir,
                socket,
                client,
                client_id,
                run_id,
                prompt: prompt.into(),
                workspace: None,
                session: None,
                config_snapshot: RefCell::new(None),
                identity: None,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir(&socket_dir);
        }
        result
    }
    fn path(&self, suffix: &str) -> Result<String, String> {
        Ok(format!(
            "/v1/workspaces/{}{}",
            self.workspace.as_deref().ok_or_else(failure)?,
            suffix
        ))
    }
    fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        status: u16,
    ) -> Result<Value, String> {
        tauri::async_runtime::block_on(async {
            let mut req = self
                .client
                .request(method, format!("http://localhost{path}"))
                .timeout(Duration::from_secs(5))
                .header(CACHE_CONTROL, "no-store");
            if let Some(body) = body {
                req = req.json(&body);
            }
            let mut response = req.send().await.map_err(|_| failure())?;
            if response.status().as_u16() != status
                || response
                    .content_length()
                    .is_some_and(|n| n > MAX_JSON as u64)
            {
                return Err(failure());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
                if bytes.len().saturating_add(chunk.len()) > MAX_JSON {
                    return Err(failure());
                }
                bytes.extend_from_slice(&chunk);
            }
            if bytes.is_empty() {
                return Ok(Value::Null);
            }
            if response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                != Some("application/json")
            {
                return Err(failure());
            }
            serde_json::from_slice(&bytes).map_err(|_| failure())
        })
    }
    fn config(&self) -> Result<(), String> {
        let v = self.request(Method::GET, &self.path("/config")?, None, 200)?;
        config_matches(&v, &self.root)?;
        let mut snapshot = self.config_snapshot.borrow_mut();
        if snapshot.as_ref().is_some_and(|old| old != &v) {
            return Err(failure());
        }
        if snapshot.is_none() {
            *snapshot = Some(v);
        }
        Ok(())
    }
    fn idle(&self) -> Result<bool, String> {
        let info = self.request(Method::GET, &self.path("/agent")?, None, 200)?;
        if info["is_ready"] != true
            || info["model"]["id"] != MODEL
            || info["model_cfg"]["provider"] != "lomi"
            || info["model_cfg"]["model"] != MODEL
        {
            return Err(failure());
        }
        let sid = self.session.as_deref().ok_or_else(failure)?;
        let session = self.request(
            Method::GET,
            &self.path(&format!("/sessions/{sid}"))?,
            None,
            200,
        )?;
        let queued = self.request(
            Method::GET,
            &self.path(&format!("/agent/sessions/{sid}/prompts/queued"))?,
            None,
            200,
        )?;
        if session["id"] != sid
            || !empty(&session["todos"])
            || session["summary_message_id"] != ""
            || session["parent_session_id"] != ""
            || session["channel"].as_str().is_some_and(|v| !v.is_empty())
        {
            return Err(failure());
        }
        Ok(info["is_busy"] == false && session["is_busy"] == false && queued == 0)
    }
    fn start(&mut self, cancel: &Arc<AtomicBool>) -> Result<Response, String> {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if cancel.load(Ordering::SeqCst) || Instant::now() >= until {
                return Err(failure());
            }
            if self.request(Method::GET, "/v1/health", None, 200).is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let identity = self.request(Method::GET, "/v1/version", None, 200)?;
        if !native_identity(&identity) {
            return Err(failure());
        }
        self.identity = Some(identity);
        let workspaces = self.request(Method::GET, "/v1/workspaces", None, 200)?;
        if !workspaces.as_array().is_some_and(Vec::is_empty) {
            return Err(failure());
        }
        let ws=self.request(Method::POST,"/v1/workspaces",Some(json!({"path":self.root,"data_dir":self.root.join("workspace-state"),"client_id":self.client_id,"yolo":false,"channels":[]})),200)?;
        let wid = ws["id"]
            .as_str()
            .filter(|id| identifier(id))
            .ok_or_else(failure)?;
        self.workspace = Some(wid.into());
        if ws["path"].as_str() != self.root.to_str()
            || ws["yolo"] == true
            || ws["data_dir"].as_str() != self.root.join("workspace-state").to_str()
            || !empty(&ws["channels"])
        {
            return Err(failure());
        }
        self.config()?;
        self.request(
            Method::POST,
            &self.path("/agent/init")?,
            Some(json!({"interactive":false})),
            200,
        )?;
        if !empty(&self.request(Method::GET, &self.path("/skills")?, None, 200)?)
            || !empty(&self.request(Method::GET, &self.path("/lsps")?, None, 200)?)
        {
            return Err(failure());
        }
        let sessions = self.request(Method::GET, &self.path("/sessions")?, None, 200)?;
        if !sessions.as_array().is_some_and(Vec::is_empty) {
            return Err(failure());
        }
        let session = self.request(
            Method::POST,
            &self.path("/sessions")?,
            Some(json!({"title":"Lomi managed text"})),
            200,
        )?;
        let sid = session["id"]
            .as_str()
            .filter(|id| identifier(id))
            .ok_or_else(failure)?;
        self.session = Some(sid.into());
        if !self.idle()? {
            return Err(failure());
        }
        tauri::async_runtime::block_on(async {
            let response = tokio::time::timeout(
                Duration::from_secs(5),
                self.client
                    .get(format!(
                        "http://localhost{}?client_id={}",
                        self.path("/events")?,
                        self.client_id
                    ))
                    .header(CACHE_CONTROL, "no-store")
                    .send(),
            )
            .await
            .map_err(|_| failure())?
            .map_err(|_| failure())?;
            if response.status().as_u16() != 200
                || response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    != Some("text/event-stream")
            {
                return Err(failure());
            }
            Ok(response)
        })
    }
    fn prompt(&self) -> Result<(), String> {
        self.request(Method::POST,&self.path("/agent")?,Some(json!({"session_id":self.session,"run_id":self.run_id,"prompt":self.prompt,"attachments":[],"hidden_user_message":false})),202)?;
        Ok(())
    }
    fn canonical(&self, progress: &Progress) -> Result<String, String> {
        if self.identity.as_ref() != Some(&self.request(Method::GET, "/v1/version", None, 200)?) {
            return Err(failure());
        }
        self.config()?;
        // RunComplete is published before the dispatcher's accepted-run
        // reservation is released. Observe its bounded quiescence explicitly.
        let until = Instant::now() + Duration::from_secs(5);
        while !self.idle()? {
            if Instant::now() >= until {
                return Err(failure());
            }
            thread::sleep(Duration::from_millis(25));
        }
        let sid = self.session.as_deref().ok_or_else(failure)?;
        let messages = self.request(
            Method::GET,
            &self.path(&format!("/sessions/{sid}/messages"))?,
            None,
            200,
        )?;
        canonical(&messages, sid, &self.prompt, progress)
    }
    fn cleanup(&self) -> Result<(), String> {
        if let (Some(_), Some(sid)) = (&self.workspace, &self.session) {
            self.request(
                Method::POST,
                &self.path(&format!("/agent/sessions/{sid}/cancel"))?,
                None,
                200,
            )?;
            let until = Instant::now() + Duration::from_secs(5);
            while !self.idle()? {
                if Instant::now() >= until {
                    return Err(failure());
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        self.request(
            Method::DELETE,
            &format!("/v1/clients/{}", self.client_id),
            None,
            200,
        )?;
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if self.request(Method::GET, "/v1/workspaces", None, 200)? == json!([]) {
                break;
            }
            if Instant::now() >= until {
                return Err(failure());
            }
            thread::sleep(Duration::from_millis(25));
        }
        self.shutdown()
    }
    fn retire(&self) -> Result<(), String> {
        self.request(
            Method::DELETE,
            &format!("/v1/clients/{}", self.client_id),
            None,
            200,
        )?;
        if self.request(Method::GET, "/v1/workspaces", None, 200)? != json!([]) {
            return Err(failure());
        }
        Ok(())
    }
    fn shutdown(&self) -> Result<(), String> {
        self.request(
            Method::POST,
            "/v1/control",
            Some(json!({"command":"shutdown_if_idle"})),
            200,
        )?;
        Ok(())
    }
}
pub(crate) fn arguments(command: &mut Command, c: &Controller) -> Result<(), String> {
    executable_admission(Path::new(command.get_program()))?;
    admission(&c.root)?;
    absent(&c.socket)?;
    let bytes = fs::read(c.root.join("config/crush.json")).map_err(|_| failure())?;
    if bytes.len() > 65536
        || serde_json::from_slice::<Value>(&bytes).map_err(|_| failure())? != settings(&c.root)
    {
        return Err(failure());
    }
    command
        .arg("server")
        .arg("--host")
        .arg(format!("unix://{}", c.socket.display()))
        .arg("--data-dir")
        .arg(c.root.join("global-state"));
    Ok(())
}
fn native_identity(v: &Value) -> bool {
    keys(
        v,
        &["version", "commit", "build_id", "go_version", "platform"],
    ) && matches!(v["version"].as_str(), Some(VERSION) | Some("v0.97.1"))
        && matches!(v["commit"].as_str(), Some(PIN) | Some("unknown"))
        && v["build_id"].as_str().is_some_and(|s| {
            !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        && v["platform"].as_str()
            == Some(if cfg!(target_arch = "aarch64") {
                "darwin/arm64"
            } else {
                "darwin/amd64"
            })
        && v["go_version"]
            .as_str()
            .is_some_and(|s| s.starts_with("go1.") && s.len() < 40)
}
fn config_matches(v: &Value, root: &Path) -> Result<(), String> {
    let providers = v["providers"].as_object().ok_or_else(failure)?;
    if providers.len() != 1 {
        return Err(failure());
    }
    let p = &v["providers"]["lomi"];
    if p["type"] != "anthropic"
        || p["base_url"] != "https://api.anthropic.com"
        || p["discover_models"] != false
        || p["disable"] == true
        || !empty(&p["oauth"])
        || !empty(&p["extra_headers"])
        || p["aws_auth_refresh"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        || !empty(&p["extra_body"])
        || !empty(&p["provider_options"])
        || p["api_key"]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 8192 || s.starts_with("Bearer "))
    {
        return Err(failure());
    }
    let models = p["models"].as_array().ok_or_else(failure)?;
    if models.len() != 1
        || models[0]["id"] != MODEL
        || models[0]["default_max_tokens"] != 1024
        || models[0]["context_window"] != 200000
        || models[0]["default_reasoning_effort"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        || !empty(&models[0]["reasoning_levels"])
    {
        return Err(failure());
    }
    if v["models"].as_object().is_none_or(|m| m.len() != 2) {
        return Err(failure());
    }
    for name in ["large", "small"] {
        let m = &v["models"][name];
        if m["model"] != MODEL
            || m["provider"] != "lomi"
            || m["max_tokens"] != 8192
            || m["think"] == true
            || m["reasoning_effort"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            || m["provider_options"] != json!({"extra_body":{"thinking":{"type":"disabled"}}})
        {
            return Err(failure());
        }
    }
    if !empty(&v["mcp"])
        || !empty(&v["lsp"])
        || !empty(&v["hooks"])
        || !empty(&v["env"])
        || !empty(&v["permissions"]["allowed_tools"])
    {
        return Err(failure());
    }
    let o = &v["options"];
    for key in [
        "disable_auto_summarize",
        "disable_provider_auto_update",
        "disable_default_providers",
        "disable_metrics",
    ] {
        if o[key] != true {
            return Err(failure());
        }
    }
    if o["auto_lsp"] != false
        || o["notifications"] != "disabled"
        || o["data_directory"].as_str() != root.join("workspace-state").to_str()
        || o["request_timeout"] != 60
    {
        return Err(failure());
    }
    for (key, wanted) in [("disabled_tools", TOOLS), ("disabled_skills", SKILLS)] {
        let a = o[key].as_array().ok_or_else(failure)?;
        if a.len() != wanted.len() || !wanted.iter().all(|s| a.contains(&json!(s))) {
            return Err(failure());
        }
    }
    Ok(())
}
fn parts(value: &Value, terminal: bool) -> Result<String, String> {
    let mut output = String::new();
    let mut finished = false;
    let parts = value.as_array().ok_or_else(failure)?;
    if parts.len() > 128 {
        return Err(failure());
    }
    for part in parts {
        if !keys(part, &["type", "data"]) || finished {
            return Err(failure());
        }
        match part["type"].as_str() {
            Some("text") => {
                let data = &part["data"];
                if !keys(data, &["text", "hidden"])
                    || data.get("hidden").is_some_and(|v| v != false)
                {
                    return Err(failure());
                }
                let text = data["text"].as_str().ok_or_else(failure)?;
                if output.len().saturating_add(text.len()) > MAX_OUTPUT {
                    return Err(failure());
                }
                output.push_str(text);
            }
            Some("finish") => {
                let d = &part["data"];
                if !keys(d, &["reason", "time", "message", "details"])
                    || d["reason"] != "end_turn"
                    || d["time"].as_u64().is_none()
                    || d.get("message").is_some_and(|v| v.as_str() != Some(""))
                    || d.get("details").is_some_and(|v| v.as_str() != Some(""))
                {
                    return Err(failure());
                }
                finished = true;
            }
            _ => return Err(failure()),
        }
    }
    if terminal && !finished {
        return Err(failure());
    }
    Ok(output)
}
fn canonical(
    messages: &Value,
    sid: &str,
    prompt: &str,
    progress: &Progress,
) -> Result<String, String> {
    let list = messages.as_array().ok_or_else(failure)?;
    if list.len() != 2 || !progress.complete {
        return Err(failure());
    }
    let user = list
        .iter()
        .find(|m| m["role"] == "user")
        .ok_or_else(failure)?;
    let assistant = list
        .iter()
        .find(|m| m["role"] == "assistant")
        .ok_or_else(failure)?;
    if user["session_id"] != sid
        || user["is_summary_message"] == true
        || user["parts"] != json!([{ "type": "text", "data": { "text": prompt } }])
        || assistant["session_id"] != sid
        || assistant["id"] != progress.message.as_deref().unwrap_or("")
        || assistant["model"] != MODEL
        || assistant["provider"] != "lomi"
        || assistant["is_summary_message"] == true
    {
        return Err(failure());
    }
    let text = parts(&assistant["parts"], true)?;
    if text.trim().is_empty() || text != progress.output {
        return Err(failure());
    }
    Ok(text)
}
struct Progress {
    session: String,
    run: String,
    message: Option<String>,
    output: String,
    complete: bool,
    effects: bool,
    events: usize,
}
impl Progress {
    fn new(c: &Controller) -> Result<Self, String> {
        Ok(Self {
            session: c.session.clone().ok_or_else(failure)?,
            run: c.run_id.clone(),
            message: None,
            output: String::new(),
            complete: false,
            effects: false,
            events: 0,
        })
    }
    fn event(&mut self, v: &Value) -> Result<bool, String> {
        self.events += 1;
        if self.events > 65536 || !keys(v, &["type", "payload"]) {
            return Err(failure());
        }
        let event = &v["payload"];
        if !keys(event, &["type", "payload"])
            || !matches!(event["type"].as_str(), Some("created" | "updated"))
        {
            return Err(failure());
        }
        let p = &event["payload"];
        match v["type"].as_str() {
            Some("message") => {
                if p["session_id"] != self.session || p["is_summary_message"] == true {
                    return Err(failure());
                }
                let text = parts(&p["parts"], false).map_err(|_| {
                    self.effects = p["parts"].as_array().is_some_and(|a| {
                        a.iter().any(|p| {
                            matches!(
                                p["type"].as_str(),
                                Some(
                                    "tool_call"
                                        | "tool_result"
                                        | "shell_command"
                                        | "binary"
                                        | "image_url"
                                )
                            )
                        })
                    });
                    failure()
                })?;
                match p["role"].as_str() {
                    Some("user") => Ok(false),
                    Some("assistant") => {
                        let id = p["id"]
                            .as_str()
                            .filter(|id| identifier(id))
                            .ok_or_else(failure)?;
                        if self.message.as_ref().is_some_and(|old| old != id)
                            || p["model"] != MODEL
                            || p["provider"] != "lomi"
                            || (!self.complete && !text.starts_with(&self.output))
                            || (self.complete && !self.output.starts_with(&text))
                        {
                            return Err(failure());
                        }
                        if self.complete {
                            return Ok(false);
                        }
                        self.message = Some(id.into());
                        let changed = text != self.output;
                        self.output = text;
                        Ok(changed)
                    }
                    _ => {
                        self.effects = true;
                        Err(failure())
                    }
                }
            }
            Some("run_complete") => {
                if self.complete
                    || event["type"] != "updated"
                    || !keys(
                        p,
                        &[
                            "session_id",
                            "run_id",
                            "message_id",
                            "text",
                            "error",
                            "cancelled",
                        ],
                    )
                    || p["session_id"] != self.session
                    || p["run_id"] != self.run
                    || p.get("cancelled").is_some_and(|v| v != false)
                    || p.get("error").is_some_and(|v| v.as_str() != Some(""))
                {
                    return Err(failure());
                }
                let id = p["message_id"]
                    .as_str()
                    .filter(|id| identifier(id))
                    .ok_or_else(failure)?;
                let text = p["text"].as_str().ok_or_else(failure)?;
                if text.trim().is_empty()
                    || text.len() > MAX_OUTPUT
                    || !text.starts_with(&self.output)
                    || self.message.as_ref().is_some_and(|old| old != id)
                {
                    return Err(failure());
                }
                self.message = Some(id.into());
                let changed = text != self.output;
                self.output = text.into();
                self.complete = true;
                Ok(changed)
            }
            Some("session") => {
                if p["id"] != self.session
                    || p["parent_session_id"] != ""
                    || !empty(&p["todos"])
                    || p["summary_message_id"] != ""
                    || p.get("channel").is_some_and(|v| v.as_str() != Some(""))
                {
                    return Err(failure());
                }
                Ok(false)
            }
            Some("update_available") => {
                if !keys(p, &["current_version", "latest_version", "is_development"])
                    || !matches!(p["current_version"].as_str(), Some("0.97.1" | "v0.97.1"))
                    || !p["latest_version"].as_str().is_some_and(|s| {
                        !s.is_empty()
                            && s.len() <= 128
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
                    })
                    || p["is_development"] != false
                {
                    return Err(failure());
                }
                Ok(false)
            }
            Some("agent_event") => {
                // Ordinary server RunAccepted emits this legacy notification
                // even with interactive=false; it is not completion authority.
                if !keys(
                    p,
                    &[
                        "type",
                        "message",
                        "session_id",
                        "session_title",
                        "run_id",
                        "error",
                    ],
                ) || p["type"] != "agent_finished"
                    || p["session_id"] != self.session
                    || p.get("run_id").is_some_and(|v| v.as_str() != Some(""))
                    || p.get("error").is_some_and(|v| v.as_str() != Some(""))
                    || p.get("session_title")
                        .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 8192))
                    || p["message"]
                        != json!({"id":"","role":"","session_id":"","parts":[],"model":"","provider":"","created_at":0,"updated_at":0})
                {
                    return Err(failure());
                }
                Ok(false)
            }
            _ => {
                self.effects = true;
                Err(failure())
            }
        }
    }
}
#[cfg(unix)]
fn prepare_pipe<R: std::os::fd::AsRawFd>(r: &R) -> Result<(), String> {
    let fd = r.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(failure());
    }
    Ok(())
}
#[cfg(not(unix))]
fn prepare_pipe<R>(_r: &R) -> Result<(), String> {
    Err(failure())
}
fn reader<R: Read + Send + 'static>(
    mut r: R,
    stop: Arc<AtomicBool>,
    bad: Arc<AtomicBool>,
) -> JoinHandle<bool> {
    thread::spawn(move || {
        let mut bytes = [0u8; 8192];
        let mut total = 0usize;
        loop {
            match r.read(&mut bytes) {
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
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    if stop.load(Ordering::SeqCst) {
                        return false;
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
pub(crate) fn drive(
    mut child: OwnedChild,
    mut c: Controller,
    cancel: &Arc<AtomicBool>,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<Outcome, String> {
    drop(child.stdin.take());
    let stdout = child.stdout.take().ok_or_else(failure)?;
    let stderr = child.stderr.take().ok_or_else(failure)?;
    prepare_pipe(&stdout)?;
    prepare_pipe(&stderr)?;
    let stop = Arc::new(AtomicBool::new(false));
    let bad = Arc::new(AtomicBool::new(false));
    let stdout = reader(stdout, stop.clone(), bad.clone());
    let stderr = reader(stderr, stop.clone(), bad.clone());
    let mut output = String::new();
    let mut effects = false;
    let mut retired = false;
    let result = (|| -> Result<(), String> {
        let mut stream = c.start(cancel)?;
        c.prompt()?;
        let mut progress = Progress::new(&c)?;
        let mut frame = Vec::new();
        let mut total = 0usize;
        let mut deadline = Instant::now() + Duration::from_secs(300);
        let turn = (|| -> Result<(), String> {
            loop {
                if progress.complete && !retired {
                    c.canonical(&progress)?;
                    c.retire()?;
                    retired = true;
                    deadline = Instant::now() + Duration::from_secs(5);
                }
                if cancel.load(Ordering::SeqCst)
                    || bad.load(Ordering::SeqCst)
                    || Instant::now() >= deadline
                {
                    return Err(failure());
                }
                let next = tauri::async_runtime::block_on(async {
                    tokio::time::timeout(Duration::from_millis(25), stream.chunk()).await
                });
                let chunk = match next {
                    Err(_) => continue,
                    Ok(Err(_)) => return Err(failure()),
                    Ok(Ok(None)) if retired && frame.is_empty() => break,
                    Ok(Ok(None)) => return Err(failure()),
                    Ok(Ok(Some(v))) => v,
                };
                total = total.saturating_add(chunk.len());
                if total > MAX_WIRE {
                    return Err(failure());
                }
                for segment in chunk.split_inclusive(|b| *b == b'\n') {
                    if frame.len().saturating_add(segment.len()) > MAX_JSON {
                        return Err(failure());
                    }
                    frame.extend_from_slice(segment);
                    if frame.ends_with(b"\n\n") {
                        let data = frame
                            .strip_prefix(b"data: ")
                            .and_then(|v| v.strip_suffix(b"\n\n"))
                            .ok_or_else(failure)?;
                        let value = serde_json::from_slice(data).map_err(|_| failure())?;
                        let changed = progress.event(&value)?;
                        frame.clear();
                        if changed {
                            checkpoint(&progress.output)?;
                        }
                    }
                }
            }
            Ok(())
        })();
        output = progress.output;
        effects = progress.effects;
        drop(stream);
        turn
    })();
    let clean = if retired {
        c.shutdown().is_ok()
    } else {
        c.cleanup().is_ok()
    };
    let until = Instant::now() + Duration::from_secs(5);
    let mut natural = false;
    if clean {
        loop {
            match super::runtime::exit_pending(&child) {
                Ok(true) => {
                    natural = true;
                    break;
                }
                Ok(false) if Instant::now() < until => thread::sleep(Duration::from_millis(25)),
                _ => break,
            }
        }
    }
    let status = child.stop_and_wait();
    drop(child);
    stop.store(true, Ordering::SeqCst);
    let out = stdout.join().unwrap_or(false);
    let err = stderr.join().unwrap_or(false);
    let drained = status.is_ok() && out && err;
    let completed = result.is_ok()
        && clean
        && natural
        && drained
        && status.is_ok_and(|s| s.success())
        && !bad.load(Ordering::SeqCst)
        && !cancel.load(Ordering::SeqCst);
    Ok(Outcome {
        completed,
        exhausted: false,
        auth_failed: false,
        effects,
        stopped: cancel.load(Ordering::SeqCst),
        drained,
        valid: completed,
        output,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_release_version_is_exact() {
        assert!(version_matches(b"crush version 0.97.1\n"));
        assert!(!version_matches(b"crush version 0.97.0"));
    }
    fn progress() -> Progress {
        Progress {
            session: "11111111-1111-4111-8111-111111111111".into(),
            run: "22222222-2222-4222-8222-222222222222".into(),
            message: None,
            output: String::new(),
            complete: false,
            effects: false,
            events: 0,
        }
    }
    #[test]
    fn terminal_requires_owned_run_and_canonical_end_turn() {
        let mut p = progress();
        let id = "33333333-3333-4333-8333-333333333333";
        let terminal = json!({"type":"run_complete","payload":{"type":"updated","payload":{"session_id":p.session,"run_id":p.run,"message_id":id,"text":"answer"}}});
        assert_eq!(p.event(&terminal), Ok(true));
        let messages = json!([{"id":"44444444-4444-4444-8444-444444444444","role":"user","session_id":p.session,"parts":[{"type":"text","data":{"text":"question"}}]},{"id":id,"role":"assistant","session_id":p.session,"model":MODEL,"provider":"lomi","parts":[{"type":"text","data":{"text":"answer"}},{"type":"finish","data":{"reason":"end_turn","time":1}}]}]);
        assert_eq!(
            canonical(&messages, &p.session, "question", &p),
            Ok("answer".into())
        );
        let mut truncated = messages.clone();
        truncated[1]["parts"][1]["data"]["reason"] = json!("max_tokens");
        assert!(canonical(&truncated, &p.session, "question", &p).is_err());
    }
    #[test]
    fn tool_part_and_retry_rollback_are_not_final_text() {
        assert!(parts(&json!([{"type":"tool_call","data":{"name":"bash"}}]), false).is_err());
        let mut p = progress();
        p.output = "partial".into();
        let rollback = json!({"type":"message","payload":{"type":"updated","payload":{"id":"33333333-3333-4333-8333-333333333333","session_id":p.session,"model":MODEL,"provider":"lomi","role":"assistant","parts":[{"type":"text","data":{"text":""}}]}}});
        assert!(p.event(&rollback).is_err());
        assert_eq!(p.output, "partial");
    }
    #[test]
    fn malformed_or_foreign_terminal_cannot_complete() {
        let p = progress();
        let terminal = json!({"type":"run_complete","payload":{"type":"updated","payload":{"session_id":p.session,"run_id":p.run,"message_id":"33333333-3333-4333-8333-333333333333","text":"answer"}}});
        for (field, value) in [
            ("error", json!({})),
            ("cancelled", json!(null)),
            ("run_id", json!("foreign")),
        ] {
            let mut malformed = terminal.clone();
            malformed["payload"]["payload"][field] = value;
            let mut p = progress();
            assert!(p.event(&malformed).is_err());
            assert!(!p.complete);
        }
        let mut wrong_kind = terminal;
        wrong_kind["payload"]["type"] = json!("created");
        assert!(progress().event(&wrong_kind).is_err());
    }
    #[test]
    fn legacy_finished_notification_has_no_completion_authority() {
        let mut p = progress();
        let event = json!({"type":"agent_event","payload":{"type":"updated","payload":{"type":"agent_finished","session_id":p.session,"session_title":"Lomi managed text","message":{"id":"","role":"","session_id":"","parts":[],"model":"","provider":"","created_at":0,"updated_at":0}}}});
        assert_eq!(p.event(&event), Ok(false));
        assert!(!p.complete);
        assert!(p.output.is_empty());
    }
    #[test]
    fn terminal_does_not_authorize_effects_in_the_stream_tail() {
        let mut p = progress();
        let terminal = json!({"type":"run_complete","payload":{"type":"updated","payload":{"session_id":p.session,"run_id":p.run,"message_id":"33333333-3333-4333-8333-333333333333","text":"answer"}}});
        assert_eq!(p.event(&terminal), Ok(true));
        let effect = json!({"type":"permission_request","payload":{"type":"created","payload":{"session_id":p.session,"tool_name":"bash"}}});
        assert!(p.event(&effect).is_err());
        assert!(p.effects);
        assert_eq!(p.output, "answer");
    }
}
