//! Owned loopback authority for Qwen Code 0.24.7's HTTP managed session store.
//! Contract: b12edec1401a28fc53cd9e714d5928b285071fc8, core managed runtime
//! http-managed-session-store.ts and production Java ManagedSessionStore.
//! Each attempt owns a fresh database. A commit is acknowledged only after FULL
//! SQLite commit; no provider credentials or external tool results are admitted.
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const MAX_BODY: usize = 24 * 1024 * 1024;
const MAX_RECORDS: usize = 8 * 1024 * 1024;
const MAX_DURABLE: i64 = 128 * 1024 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_COUNTER: u64 = 9_007_199_254_740_990;
type Result<T> = std::result::Result<T, Failure>;
#[derive(Clone, Copy, Debug)]
struct Failure {
    status: u16,
    code: &'static str,
}
fn bad() -> Failure {
    Failure {
        status: 400,
        code: "invalid_managed_session_store_request",
    }
}
fn conflict() -> Failure {
    Failure {
        status: 409,
        code: "managed_session_writer_conflict",
    }
}
fn corrupt() -> Failure {
    Failure {
        status: 409,
        code: "managed_session_journal_corrupt",
    }
}
fn effect() -> Failure {
    Failure {
        status: 409,
        code: "managed_session_effect_denied",
    }
}
fn database(_: rusqlite::Error) -> Failure {
    Failure {
        status: 500,
        code: "managed_session_store_unavailable",
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(MAX_COUNTER as u128) as u64
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
        .ok_or_else(bad)
}
fn count(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .filter(|n| *n <= MAX_COUNTER)
        .ok_or_else(bad)
}
fn stable(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn scope_id(value: &str) -> bool {
    stable(value)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn hash_field(value: &Value, key: &str, nullable: bool) -> Result<()> {
    if nullable && value[key].is_null() {
        return Ok(());
    }
    let s = text(value, key)?;
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(bad());
    }
    Ok(())
}
fn decode(value: &Value, key: &str, limit: usize) -> Result<Vec<u8>> {
    let s = value[key].as_str().ok_or_else(bad)?;
    if s.len() > limit.div_ceil(3) * 4 {
        return Err(bad());
    }
    let bytes = STANDARD.decode(s).map_err(|_| bad())?;
    if bytes.len() > limit || STANDARD.encode(&bytes) != s {
        return Err(bad());
    }
    Ok(bytes)
}
/// ECMAScript Object.keys().sort(): compare UTF-16 code units, not UTF-8.
fn canonical(value: &Value) -> Result<String> {
    fn write(v: &Value, depth: usize, out: &mut String) -> Result<()> {
        if depth > 64 {
            return Err(bad());
        }
        match v {
            Value::Array(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(v, depth + 1, out)?;
                }
                out.push(']');
            }
            Value::Object(m) => {
                out.push('{');
                let mut keys = m.keys().collect::<Vec<_>>();
                keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                for (i, key) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).map_err(|_| bad())?);
                    out.push(':');
                    write(&m[*key], depth + 1, out)?;
                }
                out.push('}');
            }
            Value::Number(n) => {
                if let Some(n) = n.as_u64() {
                    if n > MAX_COUNTER {
                        return Err(bad());
                    }
                    out.push_str(&n.to_string());
                } else if let Some(n) = n.as_i64() {
                    if n.unsigned_abs() > MAX_COUNTER {
                        return Err(bad());
                    }
                    out.push_str(&n.to_string());
                } else {
                    return Err(bad());
                }
            }
            _ => out.push_str(&v.to_string()),
        }
        if out.len() > MAX_RECORDS {
            return Err(bad());
        }
        Ok(())
    }
    let mut out = String::new();
    write(value, 0, &mut out)?;
    Ok(out)
}
#[derive(Clone)]
struct Head {
    state: String,
    generation: u64,
    lease: u64,
    token_hash: String,
    revision: u64,
    sequence: u64,
    commit: Value,
    epoch: u64,
    checkpoint: Value,
    recovery: String,
    detail: Value,
}
impl Head {
    fn value(&self) -> Value {
        json!({"state":self.state,"writerGeneration":self.generation,"leaseUntil":self.lease,"tokenHash":self.token_hash,"journalRevision":self.revision,"committedSequence":self.sequence,"lastCommitDigest":self.commit,"activationEpoch":self.epoch,"latestCheckpointResourceId":self.checkpoint,"recoveryStatus":self.recovery,"recoveryDetailCode":self.detail})
    }
    fn grant(&self, replayed: bool) -> Value {
        json!({"writerGeneration":self.generation,"leaseUntil":self.lease,"journalRevision":self.revision,"committedSequence":self.sequence,"lastCommitDigest":self.commit,"activationEpoch":self.epoch,"replayed":replayed})
    }
    fn restore(&self) -> Value {
        let mut v = self.grant(false);
        let m = v.as_object_mut().unwrap();
        m.remove("leaseUntil");
        m.remove("replayed");
        m.insert("state".into(), json!(self.state));
        m.insert("storageVersion".into(), json!(1));
        m.insert("latestCheckpointResourceId".into(), self.checkpoint.clone());
        m.insert("compactedThroughRevision".into(), json!(0));
        m.insert("recoveryStatus".into(), json!(self.recovery));
        m.insert("recoveryDetailCode".into(), self.detail.clone());
        v
    }
}
struct State {
    db: Connection,
    session: String,
    workspace: String,
    tenant: String,
    boot: Option<String>,
    head: Option<Head>,
    failed: bool,
    effect: bool,
}
struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Value,
}
#[derive(Debug)]
struct Response {
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}
impl Response {
    fn json(value: Value) -> Result<Self> {
        let body = serde_json::to_vec(&value).map_err(|_| bad())?;
        if body.len() > MAX_BODY {
            return Err(bad());
        }
        Ok(Self {
            body,
            headers: vec![("Content-Type".into(), "application/json".into())],
        })
    }
}
pub(crate) struct OwnedStore {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    address: std::net::SocketAddr,
}
impl OwnedStore {
    pub(crate) fn start(
        root: &Path,
        session_id: &str,
        workspace_id: &str,
        tenant_id: &str,
    ) -> std::result::Result<Self, String> {
        if ![session_id, workspace_id, tenant_id]
            .iter()
            .all(|id| scope_id(id))
        {
            return Err("Invalid private Qwen store identity.".into());
        }
        crate::chat::storage::reject_link(root)?;
        crate::chat::storage::private(root, true)?;
        let file = root.join("qwen-managed.sqlite");
        crate::chat::storage::reject_link(&file)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&file)
            .map_err(|_| "Cannot create private Qwen journal.")?;
        let db = Connection::open_with_flags(
            &file,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| "Cannot open private Qwen journal.")?;
        db.busy_timeout(Duration::from_secs(2))
            .map_err(|_| "Cannot bound Qwen journal ownership.")?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; CREATE TABLE head(id INTEGER PRIMARY KEY CHECK(id=1),body TEXT NOT NULL); CREATE TABLE metadata(id TEXT PRIMARY KEY,body TEXT NOT NULL); CREATE TABLE transactions(revision INTEGER PRIMARY KEY,transaction_id TEXT NOT NULL UNIQUE,operation TEXT NOT NULL,command_id TEXT NOT NULL,request_digest TEXT NOT NULL,body TEXT NOT NULL,UNIQUE(operation,command_id)); CREATE TABLE resources(id TEXT PRIMARY KEY,metadata TEXT NOT NULL,bytes BLOB NOT NULL); CREATE TABLE events(sequence INTEGER PRIMARY KEY,event_id TEXT NOT NULL UNIQUE,body TEXT NOT NULL);").map_err(|_|"Cannot initialize private Qwen journal.")?;
        db.execute(
            "INSERT INTO metadata VALUES('scope',?1)",
            [
                json!({"sessionId":session_id,"workspaceId":workspace_id,"tenantId":tenant_id})
                    .to_string(),
            ],
        )
        .map_err(|_| "Cannot commit Qwen journal identity.")?;
        crate::chat::storage::private(&file, false)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Cannot bind private Qwen journal.")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Cannot bound private Qwen listener.")?;
        let address = listener
            .local_addr()
            .map_err(|_| "Cannot resolve private Qwen listener.")?;
        let state = Arc::new(Mutex::new(State {
            db,
            session: session_id.into(),
            workspace: workspace_id.into(),
            tenant: tenant_id.into(),
            boot: None,
            head: None,
            failed: false,
            effect: false,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_state = state.clone();
        let thread_stop = stop.clone();
        let join = thread::Builder::new()
            .name("qwen-managed-store".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, peer)) => {
                            if !peer.ip().is_loopback() {
                                continue;
                            }
                            let result = read_request(&mut stream, address).and_then(|request| {
                                let mut state = thread_state
                                    .lock()
                                    .map_err(|_| database(rusqlite::Error::InvalidQuery))?;
                                if thread_stop.load(Ordering::SeqCst) {
                                    Err(conflict())
                                } else {
                                    state.dispatch(&request)
                                }
                            });
                            let _ = write_response(&mut stream, result);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => {
                            if let Ok(mut state) = thread_state.lock() {
                                state.failed = true;
                            }
                            break;
                        }
                    }
                }
            })
            .map_err(|_| "Cannot own private Qwen listener thread.")?;
        Ok(Self {
            state,
            stop,
            join: Some(join),
            address,
        })
    }
    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
    pub(crate) fn bind_boot(&self, boot: &str) -> std::result::Result<(), String> {
        if !scope_id(boot) {
            return Err("Invalid Qwen process boot identity.".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Qwen store ownership unavailable.")?;
        if state.boot.is_some() || state.head.is_some() {
            return Err("Qwen process identity is already fixed.".into());
        }
        state
            .db
            .execute("INSERT INTO metadata VALUES('boot',?1)", [boot])
            .map_err(|_| "Cannot commit Qwen process identity.")?;
        state.boot = Some(boot.into());
        Ok(())
    }
    pub(crate) fn healthy(&self) -> bool {
        self.state.lock().is_ok_and(|state| !state.failed)
    }
    pub(crate) fn effect_denied(&self) -> bool {
        self.state.lock().map_or(true, |state| state.effect)
    }
    pub(crate) fn sealed(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            state
                .head
                .as_ref()
                .is_some_and(|head| head.state == "SEALED")
        })
    }
    pub(crate) fn final_assistant(
        &self,
        prompt_id: &str,
        model: &str,
    ) -> std::result::Result<String, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "Qwen durable authority unavailable.")?;
        state.final_assistant(prompt_id, model).map_err(|_| {
            "Qwen durable assistant records do not confirm the requested completed text turn."
                .into()
        })
    }
    pub(crate) fn stop_and_join(&mut self) -> std::result::Result<(), String> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| "Private Qwen journal did not confirm shutdown.")?;
        }
        Ok(())
    }
}
impl Drop for OwnedStore {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}
fn read_request(stream: &mut TcpStream, address: std::net::SocketAddr) -> Result<Request> {
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| bad())?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| bad())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 16384 || Instant::now() >= deadline {
            return Err(bad());
        }
        stream.read_exact(&mut byte).map_err(|_| bad())?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header).map_err(|_| bad())?;
    let mut lines = header.split("\r\n");
    let first = lines.next().ok_or_else(bad)?.split(' ').collect::<Vec<_>>();
    if first.len() != 3 || first[2] != "HTTP/1.1" || !matches!(first[0], "GET" | "POST") {
        return Err(bad());
    }
    let mut headers = HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (key, value) = line.split_once(':').ok_or_else(bad)?;
        if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || headers
                .insert(key.to_ascii_lowercase(), value.trim().to_owned())
                .is_some()
        {
            return Err(bad());
        }
    }
    if headers.contains_key("origin")
        || headers.contains_key("transfer-encoding")
        || headers.contains_key("expect")
        || headers.get("host") != Some(&address.to_string())
    {
        return Err(bad());
    }
    let length = match headers.get("content-length") {
        Some(value) if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
            value.parse::<usize>().map_err(|_| bad())?
        }
        None if first[0] == "GET" => 0,
        _ => return Err(bad()),
    };
    if length > MAX_BODY || (first[0] == "GET" && length != 0) {
        return Err(bad());
    }
    if first[0] == "POST"
        && headers.get("content-type").map(String::as_str) != Some("application/json")
    {
        return Err(bad());
    }
    let mut bytes = vec![0; length];
    let mut done = 0;
    while done < length {
        if Instant::now() >= deadline {
            return Err(bad());
        }
        let end = (done + 8192).min(length);
        let n = stream.read(&mut bytes[done..end]).map_err(|_| bad())?;
        if n == 0 {
            return Err(bad());
        }
        done += n;
    }
    let (path, query) = first[1].split_once('?').unwrap_or((first[1], ""));
    if !path.starts_with('/') || path.contains('%') || path.contains('#') || path.contains("//") {
        return Err(bad());
    }
    let mut parsed = HashMap::new();
    if !query.is_empty() {
        for part in query.split('&') {
            let (k, v) = part.split_once('=').ok_or_else(bad)?;
            if !scope_id(k)
                || v.is_empty()
                || v.contains('%')
                || parsed.insert(k.into(), v.into()).is_some()
            {
                return Err(bad());
            }
        }
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|_| bad())?
    };
    Ok(Request {
        method: first[0].into(),
        path: path.into(),
        query: parsed,
        headers,
        body,
    })
}
fn write_response(stream: &mut TcpStream, result: Result<Response>) -> std::io::Result<()> {
    let(status,response)=match result{Ok(response)=>(200,response),Err(error)=>(error.status,Response{body:json!({"error":{"code":error.code,"message":"Private managed store rejected this request."}}).to_string().into_bytes(),headers:vec![("Content-Type".into(),"application/json".into())]})};
    write!(stream,"HTTP/1.1 {status} {}\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n",if status==200{"OK"}else{"Rejected"},response.body.len())?;
    for (key, value) in response.headers {
        write!(stream, "{key}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&response.body)?;
    stream.flush()
}
impl State {
    fn session_key(&self) -> Value {
        json!({"sessionId":self.session,"workspaceId":self.workspace,"tenantId":self.tenant})
    }
    fn token(&self, request: &Request) -> Result<String> {
        if request.headers.get("x-qwen-tenant-id") != Some(&self.tenant) {
            return Err(conflict());
        }
        let token = request
            .headers
            .get("x-qwen-managed-writer-token")
            .ok_or_else(conflict)?;
        if !(32..=512).contains(&token.len())
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(conflict());
        }
        let hash = digest(token.as_bytes());
        if self
            .head
            .as_ref()
            .is_some_and(|head| head.token_hash != hash)
        {
            return Err(conflict());
        }
        Ok(hash)
    }
    fn writer(&self, body: &Value, hash: &str, unexpired: bool) -> Result<Head> {
        let head = self.head.as_ref().ok_or_else(conflict)?;
        if head.state != "ACTIVE"
            || head.token_hash != hash
            || count(body, "writerGeneration")? != head.generation
            || body["writerId"].as_str() != self.boot.as_deref()
            || (unexpired && head.lease <= now())
        {
            return Err(conflict());
        }
        Ok(head.clone())
    }
    fn read_grant(&self, hash: &str) -> Result<Head> {
        let head = self.head.as_ref().ok_or_else(conflict)?;
        if head.state != "ACTIVE" || head.token_hash != hash || head.lease <= now() {
            return Err(conflict());
        }
        Ok(head.clone())
    }
    fn save_head(&mut self, head: Head) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        tx.execute(
            "INSERT INTO head VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",
            [head.value().to_string()],
        )
        .map_err(database)?;
        tx.commit().map_err(database)?;
        self.head = Some(head);
        Ok(())
    }
    // Only requests admitted to this writer's known scope can affect its health.
    // In particular, arbitrary loopback traffic must not fence a legitimate child.
    fn admit(&self, request: &Request) -> Result<()> {
        let prefix = format!(
            "/internal/managed-session-store/v1/sessions/{}/",
            self.session
        );
        let route = request.path.strip_prefix(&prefix).ok_or_else(bad)?;
        let known = match request.method.as_str() {
            "GET" => {
                matches!(route, "restore" | "transactions")
                    || route.strip_prefix("resources/").is_some_and(scope_id)
            }
            "POST" => matches!(
                route,
                "writers:acquire"
                    | "writers:renew"
                    | "writers:seal"
                    | "transactions:commit"
                    | "recovery:block"
                    | "tool-results:publish"
            ),
            _ => false,
        };
        if !known {
            return Err(bad());
        }
        self.token(request)?;
        if self.boot.is_none() {
            return Err(conflict());
        }
        if request.method == "GET" {
            if request.query.get("workspaceId") != Some(&self.workspace) {
                return Err(conflict());
            }
        } else if request.body["workspaceId"] != self.workspace
            || request.body["writerId"].as_str() != self.boot.as_deref()
        {
            return Err(conflict());
        }
        // Before initial acquire there is no authenticated writer token yet.
        if self.head.is_none() && route != "writers:acquire" {
            return Err(conflict());
        }
        Ok(())
    }
    fn dispatch(&mut self, request: &Request) -> Result<Response> {
        self.admit(request)?;
        let authenticated = self.head.is_some();
        let result = self.handle(request);
        if let Err(error) = result {
            // Initial acquire has not established a token yet. Invalid attempts
            // cannot poison it; an actual database failure still fences the store.
            if authenticated || error.status == 500 {
                if error.code == effect().code {
                    self.effect = true;
                }
                self.failed = true;
            }
        }
        result
    }
    fn handle(&mut self, request: &Request) -> Result<Response> {
        if self.failed {
            return Err(corrupt());
        }
        let prefix = format!(
            "/internal/managed-session-store/v1/sessions/{}/",
            self.session
        );
        let route = request.path.strip_prefix(&prefix).ok_or_else(bad)?;
        let hash = self.token(request)?;
        if self.boot.is_none() {
            return Err(conflict());
        }
        if request.method == "GET" {
            if request.query.get("workspaceId") != Some(&self.workspace) {
                return Err(conflict());
            }
            let head = self.read_grant(&hash)?;
            return match route {
                "restore" if request.query.len() == 1 => Response::json(head.restore()),
                "transactions" => {
                    if request.query.keys().any(|key| {
                        !["workspaceId", "afterRevision", "limit"].contains(&key.as_str())
                    }) {
                        return Err(bad());
                    }
                    let after = request
                        .query
                        .get("afterRevision")
                        .map(|v| v.parse::<u64>())
                        .transpose()
                        .map_err(|_| bad())?
                        .unwrap_or(0);
                    let limit = request
                        .query
                        .get("limit")
                        .map(|v| v.parse::<u64>())
                        .transpose()
                        .map_err(|_| bad())?
                        .unwrap_or(100);
                    if after > head.revision || !(1..=100).contains(&limit) {
                        return Err(bad());
                    }
                    let mut stmt=self.db.prepare("SELECT body FROM transactions WHERE revision>?1 ORDER BY revision LIMIT ?2").map_err(database)?;
                    let rows = stmt
                        .query_map(
                            params![
                                i64::try_from(after).map_err(|_| bad())?,
                                i64::try_from(limit).map_err(|_| bad())?
                            ],
                            |row| row.get::<_, String>(0),
                        )
                        .map_err(database)?;
                    let mut page = Vec::new();
                    let mut page_bytes = 0usize;
                    let mut next = after;
                    for row in rows {
                        let row = row.map_err(database)?;
                        if !page.is_empty() && page_bytes + row.len() > MAX_BODY - 65536 {
                            break;
                        }
                        page_bytes += row.len();
                        let value: Value = serde_json::from_str(&row).map_err(|_| corrupt())?;
                        if count(&value, "journalRevision")? != next + 1 {
                            return Err(corrupt());
                        }
                        let bytes = decode(&value, "recordBytesBase64", MAX_RECORDS)?;
                        if value["recordDigest"] != digest(&bytes)
                            || count(&value, "byteLength")? != bytes.len() as u64
                        {
                            return Err(corrupt());
                        }
                        next += 1;
                        page.push(value);
                    }
                    Response::json(
                        json!({"transactions":page,"nextRevision":next,"hasMore":next<head.revision}),
                    )
                }
                path if path.starts_with("resources/") && request.query.len() == 1 => {
                    let id = path.strip_prefix("resources/").unwrap();
                    if !scope_id(id) {
                        return Err(bad());
                    }
                    let (metadata, bytes) = self.resource(id)?;
                    Ok(Response {
                        body: bytes,
                        headers: vec![
                            ("Content-Type".into(), "application/octet-stream".into()),
                            (
                                "X-Qwen-Resource-Kind".into(),
                                text(&metadata, "kind")?.into(),
                            ),
                            (
                                "X-Qwen-Resource-Schema-Version".into(),
                                count(&metadata, "schemaVersion")?.to_string(),
                            ),
                            (
                                "X-Qwen-Resource-Digest".into(),
                                text(&metadata, "digest")?.into(),
                            ),
                        ],
                    })
                }
                _ => Err(bad()),
            };
        }
        if !request.query.is_empty()
            || request.body["workspaceId"] != self.workspace
            || request.body["writerId"].as_str() != self.boot.as_deref()
        {
            return Err(conflict());
        }
        match route {
            "writers:acquire" => {
                allowed_keys(&request.body, &["workspaceId", "writerId", "leaseMillis"])?;
                let duration = lease(&request.body)?;
                let replayed = self
                    .head
                    .as_ref()
                    .is_some_and(|head| head.state == "ACTIVE" && head.lease > now());
                let head = if let Some(head) = &self.head {
                    if !["ACTIVE", "SEALED"].contains(&head.state.as_str()) {
                        return Err(conflict());
                    }
                    let mut next = head.clone();
                    if !replayed {
                        next.generation = next
                            .generation
                            .checked_add(1)
                            .filter(|n| *n <= MAX_COUNTER)
                            .ok_or_else(conflict)?;
                    }
                    next.state = "ACTIVE".into();
                    next.lease = next.lease.max(now() + duration);
                    next
                } else {
                    Head {
                        state: "ACTIVE".into(),
                        generation: 1,
                        lease: now() + duration,
                        token_hash: hash,
                        revision: 0,
                        sequence: 0,
                        commit: Value::Null,
                        epoch: 0,
                        checkpoint: Value::Null,
                        recovery: "READY".into(),
                        detail: Value::Null,
                    }
                };
                let response = head.grant(replayed);
                self.save_head(head)?;
                Response::json(response)
            }
            "writers:renew" => {
                allowed_keys(
                    &request.body,
                    &["workspaceId", "writerId", "writerGeneration", "leaseMillis"],
                )?;
                let mut head = self.writer(&request.body, &hash, true)?;
                head.lease = head.lease.max(now() + lease(&request.body)?);
                let response = head.grant(false);
                self.save_head(head)?;
                Response::json(response)
            }
            "writers:seal" => {
                allowed_keys(
                    &request.body,
                    &["workspaceId", "writerId", "writerGeneration"],
                )?;
                let current = self.head.as_ref().ok_or_else(conflict)?;
                let replayed = current.state == "SEALED"
                    && current.generation == count(&request.body, "writerGeneration")?;
                let mut head = if replayed {
                    current.clone()
                } else {
                    self.writer(&request.body, &hash, false)?
                };
                head.state = "SEALED".into();
                let response = json!({"writerGeneration":head.generation,"state":"SEALED","replayed":replayed});
                self.save_head(head)?;
                Response::json(response)
            }
            "recovery:block" => {
                allowed_keys(
                    &request.body,
                    &[
                        "workspaceId",
                        "writerId",
                        "writerGeneration",
                        "recoveryStatus",
                        "recoveryDetailCode",
                    ],
                )?;
                let mut head = self.writer(&request.body, &hash, true)?;
                let status = text(&request.body, "recoveryStatus")?;
                if !["BLOCKED_RESOURCE", "BLOCKED_WORKSPACE", "BLOCKED_EXECUTION"].contains(&status)
                {
                    return Err(bad());
                }
                head.recovery = status.into();
                head.detail = json!(text(&request.body, "recoveryDetailCode")?);
                let response = json!({"writerGeneration":head.generation,"recoveryStatus":head.recovery,"recoveryDetailCode":head.detail,"replayed":false});
                self.save_head(head)?;
                Response::json(response)
            }
            "transactions:commit" => self.commit(&request.body, &hash),
            "tool-results:publish" => Err(effect()),
            _ => Err(bad()),
        }
    }
    fn resource(&self, id: &str) -> Result<(Value, Vec<u8>)> {
        let (metadata, bytes): (String, Vec<u8>) = self
            .db
            .query_row(
                "SELECT metadata,bytes FROM resources WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(database)?
            .ok_or(Failure {
                status: 404,
                code: "managed_session_resource_not_found",
            })?;
        let metadata: Value = serde_json::from_str(&metadata).map_err(|_| corrupt())?;
        if metadata["resourceId"] != id
            || metadata["digest"] != digest(&bytes)
            || count(&metadata, "byteLength")? != bytes.len() as u64
        {
            return Err(corrupt());
        }
        Ok((metadata, bytes))
    }
    fn commit(&mut self, body: &Value, hash: &str) -> Result<Response> {
        allowed_keys(
            body,
            &[
                "workspaceId",
                "writerId",
                "writerGeneration",
                "expectedJournalRevision",
                "expectedCommittedSequence",
                "transactionId",
                "operation",
                "commandId",
                "contentDigest",
                "firstSequence",
                "lastSequence",
                "eventCount",
                "eventsDigest",
                "previousCommitDigest",
                "commitDigest",
                "activationEpoch",
                "latestCheckpointResourceId",
                "recordCount",
                "recordBytesBase64",
                "recordDigest",
                "resources",
            ],
        )?;
        for key in ["transactionId", "operation", "commandId"] {
            if !stable(text(body, key)?) {
                return Err(bad());
            }
        }
        for key in ["contentDigest", "recordDigest"] {
            hash_field(body, key, false)?;
        }
        for key in ["eventsDigest", "previousCommitDigest", "commitDigest"] {
            hash_field(body, key, true)?;
        }
        let bytes = decode(body, "recordBytesBase64", MAX_RECORDS)?;
        if body["recordDigest"] != digest(&bytes) {
            return Err(corrupt());
        }
        let records = records(&bytes)?;
        if count(body, "recordCount")? != records.len() as u64 {
            return Err(bad());
        }
        let request_digest = digest(canonical(body)?.as_bytes());
        let prior:Option<(String,String)>=self.db.query_row("SELECT request_digest,body FROM transactions WHERE (operation=?1 AND command_id=?2) OR transaction_id=?3",params![text(body,"operation")?,text(body,"commandId")?,text(body,"transactionId")?],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(database)?;
        if let Some((prior_digest, stored)) = prior {
            if prior_digest != request_digest {
                return Err(Failure {
                    status: 409,
                    code: "managed_session_idempotency_conflict",
                });
            }
            let stored: Value = serde_json::from_str(&stored).map_err(|_| corrupt())?;
            return Response::json(receipt(&stored, true));
        }
        let head = self.head.as_ref().ok_or_else(conflict)?.clone();
        let descriptor = self.describe(&records, &bytes, head.epoch)?;
        for key in [
            "transactionId",
            "operation",
            "commandId",
            "contentDigest",
            "firstSequence",
            "lastSequence",
            "eventCount",
            "eventsDigest",
            "previousCommitDigest",
            "commitDigest",
            "activationEpoch",
            "latestCheckpointResourceId",
        ] {
            if body[key] != descriptor[key] {
                return Err(corrupt());
            }
        }
        self.writer(body, hash, true)?;
        if count(body, "expectedJournalRevision")? != head.revision
            || count(body, "expectedCommittedSequence")? != head.sequence
            || body["previousCommitDigest"] != head.commit
            || head.recovery != "READY"
        {
            return Err(Failure {
                status: 409,
                code: "managed_session_head_conflict",
            });
        }
        let genesis = body["operation"] == "session.create";
        if (genesis && (head.revision != 0 || head.sequence != 0))
            || (!genesis
                && (head.revision == 0 || count(body, "firstSequence")? != head.sequence + 1))
        {
            return Err(conflict());
        }
        let resources = body["resources"].as_array().ok_or_else(bad)?;
        if resources.len() > 1024 {
            return Err(bad());
        }
        let mut validated = BTreeMap::new();
        let mut total = 0usize;
        for resource in resources {
            allowed_keys(
                resource,
                &[
                    "resourceId",
                    "kind",
                    "schemaVersion",
                    "byteLength",
                    "digest",
                    "bytesBase64",
                ],
            )?;
            let id = text(resource, "resourceId")?;
            if !stable(id)
                || count(resource, "schemaVersion")? != 1
                || count(resource, "byteLength")? > 65536
            {
                return Err(bad());
            }
            hash_field(resource, "digest", false)?;
            let mut metadata = resource.clone();
            metadata.as_object_mut().unwrap().remove("bytesBase64");
            let supplied = if resource.get("bytesBase64").is_some_and(|v| !v.is_null()) {
                Some(decode(resource, "bytesBase64", 65536)?)
            } else {
                None
            };
            let existing: Option<(String, Vec<u8>)> = self
                .db
                .query_row(
                    "SELECT metadata,bytes FROM resources WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(database)?;
            let content = match (existing, supplied) {
                (Some((prior, bytes)), provided) => {
                    if serde_json::from_str::<Value>(&prior).map_err(|_| corrupt())? != metadata
                        || provided.as_ref().is_some_and(|supplied| supplied != &bytes)
                    {
                        return Err(corrupt());
                    }
                    bytes
                }
                (None, Some(bytes)) => bytes,
                (None, None) => {
                    return Err(Failure {
                        status: 409,
                        code: "managed_session_resource_missing",
                    })
                }
            };
            if content.len() as u64 != count(resource, "byteLength")?
                || resource["digest"] != digest(&content)
            {
                return Err(corrupt());
            }
            self.validate_resource(&metadata, &content)?;
            total += content.len();
            if total > MAX_RECORDS {
                return Err(bad());
            }
            if validated
                .insert(id.to_string(), (metadata, content))
                .is_some()
            {
                return Err(bad());
            }
        }
        let mut refs = Vec::new();
        for record in &records {
            collect_refs(record, &mut refs)?;
        }
        let mut checked = std::collections::HashSet::new();
        while let Some(reference) = refs.pop() {
            let id = text(&reference, "resourceId")?;
            let (metadata, content) = if let Some(resource) = validated.get(id) {
                resource.clone()
            } else {
                self.resource(id)?
            };
            if metadata != reference {
                return Err(corrupt());
            }
            if checked.insert(id.to_string()) {
                let parsed: Value = serde_json::from_slice(&content).map_err(|_| corrupt())?;
                collect_refs(&parsed, &mut refs)?;
            }
            if checked.len() > 4096 {
                return Err(bad());
            }
        }
        if !body["latestCheckpointResourceId"].is_null() {
            let id = text(body, "latestCheckpointResourceId")?;
            if !validated
                .get(id)
                .is_some_and(|(metadata, _)| metadata["kind"] == "managed-checkpoint")
            {
                return Err(bad());
            }
        }
        let durable:i64=self.db.query_row("SELECT coalesce((SELECT sum(length(bytes)) FROM resources),0)+coalesce((SELECT sum(length(body)) FROM transactions),0)",[],|row|row.get(0)).map_err(database)?;
        if durable + total as i64 + bytes.len() as i64 > MAX_DURABLE {
            return Err(bad());
        }
        let revision = head
            .revision
            .checked_add(1)
            .filter(|v| *v <= MAX_COUNTER)
            .ok_or_else(conflict)?;
        let mut stored = body.clone();
        let map = stored.as_object_mut().unwrap();
        for key in [
            "workspaceId",
            "writerId",
            "expectedJournalRevision",
            "expectedCommittedSequence",
            "resources",
            "recordCount",
        ] {
            map.remove(key);
        }
        map.insert("journalRevision".into(), json!(revision));
        map.insert("recordEncoding".into(), json!("identity"));
        map.insert("byteLength".into(), json!(bytes.len()));
        let mut next = head;
        next.revision = revision;
        next.sequence = count(body, "lastSequence")?;
        next.commit = body["commitDigest"].clone();
        next.epoch = count(body, "activationEpoch")?;
        if !body["latestCheckpointResourceId"].is_null() {
            next.checkpoint = body["latestCheckpointResourceId"].clone();
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        for (id, (metadata, bytes)) in validated {
            tx.execute(
                "INSERT INTO resources VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING",
                params![id, metadata.to_string(), bytes],
            )
            .map_err(database)?;
        }
        if !genesis {
            for record in &records[..records.len() - 1] {
                let event = &record["managedSession"];
                tx.execute(
                    "INSERT INTO events VALUES(?1,?2,?3)",
                    params![
                        i64::try_from(count(event, "sequence")?).map_err(|_| bad())?,
                        text(event, "eventId")?,
                        event.to_string()
                    ],
                )
                .map_err(database)?;
            }
        }
        tx.execute(
            "INSERT INTO transactions VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                i64::try_from(revision).map_err(|_| bad())?,
                text(body, "transactionId")?,
                text(body, "operation")?,
                text(body, "commandId")?,
                request_digest,
                stored.to_string()
            ],
        )
        .map_err(database)?;
        tx.execute(
            "UPDATE head SET body=?1 WHERE id=1",
            [next.value().to_string()],
        )
        .map_err(database)?;
        tx.commit().map_err(database)?;
        self.head = Some(next);
        Response::json(receipt(&stored, false))
    }
    fn describe(&self, records: &[Value], bytes: &[u8], current_epoch: u64) -> Result<Value> {
        for record in records {
            safe_content(record, 0)?;
            if record["sessionId"] != self.session || record["type"] != "system" {
                return Err(corrupt());
            }
        }
        if records.len() == 2 && records[1]["subtype"] == "managed_session_header_v1" {
            let header = &records[1]["managedSession"];
            allowed_keys(
                header,
                &[
                    "formatVersion",
                    "minimumReader",
                    "sessionKey",
                    "engine",
                    "definitionRef",
                    "rootSnapshotRef",
                    "createdBy",
                    "baseTranscriptProof",
                ],
            )?;
            text(header, "createdBy")?;
            if !header["baseTranscriptProof"].is_null() {
                validate_ref(&header["baseTranscriptProof"])?;
            }
            if records[0]["subtype"] != "session_execution_engine"
                || records[0]["systemPayload"]["version"] != 1
                || records[0]["systemPayload"]["engine"] != "managed"
                || header["sessionKey"] != self.session_key()
                || header["formatVersion"] != 1
                || header["minimumReader"] != "managed-session/1"
                || header["engine"] != "managed"
            {
                return Err(corrupt());
            }
            for key in ["definitionRef", "rootSnapshotRef"] {
                validate_ref(&header[key])?;
            }
            let id = format!("session.create:{}", self.session);
            return Ok(
                json!({"transactionId":id,"commandId":id,"operation":"session.create","contentDigest":digest(bytes),"firstSequence":0,"lastSequence":0,"eventCount":0,"eventsDigest":null,"previousCommitDigest":null,"commitDigest":null,"activationEpoch":current_epoch,"latestCheckpointResourceId":null}),
            );
        }
        if !(2..=257).contains(&records.len()) {
            return Err(bad());
        }
        let mut events = Vec::new();
        let mut epoch = current_epoch;
        let mut checkpoint = Value::Null;
        for record in &records[..records.len() - 1] {
            if record["subtype"] != "managed_session_event_v1" {
                return Err(corrupt());
            }
            let event = &record["managedSession"];
            allowed_keys(
                event,
                &[
                    "v",
                    "sequence",
                    "eventId",
                    "sessionKey",
                    "kind",
                    "occurredAt",
                    "subject",
                    "payload",
                ],
            )?;
            if event["v"] != 1
                || event["sessionKey"] != self.session_key()
                || count(event, "sequence")? == 0
                || !stable(text(event, "eventId")?)
            {
                return Err(corrupt());
            }
            if count(event, "occurredAt")? > 8_640_000_000_000_000 {
                return Err(bad());
            }
            let kind = text(event, "kind")?;
            if matches!(
                kind,
                "model.attempt" | "checkpoint.committed" | "context.compacted"
            ) && event["subject"]["type"] != "activation"
            {
                return Err(corrupt());
            }
            validate_event(kind, &event["payload"])?;
            let payload = &event["payload"];
            if !event["subject"].is_null() {
                let subject = &event["subject"];
                match text(subject, "type")? {
                    "activation" => {
                        allowed_keys(subject, &["type", "scopeId", "activationId", "epoch"])?;
                        if count(subject, "epoch")? != epoch {
                            return Err(conflict());
                        }
                    }
                    "turn" => {
                        allowed_keys(subject, &["type", "turnId"])?;
                        text(subject, "turnId")?;
                    }
                    _ => return Err(effect()),
                }
            }
            if kind == "activation.changed" {
                if payload["workerId"].as_str() != self.boot.as_deref() {
                    return Err(conflict());
                }
                let next = count(payload, "epoch")?;
                if next < epoch || next > epoch + 1 {
                    return Err(conflict());
                }
                epoch = next;
            }
            if kind == "checkpoint.committed" {
                let covered = count(payload, "coveredSequence")?;
                if covered == 0 || covered > self.head.as_ref().ok_or_else(corrupt)?.sequence {
                    return Err(corrupt());
                }
                validate_ref(&payload["stateRef"])?;
                if payload["stateRef"]["kind"] != "managed-checkpoint" {
                    return Err(effect());
                }
                checkpoint = payload["stateRef"]["resourceId"].clone();
            }
            if kind == "context.compacted" {
                let from = count(payload, "fromSequence")?;
                let to = count(payload, "toSequence")?;
                if from == 0 || to < from || to > self.head.as_ref().ok_or_else(corrupt)?.sequence {
                    return Err(corrupt());
                }
            }
            if kind == "domain.committed"
                && !matches!(
                    payload["domain"].as_str(),
                    Some("session_metadata" | "session_source" | "file_history")
                )
            {
                return Err(effect());
            }
            events.push(event.clone());
        }
        let last = records.last().unwrap();
        if last["subtype"] != "managed_session_commit_v1" {
            return Err(corrupt());
        }
        let marker = &last["managedSession"];
        allowed_keys(
            marker,
            &[
                "transactionId",
                "commandId",
                "operation",
                "contentDigest",
                "firstSequence",
                "lastSequence",
                "eventCount",
                "eventsDigest",
                "previousCommitDigest",
            ],
        )?;
        for key in ["transactionId", "commandId", "operation"] {
            if !stable(text(marker, key)?) {
                return Err(bad());
            }
        }
        hash_field(marker, "contentDigest", false)?;
        hash_field(marker, "eventsDigest", false)?;
        hash_field(marker, "previousCommitDigest", true)?;
        let first = count(marker, "firstSequence")?;
        let final_sequence = count(marker, "lastSequence")?;
        if first == 0
            || count(marker, "eventCount")? != events.len() as u64
            || final_sequence
                .checked_sub(first)
                .and_then(|n| n.checked_add(1))
                != Some(events.len() as u64)
        {
            return Err(corrupt());
        }
        for (index, event) in events.iter().enumerate() {
            if count(event, "sequence")? != first + index as u64 {
                return Err(corrupt());
            }
        }
        if marker["eventsDigest"] != digest(canonical(&json!(events))?.as_bytes()) {
            return Err(corrupt());
        }
        let mut descriptor = marker.clone();
        descriptor["commitDigest"] = json!(digest(canonical(marker)?.as_bytes()));
        descriptor["activationEpoch"] = json!(epoch);
        descriptor["latestCheckpointResourceId"] = checkpoint;
        Ok(descriptor)
    }
    fn validate_resource(&self, metadata: &Value, bytes: &[u8]) -> Result<()> {
        let kind = text(metadata, "kind")?;
        if !kind.starts_with("managed-")
            || [
                "tool",
                "shell",
                "mcp",
                "hook",
                "publication",
                "workspace-operation",
                "goal",
                "team",
                "memory",
                "schedule",
                "automation",
                "child",
            ]
            .iter()
            .any(|denied| kind.contains(denied))
        {
            return Err(effect());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| bad())?;
        safe_content(&value, 0)?;
        if kind == "managed-message" {
            match value["type"].as_str() {
                Some("assistant") => {
                    text(&value, "model")?;
                    assistant_text(&value)?;
                }
                Some("user" | "system") => {}
                _ => return Err(effect()),
            }
        }
        if kind == "managed-turn-result"
            && (value["type"] != "system" || value["subtype"] != "turn_result")
        {
            return Err(corrupt());
        }
        Ok(())
    }
    fn final_assistant(&self, prompt_id: &str, model: &str) -> Result<String> {
        if self.failed || self.effect || !stable(prompt_id) || !stable(model) {
            return Err(corrupt());
        }
        let mut statement = self
            .db
            .prepare("SELECT body FROM events ORDER BY sequence")
            .map_err(database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database)?;
        let mut output = String::new();
        let mut settled = false;
        let mut accepted = false;
        let mut settled_sequence = 0u64;
        for row in rows {
            let event: Value =
                serde_json::from_str(&row.map_err(database)?).map_err(|_| corrupt())?;
            let payload = &event["payload"];
            if event["kind"] == "input.accepted" && payload["turnId"] == prompt_id {
                if accepted || settled {
                    return Err(corrupt());
                }
                accepted = true;
            }
            if event["kind"] == "message.committed" && payload["role"] == "assistant" {
                let id = text(&payload["contentRef"], "resourceId")?;
                let (metadata, bytes) = self.resource(id)?;
                if metadata != payload["contentRef"] || metadata["kind"] != "managed-message" {
                    return Err(corrupt());
                }
                let record: Value = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
                if record["daemonPromptId"] == prompt_id {
                    if !accepted
                        || settled
                        || record["model"] != model
                        || record["sessionId"] != self.session
                        || record["uuid"] != payload["messageId"]
                    {
                        return Err(corrupt());
                    }
                    output.push_str(&assistant_text(&record)?);
                    if output.len() > MAX_OUTPUT {
                        return Err(bad());
                    }
                }
            }
            if event["kind"] == "turn.settled" && payload["turnId"] == prompt_id {
                if settled
                    || payload["outcome"] != "completed"
                    || payload["stopReason"] != "end_turn"
                {
                    return Err(corrupt());
                }
                let (metadata, bytes) =
                    self.resource(text(&payload["resultRef"], "resourceId")?)?;
                if metadata != payload["resultRef"] || metadata["kind"] != "managed-turn-result" {
                    return Err(corrupt());
                }
                let result: Value = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
                if result["sessionId"] != self.session
                    || result["systemPayload"]["promptId"] != prompt_id
                    || result["systemPayload"]["state"] != "completed"
                    || result["systemPayload"]["stopReason"] != "end_turn"
                {
                    return Err(corrupt());
                }
                settled = true;
                settled_sequence = count(&event, "sequence")?;
            }
        }
        if !accepted || !settled || settled_sequence == 0 || output.trim().is_empty() {
            return Err(corrupt());
        }
        Ok(output)
    }
}
fn lease(body: &Value) -> Result<u64> {
    let value = count(body, "leaseMillis")?;
    if !(1000..=300000).contains(&value) {
        return Err(bad());
    }
    Ok(value)
}
fn allowed_keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let map = value.as_object().ok_or_else(bad)?;
    if map.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(bad());
    }
    Ok(())
}
fn receipt(stored: &Value, replayed: bool) -> Value {
    json!({"journalRevision":stored["journalRevision"],"transactionId":stored["transactionId"],"commandId":stored["commandId"],"operation":stored["operation"],"firstSequence":stored["firstSequence"],"lastSequence":stored["lastSequence"],"committedSequence":stored["lastSequence"],"commitDigest":stored["commitDigest"],"replayed":replayed})
}
fn records(bytes: &[u8]) -> Result<Vec<Value>> {
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    if !text.ends_with('\n') {
        return Err(corrupt());
    }
    let mut result = Vec::new();
    for line in text[..text.len() - 1].split('\n') {
        if line.len() > 1024 * 1024 || result.len() >= 257 {
            return Err(bad());
        }
        let value: Value = serde_json::from_str(line).map_err(|_| corrupt())?;
        if !value.is_object() {
            return Err(corrupt());
        }
        result.push(value);
    }
    Ok(result)
}
fn validate_ref(reference: &Value) -> Result<()> {
    allowed_keys(
        reference,
        &[
            "resourceId",
            "kind",
            "schemaVersion",
            "byteLength",
            "digest",
        ],
    )?;
    if !stable(text(reference, "resourceId")?)
        || !stable(text(reference, "kind")?)
        || count(reference, "schemaVersion")? != 1
        || count(reference, "byteLength")? > 65536
    {
        return Err(bad());
    }
    hash_field(reference, "digest", false)
}
fn collect_refs(value: &Value, out: &mut Vec<Value>) -> Result<()> {
    fn walk(v: &Value, out: &mut Vec<Value>, depth: usize) -> Result<()> {
        if depth > 64 || out.len() > 4096 {
            return Err(bad());
        }
        if v.get("resourceId").is_some()
            && v.get("kind").is_some()
            && v.get("schemaVersion").is_some()
            && v.get("byteLength").is_some()
            && v.get("digest").is_some()
        {
            validate_ref(v)?;
            out.push(v.clone());
            return Ok(());
        }
        match v {
            Value::Object(m) => {
                for v in m.values() {
                    walk(v, out, depth + 1)?;
                }
            }
            Value::Array(a) => {
                for v in a {
                    walk(v, out, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, out, 0)
}
fn safe_content(value: &Value, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(bad());
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "functionCall"
                        | "functionResponse"
                        | "inlineData"
                        | "fileData"
                        | "tool_result"
                        | "toolOutcome"
                ) && !value.is_null()
                {
                    return Err(effect());
                }
                if matches!(
                    key.as_str(),
                    "accessToken" | "refreshToken" | "apiKey" | "password" | "clientSecret"
                ) && value.as_str().is_some_and(|s| !s.is_empty())
                {
                    return Err(effect());
                }
                if matches!(
                    key.as_str(),
                    "pendingTools" | "toolCalls" | "toolResults" | "pendingOwners"
                ) && value.as_array().is_some_and(|a| !a.is_empty())
                {
                    return Err(effect());
                }
                safe_content(value, depth + 1)?;
            }
        }
        Value::Array(a) => {
            for v in a {
                safe_content(v, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn assistant_text(record: &Value) -> Result<String> {
    if record["type"] != "assistant" || record["message"]["role"] != "model" {
        return Err(corrupt());
    }
    let parts = record["message"]["parts"].as_array().ok_or_else(corrupt)?;
    let mut output = String::new();
    for part in parts {
        allowed_keys(part, &["text", "thought"])?;
        if part.get("thought").is_some_and(|thought| thought != false) {
            return Err(effect());
        }
        output.push_str(part["text"].as_str().ok_or_else(corrupt)?);
        if output.len() > MAX_OUTPUT {
            return Err(bad());
        }
    }
    Ok(output)
}
fn validate_event(kind: &str, payload: &Value) -> Result<()> {
    let fields: &[&str] = match kind {
        "input.accepted" => &[
            "inputId",
            "turnId",
            "source",
            "contentRef",
            "deadline",
            "admissionRef",
        ],
        "wake.requested" => &[
            "wakeId",
            "reason",
            "subject",
            "sourceEventId",
            "requiredSequence",
        ],
        "activation.changed" => &[
            "activationId",
            "epoch",
            "workerId",
            "subject",
            "phase",
            "leaseDurationMs",
            "expiresAt",
            "installRef",
            "boundaryRef",
            "renewalSeq",
        ],
        "model.attempt" => &[
            "attemptId",
            "routeRef",
            "inputCheckpointRef",
            "state",
            "usageRef",
        ],
        "message.committed" => &[
            "messageId",
            "role",
            "contentRef",
            "modelAttemptId",
            "parentMessageId",
        ],
        "checkpoint.committed" => &[
            "checkpointId",
            "coveredSequence",
            "previousCheckpointId",
            "stateRef",
            "boundary",
        ],
        "context.compacted" => &[
            "compactionId",
            "fromSequence",
            "toSequence",
            "summaryRef",
            "replacedMessageIds",
            "tokenCountsRef",
        ],
        "cancel.requested" => &["requestId", "target", "reason", "requestedBy"],
        "turn.settled" => &[
            "turnId",
            "outcome",
            "stopReason",
            "resultRef",
            "usageRef",
            "pendingOwnersRef",
        ],
        "config.bound" => &[
            "revision",
            "previousRevision",
            "bundleRef",
            "rootSnapshotRef",
        ],
        "lifecycle.changed" => &["operationId", "from", "to", "reason", "pendingOwnersRef"],
        "domain.committed" => &["domain", "operationId", "version", "recordRef"],
        _ => return Err(effect()),
    };
    allowed_keys(payload, fields)?;
    safe_content(payload, 0)?;
    for field in fields {
        if matches!(*field, "modelAttemptId" | "renewalSeq") {
            continue;
        }
        if payload.get(*field).is_none() {
            return Err(bad());
        }
    }
    for (field, value) in payload.as_object().unwrap() {
        if field.ends_with("Ref") {
            if !value.is_null() {
                validate_ref(value)?;
            } else if matches!(
                field.as_str(),
                "contentRef"
                    | "admissionRef"
                    | "routeRef"
                    | "stateRef"
                    | "summaryRef"
                    | "bundleRef"
                    | "rootSnapshotRef"
                    | "recordRef"
            ) {
                return Err(bad());
            }
        } else if matches!(
            field.as_str(),
            "epoch"
                | "leaseDurationMs"
                | "expiresAt"
                | "renewalSeq"
                | "coveredSequence"
                | "fromSequence"
                | "toSequence"
                | "requiredSequence"
                | "revision"
                | "previousRevision"
                | "deadline"
                | "version"
        ) {
            if !value.is_null() {
                count(payload, field)?;
            } else if matches!(
                field.as_str(),
                "epoch"
                    | "coveredSequence"
                    | "fromSequence"
                    | "toSequence"
                    | "requiredSequence"
                    | "revision"
                    | "version"
            ) {
                return Err(bad());
            }
        } else if field.ends_with("Id") {
            if !value.is_null() {
                if !stable(text(payload, field)?) {
                    return Err(bad());
                }
            } else if !matches!(
                field.as_str(),
                "previousCheckpointId" | "modelAttemptId" | "parentMessageId"
            ) {
                return Err(bad());
            }
        } else if field == "subject" {
            let subject_type = text(value, "type")?;
            if !matches!(subject_type, "activation" | "turn") {
                return Err(effect());
            }
            if subject_type == "activation" {
                count(value, "epoch")?;
                text(value, "activationId")?;
                text(value, "scopeId")?;
            } else {
                text(value, "turnId")?;
            }
        } else if field == "replacedMessageIds" {
            if !value
                .as_array()
                .is_some_and(|ids| ids.iter().all(|id| id.as_str().is_some_and(stable)))
            {
                return Err(bad());
            }
        } else if field != "target" {
            if !value.is_null() {
                text(payload, field)?;
            } else if !matches!(field.as_str(), "from" | "stopReason" | "boundary") {
                return Err(bad());
            }
        }
    }
    if kind == "domain.committed"
        && (payload["version"] != 1
            || payload["recordRef"]["kind"] != format!("managed-{}", text(payload, "domain")?))
    {
        return Err(corrupt());
    }
    if kind == "activation.changed" {
        let open = matches!(payload["phase"].as_str(), Some("installing" | "active"));
        if payload["expiresAt"].is_null()
            || (open
                && (payload["installRef"].is_null()
                    || payload["leaseDurationMs"].is_null()
                    || !payload["boundaryRef"].is_null()))
            || (!open && payload["boundaryRef"].is_null())
        {
            return Err(bad());
        }
    }
    if kind == "message.committed" {
        if !matches!(
            payload["role"].as_str(),
            Some("assistant" | "user" | "system")
        ) {
            return Err(effect());
        }
        validate_ref(&payload["contentRef"])?;
    }
    if kind == "activation.changed" {
        if !matches!(
            payload["phase"].as_str(),
            Some("installing" | "active" | "released" | "revoked")
        ) {
            return Err(bad());
        }
        count(payload, "epoch")?;
    }
    if kind == "model.attempt"
        && !matches!(
            payload["state"].as_str(),
            Some("started" | "output_committed" | "abandoned")
        )
    {
        return Err(bad());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn resource(id: &str, kind: &str, content: Value) -> Value {
        let bytes = serde_json::to_vec(&content).unwrap();
        json!({"resourceId":id,"kind":kind,"schemaVersion":1,"byteLength":bytes.len(),"digest":digest(&bytes),"bytesBase64":STANDARD.encode(bytes)})
    }
    fn reference(mut resource: Value) -> Value {
        resource.as_object_mut().unwrap().remove("bytesBase64");
        resource
    }
    fn setup() -> (tempfile::TempDir, OwnedStore) {
        let parent = tempfile::tempdir().unwrap();
        let store = OwnedStore::start(parent.path(), "session", "workspace", "tenant").unwrap();
        store.bind_boot("boot").unwrap();
        (parent, store)
    }
    fn acquire(state: &mut State) -> String {
        let token = "a".repeat(43);
        let request = Request {
            method: "POST".into(),
            path: "/internal/managed-session-store/v1/sessions/session/writers:acquire".into(),
            query: HashMap::new(),
            headers: HashMap::from([
                ("x-qwen-tenant-id".into(), "tenant".into()),
                ("x-qwen-managed-writer-token".into(), token.clone()),
            ]),
            body: json!({"workspaceId":"workspace","writerId":"boot","leaseMillis":60000}),
        };
        state.handle(&request).unwrap();
        digest(token.as_bytes())
    }
    fn genesis(state: &State) -> Value {
        let definition = resource(
            "definition",
            "managed-definition",
            json!({"version":1,"engine":"managed","model":"qwen-fixture"}),
        );
        let root = resource("root", "managed-root", json!({"version":1,"messages":[]}));
        let records = vec![
            json!({"type":"system","sessionId":"session","subtype":"session_execution_engine","systemPayload":{"version":1,"engine":"managed"}}),
            json!({"type":"system","sessionId":"session","subtype":"managed_session_header_v1","managedSession":{"formatVersion":1,"minimumReader":"managed-session/1","sessionKey":state.session_key(),"engine":"managed","definitionRef":reference(definition.clone()),"rootSnapshotRef":reference(root.clone()),"createdBy":"fixture"}}),
        ];
        let bytes = (records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n")
            .into_bytes();
        let mut body = state.describe(&records, &bytes, 0).unwrap();
        let map = body.as_object_mut().unwrap();
        map.extend(json!({"workspaceId":"workspace","writerId":"boot","writerGeneration":1,"expectedJournalRevision":0,"expectedCommittedSequence":0,"recordCount":2,"recordBytesBase64":STANDARD.encode(&bytes),"recordDigest":digest(&bytes),"resources":[definition,root]}).as_object().unwrap().clone());
        body
    }
    fn transaction(state: &State, events: Vec<Value>, resources: Vec<Value>) -> Value {
        let first = events[0]["sequence"].clone();
        let last = events.last().unwrap()["sequence"].clone();
        let marker = json!({"transactionId":"turn-transaction","commandId":"prompt","operation":"settleTurn","contentDigest":"c".repeat(64),"firstSequence":first,"lastSequence":last,"eventCount":events.len(),"eventsDigest":digest(canonical(&json!(events)) .unwrap().as_bytes()),"previousCommitDigest":state.head.as_ref().unwrap().commit});
        let mut records=events.iter().map(|event|json!({"type":"system","sessionId":"session","subtype":"managed_session_event_v1","managedSession":event})).collect::<Vec<_>>();
        records.push(json!({"type":"system","sessionId":"session","subtype":"managed_session_commit_v1","managedSession":marker}));
        let bytes = (records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n")
            .into_bytes();
        let mut body = state
            .describe(&records, &bytes, state.head.as_ref().unwrap().epoch)
            .unwrap();
        body.as_object_mut().unwrap().extend(json!({"workspaceId":"workspace","writerId":"boot","writerGeneration":1,"expectedJournalRevision":state.head.as_ref().unwrap().revision,"expectedCommittedSequence":state.head.as_ref().unwrap().sequence,"recordCount":records.len(),"recordBytesBase64":STANDARD.encode(&bytes),"recordDigest":digest(&bytes),"resources":resources}).as_object().unwrap().clone());
        body
    }
    #[test]
    fn authoritative_final_text_requires_committed_prompt_model_and_settlement() {
        let (_parent, mut store) = setup();
        {
            let mut state = store.state.lock().unwrap();
            let hash = acquire(&mut state);
            let initial = genesis(&state);
            state.commit(&initial, &hash).unwrap();
            let input = resource("input", "managed-input", json!({"prompt":"hello"}));
            let admission = resource(
                "admission",
                "managed-admission",
                json!({"promptId":"prompt"}),
            );
            let assistant = resource(
                "assistant",
                "managed-message",
                json!({"uuid":"message","sessionId":"session","type":"assistant","daemonPromptId":"prompt","model":"exact-model","message":{"role":"model","parts":[{"text":"Hello"},{"text":" world"}]}}),
            );
            let result = resource(
                "result",
                "managed-turn-result",
                json!({"sessionId":"session","type":"system","subtype":"turn_result","systemPayload":{"promptId":"prompt","state":"completed","stopReason":"end_turn"}}),
            );
            let events = vec![
                json!({"v":1,"sequence":1,"eventId":"input-event","sessionKey":state.session_key(),"kind":"input.accepted","occurredAt":1,"payload":{"inputId":"prompt","turnId":"prompt","source":"user","contentRef":reference(input.clone()),"deadline":null,"admissionRef":reference(admission.clone())}}),
                json!({"v":1,"sequence":2,"eventId":"message-event","sessionKey":state.session_key(),"kind":"message.committed","occurredAt":2,"payload":{"messageId":"message","role":"assistant","contentRef":reference(assistant.clone()),"parentMessageId":null}}),
                json!({"v":1,"sequence":3,"eventId":"settled-event","sessionKey":state.session_key(),"kind":"turn.settled","occurredAt":3,"payload":{"turnId":"prompt","outcome":"completed","stopReason":"end_turn","resultRef":reference(result.clone()),"usageRef":null,"pendingOwnersRef":null}}),
            ];
            let body = transaction(&state, events, vec![input, admission, assistant, result]);
            state.commit(&body, &hash).unwrap();
            assert_eq!(
                state.final_assistant("prompt", "exact-model").unwrap(),
                "Hello world"
            );
            assert!(state.final_assistant("prompt", "other-model").is_err());
            assert!(state
                .final_assistant("other-prompt", "exact-model")
                .is_err());
        }
        store.stop_and_join().unwrap();
    }
    #[test]
    fn full_commit_precedes_receipt_and_identical_request_replays() {
        let (_parent, mut store) = setup();
        {
            let mut state = store.state.lock().unwrap();
            let hash = acquire(&mut state);
            let body = genesis(&state);
            let receipt = state.commit(&body, &hash).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&receipt.body).unwrap()["journalRevision"],
                1
            );
            assert_eq!(
                state
                    .db
                    .query_row("SELECT count(*) FROM transactions", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                state
                    .db
                    .query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
            let replay = state.commit(&body, &hash).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&replay.body).unwrap()["replayed"],
                true
            );
            assert_eq!(state.head.as_ref().unwrap().revision, 1);
            let mut changed = body;
            changed["contentDigest"] = json!("b".repeat(64));
            assert_eq!(
                state.commit(&changed, &hash).unwrap_err().code,
                "managed_session_idempotency_conflict"
            );
        }
        store.stop_and_join().unwrap();
    }
    #[test]
    fn resources_and_journal_are_rolled_back_on_corrupt_bytes() {
        let (_parent, mut store) = setup();
        {
            let mut state = store.state.lock().unwrap();
            let hash = acquire(&mut state);
            let mut body = genesis(&state);
            body["resources"][1]["digest"] = json!("f".repeat(64));
            assert!(state.commit(&body, &hash).is_err());
            assert_eq!(state.head.as_ref().unwrap().revision, 0);
            assert_eq!(
                state
                    .db
                    .query_row("SELECT count(*) FROM resources", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        store.stop_and_join().unwrap();
    }
    #[test]
    fn tools_media_and_plaintext_credentials_are_rejected_before_commit() {
        for kind in [
            "tool.intent",
            "action.changed",
            "tool.receipt",
            "unknown.event",
        ] {
            assert_eq!(
                validate_event(kind, &json!({})).unwrap_err().code,
                effect().code
            );
        }
        for content in [
            json!({"functionCall":{"name":"shell"}}),
            json!({"inlineData":{"data":"secret"}}),
            json!({"refreshToken":"secret"}),
        ] {
            assert_eq!(safe_content(&content, 0).unwrap_err().code, effect().code);
        }
        assert!(assistant_text(&json!({"type":"assistant","message":{"role":"model","parts":[{"text":"hidden","thought":true}]}})).is_err());
    }
    #[test]
    fn initial_binding_fences_boot_token_and_expired_lease() {
        let (_parent, mut store) = setup();
        {
            let mut state = store.state.lock().unwrap();
            let hash = acquire(&mut state);
            let body = json!({"writerId":"boot","writerGeneration":1});
            assert!(state.writer(&body, &hash, true).is_ok());
            assert!(state.writer(&body, &"b".repeat(64), true).is_err());
            assert!(state
                .writer(
                    &json!({"writerId":"another","writerGeneration":1}),
                    &hash,
                    true
                )
                .is_err());
            state.head.as_mut().unwrap().lease = 0;
            assert!(state.writer(&body, &hash, true).is_err());
            assert!(state.writer(&body, &hash, false).is_ok());
        }
        assert!(store.bind_boot("another").is_err());
        store.stop_and_join().unwrap();
    }
    #[test]
    fn unrelated_requests_do_not_poison_but_writer_integrity_failures_do() {
        let (_parent, mut store) = setup();
        {
            let mut state = store.state.lock().unwrap();
            let initial = Request {
                method: "POST".into(),
                path: "/internal/managed-session-store/v1/sessions/session/writers:acquire".into(),
                query: HashMap::new(),
                headers: HashMap::from([
                    ("x-qwen-tenant-id".into(), "tenant".into()),
                    ("x-qwen-managed-writer-token".into(), "a".repeat(43)),
                ]),
                body: json!({"workspaceId":"workspace","writerId":"boot","leaseMillis":0}),
            };
            assert!(state.dispatch(&initial).is_err());
            assert!(!state.failed && state.head.is_none());
            acquire(&mut state);
            let mut request = Request {
                method: "POST".into(),
                path: "/internal/managed-session-store/v1/sessions/session/writers:renew".into(),
                query: HashMap::new(),
                headers: HashMap::from([
                    ("x-qwen-tenant-id".into(), "tenant".into()),
                    ("x-qwen-managed-writer-token".into(), "a".repeat(43)),
                ]),
                body: json!({"workspaceId":"workspace","writerId":"boot","writerGeneration":1,"leaseMillis":60000}),
            };
            let original = request.headers.clone();
            request.headers.clear();
            assert!(state.dispatch(&request).is_err());
            assert!(!state.failed && !state.effect);
            request.headers = original;
            request.body["workspaceId"] = json!("another");
            assert!(state.dispatch(&request).is_err());
            assert!(!state.failed && !state.effect);
            request.body["workspaceId"] = json!("workspace");
            request.path = "/internal/managed-session-store/v1/sessions/session/unknown".into();
            assert!(state.dispatch(&request).is_err());
            assert!(!state.failed && !state.effect);
            request.path =
                "/internal/managed-session-store/v1/sessions/session/writers:renew".into();
            assert!(state.dispatch(&request).is_ok());
            request.body["writerGeneration"] = json!(2);
            assert!(state.dispatch(&request).is_err());
            assert!(state.failed && !state.effect);
        }
        store.stop_and_join().unwrap();
    }
    #[test]
    fn canonical_digest_is_key_order_independent_and_utf16_sorted() {
        assert_eq!(
            canonical(&json!({"z":1,"a":[true,null]})).unwrap(),
            "{\"a\":[true,null],\"z\":1}"
        );
        let value = json!({"\u{10000}":1,"\u{e000}":2});
        assert_eq!(canonical(&value).unwrap(), "{\"𐀀\":1,\"\":2}");
        assert!(decode(&json!({"bytes":"YQ"}), "bytes", 64).is_err());
    }
}
