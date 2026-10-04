//! Owned HTTP gateway with explicit, validated protocol conversion.
//! Native tools remain client owned; responses are never replayed.
//! Account selection and durable intent/receipt ownership remain in the router.
use super::gateway_profiles::Protocol;
use reqwest::{Client, Url};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const MAX_HEADERS: usize = 16 * 1024;
const MAX_REQUEST: usize = 16 * 1024 * 1024;
const MAX_RESPONSE: usize = 64 * 1024 * 1024;
const MAX_ATTEMPTS: usize = 32;

pub(crate) struct Upstream {
    pub(crate) protocol: Protocol,
    pub(crate) profile_id: String,
    pub(crate) revision: u64,
    /// HTTPS origin, optionally with the native API prefix (/v1 or /v1beta).
    /// Root must validate approved provider/custom endpoints before selection.
    pub(crate) base_url: String,
    pub(crate) api_key: Zeroizing<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WireSummary {
    pub(crate) upstream_protocol: Protocol,
    pub(crate) path: String,
    pub(crate) body_bytes: usize,
    pub(crate) body_sha256: String,
}
pub(crate) struct RequestSummary {
    pub(crate) wire: Option<WireSummary>,
    pub(crate) id: String,
    pub(crate) upstream_attempt_id: String,
    pub(crate) attempt_index: usize,
    pub(crate) protocol: Protocol,
    pub(crate) path: String,
    pub(crate) model: String,
    pub(crate) body_bytes: usize,
    pub(crate) body_sha256: String,
    pub(crate) account_bound: bool,
}
pub(crate) struct Receipt {
    pub(crate) request_id: String,
    pub(crate) upstream_attempt_id: String,
    pub(crate) attempt_index: usize,
    pub(crate) profile_id: String,
    pub(crate) status: Option<u16>,
    pub(crate) downstream_started: bool,
    pub(crate) complete: bool,
    pub(crate) cancelled: bool,
}
pub(crate) trait UpstreamOwner: Send + Sync {
    // Callbacks must finish within the owner's bounded local storage deadline;
    // they must not perform network I/O or open-ended credential acquisition.
    /// Return a frozen approved account for this exact native model/protocol.
    /// `excluded` contains every account that already returned HTTP 429.
    fn select(&self, request: &RequestSummary, excluded: &[String]) -> Result<Upstream, String>;
    /// Recheck configuration, account ownership and cancellation under the
    /// owner's dispatch fence, committing durable intent before any HTTP send.
    fn fence_and_checkpoint(
        &self,
        request: &RequestSummary,
        upstream: &Upstream,
    ) -> Result<(), String>;
    /// Only called for authoritative HTTP 429 before downstream headers/body.
    fn rate_limited(
        &self,
        request: &RequestSummary,
        upstream: &Upstream,
        retry_after: Option<&str>,
    ) -> Result<(), String>;
    /// Opaque continuity may remain pinned to its creator account. The owner
    /// can additionally qualify portable native formats for the frozen pool.
    fn can_failover(&self, request: &RequestSummary, _upstream: &Upstream) -> bool {
        !request.account_bound
    }
    fn finish(&self, receipt: &Receipt) -> Result<(), String>;
}
pub(crate) struct Controller {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
}
impl Controller {
    pub(crate) fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
    }
    pub(crate) fn bind(
        protocol: Protocol,
        client_token: String,
        selected_model: String,
        owner: Arc<dyn UpstreamOwner>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        if !(32..=256).contains(&client_token.len())
            || !client_token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || selected_model.is_empty()
            || selected_model.len() > 256
            || !selected_model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        {
            return Err(error());
        }
        let listener =
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|_| error())?;
        listener.set_nonblocking(true).map_err(|_| error())?;
        let address = listener.local_addr().map_err(|_| error())?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(0)
            .http1_only()
            .build()
            .map_err(|_| error())?;
        let stop = Arc::new(AtomicBool::new(false));
        let local_stop = stop.clone();
        let token = Zeroizing::new(client_token);
        let thread = thread::spawn(move || {
            let requests = Arc::new(Mutex::new(()));
            let active = Arc::new(AtomicUsize::new(0));
            let fatal = Arc::new(AtomicBool::new(false));
            let mut workers: Vec<JoinHandle<Result<(), String>>> = Vec::new();
            let mut failed = false;
            while !halted(&local_stop, &cancelled) && !fatal.load(Ordering::SeqCst) {
                let mut i = 0;
                while i < workers.len() {
                    if workers[i].is_finished() {
                        failed |= !matches!(workers.swap_remove(i).join(), Ok(Ok(())));
                    } else {
                        i += 1;
                    }
                }
                if failed {
                    fatal.store(true, Ordering::SeqCst);
                    break;
                }
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        if !peer.ip().is_loopback() || active.load(Ordering::SeqCst) >= 4 {
                            let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                            let _ = reject(&mut stream, 503);
                            continue;
                        }
                        active.fetch_add(1, Ordering::SeqCst);
                        let (client, owner, stop, cancelled, active, token, model) = (
                            client.clone(),
                            owner.clone(),
                            local_stop.clone(),
                            cancelled.clone(),
                            active.clone(),
                            token.clone(),
                            selected_model.clone(),
                        );
                        let fatal = fatal.clone();
                        let requests = requests.clone();
                        workers.push(thread::spawn(move || {
                            let accepted = AtomicBool::new(false);
                            let context = RequestContext {
                                address,
                                protocol,
                                token: &token,
                                model: &model,
                                client: &client,
                                owner: &*owner,
                                stop: &stop,
                                cancel: &cancelled,
                                accepted: &accepted,
                                fatal: &fatal,
                                requests: &requests,
                            };
                            let result = serve(stream, context);
                            active.fetch_sub(1, Ordering::SeqCst);
                            classify_worker(result, accepted.load(Ordering::SeqCst), &fatal)
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            // No new requests can enter; every active HTTP future observes stop.
            local_stop.store(true, Ordering::SeqCst);
            drop(listener);
            for worker in workers {
                failed |= !matches!(worker.join(), Ok(Ok(())));
            }
            failed |= fatal.load(Ordering::SeqCst);
            if failed {
                Err(error())
            } else {
                Ok(())
            }
        });
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }
    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
    pub(crate) fn stop_and_join(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread
            .take()
            .map_or(Ok(()), |h| h.join().map_err(|_| error())?)
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}
fn error() -> String {
    "The owned native API gateway could not complete this request.".into()
}
fn body_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn classify_worker(
    result: Result<(), String>,
    accepted: bool,
    fatal: &AtomicBool,
) -> Result<(), String> {
    if accepted && result.is_err() {
        fatal.store(true, Ordering::SeqCst);
        result
    } else {
        // Local auth/framing rejection never touched provider intent and cannot
        // poison another authenticated native request.
        Ok(())
    }
}
fn halted(stop: &AtomicBool, cancelled: &AtomicBool) -> bool {
    stop.load(Ordering::SeqCst) || cancelled.load(Ordering::SeqCst)
}
fn may_failover(status: u16, downstream_started: bool, account_bound: bool) -> bool {
    status == 429 && !downstream_started && !account_bound
}
fn constant_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for i in 0..left.len().max(right.len()) {
        difference |=
            (left.get(i).copied().unwrap_or(0) ^ right.get(i).copied().unwrap_or(0)) as usize;
    }
    difference == 0
}
struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Zeroizing<Vec<u8>>,
}
fn headers(bytes: &[u8], host: &str) -> Result<Request, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| error())?;
    if !text.ends_with("\r\n\r\n")
        || text
            .bytes()
            .any(|b| b == 0 || (b < 32 && !b"\r\n".contains(&b)))
    {
        return Err(error());
    }
    let mut lines = text
        .strip_suffix("\r\n\r\n")
        .ok_or_else(error)?
        .split("\r\n");
    let first: Vec<_> = lines.next().ok_or_else(error)?.split(' ').collect();
    if first.len() != 3
        || first[2] != "HTTP/1.1"
        || !matches!(first[0], "GET" | "POST")
        || !first[1].starts_with('/')
        || first[1].contains(['#', '\\'])
    {
        return Err(error());
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        let (key, value) = line.split_once(':').ok_or_else(error)?;
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.bytes().any(|b| b < 32 || b == 127)
        {
            return Err(error());
        }
        let key = key.to_ascii_lowercase();
        if fields.insert(key, value.trim().to_string()).is_some() {
            return Err(error());
        }
    }
    if fields.get("host").map(String::as_str) != Some(host)
        || fields.contains_key("transfer-encoding")
        || fields.contains_key("expect")
        || fields.contains_key("origin")
        || fields
            .keys()
            .any(|k| k.starts_with("sec-fetch-") || k.starts_with("access-control-"))
        || fields.contains_key("proxy-authorization")
        || fields.contains_key("cookie")
        || fields
            .get("content-encoding")
            .is_some_and(|v| v != "identity")
        || fields.get("user-agent").is_some_and(|v| {
            v.contains("Mozilla/") || v.contains("AppleWebKit/") || v.contains("Electron/")
        })
    {
        return Err(error());
    }
    Ok(Request {
        method: first[0].into(),
        path: first[1].into(),
        headers: fields,
        body: Zeroizing::new(Vec::new()),
    })
}
fn authenticate(request: &Request, protocol: Protocol, token: &str) -> bool {
    let auth = request
        .headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "));
    let api = request.headers.get("x-api-key").map(String::as_str);
    let goog = request.headers.get("x-goog-api-key").map(String::as_str);
    // A conflicting authentication field cannot be silently ignored.
    let supplied: Vec<_> = [auth, api, goog].into_iter().flatten().collect();
    if supplied.len() != 1 || request.headers.contains_key("authorization") && auth.is_none() {
        return false;
    }
    let allowed = match protocol {
        Protocol::Anthropic => api.or(auth),
        Protocol::OpenAiChat | Protocol::OpenAiResponses => auth,
        Protocol::Gemini => goog.or(auth),
    };
    allowed.is_some_and(|v| constant_equal(v.as_bytes(), token.as_bytes()))
}
fn read_request(
    stream: &mut TcpStream,
    host: &str,
    stop: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<Request, String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| error())?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| error())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut raw = Zeroizing::new(Vec::new());
    let split = loop {
        if halted(stop, cancel) || Instant::now() >= deadline {
            return Err(error());
        }
        if raw.len() >= MAX_HEADERS {
            return Err(error());
        }
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(1) => {
                raw.push(byte[0]);
                if raw.ends_with(b"\r\n\r\n") {
                    break raw.len();
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            _ => return Err(error()),
        }
    };
    let mut request = headers(&raw[..split], host)?;
    let length = match request.headers.get("content-length") {
        Some(v) if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => {
            v.parse::<usize>().map_err(|_| error())?
        }
        None if request.method == "GET" => 0,
        _ => return Err(error()),
    };
    if length > MAX_REQUEST || request.method == "GET" && length != 0 {
        return Err(error());
    }
    request.body.resize(length, 0);
    let mut offset = 0;
    while offset < length {
        if halted(stop, cancel) || Instant::now() >= deadline {
            return Err(error());
        }
        match stream.read(&mut request.body[offset..]) {
            Ok(0) => return Err(error()),
            Ok(n) => offset += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => return Err(error()),
        }
    }
    Ok(request)
}
fn account_bound(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "previous_response_id"
                    | "conversation"
                    | "encrypted_content"
                    | "cachedContent"
                    | "thoughtSignature"
                    | "signature"
            ) || key == "type"
                && matches!(value.as_str(), Some("item_reference" | "redacted_thinking"))
                || account_bound(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(account_bound),
        _ => false,
    }
}
struct UniqueJson(serde_json::Value);
impl<'de> serde::Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueJson(n.into()))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(serde_json::Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueJson(values.into()))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                    let UniqueJson(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueJson(values.into()))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
fn route(request: &Request, protocol: Protocol, model: &str) -> Result<bool, String> {
    if request.method == "GET" && discovery(&request.path, protocol, model).is_some() {
        return Ok(false);
    }
    if request.method != "POST"
        || request.headers.get("content-type").is_none_or(|s| {
            !matches!(
                s.as_str(),
                "application/json" | "application/json; charset=utf-8"
            )
        })
    {
        return Err(error());
    }
    let UniqueJson(value) = serde_json::from_slice(&request.body).map_err(|_| error())?;
    if !value.is_object() {
        return Err(error());
    }
    let valid = match protocol {
        Protocol::Anthropic => {
            matches!(
                request.path.as_str(),
                "/v1/messages" | "/v1/messages/count_tokens"
            ) && value["model"] == model
        }
        Protocol::OpenAiChat => request.path == "/v1/chat/completions" && value["model"] == model,
        Protocol::OpenAiResponses => request.path == "/v1/responses" && value["model"] == model,
        Protocol::Gemini => {
            ["/v1/", "/v1beta/"].iter().any(|prefix| {
                let path = format!("{prefix}models/{model}");
                request.path == format!("{path}:generateContent")
                    || request.path == format!("{path}:streamGenerateContent?alt=sse")
                    || request.path == format!("{path}:countTokens")
            }) && value
                .get("model")
                .is_none_or(|v| v == model || v == &format!("models/{model}"))
                && value.get("generateContentRequest").is_none_or(|nested| {
                    nested.is_object()
                        && nested
                            .get("model")
                            .is_none_or(|v| v == model || v == &format!("models/{model}"))
                })
        }
    };
    if valid {
        Ok(account_bound(&value))
    } else {
        Err(error())
    }
}
fn discovery(path: &str, protocol: Protocol, model: &str) -> Option<serde_json::Value> {
    if protocol != Protocol::Gemini {
        return (path == "/v1/models").then(|| {
            serde_json::json!({
                "object":"list", "data":[{"id":model,"object":"model","owned_by":"lomi-gateway"}]
            })
        });
    }
    let entry = serde_json::json!({"name":format!("models/{model}"), "displayName":model,
        "supportedGenerationMethods":["generateContent","countTokens"]});
    for prefix in ["/v1", "/v1beta"] {
        if path == format!("{prefix}/models") {
            return Some(serde_json::json!({"models":[entry]}));
        }
        if path == format!("{prefix}/models/{model}") {
            return Some(entry);
        }
    }
    None
}
fn endpoint(upstream: &Upstream, protocol: Protocol, path: &str) -> Result<Url, String> {
    super::gateway_url::join(&upstream.base_url, protocol, path)
}
fn reject(stream: &mut TcpStream, status: u16) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status} Gateway Rejection\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n")
}
fn write_bounded(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    stop: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !bytes.is_empty() {
        if halted(stop, cancel) || Instant::now() >= deadline {
            return Err(error());
        }
        match stream.write(bytes) {
            Ok(0) => return Err(error()),
            Ok(n) => bytes = &bytes[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => return Err(error()),
        }
    }
    Ok(())
}
async fn cancellable<T>(
    future: impl std::future::Future<Output = T>,
    stop: &AtomicBool,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        if halted(stop, cancel) || Instant::now() >= deadline {
            return Err(error());
        }
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep(Duration::from_millis(25)) => {},
        }
    }
}
struct RequestContext<'a> {
    address: SocketAddr,
    protocol: Protocol,
    token: &'a str,
    model: &'a str,
    client: &'a Client,
    owner: &'a dyn UpstreamOwner,
    stop: &'a AtomicBool,
    cancel: &'a AtomicBool,
    accepted: &'a AtomicBool,
    fatal: &'a AtomicBool,
    requests: &'a Mutex<()>,
}

