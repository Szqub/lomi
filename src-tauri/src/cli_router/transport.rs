use super::{codex, runtime, vibe, CliRouterService};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::AppHandle;
use zeroize::{Zeroize, Zeroizing};

const MAX_FRAME: usize = 4 * 1024 * 1024;
const MAX_WIRE: usize = 16 * 1024 * 1024;

pub(super) enum Protocol {
    Codex(codex::Protocol),
    Vibe(vibe::Protocol),
}

pub(super) struct Context<'a> {
    pub(super) state: &'a CliRouterService,
    pub(super) app: &'a AppHandle,
    pub(super) run_id: &'a str,
    pub(super) generation: u64,
    pub(super) cancelled: &'a Arc<AtomicBool>,
}

#[derive(Default)]
pub(super) struct Outcome {
    pub(super) completed: bool,
    pub(super) exhausted: bool,
    pub(super) auth_failed: bool,
    pub(super) effects: bool,
    pub(super) stopped: bool,
    pub(super) drained: bool,
    pub(super) valid: bool,
    pub(super) output: String,
}

fn enqueue(sender: &mpsc::SyncSender<Zeroizing<Vec<u8>>>, mut frame: Value) -> Result<(), String> {
    let encoded = serde_json::to_vec(&frame);
    if let Some(Value::String(token)) = frame.pointer_mut("/params/accessToken") {
        token.zeroize();
    }
    let mut bytes = Zeroizing::new(encoded.map_err(|_| "Cannot encode a CLI request.")?);
    if bytes.len() >= MAX_FRAME {
        return Err("CLI request exceeds the bounded transport size.".into());
    }
    bytes.push(b'\n');
    sender
        .try_send(bytes)
        .map_err(|_| "The CLI stopped accepting its owned requests.".into())
}

