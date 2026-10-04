//! Persistent coding transport. The supervisor owns approval, storage and every
//! file operation. No native turn or tool result leaves this driver without its
//! supervisor's durable receipt and final owner fence.
use super::{codex, runtime::OwnedChild, transport::Outcome};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_WIRE: usize = 128 * 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 1024 * 1024;

pub(crate) struct DurableToolReply {
    pub(crate) content_items: Value,
    pub(crate) success: bool,
    pub(crate) receipt: String,
}

pub(crate) trait Supervisor {
    /// Commit binding/input intent, then release and enqueue under the native
    /// owner lock. A returned receipt alone does not serialize enqueue with Stop.
    fn release_turn(
        &mut self,
        protocol: &mut codex::CodingProtocol,
        enqueue: &mut dyn FnMut(Value) -> Result<(), String>,
    ) -> Result<(), String>;
    /// Match the complete native call. Any write waits for one-use Main consent
    /// and persists its result before returning. Cancellation cannot authorize it.
    fn tool(
        &mut self,
        request: &codex::ToolRequest,
        cancelled: &Arc<AtomicBool>,
        healthy: &Arc<AtomicBool>,
    ) -> Result<DurableToolReply, String>;
    /// Revalidate owner and transport and enqueue the durable result under the
    /// same owner lock used by Stop and account/grant mutations.
    fn deliver_tool(
        &mut self,
        protocol: &mut codex::CodingProtocol,
        request: &codex::ToolRequest,
        reply: DurableToolReply,
        healthy: &Arc<AtomicBool>,
        enqueue: &mut dyn FnMut(Value) -> Result<(), String>,
    ) -> Result<(), String>;
    fn output(&mut self, full_text: &str) -> Result<(), String>;
    fn checkpoint(&mut self, protocol: &codex::CodingProtocol) -> Result<(), String>;
    /// Retain interrupted observations even when no canonical checkpoint exists.
    fn observations(&mut self, protocol: &codex::CodingProtocol) -> Result<(), String>;
}

fn enqueue(sender: &mpsc::SyncSender<Zeroizing<Vec<u8>>>, mut frame: Value) -> Result<(), String> {
    let encoded = serde_json::to_vec(&frame);
    if let Some(Value::String(token)) = frame.pointer_mut("/params/accessToken") {
        token.zeroize();
    }
    let mut bytes = Zeroizing::new(encoded.map_err(|_| "Cannot encode owned coding request.")?);
    if bytes.len() > MAX_FRAME {
        return Err("Coding request exceeds its transport bound.".into());
    }
    bytes.push(b'\n');
    sender
        .try_send(bytes)
        .map_err(|_| "The owned coding request queue is unavailable.".into())
}

fn nonblocking(pipe: &impl AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("Cannot bound the owned coding pipes.".into());
    }
    Ok(())
}