fn serve(mut stream: TcpStream, context: RequestContext<'_>) -> Result<(), String> {
    let RequestContext {
        address,
        protocol,
        token,
        model,
        client,
        owner,
        stop,
        cancel,
        accepted,
        fatal,
        requests,
    } = context;
    let request = match read_request(&mut stream, &address.to_string(), stop, cancel) {
        Ok(v) => v,
        Err(e) => {
            let _ = reject(&mut stream, 400);
            return Err(e);
        }
    };
    if !authenticate(&request, protocol, token) {
        let _ = reject(&mut stream, 401);
        return Err(error());
    }
    let account_bound = match route(&request, protocol, model) {
        Ok(v) => v,
        Err(e) => {
            let _ = reject(&mut stream, 400);
            return Err(e);
        }
    };
    if request.method == "GET" {
        let bytes =
            serde_json::to_vec(&discovery(&request.path, protocol, model).ok_or_else(error)?)
                .map_err(|_| error())?;
        let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", bytes.len());
        write_bounded(&mut stream, header.as_bytes(), stop, cancel)?;
        return write_bounded(&mut stream, &bytes, stop, cancel);
    }
    // Native clients can issue title/compaction requests alongside their main
    // stream. Serialize accepted inference requests so account affinity cannot
    // move while another account is still creating opaque native context.
    let wait_deadline = Instant::now() + Duration::from_secs(300);
    let _request_owner = loop {
        if halted(stop, cancel) || fatal.load(Ordering::SeqCst) || Instant::now() >= wait_deadline {
            return Err(error());
        }
        match requests.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(10)),
            Err(_) => return Err(error()),
        }
    };
    let mut summary = RequestSummary {
        wire: None,
        id: super::new_id()?,
        upstream_attempt_id: String::new(),
        attempt_index: 0,
        protocol,
        path: request.path.clone(),
        model: model.into(),
        body_bytes: request.body.len(),
        body_sha256: body_digest(&request.body),
        account_bound,
    };
    let mut excluded = Vec::new();
    let mut receipt = Receipt {
        request_id: summary.id.clone(),
        upstream_attempt_id: String::new(),
        attempt_index: 0,
        profile_id: String::new(),
        status: None,
        downstream_started: false,
        complete: false,
        cancelled: false,
    };
    let mut active_receipt = false;
    let deadline = Instant::now() + Duration::from_secs(300);
    let result = (|| -> Result<(), String> {
        for attempt in 0..MAX_ATTEMPTS {
            if halted(stop, cancel) || fatal.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return Err(error());
            }
            summary.upstream_attempt_id = super::new_id()?;
            summary.attempt_index = attempt;
            let upstream = owner.select(&summary, &excluded)?;
            if upstream.profile_id.is_empty()
                || excluded.contains(&upstream.profile_id)
                || upstream.api_key.is_empty()
            {
                return Err(error());
            }
            let converted = upstream.protocol != protocol;
            // Conversion must finish before durable dispatch ownership or HTTP send.
            // In particular, token counts and opaque provider state cannot be guessed.
            let prepared = if converted {
                Some(super::gateway_transform::request(
                    protocol,
                    upstream.protocol,
                    &request.path,
                    &request.body,
                    model,
                )?)
            } else {
                None
            };
            let wire_path = prepared
                .as_ref()
                .map_or(request.path.as_str(), |value| value.path.as_str());
            let wire_body = prepared
                .as_ref()
                .map_or(request.body.as_slice(), |value| value.body.as_slice());
            summary.wire = prepared.as_ref().map(|value| WireSummary {
                upstream_protocol: upstream.protocol,
                path: value.path.clone(),
                body_bytes: value.body.len(),
                body_sha256: body_digest(&value.body),
            });
            let url = endpoint(&upstream, upstream.protocol, wire_path)?;
            receipt.profile_id = upstream.profile_id.clone();
            receipt.upstream_attempt_id = summary.upstream_attempt_id.clone();
            receipt.attempt_index = attempt;
            receipt.status = None;
            let mut builder = client
                .post(url)
                .header("content-type", "application/json")
                .header("connection", "close");
            match upstream.protocol {
                Protocol::Anthropic => {
                    builder = builder.header("x-api-key", upstream.api_key.as_str());
                    let version = request
                        .headers
                        .get("anthropic-version")
                        .map(String::as_str)
                        .unwrap_or("2023-06-01");
                    if version.len() != 10
                        || !version.bytes().all(|b| b.is_ascii_digit() || b == b'-')
                    {
                        return Err(error());
                    }
                    builder = builder.header("anthropic-version", version);
                    if let Some(beta) = request.headers.get("anthropic-beta").filter(|_| !converted)
                    {
                        if beta.len() > 2048
                            || !beta
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"-,_.".contains(&b))
                        {
                            return Err(error());
                        }
                        builder = builder.header("anthropic-beta", beta);
                    }
                }
                Protocol::OpenAiChat | Protocol::OpenAiResponses => {
                    builder = builder.bearer_auth(upstream.api_key.as_str());
                    if let Some(beta) = request.headers.get("openai-beta").filter(|_| !converted) {
                        if beta.len() > 2048
                            || !beta
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"-=,_. ".contains(&b))
                        {
                            return Err(error());
                        }
                        builder = builder.header("openai-beta", beta);
                    }
                }
                Protocol::Gemini => {
                    builder = builder.header("x-goog-api-key", upstream.api_key.as_str());
                }
            }
            let builder = builder.body(wire_body.to_vec());
            active_receipt = true;
            accepted.store(true, Ordering::SeqCst);
            if fatal.load(Ordering::SeqCst) {
                return Err(error());
            }
            owner.fence_and_checkpoint(&summary, &upstream)?;
            if halted(stop, cancel) || fatal.load(Ordering::SeqCst) {
                return Err(error());
            }
            let mut response = tauri::async_runtime::block_on(cancellable(
                builder.send(),
                stop,
                cancel,
                deadline,
            ))?
            .map_err(|_| error())?;
            receipt.status = Some(response.status().as_u16());
            if response.status().as_u16() == 429 {
                let retry = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .filter(|v| v.len() <= 128);
                owner.rate_limited(&summary, &upstream, retry)?;
            }
            if may_failover(
                response.status().as_u16(),
                receipt.downstream_started,
                !owner.can_failover(&summary, &upstream),
            ) {
                owner.finish(&Receipt {
                    request_id: summary.id.clone(),
                    upstream_attempt_id: summary.upstream_attempt_id.clone(),
                    attempt_index: attempt,
                    profile_id: upstream.profile_id.clone(),
                    status: Some(429),
                    downstream_started: false,
                    complete: true,
                    cancelled: false,
                })?;
                active_receipt = false;
                excluded.push(upstream.profile_id.clone());
                continue;
            }
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream");
            if content_type.len() > 256
                || !content_type.bytes().all(|b| (32..127).contains(&b))
                || response
                    .content_length()
                    .is_some_and(|n| n > MAX_RESPONSE as u64)
            {
                return Err(error());
            }
            if let Some(prepared) = &prepared {
                // Never expose an upstream-family error envelope as native JSON/SSE.
                if !response.status().is_success()
                    || response.headers().get("content-encoding").is_some()
                    || (prepared.stream && !content_type.starts_with("text/event-stream"))
                    || (!prepared.stream && !content_type.starts_with("application/json"))
                {
                    return Err(error());
                }
                let mut buffered = Vec::new();
                loop {
                    let chunk = tauri::async_runtime::block_on(cancellable(
                        response.chunk(),
                        stop,
                        cancel,
                        deadline,
                    ))?
                    .map_err(|_| error())?;
                    let Some(chunk) = chunk else {
                        break;
                    };
                    if buffered.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                        return Err(error());
                    }
                    buffered.extend_from_slice(&chunk);
                }
                let encoded = if prepared.stream {
                    super::gateway_transform::stream_response(
                        protocol,
                        upstream.protocol,
                        &buffered,
                        model,
                    )?
                } else {
                    super::gateway_transform::response(
                        protocol,
                        upstream.protocol,
                        &buffered,
                        model,
                    )?
                };
                if encoded.len() > MAX_RESPONSE
                    || halted(stop, cancel)
                    || Instant::now() >= deadline
                {
                    return Err(error());
                }
                let native_type = if prepared.stream {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let header = format!("HTTP/1.1 {} Upstream\r\nContent-Type: {native_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", response.status().as_u16(), encoded.len());
                receipt.downstream_started = true;
                write_bounded(&mut stream, header.as_bytes(), stop, cancel)?;
                write_bounded(&mut stream, &encoded, stop, cancel)?;
                receipt.complete = true;
                return Ok(());
            }
            let mut header = format!("HTTP/1.1 {} Upstream\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nCache-Control: no-store\r\n", response.status().as_u16());
            // Native request IDs are receipts, never client-controlled secrets.
            for key in [
                "request-id",
                "x-request-id",
                "retry-after",
                "content-encoding",
                "x-should-retry",
            ] {
                if let Some(value) = response
                    .headers()
                    .get(key)
                    .and_then(|v| v.to_str().ok())
                    .filter(|v| v.len() <= 256 && v.bytes().all(|b| (32..127).contains(&b)))
                {
                    header.push_str(&format!("{key}: {value}\r\n"));
                }
            }
            header.push_str("\r\n");
            receipt.downstream_started = true;
            write_bounded(&mut stream, header.as_bytes(), stop, cancel)?;
            let mut total = 0usize;
            loop {
                let chunk = tauri::async_runtime::block_on(cancellable(
                    response.chunk(),
                    stop,
                    cancel,
                    deadline,
                ))?
                .map_err(|_| error())?;
                let Some(chunk) = chunk else {
                    break;
                };
                total = total.saturating_add(chunk.len());
                if total > MAX_RESPONSE {
                    return Err(error());
                }
                if chunk.is_empty() {
                    continue;
                }
                write_bounded(
                    &mut stream,
                    format!("{:x}\r\n", chunk.len()).as_bytes(),
                    stop,
                    cancel,
                )?;
                write_bounded(&mut stream, &chunk, stop, cancel)?;
                write_bounded(&mut stream, b"\r\n", stop, cancel)?;
            }
            write_bounded(&mut stream, b"0\r\n\r\n", stop, cancel)?;
            receipt.complete = true;
            return Ok(());
        }
        Err(error())
    })();
    receipt.cancelled = halted(stop, cancel);
    let recorded = if active_receipt {
        owner.finish(&receipt)
    } else {
        Ok(())
    };
    if result.is_err() && !receipt.downstream_started {
        let _ = reject(&mut stream, if receipt.cancelled { 503 } else { 502 });
    }
    result.and(recorded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gemini_discovery_and_token_count_keep_the_native_schema_and_owned_model() {
        let host = "127.0.0.1:12345";
        let list = discovery("/v1beta/models", Protocol::Gemini, "owned").unwrap();
        assert_eq!(list["models"][0]["name"], "models/owned");
        assert!(list.get("data").is_none());
        assert!(discovery("/v1beta/models/other", Protocol::Gemini, "owned").is_none());
        let mut request = headers(format!("POST /v1beta/models/owned:countTokens HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: 0\r\n\r\n").as_bytes(),host).unwrap();
        request.body.extend_from_slice(
            br#"{"generateContentRequest":{"model":"models/owned","contents":[]}}"#,
        );
        assert!(route(&request, Protocol::Gemini, "owned").is_ok());
        assert_eq!(
            endpoint(
                &Upstream {
                    protocol: Protocol::Gemini,
                    profile_id: "p".into(),
                    revision: 1,
                    base_url: "https://generativelanguage.googleapis.com".into(),
                    api_key: Zeroizing::new("key".into())
                },
                Protocol::Gemini,
                &request.path
            )
            .unwrap()
            .as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/owned:countTokens"
        );
        request.body.clear();
        request
            .body
            .extend_from_slice(br#"{"generateContentRequest":{"model":"models/other"}}"#);
        assert!(route(&request, Protocol::Gemini, "owned").is_err());
    }
    #[test]
    fn accepted_journal_failure_is_fatal_but_local_rejection_is_not() {
        let fatal = AtomicBool::new(false);
        assert!(classify_worker(Err(error()), false, &fatal).is_ok());
        assert!(!fatal.load(Ordering::SeqCst));
        assert!(classify_worker(Ok(()), true, &fatal).is_ok());
        assert!(!fatal.load(Ordering::SeqCst));
        assert!(classify_worker(Err(error()), true, &fatal).is_err());
        assert!(fatal.load(Ordering::SeqCst));
    }
    #[test]
    fn only_authoritative_429_before_downstream_bytes_can_fail_over() {
        assert!(may_failover(429, false, false));
        assert!(!may_failover(429, true, false));
        assert!(!may_failover(429, false, true));
        assert!(!may_failover(500, false, false));
        assert!(!may_failover(401, false, false));
        assert!(!may_failover(200, true, false));
    }
    #[test]
    fn opaque_native_continuity_is_account_bound_and_is_not_transformed() {
        for body in [
            serde_json::json!({"previous_response_id":"resp_owned"}),
            serde_json::json!({"conversation":{"id":"conv_owned"}}),
            serde_json::json!({"input":[{"type":"item_reference","id":"item_owned"}]}),
            serde_json::json!({"input":[{"encrypted_content":"opaque"}]}),
            serde_json::json!({"messages":[{"content":[{"type":"thinking","thinking":"native","signature":"opaque"}]}]}),
            serde_json::json!({"messages":[{"content":[{"type":"redacted_thinking","data":"opaque"}]}]}),
            serde_json::json!({"cachedContent":"projects/owned/cache"}),
            serde_json::json!({"contents":[{"parts":[{"thoughtSignature":"opaque"}]}]}),
        ] {
            assert!(account_bound(&body));
        }
        assert!(!account_bound(
            &serde_json::json!({"model":"owned","messages":[{"role":"user","content":"plain stateless text"}]})
        ));
    }
    #[test]
    fn intent_digest_binds_exact_original_bytes_including_formatting() {
        let first = br#"{"model":"owned"}"#;
        let different = br#"{"model":"other"}"#;
        assert_eq!(first.len(), different.len());
        assert_ne!(body_digest(first), body_digest(different));
        assert_ne!(body_digest(first), body_digest(br#"{ "model": "owned" }"#));
        assert_eq!(
            body_digest(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn duplicate_json_keys_cannot_bypass_model_or_continuity_admission() {
        assert!(
            serde_json::from_slice::<UniqueJson>(br#"{"model":"other","model":"owned"}"#).is_err()
        );
        assert!(serde_json::from_slice::<UniqueJson>(
            br#"{"input":[{"type":"item_reference","type":"text"}]}"#
        )
        .is_err());
        assert!(serde_json::from_slice::<UniqueJson>(
            br#"{"model":"owned","input":[{"type":"text","text":"native"}]}"#
        )
        .is_ok());
    }
    #[test]
    fn smuggling_browser_and_conflicting_authentication_are_rejected() {
        let host = "127.0.0.1:12345";
        for extra in [
            "Content-Length: 0\r\nContent-Length: 0\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Origin: https://example.com\r\n",
            "Sec-Fetch-Site: same-origin\r\n",
        ] {
            assert!(headers(
                format!("POST /v1/messages HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n").as_bytes(),
                host
            )
            .is_err());
        }
        let token = "a".repeat(32);
        let request = headers(format!("POST /v1/messages HTTP/1.1\r\nHost: {host}\r\nx-api-key: {token}\r\nAuthorization: Bearer other\r\nContent-Length: 0\r\n\r\n").as_bytes(), host).unwrap();
        assert!(!authenticate(&request, Protocol::Anthropic, &token));
    }
    #[test]
    fn native_model_and_query_key_are_owned() {
        let host = "127.0.0.1:12345";
        let mut request = headers(format!("POST /v1/messages HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: 0\r\n\r\n").as_bytes(), host).unwrap();
        request
            .body
            .extend_from_slice(br#"{"model":"owned","tools":[{"name":"native"}]}"#);
        assert!(route(&request, Protocol::Anthropic, "owned").is_ok());
        assert!(route(&request, Protocol::Anthropic, "other").is_err());
        request.path.push_str("?api_key=secret");
        assert!(route(&request, Protocol::Anthropic, "owned").is_err());
    }
}
