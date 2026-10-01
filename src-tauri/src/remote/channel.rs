use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use lomi_remote_crypto::{Channel, ChannelContext, ChannelPurpose, Handshake};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const MAX_CHANNEL_ATTEMPTS: usize = 256;

impl Core {
    fn channel_seen(&mut self, id: &str, timestamp: u64) -> bool {
        self.attempted_channels
            .retain(|_, expiry| *expiry > timestamp);
        self.attempted_channels.contains_key(id) || self.channels.contains_key(id)
    }

    fn reserve_channel(
        &mut self,
        id: &str,
        expires_at: u64,
        timestamp: u64,
    ) -> Result<bool, String> {
        if self.channel_seen(id, timestamp) {
            return Ok(false);
        }
        if expires_at <= timestamp || expires_at > timestamp.saturating_add(120) {
            return Err("Remote channel expiry is invalid.".into());
        }
        if self.attempted_channels.len() >= MAX_CHANNEL_ATTEMPTS {
            return Err("Remote channel attempt budget exhausted.".into());
        }
        // Reserve before requesting a ticket. Completion, failed setup and poll
        // omission must never permit a second attempt with the same channel ID.
        self.attempted_channels.insert(id.to_owned(), expires_at);
        Ok(true)
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    id: String,
    session_id: String,
    host_id: String,
    device_id: String,
    grant_id: String,
    context: ChannelContext,
    host_bundle: SignedBundle,
    device_bundle: SignedBundle,
    signed_approval: SignedPeerApproval,
    #[serde(deserialize_with = "super::deserialize_expiry")]
    expires_at: u64,
}

pub(super) async fn start(
    remote: Remote,
    app: tauri::AppHandle,
    value: Value,
) -> Result<(), String> {
    let wire: Wire =
        serde_json::from_value(value.clone()).map_err(|_| "Invalid Remote channel.")?;
    let binding = app.state::<AuthController>().remote_binding()?;
    let (handshake, stop, epoch, permissions) = {
        let mut c = remote.core.lock().map_err(|_| "Remote unavailable.")?;
        if c.channel_seen(&wire.id, now()) {
            return Ok(());
        }
        if !c.enabled
            || c.binding.as_ref() != Some(&binding)
            || !c.deadline.is_some_and(|d| d > Instant::now())
        {
            return Err("Remote authorization expired.".into());
        }
        let p = c.policy.as_ref().ok_or("Remote identity unavailable.")?;
        let local = p
            .grants
            .iter()
            .find(|g| g.id == wire.grant_id && g.confirmed && !g.revoked)
            .ok_or("Channel has no local permission.")?;
        let a = &local.approval.approval;
        if wire.expires_at <= now()
            || wire.expires_at > now() + 120
            || wire.host_bundle != p.bundle
            || wire.device_bundle != local.device_bundle
            || wire.signed_approval != local.approval
            || wire.context.channel_id != uuid_bytes(&wire.id)?
            || wire.context.session_id != uuid_bytes(&wire.session_id)?
            || wire.context.grant_id != uuid_bytes(&wire.grant_id)?
            || wire.context.host_id != uuid_bytes(&wire.host_id)?
            || wire.context.device_id != uuid_bytes(&wire.device_id)?
            || wire.host_id != p.host_id
            || wire.context.account_id != a.account_id
            || wire.context.host_id != a.host_id
            || wire.context.device_id != a.device_id
            || wire.context.access_epoch != a.access_epoch
            || wire.context.revision != a.revision
            || (a.version == 1
                && (!a.session_ids.contains(&wire.context.session_id)
                    || !c.shares.contains(&wire.session_id)))
        {
            return Err("Remote channel does not match local permission.".into());
        }
        local.approval.verify(
            &p.bundle,
            &a.host_fingerprint,
            a,
            now(),
            wire.context.access_epoch,
        )?;
        wire.context.validate_approval(a)?;
        let metadata = wire.context.purpose == Some(ChannelPurpose::Metadata);
        if a.version == 2 && !workspace_allowed(&c, &wire) {
            return Err("Workspace permission changed.".into());
        }
        let epoch = if metadata {
            uuid_text(
                &wire
                    .context
                    .workspace_epoch
                    .ok_or("Missing workspace epoch.")?,
            )
        } else {
            remote
                .runtime
                .lock()
                .map_err(|_| "Terminal model unavailable.")?
                .sessions
                .get(&wire.session_id)
                .ok_or("Terminal closed.")?
                .epoch
                .clone()
        };
        if a.version == 2 && wire.context.session_epoch != Some(uuid_bytes(&epoch)?) {
            return Err("Terminal epoch changed.".into());
        }
        let h = Handshake::new(
            false,
            c.identity.as_ref().ok_or("Remote identity unavailable.")?,
            &wire.context,
            &local.device_bundle,
            &a.device_fingerprint,
        )?;
        let permissions = a.permissions;
        if !c.reserve_channel(&wire.id, wire.expires_at, now())? {
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        c.channels.insert(wire.id.clone(), stop.clone());
        c.channel_grants
            .insert(wire.id.clone(), wire.grant_id.clone());
        if let Some(id) = wire.context.workspace_id {
            c.channel_workspaces.insert(wire.id.clone(), uuid_text(&id));
        }
        (h, stop, epoch, permissions)
    };
    let (_, ticket) = app
        .state::<AuthController>()
        .remote_request(
            reqwest::Method::POST,
            &format!("/v1/remote/native/channels/{}/ticket", wire.id),
            Some(&json!({})),
        )
        .await?;
    if ticket.get("channel") != Some(&value) {
        stop.store(true, Ordering::SeqCst);
        return Err("Remote ticket context changed.".into());
    }
    let raw = ticket
        .get("ticket")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .ok_or("Invalid Remote ticket.")?
        .to_owned();
    let url = ticket
        .get("relayUrl")
        .and_then(Value::as_str)
        .ok_or("Invalid Remote relay.")?
        .to_owned();
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Invalid Remote relay.")?;
    let origin = parsed.origin().ascii_serialization();
    let expected_relay = match app.state::<AuthController>().remote_environment()?.as_str() {
        "auth-lomi-dev" => "wss://relay.lomi.dev",
        "auth-staging-lomi-dev" => "wss://relay-staging.lomi.dev",
        "development" if cfg!(debug_assertions) => "ws://127.0.0.1:3004",
        _ => return Err("Remote relay environment is unavailable.".into()),
    };
    if origin != expected_relay
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/v1/relay"
    {
        return Err("Untrusted Remote relay rejected.".into());
    }
    tauri::async_runtime::spawn(async move {
        let _ = run(
            &remote,
            &app,
            &wire,
            &binding,
            handshake,
            &stop,
            &epoch,
            permissions,
            &url,
            raw,
        )
        .await;
        stop.store(true, Ordering::SeqCst);
        if let Ok(mut c) = remote.core.lock() {
            if c.channels
                .get(&wire.id)
                .is_none_or(|current| Arc::ptr_eq(current, &stop))
            {
                app.state::<Terminals>()
                    .remote_revoke_owner(&wire.session_id, &wire.id);
                c.channels.remove(&wire.id);
                c.channel_workspaces.remove(&wire.id);
                c.channel_grants.remove(&wire.id);
            }
        }
    });
    Ok(())
}

fn workspace_allowed(c: &Core, wire: &Wire) -> bool {
    let a = &wire.signed_approval.approval;
    let Some(p) = c.policy.as_ref() else {
        return false;
    };
    if a.account_id != p.bundle.bundle.account_id || uuid_bytes(&p.host_id).ok() != Some(a.host_id)
    {
        return false;
    }
    let Some(w) = p.workspaces.iter().find(|w| {
        w.shared
            && uuid_bytes(&w.id).ok() == a.workspace_id
            && uuid_bytes(&w.epoch).ok() == a.workspace_epoch
    }) else {
        return false;
    };
    if c.domain.revision == 0
        || c.domain.workspaces.iter().find(|d| d.id == w.id) != w.projection.as_ref()
        || w.revision != a.revision
        || a.session_ids.len() != w.sessions.len()
    {
        return false;
    }
    if !w
        .sessions
        .iter()
        .zip(a.session_ids.iter().zip(&a.session_epochs))
        .all(|(s, (id, epoch))| {
            uuid_bytes(&s.0).ok() == Some(*id) && uuid_bytes(&s.1).ok() == Some(*epoch)
        })
    {
        return false;
    }
    wire.context.validate_approval(a).is_ok()
}

fn allowed(
    remote: &Remote,
    app: &tauri::AppHandle,
    wire: &Wire,
    binding: &crate::auth::controller::RemoteBinding,
    epoch: &str,
    stop: &AtomicBool,
) -> bool {
    if stop.load(Ordering::SeqCst)
        || !remote.healthy.load(Ordering::SeqCst)
        || wire.expires_at <= now()
        || app.state::<AuthController>().remote_binding().as_ref() != Ok(binding)
    {
        return false;
    }
    let Ok(c) = remote.core.lock() else {
        return false;
    };
    c.enabled
        && c.binding.as_ref() == Some(binding)
        && c.deadline.is_some_and(|d| d > Instant::now())
        && (if wire.context.version == 2 {
            workspace_allowed(&c, wire)
        } else {
            c.shares.contains(&wire.session_id)
        })
        && c.policy.as_ref().is_some_and(|p| {
            p.grants.iter().any(|g| {
                g.id == wire.grant_id
                    && g.confirmed
                    && !g.revoked
                    && g.approval == wire.signed_approval
                    && g.approval.approval.expires_at > now()
            })
        })
        && (wire.context.purpose == Some(ChannelPurpose::Metadata)
            || remote
                .runtime
                .lock()
                .map(|r| {
                    r.sessions
                        .get(&wire.session_id)
                        .is_some_and(|s| s.epoch == epoch && s.available)
                })
                .unwrap_or(false))
}

#[allow(clippy::too_many_arguments)]
async fn run(
    remote: &Remote,
    app: &tauri::AppHandle,
    wire: &Wire,
    binding: &crate::auth::controller::RemoteBinding,
    mut handshake: Handshake,
    stop: &Arc<AtomicBool>,
    epoch: &str,
    permissions: Permissions,
    url: &str,
    ticket: String,
) -> Result<(), String> {
    let permit = || allowed(remote, app, wire, binding, epoch, stop);
    let mut assembler = Fragments::default();
    let (mut ws, _) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async_with_config(
            url,
            Some(
                tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                    .max_message_size(Some(17 * 1024))
                    .max_frame_size(Some(17 * 1024))
                    .write_buffer_size(0)
                    .max_write_buffer_size(64 * 1024),
            ),
            false,
        ),
    )
    .await
    .map_err(|_| "Remote connection deadline.")?
    .map_err(|_| "Remote connection failed.")?;
    ws.send(Message::Text(
        json!({"type":"authenticate","ticket":ticket})
            .to_string()
            .into(),
    ))
    .await
    .map_err(|_| "Remote authentication failed.")?;
    // Relay readiness carries no authority; the authenticated Noise transcript does.
    loop {
        let msg = next(&mut ws).await?;
        if let Message::Text(text) = msg {
            let v: Value = serde_json::from_str(&text).map_err(|_| "Invalid relay response.")?;
            if v.get("type").and_then(Value::as_str) == Some("ready") {
                break;
            }
            if v.get("type").and_then(Value::as_str) != Some("authenticated") {
                return Err("Relay authentication rejected.".into());
            }
        } else {
            return Err("Relay readiness missing.".into());
        }
    }
    if !allowed(remote, app, wire, binding, epoch, stop) {
        return Err("Remote authorization expired.".into());
    }
    let incoming = binary(next(&mut ws).await?)?;
    if !permit() {
        return Err("Remote authorization expired.".into());
    }
    handshake.read(&incoming)?;
    ws.send(Message::Binary(handshake.write()?.into()))
        .await
        .map_err(|_| "Noise handshake failed.")?;
    let incoming = binary(next(&mut ws).await?)?;
    if !permit() {
        return Err("Remote authorization expired.".into());
    }
    handshake.read(&incoming)?;
    if !permit() {
        return Err("Remote authorization expired.".into());
    }
    let mut crypto = handshake.finish()?;
    let route = wire.context.channel_id;
    if let Some(id) = wire.context.workspace_id {
        let metadata = remote.workspace_metadata(&uuid_text(&id))?;
        send(&permit, &mut ws, &mut crypto, route, metadata).await?;
        if wire.context.purpose == Some(ChannelPurpose::Metadata) {
            ws.close(None)
                .await
                .map_err(|_| "Remote metadata close failed.")?;
            return Ok(());
        }
    }
    // Drop handshake-era observer backlog before the atomic current snapshot.
    let mut events = remote.events.subscribe();
    let snapshot = remote
        .runtime
        .lock()
        .map_err(|_| "Terminal unavailable.")?
        .snapshot(&wire.session_id)?;
    let mut seq = snapshot
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or("Snapshot sequence unavailable.")?;
    send(&permit, &mut ws,&mut crypto,route,json!({"v":1,"type":"hello","sessionId":wire.session_id,"epoch":epoch,"permissions":permissions,"cols":snapshot["cols"],"rows":snapshot["rows"]})).await?;
    send(&permit, &mut ws, &mut crypto, route, snapshot).await?;
    let mut lease: Option<String> = None;
    let mut seen = HashSet::<String>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        if !allowed(remote, app, wire, binding, epoch, stop) {
            return Err("Remote authorization expired.".into());
        }
        tokio::select! {
            _=tick.tick()=> {
                assembler.check_deadline()?;
                if let Some(id)=lease.as_ref() {
                    if !app.state::<Terminals>().remote_lease_live(&wire.session_id,&wire.id,id) { lease=None; send(&permit, &mut ws,&mut crypto,route,json!({"v":1,"type":"lease","leaseId":null})).await?; }
                }
            },
            delta=events.recv()=> {
                let mut delta=delta.map_err(|_| "Remote observer overflow.")?;
                if delta.get("sessionId").and_then(Value::as_str)!=Some(wire.session_id.as_str()) { continue; }
                let next=delta.get("seq").and_then(Value::as_u64).ok_or("Remote sequence invalid.")?;
                if next<=seq { continue; }
                if next!=seq+1 { return Err("Remote sequence gap.".into()); }
                delta.as_object_mut().ok_or("Remote message invalid.")?.remove("sessionId");
                send(&permit, &mut ws,&mut crypto,route,delta).await?; seq=next;
            },
            message=ws.next()=> {
                let message=message.ok_or("Remote disconnected.")?.map_err(|_| "Remote disconnected.")?;
                match message { Message::Ping(_)=> { tokio::time::timeout(Duration::from_secs(2),ws.flush()).await.map_err(|_| "Remote pong deadline.")?.map_err(|_| "Remote pong failed.")?; continue; },Message::Pong(_)=>continue,Message::Close(_)=>return Err("Remote disconnected.".into()),_=>{} }
                let frame=binary(message)?;
                let plaintext=crypto.open_binary(route,&frame)?;
                let Some(input)=assembler.push(&plaintext)? else { continue; };
                if input.get("v").and_then(Value::as_u64)!=Some(1) { return Err("Invalid Remote protocol.".into()); }
                match input.get("type").and_then(Value::as_str) {
                    Some("resync")=> {
                        lease=None; app.state::<Terminals>().remote_revoke_owner(&wire.session_id,&wire.id);
                        let snapshot=remote.runtime.lock().map_err(|_| "Terminal unavailable.")?.snapshot(&wire.session_id)?;
                        seq=snapshot.get("seq").and_then(Value::as_u64).ok_or("Invalid snapshot.")?;
                        send(&permit, &mut ws,&mut crypto,route,snapshot).await?;
                    },
                    Some("claim") if permissions==Permissions::Control=> {
                        let id=uuid()?;
                        if app.state::<Terminals>().remote_claim(&wire.session_id,&wire.id,&id).is_ok() { lease=Some(id.clone()); send(&permit, &mut ws,&mut crypto,route,json!({"v":1,"type":"lease","leaseId":id,"expiresAt":now()+5})).await?; }
                        else { send(&permit, &mut ws,&mut crypto,route,json!({"v":1,"type":"lease","leaseId":null})).await?; }
                    },
                    Some("renew") if permissions==Permissions::Control=> {
                        let renewed=lease.as_ref().filter(|id| input.get("leaseId").and_then(Value::as_str)==Some(id.as_str()))
                            .filter(|id| app.state::<Terminals>().remote_renew(&wire.session_id,&wire.id,id).is_ok()).cloned();
                        if let Some(id)=renewed { send(&permit, &mut ws,&mut crypto,route,json!({"v":1,"type":"lease","leaseId":id,"expiresAt":now()+5})).await?; }
                        else {
                            lease=None; app.state::<Terminals>().remote_revoke_owner(&wire.session_id,&wire.id);
                            send(&permit, &mut ws,&mut crypto,route,json!({"v":1,"type":"lease","leaseId":null})).await?;
                        }
                    },
                    Some("input")=> {
                        let id=input.get("id").and_then(Value::as_str).ok_or("Input id missing.")?; uuid_bytes(id)?;
                        if seen.len()>=1024 || !seen.insert(id.to_string()) { return Err("Duplicate or excessive Remote input.".into()); }
                        let encoded=input.get("data").and_then(Value::as_str).filter(|s| s.len()<=21848).ok_or("Input exceeds its budget.")?;
                        let data=STANDARD.decode(encoded).map_err(|_| "Input encoding invalid.")?;
                        let lease_id=input.get("leaseId").and_then(Value::as_str).unwrap_or("");
                        let receipt=if permissions==Permissions::Control {
                            let r=remote.clone(); let a=app.clone(); let w=wire.clone(); let b=binding.clone(); let e=epoch.to_string(); let stop=stop.clone(); let lease_id=lease_id.to_string();
                            tauri::async_runtime::spawn_blocking(move || a.state::<Terminals>().remote_input(&w.session_id,&w.id,&lease_id,&data,|| allowed(&r,&a,&w,&b,&e,&stop))).await.map_err(|_| "Remote writer unavailable.")?
                        } else { crate::terminal::RemoteReceipt {written:0,status:"rejected"} };
                        send(&permit, &mut ws,&mut crypto,route,json!({"v":1,"type":"receipt","id":id,"written":receipt.written,"status":receipt.status})).await?;
                    },
                    _=> return Err("Remote message rejected.".into()),
                }
            },
        }
    }
}