pub(crate) fn drive(
    mut child: OwnedChild,
    mut protocol: codex::CodingProtocol,
    cancelled: &Arc<AtomicBool>,
    auth: crate::cli_usage::CodexProfileAuth,
    supervisor: &mut impl Supervisor,
) -> Result<Outcome, String> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Owned coding output is unavailable.")?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("Owned coding diagnostics are unavailable.")?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Owned coding input is unavailable.")?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    nonblocking(&stdin)?;
    let stop_io = Arc::new(AtomicBool::new(false));
    let bad_io = Arc::new(AtomicBool::new(false));
    let healthy = Arc::new(AtomicBool::new(true));
    let (sender, input) = mpsc::sync_channel::<Zeroizing<Vec<u8>>>(8);
    let writer_stop = stop_io.clone();
    let writer_bad = bad_io.clone();
    let writer_health = healthy.clone();
    let writer = thread::spawn(move || -> std::io::Result<()> {
        for frame in input {
            let mut offset = 0;
            while offset < frame.len() {
                if writer_stop.load(Ordering::SeqCst) {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                match stdin.write(&frame[offset..]) {
                    Ok(0) => {
                        writer_health.store(false, Ordering::SeqCst);
                        writer_bad.store(true, Ordering::SeqCst);
                        return Err(std::io::ErrorKind::WriteZero.into());
                    }
                    Ok(n) => offset += n,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        writer_health.store(false, Ordering::SeqCst);
                        writer_bad.store(true, Ordering::SeqCst);
                        return Err(error);
                    }
                }
            }
        }
        drop(stdin);
        Ok(())
    });
    let (events, receiver) = mpsc::sync_channel(16);
    let reader_stop = stop_io.clone();
    let reader_bad = bad_io.clone();
    let reader_health = healthy.clone();
    let reader = thread::spawn(move || -> Result<(), ()> {
        let mut total = 0usize;
        let mut pending = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            if reader_stop.load(Ordering::SeqCst) {
                return Err(());
            }
            let n = match stdout.read(&mut chunk) {
                Ok(0) => {
                    reader_health.store(false, Ordering::SeqCst);
                    if pending.is_empty() {
                        return Ok(());
                    }
                    reader_health.store(false, Ordering::SeqCst);
                    reader_bad.store(true, Ordering::SeqCst);
                    return Err(());
                }
                Ok(n) => n,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    reader_health.store(false, Ordering::SeqCst);
                    reader_bad.store(true, Ordering::SeqCst);
                    return Err(());
                }
            };
            total = total.saturating_add(n);
            if total > MAX_WIRE {
                reader_health.store(false, Ordering::SeqCst);
                reader_bad.store(true, Ordering::SeqCst);
                return Err(());
            }
            pending.extend_from_slice(&chunk[..n]);
            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                if end >= MAX_FRAME {
                    reader_health.store(false, Ordering::SeqCst);
                    reader_bad.store(true, Ordering::SeqCst);
                    return Err(());
                }
                let mut frame = serde_json::from_slice::<Value>(&pending[..end]).map_err(|_| ());
                pending.drain(..=end);
                if frame.is_err() {
                    reader_health.store(false, Ordering::SeqCst);
                    reader_bad.store(true, Ordering::SeqCst);
                }
                loop {
                    if reader_stop.load(Ordering::SeqCst) {
                        return Err(());
                    }
                    match events.try_send(frame) {
                        Ok(()) => break,
                        Err(mpsc::TrySendError::Full(retained)) => {
                            frame = retained;
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => return Err(()),
                    }
                }
            }
            if pending.len() > MAX_FRAME {
                reader_health.store(false, Ordering::SeqCst);
                reader_bad.store(true, Ordering::SeqCst);
                return Err(());
            }
        }
    });
    let error_stop = stop_io.clone();
    let error_bad = bad_io.clone();
    let error_health = healthy.clone();
    let diagnostics = thread::spawn(move || -> Result<(), ()> {
        let mut total = 0usize;
        let mut bytes = [0u8; 4096];
        loop {
            if error_stop.load(Ordering::SeqCst) {
                return Err(());
            }
            let n = match stderr.read(&mut bytes) {
                Ok(n) => n,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    error_health.store(false, Ordering::SeqCst);
                    error_bad.store(true, Ordering::SeqCst);
                    return Err(());
                }
            };
            if n == 0 {
                return Ok(());
            }
            total = total.saturating_add(n);
            if total > MAX_DIAGNOSTICS {
                error_health.store(false, Ordering::SeqCst);
                error_bad.store(true, Ordering::SeqCst);
                return Err(());
            }
        }
    });
    let mut sender = Some(sender);
    let mut auth = Some(auth);
    let mut valid = true;
    let mut checkpoint_saved = false;
    let mut eof = false;
    let mut exited = false;
    let mut drain_deadline = None;
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut stopping = None;
    let send = |sender: &mpsc::SyncSender<Zeroizing<Vec<u8>>>, frame: Value| {
        if !healthy.load(Ordering::SeqCst) {
            return Err("Coding transport closed before input admission.".into());
        }
        enqueue(sender, frame)
    };
    let execution = (|| -> Result<(), String> {
        send(sender.as_ref().unwrap(), protocol.initialize())?;
        loop {
            if bad_io.load(Ordering::SeqCst) {
                valid = false;
            }
            if cancelled.load(Ordering::SeqCst) && stopping.is_none() {
                stopping = Some(Instant::now() + Duration::from_secs(5));
                if let (Some(frame), Some(sender)) = (protocol.interrupt(), sender.as_ref()) {
                    send(sender, frame)?;
                } else if !protocol.terminal {
                    break;
                }
            }
            if Instant::now() >= deadline || stopping.is_some_and(|t| Instant::now() >= t) {
                valid = false;
                break;
            }
            if !eof {
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(Ok(event)) => {
                        let frames = match protocol.consume(&event) {
                            Ok(frames) => frames,
                            Err(rejection) => {
                                valid = false;
                                if event.get("id").is_some() && event.get("method").is_some() {
                                    if let Some(sender) = sender.as_ref() {
                                        let _ = send(
                                            sender,
                                            json!({"id":event["id"],"error":{"code":-32000,"message":"This native request cannot be authorized."}}),
                                        );
                                    }
                                }
                                let _ = rejection;
                                break;
                            }
                        };
                        // Preserve an already queued valid prefix after an IO
                        // rejection, while fencing every further native input
                        // and effect. Reader order still makes malformed tails
                        // terminal and never canonical success.
                        supervisor.output(&protocol.output)?;
                        if bad_io.load(Ordering::SeqCst) {
                            valid = false;
                            continue;
                        }
                        for frame in frames {
                            send(
                                sender
                                    .as_ref()
                                    .ok_or("Coding requested input after channel closure.")?,
                                frame,
                            )?;
                        }
                        if protocol.needs_login() {
                            let credentials = auth
                                .take()
                                .ok_or("Coding authentication ownership changed.")?;
                            let frame = protocol
                                .login(credentials.access_token.as_str(), &credentials.account_id)
                                .map_err(|_| "Coding authentication admission failed.")?;
                            send(
                                sender.as_ref().ok_or(
                                    "Coding requested authentication after channel closure.",
                                )?,
                                frame,
                            )?;
                        }
                        if protocol.pre_turn_ready() {
                            if cancelled.load(Ordering::SeqCst) {
                                break;
                            }
                            supervisor.release_turn(&mut protocol, &mut |frame| {
                                send(
                                    sender
                                        .as_ref()
                                        .ok_or("Coding turn requested after channel closure.")?,
                                    frame,
                                )
                            })?;
                        }
                        while let Some(call) = protocol.take_tool_request() {
                            let reply = supervisor.tool(&call, cancelled, &healthy)?;
                            if !healthy.load(Ordering::SeqCst) || cancelled.load(Ordering::SeqCst) {
                                valid = false;
                                break;
                            }
                            supervisor.deliver_tool(
                                &mut protocol,
                                &call,
                                reply,
                                &healthy,
                                &mut |frame| {
                                    send(
                                        sender.as_ref().ok_or(
                                            "Coding tool requested after channel closure.",
                                        )?,
                                        frame,
                                    )
                                },
                            )?;
                        }
                        if protocol.checkpointed && !checkpoint_saved {
                            supervisor.checkpoint(&protocol)?;
                            checkpoint_saved = true;
                            sender.take();
                        }
                    }
                    Ok(Err(())) => {
                        valid = false;
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        eof = true;
                        if bad_io.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if bad_io.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(25));
            }
            if !exited {
                exited = super::runtime::exit_pending(&child)?;
                if exited {
                    child.stop_group();
                    drain_deadline = Some(Instant::now() + Duration::from_secs(5));
                }
            }
            if eof && exited && diagnostics.is_finished() && writer.is_finished() {
                break;
            }
            if drain_deadline.is_some_and(|t| Instant::now() >= t) {
                valid = false;
                break;
            }
        }
        Ok(())
    })();
    let status = child.stop_and_wait();
    drop(child);
    sender.take();
    drop(receiver);
    // Nonblocking readers also terminate if an escaped descendant retains a
    // pipe. Such cleanup cannot qualify as a natural canonical drain.
    stop_io.store(true, Ordering::SeqCst);
    let read_ok = reader.join().is_ok_and(|value| value.is_ok());
    let error_ok = diagnostics.join().is_ok_and(|value| value.is_ok());
    let wrote = writer.join().is_ok_and(|value| value.is_ok());
    // These writes follow drain even on transport or checkpoint failure.
    let output_saved = supervisor.output(&protocol.output);
    let observations_saved = supervisor.observations(&protocol);
    execution?;
    output_saved?;
    observations_saved?;
    let status = status?;
    let stopped = cancelled.load(Ordering::SeqCst);
    let drained = eof && exited && read_ok && error_ok && wrote && status.success();
    let valid = valid && protocol.rejection.is_none() && drained;
    Ok(Outcome {
        completed: valid && protocol.completed && checkpoint_saved && !stopped && !protocol.effect,
        exhausted: valid && protocol.exhausted && checkpoint_saved && !stopped && !protocol.effect,
        auth_failed: protocol.rejection == Some(codex::Rejection::Auth),
        effects: protocol.effect,
        stopped,
        drained,
        valid,
        output: protocol.output,
    })
}
