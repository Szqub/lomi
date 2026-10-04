//! Bounded ownership of native stdio clients and authenticated loopback servers.
//! Provider credentials stay in each native account's private namespace.
use super::{
    gateway_runtime, native_accounts,
    native_wire::{strict_json, NativeKind},
    runtime,
};
use crate::{cli_catalog::TitleCli, terminal::Shells};
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, Method, Response, Url};
use serde_json::Value;
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    os::unix::{fs::MetadataExt, io::AsRawFd},
    path::{Path, PathBuf},
    process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};
use zeroize::Zeroizing;

const MAX_FRAME: usize = super::native_wire::MAX_FRAME;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
const MAX_QUEUE: usize = 256;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn failure() -> String {
    "The owned native client transport could not establish a bounded, authenticated response."
        .into()
}
fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("The native client is stopping.".into())
    } else {
        Ok(())
    }
}
fn nonblocking(pipe: &impl AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(failure());
    }
    Ok(())
}
async fn bounded<T>(
    future: impl std::future::Future<Output = T>,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        cancelled(cancel)?;
        if Instant::now() >= deadline {
            return Err(failure());
        }
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep(Duration::from_millis(25)) => {},
        }
    }
}
fn command(
    program: &Path,
    cwd: &Path,
    launch: &super::gateway_profiles::Launch,
) -> Result<Command, String> {
    use std::os::unix::process::CommandExt;
    let mut result = Command::new(program);
    let mut paths = vec![program.parent().unwrap_or(Path::new("/usr/bin")).to_owned()];
    if let Some(value) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&value));
    }
    paths.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .map(PathBuf::from),
    );
    result
        .env_clear()
        .envs(launch.environment.iter().cloned())
        .env("PATH", std::env::join_paths(paths).map_err(|_| failure())?)
        .env("LANG", "en_US.UTF-8")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // Pi's openSync(path, "wx") uses 0666 masked by the child umask. Native
    // histories and OAuth files must stay private without changing Lomi's
    // process-wide umask or relying on the interactive shell's defaults.
    unsafe {
        result.pre_exec(|| {
            libc::umask(0o077);
            Ok(())
        });
    }
    Ok(result)
}