async fn next(ws: &mut Socket) -> Result<Message, String> {
    tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .map_err(|_| "Remote handshake deadline.")?
        .ok_or("Remote disconnected.")?
        .map_err(|_| "Remote disconnected.".into())
}
fn binary(message: Message) -> Result<Vec<u8>, String> {
    match message {
        Message::Binary(b) if b.len() <= 17 * 1024 => Ok(b.to_vec()),
        _ => Err("Invalid Remote binary frame.".into()),
    }
}
async fn send(
    permit: &(impl Fn() -> bool + Sync),
    ws: &mut Socket,
    crypto: &mut Channel,
    route: [u8; 16],
    message: Value,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(&message).map_err(|_| "Remote message invalid.")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Remote snapshot exceeds its budget.".into());
    }
    if bytes.len() <= lomi_remote_crypto::MAX_PLAINTEXT {
        return send_bytes(permit, ws, crypto, route, &bytes).await;
    }
    let id = uuid()?;
    let total = bytes.len().div_ceil(8192);
    let deadline = Instant::now() + Duration::from_secs(5);
    for (index, chunk) in bytes.chunks(8192).enumerate() {
        if Instant::now() > deadline {
            return Err("Remote transfer deadline.".into());
        }
        let part=serde_json::to_vec(&json!({"v":1,"type":"fragment","id":id,"index":index,"total":total,"data":STANDARD.encode(chunk)})).map_err(|_| "Remote fragment invalid.")?;
        send_bytes(permit, ws, crypto, route, &part).await?;
    }
    Ok(())
}
async fn send_bytes(
    permit: &(impl Fn() -> bool + Sync),
    ws: &mut Socket,
    crypto: &mut Channel,
    route: [u8; 16],
    bytes: &[u8],
) -> Result<(), String> {
    if !permit() {
        return Err("Remote authorization expired.".into());
    }
    let frame = crypto.seal_binary(route, bytes)?;
    use std::future::Future;
    let mut pending = Box::pin(ws.send(Message::Binary(frame.into())));
    tokio::time::timeout(
        Duration::from_secs(2),
        futures_util::future::poll_fn(|cx| {
            if !permit() {
                return std::task::Poll::Ready(Err("Remote authorization expired.".to_string()));
            }
            pending
                .as_mut()
                .poll(cx)
                .map(|result| result.map_err(|_| "Remote disconnected.".to_string()))
        }),
    )
    .await
    .map_err(|_| "Remote send deadline.")?
}

