#![cfg(unix)]
mod endpoint;
use lomi_session_core::{Session, Size};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
const MAX_REQUEST: usize = 128 * 1024;
const MAX_REPLY: usize = 2 * 1024 * 1024;
const MAX_SESSIONS: usize = 32;
const MAX_CLIENTS: usize = 16;
#[derive(Deserialize)]
struct Request {
    version: u32,
    #[serde(flatten)]
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Status,
    List,
    Create {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        cwd: String,
        size: Size,
    },
    Snapshot {
        session_epoch: String,
    },
    Replay {
        session_epoch: String,
        after: u64,
    },
    Input {
        session_epoch: String,
        bytes: Vec<u8>,
    },
    Resize {
        session_epoch: String,
        size: Size,
    },
    Close {
        session_epoch: String,
    },
}
type Sessions = Arc<Mutex<BTreeMap<String, Session>>>;
fn dispatch(request: Request, sessions: &Sessions) -> Result<Value, String> {
    if request.version != 1 {
        return Err("unsupported IPC version".into());
    }
    match request.command {
        Command::Status => Ok(
            json!({"version":1,"remote_control":false,"local_authority":"same_uid","profile":"vt100-0.16-replay-v1-unqualified"}),
        ),
        Command::List => {
            let map = sessions.lock().unwrap();
            Ok(json!(map
                .iter()
                .map(|(id, s)| json!({"session_epoch":id,"ended":s.ended()}))
                .collect::<Vec<_>>()))
        }
        Command::Create {
            program,
            args,
            cwd,
            size,
        } => {
            let mut map = sessions.lock().unwrap();
            if map.len() >= MAX_SESSIONS {
                return Err("session limit; restart or close ended sessions".into());
            }
            let session = Session::spawn(&program, &args, &cwd, size)?;
            let id = session.epoch().to_string();
            map.insert(id.clone(), session);
            Ok(json!({"session_epoch":id}))
        }
        command => {
            let id = match &command {
                Command::Snapshot { session_epoch }
                | Command::Replay { session_epoch, .. }
                | Command::Input { session_epoch, .. }
                | Command::Resize { session_epoch, .. }
                | Command::Close { session_epoch } => session_epoch,
                _ => unreachable!(),
            };
            let session = sessions
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or("unknown session epoch")?;
            match command {
                Command::Snapshot { .. } => {
                    serde_json::to_value(session.snapshot()?).map_err(|e| e.to_string())
                }
                Command::Replay { after, .. } => {
                    serde_json::to_value(session.replay_after(after)?).map_err(|e| e.to_string())
                }
                Command::Input { bytes, .. } => {
                    Ok(json!({"receipt":"dispatched","bytes":session.input(&bytes)?}))
                }
                Command::Resize { size, .. } => {
                    session.resize(size)?;
                    Ok(json!({"receipt":"dispatched"}))
                }
                Command::Close { session_epoch } => {
                    if session.ended() {
                        sessions.lock().unwrap().remove(&session_epoch);
                        Ok(json!({"removed":true}))
                    } else {
                        session.close()?;
                        Ok(json!({"termination_requested":true,"completion":"pending"}))
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}
fn serve(mut stream: UnixStream, sessions: Sessions) -> io::Result<()> {
    if !endpoint::same_uid(&stream)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer UID mismatch",
        ));
    }
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut line = Vec::new();
        loop {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                if line.is_empty() {
                    return Ok(());
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete request",
                ));
            }
            let count = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |n| n + 1);
            if line.len() + count > MAX_REQUEST {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "request limit"));
            }
            let complete = available[count - 1] == b'\n';
            line.extend_from_slice(&available[..count]);
            reader.consume(count);
            if complete {
                break;
            }
        }
        let result = serde_json::from_slice::<Request>(&line)
            .map_err(|_| "invalid request".into())
            .and_then(|r| dispatch(r, &sessions));
        let response = match result {
            Ok(value) => json!({"version":1,"ok":true,"result":value}),
            Err(error) => json!({"version":1,"ok":false,"error":error}),
        };
        let mut encoded = serde_json::to_vec(&response)?;
        if encoded.len() > MAX_REPLY {
            encoded = serde_json::to_vec(
                &json!({"version":1,"ok":false,"error":"reply resource limit"}),
            )?;
        }
        encoded.push(b'\n');
        stream.write_all(&encoded)?;
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--socket-dir") {
        return Err("usage: lomi-session-host --socket-dir PRIVATE_DIRECTORY".into());
    }
    let directory = args.next().ok_or("missing socket directory")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let endpoint = endpoint::Endpoint::bind(Path::new(&directory)).map_err(|e| e.to_string())?;
    let sessions = Arc::new(Mutex::new(BTreeMap::new()));
    let clients = Arc::new(AtomicUsize::new(0));
    for connection in endpoint.listener.incoming() {
        let stream = connection.map_err(|e| e.to_string())?;
        if clients
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CLIENTS).then_some(n + 1)
            })
            .is_err()
        {
            drop(stream);
            continue;
        }
        let sessions = sessions.clone();
        let clients = clients.clone();
        thread::spawn(move || {
            let _ = serve(stream, sessions);
            clients.fetch_sub(1, Ordering::AcqRel);
        });
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("lomi-session-host: {error}");
        std::process::exit(1);
    }
}