pub(super) fn drive(
    mut child: runtime::OwnedChild,
    mut protocol: Protocol,
    context: Context<'_>,
    auth: Option<crate::cli_usage::CodexProfileAuth>,
) -> Result<Outcome, String> {
    let stdout = child.stdout.take().ok_or("CLI output is unavailable.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("CLI diagnostics are unavailable.")?;
    let mut stdin = child.stdin.take().ok_or("CLI input is unavailable.")?;
    let (sender, input) = mpsc::sync_channel::<Zeroizing<Vec<u8>>>(8);
    let writer = thread::spawn(move || -> std::io::Result<()> {
        for bytes in input {
            stdin.write_all(&bytes)?;
            stdin.flush()?;
        }
        drop(stdin);
        Ok(())
    });
    let (events, receiver) = mpsc::sync_channel::<Result<Value, ()>>(16);
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut total = 0usize;
        loop {
            let mut bytes = Vec::new();
            match (&mut reader)
                .take(MAX_FRAME as u64 + 1)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) => break,
                Ok(_) => {
                    total = total.saturating_add(bytes.len());
                    if bytes.len() > MAX_FRAME || total > MAX_WIRE || bytes.last() != Some(&b'\n') {
                        let _ = events.send(Err(()));
                        break;
                    }
                    let event = serde_json::from_slice(&bytes).map_err(|_| ());
                    if events.send(event).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = events.send(Err(()));
                    break;
                }
            }
        }
    });
    let diagnostics_overflow = Arc::new(AtomicBool::new(false));
    let overflow = diagnostics_overflow.clone();
    let diagnostics = thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = [0u8; 4096];
        let mut total = 0usize;
        loop {
            match stderr.read(&mut bytes) {
                Ok(0) => break,
                Ok(size) => {
                    total = total.saturating_add(size);
                    if total > MAX_WIRE {
                        overflow.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                Err(_) => {
                    overflow.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
    });
    let mut sender = Some(sender);
    let initial = match &mut protocol {
        Protocol::Codex(protocol) => protocol.initialize(),
        Protocol::Vibe(protocol) => protocol.initialize(),
    };
    let started = Instant::now();
    let mut outcome = Outcome {
        valid: true,
        ..Outcome::default()
    };
    let mut exited = false;
    let mut eof = false;
    let mut closed = false;
    let mut persisted = 0usize;
    let mut last_save = Instant::now();
    let mut stop_deadline = None;
    let mut auth = auth;
    let result = (|| -> Result<(), String> {
        enqueue(sender.as_ref().unwrap(), initial)?;
        loop {
            if context.cancelled.load(Ordering::SeqCst)
                || started.elapsed() > Duration::from_secs(300)
            {
                outcome.stopped = context.cancelled.load(Ordering::SeqCst);
                outcome.valid = false;
                if stop_deadline.is_none() {
                    let frames = match &mut protocol {
                        Protocol::Codex(protocol) => protocol.interrupt().into_iter().collect(),
                        Protocol::Vibe(protocol) => protocol.cancel(),
                    };
                    if let Some(sender) = sender.as_ref() {
                        for frame in frames {
                            enqueue(sender, frame)?;
                        }
                    }
                    stop_deadline = Some(Instant::now() + Duration::from_secs(2));
                }
            }
            if diagnostics_overflow.load(Ordering::SeqCst) {
                outcome.valid = false;
                break;
            }
            if stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            if eof {
                thread::sleep(Duration::from_millis(20));
            } else {
                match receiver.recv_timeout(Duration::from_millis(20)) {
                    Ok(Ok(event)) => {
                        let mut frames = Vec::new();
                        match &mut protocol {
                            Protocol::Codex(protocol) => {
                                if event.get("id").is_some() && event.get("method").is_some() {
                                    if let Some(sender) = sender.as_ref() {
                                        enqueue(
                                            sender,
                                            json!({"id":event["id"],"error":{"code":-32000,"message":"Managed text turns require explicit authentication renewal and deny tools."}}),
                                        )?;
                                    }
                                }
                                match protocol.consume(&event) {
                                    Ok(outbound) => frames = outbound,
                                    Err(codex::Rejection::Failed) if protocol.terminal => {}
                                    Err(rejection) => {
                                        outcome.valid = false;
                                        outcome.auth_failed = rejection == codex::Rejection::Auth;
                                        outcome.effects |= rejection == codex::Rejection::Effect;
                                    }
                                }
                                if let Some(text) = protocol.take_delta() {
                                    if outcome.output.len().saturating_add(text.len())
                                        > runtime::MAX_OUTPUT
                                    {
                                        return Err(
                                            "CLI output exceeds its saved-history limit.".into()
                                        );
                                    }
                                    outcome.output.push_str(&text);
                                }
                                outcome.completed = protocol.completed;
                                outcome.exhausted = protocol.exhausted;
                                outcome.effects |= protocol.effect;
                                closed = protocol.terminal;
                                if protocol.needs_login() {
                                    let credentials = auth.take().ok_or(
                                        "The owned Codex credential binding is unavailable.",
                                    )?;
                                    let frame = protocol
                                        .login(
                                            credentials.access_token.as_str(),
                                            &credentials.account_id,
                                        )
                                        .map_err(|_| {
                                            "The owned Codex authentication phase changed."
                                        })?;
                                    frames.push(frame);
                                }
                            }
                            Protocol::Vibe(protocol) => match protocol.consume(&event) {
                                Ok(progress) => {
                                    frames = progress.outbound;
                                    if let Some(text) = progress.text {
                                        if !outcome.output.is_empty()
                                            || text.len() > runtime::MAX_OUTPUT
                                        {
                                            return Err(
                                                "Vibe returned inconsistent saved output.".into()
                                            );
                                        }
                                        outcome.output = text;
                                    }
                                    outcome.completed |= progress.complete;
                                    closed |= progress.closed;
                                    outcome.effects |= progress.effect_denied;
                                    if progress.effect_denied {
                                        outcome.valid = false;
                                    }
                                }
                                Err(_) => outcome.valid = false,
                            },
                        }
                        if let Some(sender) = sender.as_ref() {
                            for frame in frames {
                                enqueue(sender, frame)?;
                            }
                        } else if !frames.is_empty() {
                            outcome.valid = false;
                        }
                        if closed {
                            sender.take();
                        }
                        if !outcome.valid && stop_deadline.is_none() {
                            break;
                        }
                    }
                    Ok(Err(())) => {
                        outcome.valid = false;
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => eof = true,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            if outcome.output.len() > persisted && last_save.elapsed() > Duration::from_millis(500)
            {
                runtime::save_output(
                    context.state,
                    context.app,
                    context.run_id,
                    context.generation,
                    persisted,
                    &outcome.output[persisted..],
                )?;
                persisted = outcome.output.len();
                last_save = Instant::now();
            }
            if !exited {
                exited = runtime::exit_pending(&child)?;
                if exited {
                    child.stop_group();
                }
            }
            if exited && eof {
                break;
            }
        }
        Ok(())
    })();
    // Every path kills descendants before reaping, then unblocks and joins both
    // bounded readers and the writer. No callback/result or EOF proves drain.
    let status = child.stop_and_wait();
    drop(child);
    sender.take();
    drop(receiver);
    let read_joined = reader.join().is_ok();
    let diagnostics_joined = diagnostics.join().is_ok();
    let input_written = writer.join().is_ok_and(|result| result.is_ok());
    if outcome.output.len() > persisted {
        runtime::save_output(
            context.state,
            context.app,
            context.run_id,
            context.generation,
            persisted,
            &outcome.output[persisted..],
        )?;
    }
    result?;
    let status = status?;
    outcome.stopped |= context.cancelled.load(Ordering::SeqCst);
    outcome.drained =
        exited && eof && input_written && read_joined && diagnostics_joined && status.success();
    outcome.valid &= !diagnostics_overflow.load(Ordering::SeqCst);
    outcome.completed &= outcome.valid
        && closed
        && outcome.drained
        && !outcome.stopped
        && !outcome.effects
        && !outcome.output.trim().is_empty();
    outcome.exhausted &=
        outcome.valid && closed && outcome.drained && !outcome.stopped && !outcome.effects;
    Ok(outcome)
}