struct ProbeDirectory(PathBuf);
impl Drop for ProbeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn probe_directory(profile: &Path) -> Result<ProbeDirectory, String> {
    use std::os::unix::fs::DirBuilderExt;
    let parent = profile.parent().ok_or_else(failure)?;
    let path = parent.join(format!("native-version-{}", super::new_id()?));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|_| failure())?;
    Ok(ProbeDirectory(path))
}
fn drain(
    pipe: &mut impl Read,
    bytes: &mut Vec<u8>,
    total: &mut usize,
    limit: usize,
) -> Result<bool, String> {
    let mut buffer = [0u8; 16384];
    // A noisy producer cannot monopolize the native cancellation fence.
    for _ in 0..16 {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                *total = total.checked_add(n).ok_or_else(failure)?;
                if *total > limit || bytes.len().saturating_add(n) > MAX_FRAME {
                    return Err(failure());
                }
                bytes.extend_from_slice(&buffer[..n]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => return Err(failure()),
        }
    }
    Ok(false)
}
fn version(
    identity: &ExecutableIdentity,
    grok_identity: Option<&super::grok_artifact::FileIdentity>,
    profile: &Path,
    cli: TitleCli,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let directory = probe_directory(profile)?;
    let launch = native_accounts::prepare(&directory.0, cli)?;
    let mut cmd = command(&identity.0, &directory.0, &launch)?;
    cmd.args(&launch.arguments)
        .arg("--version")
        .stdin(Stdio::null());
    if gateway_runtime::executable(&identity.0)? != *identity {
        return Err("The reviewed native executable changed before version inspection.".into());
    }
    native_accounts::admit_executable(cli, &identity.0)?;
    if let Some(expected) = grok_identity {
        if &super::grok_artifact::admit(&identity.0)? != expected {
            return Err(failure());
        }
    }
    cancelled(cancel)?;
    let mut child = runtime::OwnedChild::new(cmd.spawn().map_err(|_| failure())?);
    let mut stdout = child.stdout.take().ok_or_else(failure)?;
    let mut stderr = child.stderr.take().ok_or_else(failure)?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut output, mut diagnostics) = (Vec::new(), Vec::new());
    let (mut out_count, mut err_count) = (0, 0);
    let (mut out_eof, mut err_eof) = (false, false);
    loop {
        cancelled(cancel)?;
        if Instant::now() >= deadline {
            return Err(failure());
        }
        if !out_eof {
            out_eof = drain(&mut stdout, &mut output, &mut out_count, 32768)?;
        }
        if !err_eof {
            err_eof = drain(&mut stderr, &mut diagnostics, &mut err_count, 32768)?;
        }
        if runtime::exit_pending(&child)? {
            let status = child.stop_and_wait()?;
            if !out_eof {
                out_eof = drain(&mut stdout, &mut output, &mut out_count, 32768)?;
            }
            if !err_eof {
                err_eof = drain(&mut stderr, &mut diagnostics, &mut err_count, 32768)?;
            }
            if !status.success() || !out_eof || !err_eof {
                return Err(failure());
            }
            NativeKind::from_version(cli, std::str::from_utf8(&output).map_err(|_| failure())?)?;
            return super::gateway_profiles::admitted_version(cli, &output).ok_or_else(|| {
                "This native client version has no reviewed protocol contract.".into()
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
}

enum Authentication {
    Bearer(Zeroizing<String>),
    Basic {
        username: &'static str,
        password: Zeroizing<String>,
    },
}
struct Local {
    origin: Url,
    client: Client,
    auth: Authentication,
    cwd: String,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && !matches!(value, "." | "..")
}
fn allowed_path(cli: TitleCli, method: &str, path: &str) -> bool {
    if cli == TitleCli::Kimi && method == "GET" {
        if let Some(path) = path.strip_suffix("?status=pending") {
            let parts: Vec<_> = path
                .strip_prefix('/')
                .unwrap_or_default()
                .split('/')
                .collect();
            return matches!(parts.as_slice(), ["sessions", session, "approvals"] if identifier(session));
        }
    }
    if path.len() > 2048 || !path.starts_with('/') || path.contains(['?', '#', '%', '\\']) {
        return false;
    }
    let parts: Vec<_> = path[1..].split('/').collect();
    match cli {
        TitleCli::Kimi => match (method, parts.as_slice()) {
            ("GET", ["meta"] | ["sessions"] | ["oauth", "userinfo"] | ["oauth", "usage"]) => true,
            ("POST", ["sessions"]) => true,
            ("GET", ["sessions", session]) => identifier(session),
            ("GET", ["sessions", session, "snapshot" | "prompts" | "tasks"]) => identifier(session),
            ("POST", ["sessions", session, "prompts"]) => identifier(session),
            ("POST", ["sessions", session, "approvals" | "questions", request]) => {
                identifier(session) && identifier(request)
            }
            ("POST", ["sessions", session, "prompts", request]) => {
                identifier(session) && request.strip_suffix(":abort").is_some_and(identifier)
            }
            ("POST", ["sessions", session]) => {
                session.strip_suffix(":abort").is_some_and(identifier)
            }
            _ => false,
        },
        TitleCli::Kilo | TitleCli::Opencode => match (method, parts.as_slice()) {
            (
                "GET",
                ["global", "health"]
                | ["event"]
                | ["session"]
                | ["provider"]
                | ["permission"]
                | ["question"],
            ) => true,
            ("GET", ["kilo", "profile" | "auth-status"]) => cli == TitleCli::Kilo,
            ("POST", ["session"]) => true,
            ("GET", ["session", session]) => identifier(session),
            ("GET", ["session", session, "message" | "status" | "todo" | "diff"]) => {
                identifier(session)
            }
            ("POST", ["session", session, "prompt_async" | "abort"]) => identifier(session),
            ("POST", ["permission" | "question", request, "reply" | "reject"]) => {
                identifier(request)
            }
            ("POST", ["session", session, "permissions", request]) => {
                identifier(session) && identifier(request)
            }
            _ => false,
        },
        _ => false,
    }
}
fn ready_origin(cli: TitleCli, line: &[u8], auth: &Authentication) -> Result<Option<Url>, String> {
    let line = std::str::from_utf8(line)
        .map_err(|_| failure())?
        .trim_end_matches(['\r', '\n']);
    let prefix = match cli {
        TitleCli::Kimi => "Kimi server: ",
        TitleCli::Kilo => "kilo server listening on ",
        TitleCli::Opencode => "opencode server listening on ",
        _ => return Err(failure()),
    };
    let Some(rest) = line.strip_prefix(prefix) else {
        return Ok(None);
    };
    let raw = match auth {
        Authentication::Bearer(token) => {
            let (origin, actual) = rest.split_once("/#token=").ok_or_else(failure)?;
            if actual != token.as_str() {
                return Err(failure());
            }
            origin
        }
        Authentication::Basic { .. } => rest,
    };
    let url = Url::parse(raw).map_err(|_| failure())?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none_or(|port| port == 0)
        || url.path() != "/"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || raw.trim_end_matches('/')
            != format!("http://127.0.0.1:{}", url.port().ok_or_else(failure)?)
    {
        return Err(failure());
    }
    Ok(Some(url))
}

fn native_object(bytes: &[u8]) -> Result<Value, String> {
    let value = strict_json(bytes)?;
    if !value.is_object() {
        return Err("Native transport emitted a non-object record.".into());
    }
    Ok(value)
}

// Pinned Kimi v1 websocket heartbeat is application JSON, not WS Ping/Pong.
// Echo only the reviewed local nonce; this never creates a provider request.
fn kimi_heartbeat(value: &Value) -> Result<Option<Value>, String> {
    if value["type"] != "ping" {
        return Ok(None);
    }
    let object = value.as_object().ok_or_else(failure)?;
    let payload = value["payload"].as_object().ok_or_else(failure)?;
    let timestamp = value["timestamp"].as_str().ok_or_else(failure)?;
    let nonce = value["payload"]["nonce"].as_str().ok_or_else(failure)?;
    if object.len() != 3
        || !object.contains_key("type")
        || !object.contains_key("timestamp")
        || !object.contains_key("payload")
        || payload.len() != 1
        || !payload.contains_key("nonce")
        || timestamp.len() != 24
        || !timestamp.ends_with('Z')
        || chrono::DateTime::parse_from_rfc3339(timestamp).is_err()
        || nonce.len() != 26
        || !matches!(nonce.as_bytes().first(), Some(b'0'..=b'7'))
        || !nonce
            .bytes()
            .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
    {
        return Err(failure());
    }
    Ok(Some(
        serde_json::json!({"type":"pong","payload":{"nonce":nonce}}),
    ))
}

impl Process {
    pub(crate) fn exited(&self) -> bool {
        self.finished.is_some()
    }
    pub(crate) fn drained(&self) -> bool {
        self.drained && !self.transport_failed && !self.cancel.load(Ordering::SeqCst)
    }
    pub(crate) fn stop(&mut self) -> Result<(), String> {
        // Emergency cleanup deliberately provides no observation/drain proof.
        self.transport_failed = true;
        self.drained = false;
        self.events.take();
        self.socket.take();
        self.stdin.take();
        if self.finished.is_none() {
            self.finished = Some(self.child.stop_and_wait()?.success());
        }
        Ok(())
    }
    /// Return every remaining structured observation before accepting a clean
    /// transport boundary. The caller must still validate native idle/session
    /// state and reject post-terminal effects; transport drain is not settlement.
    pub(crate) fn drain(&mut self) -> Result<Vec<Value>, String> {
        let result = self.drain_inner();
        if result.is_err() {
            self.transport_failed = true;
            self.drained = false;
            self.stop()?;
        }
        result
    }
    fn drain_inner(&mut self) -> Result<Vec<Value>, String> {
        cancelled(&self.cancel)?;
        if self.transport_failed {
            return Err("Native transport observations were incomplete before drain.".into());
        }
        if self.drained {
            return Ok(Vec::new());
        }
        self.draining = true;
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let mut remaining = Vec::new();
        self.stdin.take();
        // Close Kimi's local websocket in protocol order while the owned server
        // is alive. An abrupt server kill cannot prove a valid WS close boundary.
        if let Some(socket) = &mut self.socket {
            self.ws_closing = true;
            tauri::async_runtime::block_on(bounded(socket.close(None), &self.cancel, deadline))?
                .map_err(|_| failure())?;
            while !self.ws_eof {
                cancelled(&self.cancel)?;
                if Instant::now() >= deadline {
                    return Err(
                        "Native WebSocket close did not complete before drain deadline.".into(),
                    );
                }
                if let Some(value) = self.poll_inner()? {
                    remaining.push(value);
                }
                if remaining.len() > 4096 {
                    return Err(failure());
                }
            }
            if !self.ws_close_received {
                return Err("Native WebSocket ended without a peer close acknowledgement.".into());
            }
        }
        // Request source-native orderly shutdown. Consume the HTTP stream while
        // it closes; truncated chunked HTTP is an error even after native idle.
        if self.finished.is_none() {
            let result = unsafe { libc::kill(-(self.child.id() as i32), libc::SIGTERM) };
            if result != 0 {
                return Err("Could not request native process-group shutdown.".into());
            }
        }
        loop {
            cancelled(&self.cancel)?;
            if Instant::now() >= deadline {
                return Err(
                    "Native process or event stream did not drain before its deadline.".into(),
                );
            }
            if let Some(value) = self.poll_inner()? {
                remaining.push(value);
            }
            if remaining.len() > 4096 {
                return Err(failure());
            }
            if self.finished.is_some()
                && self.stdout_eof
                && self.stderr_eof
                && self.queue.is_empty()
                && (self.events.is_none() || self.sse_eof)
                && (self.socket.is_none() || self.ws_eof)
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        if !self.stdout_buffer.is_empty() || !self.event_buffer.is_empty() {
            return Err("Native transport ended with a partial frame.".into());
        }
        cancelled(&self.cancel)?;
        self.events.take();
        self.socket.take();
        self.draining = false;
        self.drained = true;
        Ok(remaining)
    }
    fn healthy(&mut self) -> Result<(), String> {
        if cancelled(&self.cancel).is_err() {
            self.stop()?;
            return Err("The native client is stopping.".into());
        }
        if self.finished.is_some() {
            return Err("The owned native client has exited.".into());
        }
        Ok(())
    }
    fn lines(&mut self, stderr: bool) -> Result<(), String> {
        let mut lines = Vec::new();
        {
            let buffer = if stderr {
                &mut self.stderr_buffer
            } else {
                &mut self.stdout_buffer
            };
            while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                lines.push(buffer.drain(..=end).collect::<Vec<_>>());
                if lines.len() > MAX_QUEUE {
                    return Err(failure());
                }
            }
        }
        for line in lines {
            if let Some(auth) = &self.server_auth {
                if let Some(origin) = ready_origin(self.cli, &line, auth)? {
                    if self.local.is_some() {
                        return Err(
                            "The native server emitted more than one readiness origin.".into()
                        );
                    }
                    let client = Client::builder()
                        .no_proxy()
                        .redirect(reqwest::redirect::Policy::none())
                        .connect_timeout(Duration::from_secs(2))
                        .pool_max_idle_per_host(0)
                        .build()
                        .map_err(|_| failure())?;
                    let auth = match auth {
                        Authentication::Bearer(token) => Authentication::Bearer(token.clone()),
                        Authentication::Basic { username, password } => Authentication::Basic {
                            username,
                            password: password.clone(),
                        },
                    };
                    self.local = Some(Local {
                        origin,
                        client,
                        auth,
                        cwd: self.cwd.clone(),
                    });
                }
                // Server diagnostics never enter the native event/quota stream.
            } else if !stderr && !line.iter().all(u8::is_ascii_whitespace) {
                if self.queue.len() >= MAX_QUEUE {
                    return Err(failure());
                }
                self.queue.push_back(native_object(&line)?);
            }
        }
        Ok(())
    }
    fn read_pipes(&mut self) -> Result<(), String> {
        if !self.stdout_eof {
            self.stdout_eof = drain(
                &mut self.stdout,
                &mut self.stdout_buffer,
                &mut self.stdout_total,
                MAX_TOTAL,
            )?;
        }
        if !self.stderr_eof {
            self.stderr_eof = drain(
                &mut self.stderr,
                &mut self.stderr_buffer,
                &mut self.stderr_total,
                MAX_TOTAL,
            )?;
        }
        self.lines(false)?;
        self.lines(true)?;
        Ok(())
    }
    fn pump(&mut self) -> Result<(), String> {
        self.healthy()?;
        self.read_pipes()?;
        // Keep the group leader unreaped until descendants holding pipes are
        // stopped. An EOF cannot stand in for this process ownership fence.
        if runtime::exit_pending(&self.child)? {
            self.finished = Some(self.child.stop_and_wait()?.success());
            self.stdin.take();
            self.read_pipes()?;
            if !self.stdout_eof || !self.stderr_eof || !self.stdout_buffer.is_empty() {
                return Err(failure());
            }
        } else if self.stdout_eof && !self.draining {
            return Err("Native stdout closed before confirmed process-group drain.".into());
        }
        Ok(())
    }
    fn wait_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        while self.local.is_none() {
            self.pump()?;
            if self.finished.is_some() || Instant::now() >= deadline {
                return Err(failure());
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.healthy()
    }
    pub(crate) fn send(&mut self, value: &Value) -> Result<(), String> {
        self.healthy()?;
        let bytes = serde_json::to_vec(value).map_err(|_| failure())?;
        if bytes.len() > MAX_FRAME {
            return Err(failure());
        }
        if let Some(socket) = &mut self.socket {
            let message = String::from_utf8(bytes).map_err(|_| failure())?;
            return tauri::async_runtime::block_on(bounded(
                socket.send(Message::Text(message.into())),
                &self.cancel,
                Instant::now() + OPERATION_TIMEOUT,
            ))?
            .map_err(|_| failure());
        }
        if self.server_auth.is_some() {
            return Err("Native server messages require the authenticated control plane.".into());
        }
        let mut bytes = bytes;
        bytes.push(b'\n');
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let mut written = 0;
        while written < bytes.len() {
            self.healthy()?;
            if Instant::now() >= deadline {
                return Err(failure());
            }
            match self
                .stdin
                .as_mut()
                .ok_or_else(failure)?
                .write(&bytes[written..])
            {
                Ok(0) => return Err(failure()),
                Ok(count) => written += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    self.pump()?;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return Err(failure()),
            }
        }
        Ok(())
    }
    pub(crate) fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, String> {
        let (status, value) = self.request_with_status(method, path, body)?;
        if !(200..300).contains(&status) {
            return Err(
                "The native read-only request returned an unsuccessful HTTP status.".into(),
            );
        }
        Ok(value)
    }
    pub(crate) fn request_with_status(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value), String> {
        let path = if self.cli == TitleCli::Kimi {
            path.strip_prefix("/api/v1").unwrap_or(path)
        } else {
            path
        };
        if !allowed_path(self.cli, method, path) || method == "GET" && body.is_some() {
            return Err("Unqualified native control-plane route.".into());
        }
        self.wait_ready()?;
        let path = if self.cli == TitleCli::Kimi {
            format!("/api/v1{path}")
        } else {
            path.into()
        };
        let local = self.local.as_ref().ok_or_else(failure)?;
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| failure())?;
        let mut builder = local.builder(method, &path)?;
        if let Some(body) = body {
            let bytes = serde_json::to_vec(body).map_err(|_| failure())?;
            if bytes.len() > MAX_FRAME {
                return Err(failure());
            }
            builder = builder
                .header("content-type", "application/json")
                .body(bytes);
        }
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let cancel = self.cancel.clone();
        tauri::async_runtime::block_on(async {
            let mut response = bounded(builder.send(), &cancel, deadline)
                .await?
                .map_err(|_| failure())?;
            let status = response.status().as_u16();
            if response.status().is_redirection()
                || response.headers().contains_key("content-encoding")
                || response
                    .content_length()
                    .is_some_and(|count| count > MAX_FRAME as u64)
            {
                return Err(failure());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = bounded(response.chunk(), &cancel, deadline)
                .await?
                .map_err(|_| failure())?
            {
                if bytes.len().saturating_add(chunk.len()) > MAX_FRAME {
                    return Err(failure());
                }
                bytes.extend_from_slice(&chunk);
            }
            if status == 204 && bytes.is_empty() {
                return Ok((status, Value::Null));
            }
            if response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                != Some("application/json")
            {
                return Err(failure());
            }
            Ok((status, strict_json(&bytes)?))
        })
    }
    pub(crate) fn open_events(&mut self, path: &str) -> Result<(), String> {
        if self.events.is_some()
            || self.socket.is_some()
            || !matches!(self.cli, TitleCli::Kilo | TitleCli::Opencode)
            || path != "/event"
        {
            return Err("Unqualified native event stream.".into());
        }
        self.wait_ready()?;
        let builder = self
            .local
            .as_ref()
            .ok_or_else(failure)?
            .builder(Method::GET, path)?;
        let response = tauri::async_runtime::block_on(bounded(
            builder.send(),
            &self.cancel,
            Instant::now() + OPERATION_TIMEOUT,
        ))?
        .map_err(|_| failure())?;
        if response.status().as_u16() != 200
            || response.headers().contains_key("content-encoding")
            || response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                != Some("text/event-stream")
        {
            return Err(failure());
        }
        self.events = Some(response);
        Ok(())
    }
    pub(crate) fn open_websocket(&mut self, path: &str) -> Result<(), String> {
        if self.cli != TitleCli::Kimi
            || !matches!(path, "/ws" | "/api/v1/ws")
            || self.socket.is_some()
            || self.events.is_some()
        {
            return Err("Unqualified native WebSocket route.".into());
        }
        self.wait_ready()?;
        let local = self.local.as_ref().ok_or_else(failure)?;
        let Authentication::Bearer(token) = &local.auth else {
            return Err(failure());
        };
        let mut url = local.origin.clone();
        url.set_scheme("ws").map_err(|_| failure())?;
        url.set_path("/api/v1/ws");
        let mut request = url.as_str().into_client_request().map_err(|_| failure())?;
        let mut auth = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&format!(
            "Bearer {}",
            token.as_str()
        ))
        .map_err(|_| failure())?;
        auth.set_sensitive(true);
        request.headers_mut().insert("authorization", auth);
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let (socket, response) = tauri::async_runtime::block_on(bounded(
            connect_async_with_config(request, Some(config), true),
            &self.cancel,
            Instant::now() + OPERATION_TIMEOUT,
        ))?
        .map_err(|_| failure())?;
        if response.status().as_u16() != 101 {
            return Err(failure());
        }
        self.socket = Some(socket);
        Ok(())
    }
    pub(crate) fn poll(&mut self) -> Result<Option<Value>, String> {
        let result = self.poll_inner();
        if result.is_err() {
            self.stop()?;
        }
        result
    }
    fn poll_inner(&mut self) -> Result<Option<Value>, String> {
        if cancelled(&self.cancel).is_err() {
            return Err("The native client is stopping.".into());
        }
        if self.transport_failed {
            return Err("Native transport observations are incomplete.".into());
        }
        if let Some(value) = self.queue.pop_front() {
            return Ok(Some(value));
        }
        if self.finished.is_none() {
            self.pump()?;
        } else {
            self.read_pipes()?;
        }
        if let Some(value) = self.queue.pop_front() {
            return Ok(Some(value));
        }
        if let Some(socket) = self.socket.as_mut().filter(|_| !self.ws_eof) {
            let frame = tauri::async_runtime::block_on(async {
                cancelled(&self.cancel)?;
                tokio::select! {
                    frame = socket.next() => Ok::<_, String>(Some(frame)),
                    _ = tokio::time::sleep(Duration::from_millis(50)) => Ok::<_, String>(None),
                }
            })?;
            if let Some(frame) = frame {
                let Some(frame) = frame else {
                    if !self.ws_close_received {
                        return Err(
                            "Native WebSocket ended without a validated close frame.".into()
                        );
                    }
                    self.ws_eof = true;
                    return Ok(None);
                };
                let frame = frame.map_err(|_| failure())?;
                match frame {
                    Message::Text(text) => {
                        self.event_total = self
                            .event_total
                            .checked_add(text.len())
                            .ok_or_else(failure)?;
                        if self.event_total > MAX_TOTAL {
                            return Err(failure());
                        }
                        if self.ws_close_received {
                            return Err(
                                "Native WebSocket emitted data after its close boundary.".into()
                            );
                        }
                        let value = native_object(text.as_bytes())?;
                        if let Some(pong) = kimi_heartbeat(&value)? {
                            if !self.ws_closing {
                                let bytes = serde_json::to_string(&pong).map_err(|_| failure())?;
                                tauri::async_runtime::block_on(bounded(
                                    socket.send(Message::Text(bytes.into())),
                                    &self.cancel,
                                    Instant::now() + OPERATION_TIMEOUT,
                                ))?
                                .map_err(|_| failure())?;
                            }
                            return Ok(None);
                        }
                        return Ok(Some(value));
                    }
                    Message::Ping(_) | Message::Pong(_) => return Ok(None),
                    Message::Close(frame) => {
                        if frame.as_ref().is_some_and(|frame| !matches!(frame.code,
                            tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal |
                            tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Away)) {
                            return Err("Native WebSocket closed with an error status.".into());
                        }
                        if !self.draining && self.finished.is_none() {
                            return Err(
                                "Native WebSocket closed before owned process drain.".into()
                            );
                        }
                        self.ws_close_received = true;
                        return Ok(None);
                    }
                    _ => return Err(failure()),
                }
            }
        } else if let Some(response) = self.events.as_mut().filter(|_| !self.sse_eof) {
            let chunk = tauri::async_runtime::block_on(async {
                cancelled(&self.cancel)?;
                tokio::select! {
                    chunk = response.chunk() => Ok::<_, String>(Some(chunk)),
                    _ = tokio::time::sleep(Duration::from_millis(50)) => Ok::<_, String>(None),
                }
            })?;
            if let Some(chunk) = chunk {
                let Some(chunk) = chunk.map_err(|_| failure())? else {
                    if !self.draining && self.finished.is_none() {
                        return Err("Native event stream ended before owned process drain.".into());
                    }
                    self.sse_records()?;
                    if !self.event_buffer.is_empty() {
                        return Err("Native SSE stream ended with a truncated record.".into());
                    }
                    self.sse_eof = true;
                    return Ok(self.queue.pop_front());
                };
                self.event_total = self
                    .event_total
                    .checked_add(chunk.len())
                    .ok_or_else(failure)?;
                if self.event_total > MAX_TOTAL
                    || self.event_buffer.len().saturating_add(chunk.len()) > MAX_FRAME
                {
                    return Err(failure());
                }
                self.event_buffer.extend_from_slice(&chunk);
                self.sse_records()?;
                return Ok(self.queue.pop_front());
            }
        }
        if self.finished.is_some()
            && self.stdout_eof
            && self.stderr_eof
            && self.queue.is_empty()
            && (self.socket.is_none() || self.ws_eof)
            && (self.events.is_none() || self.sse_eof)
        {
            if !self.stdout_buffer.is_empty() || !self.event_buffer.is_empty() {
                return Err("Native transport ended with a partial frame.".into());
            }
            cancelled(&self.cancel)?;
            self.drained = true;
        }
        Ok(None)
    }
    fn sse_records(&mut self) -> Result<(), String> {
        loop {
            let boundary = [
                self.event_buffer
                    .windows(2)
                    .position(|bytes| bytes == b"\n\n")
                    .map(|index| (index, 2)),
                self.event_buffer
                    .windows(4)
                    .position(|bytes| bytes == b"\r\n\r\n")
                    .map(|index| (index, 4)),
            ]
            .into_iter()
            .flatten()
            .min_by_key(|(index, _)| *index);
            let Some((end, length)) = boundary else {
                return Ok(());
            };
            let record: Vec<_> = self.event_buffer.drain(..end + length).collect();
            let record = std::str::from_utf8(&record).map_err(|_| failure())?;
            let mut data = String::new();
            for line in record.lines() {
                if let Some(value) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value.strip_prefix(' ').unwrap_or(value));
                }
            }
            if data.is_empty() {
                continue;
            }
            if self.queue.len() >= MAX_QUEUE {
                return Err(failure());
            }
            self.queue.push_back(native_object(data.as_bytes())?);
        }
    }
}
impl Local {
    fn builder(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder, String> {
        let mut url = self.origin.clone();
        let (path, query) = path
            .split_once('?')
            .map_or((path, None), |(path, query)| (path, Some(query)));
        url.set_path(path);
        url.set_query(query);
        if matches!(self.auth, Authentication::Basic { .. }) {
            url.query_pairs_mut().append_pair("directory", &self.cwd);
        }
        let mut builder = self.client.request(method, url);
        match &self.auth {
            Authentication::Bearer(token) => builder = builder.bearer_auth(token.as_str()),
            Authentication::Basic { username, password } => {
                let directory_header = if *username == "kilo" {
                    "x-kilo-directory"
                } else {
                    "x-opencode-directory"
                };
                builder = builder.basic_auth(*username, Some(password.as_str()));
                if self.cwd.is_ascii() {
                    builder = builder.header(directory_header, &self.cwd);
                }
            }
        }
        Ok(builder.header("cache-control", "no-store"))
    }
}

pub(crate) struct Process {
    child: runtime::OwnedChild,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: ChildStderr,
    stdout_buffer: Vec<u8>,
    stderr_buffer: Vec<u8>,
    stdout_total: usize,
    stderr_total: usize,
    stdout_eof: bool,
    stderr_eof: bool,
    queue: VecDeque<Value>,
    cli: TitleCli,
    cancel: Arc<AtomicBool>,
    finished: Option<bool>,
    transport_failed: bool,
    drained: bool,
    draining: bool,
    sse_eof: bool,
    ws_eof: bool,
    ws_closing: bool,
    ws_close_received: bool,
    local: Option<Local>,
    events: Option<Response>,
    event_buffer: Vec<u8>,
    event_total: usize,
    socket: Option<Socket>,
    server_auth: Option<Authentication>,
    cwd: String,
}

type ExecutableIdentity = (PathBuf, u64, u64, u64, i64, i64);
pub(crate) struct Prepared {
    identity: ExecutableIdentity,
    grok_identity: Option<super::grok_artifact::FileIdentity>,
    launch: super::gateway_profiles::Launch,
    cli: TitleCli,
    kind: NativeKind,
    version: String,
    cwd: String,
    cancel: Arc<AtomicBool>,
}
impl Prepared {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        app: &AppHandle,
        shells: &Shells,
        shell_profile_id: &str,
        cwd: &str,
        cli: TitleCli,
        profile_directory: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        cancelled(&cancel)?;
        if cwd.len() > 16 * 1024
            || cwd.chars().any(char::is_control)
            || !Path::new(cwd).is_absolute()
            || !Path::new(cwd).is_dir()
        {
            return Err(
                "The native process requires an admitted absolute workspace directory.".into(),
            );
        }
        let shell = runtime::shell_profile(shells, shell_profile_id)?;
        let program = if cli == TitleCli::Grok {
            super::grok_artifact::resolve(&app.path().resource_dir().map_err(|_| failure())?)?
        } else {
            runtime::resolve(shells, shell, cwd, cli)?
        };
        let identity = gateway_runtime::executable(&program)?;
        native_accounts::admit_executable(cli, &identity.0)?;
        let grok_identity = if cli == TitleCli::Grok {
            Some(super::grok_artifact::admit(&identity.0)?)
        } else {
            None
        };
        // Account preparation validates its parent before the private probe is made.
        let launch = native_accounts::prepare(profile_directory, cli)?;
        let version = version(
            &identity,
            grok_identity.as_ref(),
            &launch.private_home,
            cli,
            &cancel,
        )?;
        let kind = NativeKind::from_cli(cli)
            .filter(|kind| kind.versions().contains(&version.as_str()))
            .ok_or("This native client family/version has no reviewed control-plane contract.")?;
        let result = Self {
            identity,
            grok_identity,
            launch,
            cli,
            kind,
            version,
            cwd: cwd.into(),
            cancel,
        };
        result.admit()?;
        Ok(result)
    }
    pub(crate) fn version(&self) -> &str {
        &self.version
    }
    pub(crate) fn kind(&self) -> NativeKind {
        self.kind
    }
    fn admit(&self) -> Result<(), String> {
        cancelled(&self.cancel)?;
        if gateway_runtime::executable(&self.identity.0)? != self.identity {
            return Err("The reviewed native executable changed before launch.".into());
        }
        native_accounts::admit_executable(self.cli, &self.identity.0)?;
        if let Some(expected) = &self.grok_identity {
            if &super::grok_artifact::admit(&self.identity.0)? != expected {
                return Err("The owned Grok executable changed before launch.".into());
            }
        }
        Ok(())
    }
    /// Preparation can run outside the store lock; this fence runs immediately
    /// before final executable admission and spawn under the caller's ownership.
    pub(crate) fn spawn(
        self,
        launch_args: &[String],
        fence: impl FnOnce() -> Result<(), String>,
    ) -> Result<Process, String> {
        if launch_args
            .iter()
            .any(|arg| arg.len() > MAX_FRAME || arg.contains('\0'))
            || launch_args.len() > 64
        {
            return Err(failure());
        }
        let descriptor = self.kind.launch();
        if descriptor.approvals == super::native_wire::ApprovalSurface::NativePtyOnly {
            return Err("Agy coding requires its native PTY approval surface.".into());
        }
        let expected_transport = match self.kind {
            NativeKind::Kimi => super::native_wire::NativeTransport::HttpWebsocket,
            NativeKind::Kilo | NativeKind::OpenCode => super::native_wire::NativeTransport::HttpSse,
            _ => super::native_wire::NativeTransport::Stdio,
        };
        if descriptor.transport != expected_transport {
            return Err(
                "Native launch transport does not match its reviewed process owner.".into(),
            );
        }
        // The native wire owns model/resume additions; require its reviewed
        // base control-plane arguments instead of accepting another CLI mode.
        if !launch_args.starts_with(&descriptor.arguments) {
            return Err(
                "Native launch arguments do not preserve the reviewed transport mode.".into(),
            );
        }
        let mut cmd = command(&self.identity.0, Path::new(&self.cwd), &self.launch)?;
        cmd.args(&self.launch.arguments).args(launch_args);
        let mut server_auth = None;
        if let Some(variable) = descriptor.local_password_variable {
            let password = Zeroizing::new(format!("{}{}", super::new_id()?, super::new_id()?));
            cmd.env(variable, password.as_str());
            server_auth = Some(Authentication::Basic {
                username: descriptor.local_username.ok_or_else(failure)?,
                password,
            });
        }
        fence()?;
        self.admit()?;
        if descriptor.native_server_token {
            let token = prepare_token(&self.launch.private_home)?;
            server_auth = Some(Authentication::Bearer(token));
        }
        let mut child = runtime::OwnedChild::new(cmd.spawn().map_err(|_| failure())?);
        let stdin = child.stdin.take().ok_or_else(failure)?;
        let stdout = child.stdout.take().ok_or_else(failure)?;
        let stderr = child.stderr.take().ok_or_else(failure)?;
        nonblocking(&stdin)?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        Ok(Process {
            child,
            stdin: Some(stdin),
            stdout,
            stderr,
            stdout_buffer: Vec::new(),
            stderr_buffer: Vec::new(),
            stdout_total: 0,
            stderr_total: 0,
            stdout_eof: false,
            stderr_eof: false,
            queue: VecDeque::new(),
            cli: self.cli,
            cancel: self.cancel,
            finished: None,
            transport_failed: false,
            drained: false,
            draining: false,
            sse_eof: false,
            ws_eof: false,
            ws_closing: false,
            ws_close_received: false,
            local: None,
            events: None,
            event_buffer: Vec::new(),
            event_total: 0,
            socket: None,
            server_auth,
            cwd: self.cwd,
        })
    }
    /// Source-qualified read-only collectors return their actual bytes. They
    /// never manufacture a JSON wrapper around textual native output.
    pub(crate) fn collect_command(
        self,
        args: &[String],
        fence: impl FnOnce() -> Result<(), String>,
    ) -> Result<Vec<u8>, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let approved = match self.kind {
            NativeKind::Claude => words == ["auth", "status"],
            NativeKind::Codex => words == ["login", "status"],
            NativeKind::Agy => words == ["-p", "/usage", "--output-format", "json"],
            _ => false,
        };
        if !approved {
            return Err("This native family has no qualified read-only collector command.".into());
        }
        let mut cmd = command(&self.identity.0, Path::new(&self.cwd), &self.launch)?;
        cmd.args(&self.launch.arguments)
            .args(args)
            .stdin(Stdio::null());
        fence()?;
        self.admit()?;
        let mut child = runtime::OwnedChild::new(cmd.spawn().map_err(|_| failure())?);
        let mut stdout = child.stdout.take().ok_or_else(failure)?;
        let mut stderr = child.stderr.take().ok_or_else(failure)?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let (mut output, mut diagnostics) = (Vec::new(), Vec::new());
        let (mut out_count, mut err_count) = (0, 0);
        let (mut out_eof, mut err_eof) = (false, false);
        loop {
            cancelled(&self.cancel)?;
            if Instant::now() >= deadline {
                return Err(failure());
            }
            if !out_eof {
                out_eof = drain(&mut stdout, &mut output, &mut out_count, MAX_FRAME)?;
            }
            if !err_eof {
                err_eof = drain(&mut stderr, &mut diagnostics, &mut err_count, MAX_FRAME)?;
            }
            if runtime::exit_pending(&child)? {
                let status = child.stop_and_wait()?;
                if !out_eof {
                    out_eof = drain(&mut stdout, &mut output, &mut out_count, MAX_FRAME)?;
                }
                if !err_eof {
                    err_eof = drain(&mut stderr, &mut diagnostics, &mut err_count, MAX_FRAME)?;
                }
                if !status.success() || !out_eof || !err_eof {
                    return Err(failure());
                }
                return Ok(output);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn prepare_token(home: &Path) -> Result<Zeroizing<String>, String> {
    // npm Kimi Code 2.1.1 names this file server.token (not Python/local-server
    // family token filenames). It is local control-plane auth only.
    let path = home.join("server.token");
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o777 == 0o600
                && metadata.nlink() == 1
                && metadata.len() <= 256 => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err("The native local server token is not a private owned file.".into()),
    }
    let token = Zeroizing::new(format!("{}{}", super::new_id()?, super::new_id()?));
    crate::chat::storage::atomic(&path, token.as_bytes())?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| failure())?;
    if metadata.mode() & 0o777 != 0o600
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(failure());
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_default_mode_file_creation_is_private_without_chmod() {
        use std::os::unix::fs::MetadataExt;
        let directory = tempfile::tempdir().unwrap();
        let launch = super::super::gateway_profiles::Launch {
            arguments: vec![],
            environment: vec![],
            private_home: directory.path().to_owned(),
        };
        // Shell redirection, like Pi's openSync("wx"), supplies a default 0666
        // creation mode. Exercise the actual production command constructor.
        let status = command(Path::new("/bin/sh"), directory.path(), &launch)
            .unwrap()
            .args(["-c", ": > native-session.jsonl"])
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::metadata(directory.path().join("native-session.jsonl"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn readiness_binds_actual_loopback_port_and_owned_local_token() {
        let auth = Authentication::Bearer(Zeroizing::new("owned-control-token".into()));
        assert_eq!(
            ready_origin(
                TitleCli::Kimi,
                b"Kimi server: http://127.0.0.1:4096/#token=owned-control-token\n",
                &auth
            )
            .unwrap()
            .unwrap()
            .port(),
            Some(4096)
        );
        for line in [
            "Kimi server: http://127.0.0.1:0/#token=owned-control-token\n",
            "Kimi server: http://127.0.0.1:4096/#token=another-token\n",
            "Kimi server: http://example.com:4096/#token=owned-control-token\n",
            "Kimi server: http://127.0.0.1:4096/private/#token=owned-control-token\n",
            "Kimi server: http://user@127.0.0.1:4096/#token=owned-control-token\n",
        ] {
            assert!(ready_origin(TitleCli::Kimi, line.as_bytes(), &auth).is_err());
        }
        let auth = Authentication::Basic {
            username: "kilo",
            password: Zeroizing::new("private".into()),
        };
        assert_eq!(
            ready_origin(
                TitleCli::Kilo,
                b"kilo server listening on http://127.0.0.1:43210\n",
                &auth
            )
            .unwrap()
            .unwrap()
            .port(),
            Some(43210)
        );
        assert!(ready_origin(
            TitleCli::Kilo,
            b"kilo server listening on http://0.0.0.0:4096\n",
            &auth
        )
        .is_err());
        assert!(ready_origin(
            TitleCli::Kilo,
            b"opencode server listening on http://127.0.0.1:4096\n",
            &auth
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn control_routes_exclude_auth_mutations_and_caller_supplied_destinations() {
        assert!(allowed_path(TitleCli::Kimi, "GET", "/oauth/userinfo"));
        assert!(allowed_path(TitleCli::Kimi, "GET", "/oauth/usage"));
        assert!(allowed_path(
            TitleCli::Kimi,
            "GET",
            "/sessions/owned/approvals?status=pending"
        ));
        assert!(!allowed_path(
            TitleCli::Kimi,
            "GET",
            "/sessions/owned/approvals?status=pending&directory=/other"
        ));
        assert!(!allowed_path(
            TitleCli::Opencode,
            "GET",
            "/sessions/owned/approvals?status=pending"
        ));
        assert!(allowed_path(
            TitleCli::Kimi,
            "POST",
            "/sessions/owned/prompts/prompt:abort"
        ));
        assert!(allowed_path(TitleCli::Kilo, "GET", "/kilo/profile"));
        assert!(!allowed_path(TitleCli::Opencode, "GET", "/kilo/profile"));
        for path in [
            "https://example.com/session",
            "//example.com/session",
            "/session/../prompt_async",
            "/session/%2e%2e/prompt_async",
            "/session/owned/prompt_async?directory=/other",
            "/oauth/logout",
            "/auth/kilo",
            "/sessions/owned/prompts/prompt::abort",
        ] {
            assert!(!allowed_path(TitleCli::Kimi, "POST", path));
            assert!(!allowed_path(TitleCli::Kilo, "POST", path));
        }
    }
}