#[derive(Default)]
struct Fragments {
    transfer: Option<(String, usize, usize, Instant, Vec<u8>)>,
}
impl Fragments {
    fn check_deadline(&self) -> Result<(), String> {
        if self.transfer.as_ref().is_some_and(|t| Instant::now() > t.3) {
            Err("Remote fragment deadline.".into())
        } else {
            Ok(())
        }
    }
    fn push(&mut self, bytes: &[u8]) -> Result<Option<Value>, String> {
        self.check_deadline()?;
        let value: Value = serde_json::from_slice(bytes).map_err(|_| "Remote message invalid.")?;
        if value.get("type").and_then(Value::as_str) != Some("fragment") {
            if self.transfer.is_some() {
                return Err("Remote fragments interrupted.".into());
            }
            return Ok(Some(value));
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Remote fragment id missing.")?;
        uuid_bytes(id)?;
        let index = value
            .get("index")
            .and_then(Value::as_u64)
            .ok_or("Remote fragment index invalid.")? as usize;
        let total = value
            .get("total")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0 && *n <= 1024)
            .ok_or("Remote fragment count invalid.")? as usize;
        let encoded = value
            .get("data")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 10924)
            .ok_or("Remote fragment exceeds its budget.")?;
        let decoded = STANDARD
            .decode(encoded)
            .map_err(|_| "Remote fragment bytes invalid.")?;
        if decoded.len() > 8192 || value.get("v").and_then(Value::as_u64) != Some(1) {
            return Err("Remote fragment invalid.".into());
        }
        if self.transfer.is_none() {
            if index != 0 {
                return Err("Remote fragment sequence invalid.".into());
            }
            self.transfer = Some((
                id.into(),
                total,
                0,
                Instant::now() + Duration::from_secs(5),
                Vec::new(),
            ));
        }
        let t = self
            .transfer
            .as_mut()
            .ok_or("Remote fragment unavailable.")?;
        if t.0 != id || t.1 != total || t.2 != index || t.4.len() + decoded.len() > 8 * 1024 * 1024
        {
            return Err("Remote fragment sequence invalid.".into());
        }
        t.2 += 1;
        t.4.extend(decoded);
        if t.2 != total {
            return Ok(None);
        }
        let bytes = self
            .transfer
            .take()
            .ok_or("Remote fragment unavailable.")?
            .4;
        let result: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Remote assembled message invalid.")?;
        if result.get("type").and_then(Value::as_str) == Some("fragment") {
            return Err("Nested Remote fragments rejected.".into());
        }
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_metadata_and_terminal_channels_cannot_restart_on_subsequent_polls() {
        for purpose in [ChannelPurpose::Metadata, ChannelPurpose::Terminal] {
            let mut core = Core::default();
            let id = format!("{purpose:?}");
            assert!(core.reserve_channel(&id, 220, 100).unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            core.channels.insert(id.clone(), stop.clone());
            assert!(!core.reserve_channel(&id, 220, 102).unwrap());
            stop.store(true, Ordering::SeqCst);
            core.channels.remove(&id);
            for poll in (104..220).step_by(2) {
                assert!(core.channel_seen(&id, poll));
                // An altered response cannot extend a consumed ID's expiry.
                assert!(!core.reserve_channel(&id, poll + 120, poll).unwrap());
                assert_eq!(core.attempted_channels[&id], 220);
            }
            assert!(!core.channel_seen(&id, 220));
            assert!(core.attempted_channels.is_empty());
        }
    }

    #[test]
    fn channel_attempt_budget_rejects_new_ids_until_expiry_without_evicting_history() {
        let mut core = Core::default();
        for i in 0..MAX_CHANNEL_ATTEMPTS {
            assert!(core.reserve_channel(&i.to_string(), 220, 100).unwrap());
        }
        // Poll authorization renewal and account resets cannot reopen consumed IDs.
        core.deadline = Some(Instant::now() + Duration::from_secs(10));
        assert!(core.reserve_channel("overflow", 221, 101).is_err());
        assert!(core.deadline.is_some());
        core.reset_bound_authority();
        assert!(!core.reserve_channel("0", 222, 102).unwrap());
        assert!(core.reserve_channel("overflow", 222, 102).is_err());
        assert_eq!(core.attempted_channels.len(), MAX_CHANNEL_ATTEMPTS);
        assert!(core.reserve_channel("fresh", 340, 220).unwrap());
        assert_eq!(core.attempted_channels.len(), 1);
    }

    #[test]
    fn channel_attempt_reservations_reject_expired_or_unbounded_deadlines() {
        let mut core = Core::default();
        assert!(core.reserve_channel("expired", 100, 100).is_err());
        assert!(core.reserve_channel("unbounded", 221, 100).is_err());
        assert!(core.attempted_channels.is_empty());
        assert!(core.reserve_channel("valid", 220, 100).unwrap());
        // Failed ticket/setup attempts retain their reservation without an active socket.
        assert!(core.channels.is_empty());
        assert!(!core.reserve_channel("valid", 220, 102).unwrap());
    }

    #[test]
    fn native_workspace_authority_rejects_stale_scope_account_move_and_purpose() {
        let (mut policy, host) = Policy::probe("account", "parent", [1; 16]).unwrap();
        let device_id = uuid().unwrap();
        let device = Identity::generate(
            [1; 16],
            uuid_bytes(&device_id).unwrap(),
            lomi_remote_crypto::Role::Device,
            1,
        )
        .unwrap();
        let id = uuid().unwrap();
        let epoch = uuid().unwrap();
        let terminal = uuid().unwrap();
        let terminal_epoch = uuid().unwrap();
        let grant = uuid().unwrap();
        let channel = uuid().unwrap();
        let projection = workspace::WorkspaceProjection {
            id: id.clone(),
            name: "Private".into(),
            terminals: vec![workspace::TerminalProjection {
                pane_id: "pane".into(),
                session_id: Some(terminal.clone()),
                title: "Private shell".into(),
            }],
        };
        policy.workspaces.push(workspace::Consent {
            id: id.clone(),
            epoch: epoch.clone(),
            revision: 3,
            shared: true,
            sessions: vec![(terminal.clone(), terminal_epoch.clone())],
            projection: Some(projection.clone()),
        });
        let approval = host
            .sign_peer_approval(PeerApproval {
                version: 2,
                account_id: [1; 16],
                host_id: uuid_bytes(&policy.host_id).unwrap(),
                device_id: uuid_bytes(&device_id).unwrap(),
                host_fingerprint: policy.bundle.bundle.fingerprint().unwrap(),
                device_fingerprint: device.public_bundle().bundle.fingerprint().unwrap(),
                pairing_nonce: [7; 32],
                grant_id: uuid_bytes(&grant).unwrap(),
                workspace_id: Some(uuid_bytes(&id).unwrap()),
                workspace_epoch: Some(uuid_bytes(&epoch).unwrap()),
                session_ids: vec![uuid_bytes(&terminal).unwrap()],
                session_epochs: vec![uuid_bytes(&terminal_epoch).unwrap()],
                permissions: Permissions::Control,
                access_epoch: 1,
                revision: 3,
                expires_at: now() + 60,
            })
            .unwrap();
        let context = ChannelContext {
            version: 2,
            account_id: [1; 16],
            host_id: uuid_bytes(&policy.host_id).unwrap(),
            device_id: uuid_bytes(&device_id).unwrap(),
            initiator_role: lomi_remote_crypto::Role::Device,
            responder_role: lomi_remote_crypto::Role::Host,
            channel_id: uuid_bytes(&channel).unwrap(),
            grant_id: uuid_bytes(&grant).unwrap(),
            session_id: uuid_bytes(&terminal).unwrap(),
            access_epoch: 1,
            revision: 3,
            workspace_id: Some(uuid_bytes(&id).unwrap()),
            workspace_epoch: Some(uuid_bytes(&epoch).unwrap()),
            session_epoch: Some(uuid_bytes(&terminal_epoch).unwrap()),
            purpose: Some(ChannelPurpose::Terminal),
        };
        let wire = Wire {
            id: channel,
            session_id: terminal,
            host_id: policy.host_id.clone(),
            device_id,
            grant_id: grant,
            context,
            host_bundle: policy.bundle.clone(),
            device_bundle: device.public_bundle(),
            signed_approval: approval,
            expires_at: now() + 60,
        };
        let mut core = Core {
            policy: Some(policy),
            ..Core::default()
        };
        let domain_epoch = core.domain.begin().unwrap();
        core.domain
            .sync(&domain_epoch, 1, vec![projection])
            .unwrap();
        assert!(workspace_allowed(&core, &wire));
        let mut forged = wire.clone();
        forged.context.session_epoch = Some([99; 16]);
        assert!(!workspace_allowed(&core, &forged));
        forged = wire.clone();
        forged.context.purpose = Some(ChannelPurpose::Metadata);
        assert!(!workspace_allowed(&core, &forged));
        forged = wire.clone();
        forged.signed_approval.approval.account_id = [2; 16];
        forged.context.account_id = [2; 16];
        assert!(!workspace_allowed(&core, &forged));
        core.domain.workspaces[0].terminals.clear();
        assert!(!workspace_allowed(&core, &wire));
        core.domain.workspaces = vec![core.policy.as_ref().unwrap().workspaces[0]
            .projection
            .clone()
            .unwrap()];
        core.policy.as_mut().unwrap().workspaces[0].revision = 4;
        assert!(!workspace_allowed(&core, &wire));
        core.policy.as_mut().unwrap().workspaces[0].revision = 3;
        core.policy.as_mut().unwrap().workspaces[0].shared = false;
        assert!(!workspace_allowed(&core, &wire));
    }
    #[test]
    fn remote_fragments_reassemble_maximum_input_and_reject_reorder() {
        let message = json!({"v":1,"type":"input","id":uuid().unwrap(),"leaseId":uuid().unwrap(),"data":STANDARD.encode(vec![42;16384])});
        let bytes = serde_json::to_vec(&message).unwrap();
        let id = uuid().unwrap();
        let total = bytes.len().div_ceil(8192);
        let parts:Vec<_>=bytes.chunks(8192).enumerate().map(|(index,chunk)| serde_json::to_vec(&json!({"v":1,"type":"fragment","id":id,"index":index,"total":total,"data":STANDARD.encode(chunk)})).unwrap()).collect();
        let mut a = Fragments::default();
        for part in &parts[..parts.len() - 1] {
            assert!(a.push(part).unwrap().is_none());
        }
        assert_eq!(a.push(parts.last().unwrap()).unwrap().unwrap(), message);
        assert!(Fragments::default().push(&parts[1]).is_err());
        let mut a = Fragments::default();
        a.push(&parts[0]).unwrap();
        assert!(a.push(&parts[0]).is_err());
        let mut a = Fragments::default();
        a.push(&parts[0]).unwrap();
        a.transfer.as_mut().unwrap().3 = Instant::now() - Duration::from_millis(1);
        assert!(a.push(&parts[1]).is_err());
    }
}
