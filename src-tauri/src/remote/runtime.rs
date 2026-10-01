use super::{json, uuid, Value};
use crate::terminal::RemoteTerminalEvent;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::Manager;

const MAX_LINE: usize = 12 * 1024 * 1024;

pub(super) struct Session {
    pub id: String,
    pub epoch: String,
    pub label: String,
    pub cols: u16,
    pub rows: u16,
    pub available: bool,
    pub unavailable_reason: Option<&'static str>,
    seq: u64,
}
#[derive(Default)]
pub(super) struct Runtime {
    pub sessions: HashMap<String, Session>,
    helper: Option<Helper>,
}

impl Runtime {
    pub fn observe(
        &mut self,
        app: &tauri::AppHandle,
        event: RemoteTerminalEvent,
    ) -> Result<Option<Value>, String> {
        if self.helper.is_none() {
            self.helper = Some(Helper::start(app)?);
        }
        self.apply(event)
    }

    fn apply(&mut self, event: RemoteTerminalEvent) -> Result<Option<Value>, String> {
        let helper = self
            .helper
            .as_mut()
            .ok_or("Remote terminal helper unavailable.")?;
        match event {
            RemoteTerminalEvent::Start { id, cols, rows } => {
                super::uuid_bytes(&id)?;
                let epoch = uuid()?;
                let mut unavailable_reason = None;
                let available=match helper.request(json!({"op":"create","sessionId":id,"epoch":epoch,"seq":0,"cols":cols,"rows":rows})) {
                    Ok(_)=>true,
                    Err(error) if matches!(error.as_str(),"SESSION_LIMIT"|"INVALID_DIMENSIONS"|"MODEL_LIMIT")=>{unavailable_reason=unavailable_message(&error);false},
                    Err(error)=>return Err(error),
                };
                self.sessions.insert(
                    id.clone(),
                    Session {
                        id: id.clone(),
                        epoch,
                        label: format!("Terminal {}", self.sessions.len() + 1),
                        cols,
                        rows,
                        seq: 0,
                        available,
                        unavailable_reason,
                    },
                );
                Ok(None)
            }
            RemoteTerminalEvent::Output { id, data } => {
                let s = self
                    .sessions
                    .get_mut(&id)
                    .ok_or("Remote terminal sequence unavailable.")?;
                if !s.available {
                    return Ok(None);
                }
                s.seq = s
                    .seq
                    .checked_add(1)
                    .filter(|n| *n <= 9_007_199_254_740_991)
                    .ok_or("Remote sequence exhausted.")?;
                let data = STANDARD.encode(data);
                if let Err(error) = helper.request(
                    json!({"op":"write","sessionId":id,"epoch":s.epoch,"seq":s.seq,"data":data}),
                ) {
                    if matches!(error.as_str(), "MODEL_LIMIT" | "SNAPSHOT_LIMIT") {
                        s.available = false;
                        s.unavailable_reason = unavailable_message(&error);
                        helper.request(
                            json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq}),
                        )?;
                        return Ok(Some(json!({"v":1,"type":"unavailable","sessionId":id})));
                    }
                    return Err(error);
                }
                Ok(Some(
                    json!({"v":1,"type":"output","sessionId":id,"seq":s.seq,"data":data}),
                ))
            }
            RemoteTerminalEvent::Resize { id, cols, rows } => {
                let s = self
                    .sessions
                    .get_mut(&id)
                    .ok_or("Remote terminal sequence unavailable.")?;
                if !s.available {
                    s.cols = cols;
                    s.rows = rows;
                    return Ok(None);
                }
                s.seq += 1;
                if let Err(error)=helper.request(json!({"op":"resize","sessionId":id,"epoch":s.epoch,"seq":s.seq,"cols":cols,"rows":rows})) {
                    if matches!(error.as_str(),"MODEL_LIMIT"|"SNAPSHOT_LIMIT"|"INVALID_DIMENSIONS") { s.available=false; s.unavailable_reason=unavailable_message(&error); helper.request(json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":if error=="INVALID_DIMENSIONS" {s.seq-1} else {s.seq}}))?; return Ok(Some(json!({"v":1,"type":"unavailable","sessionId":id}))); }
                    return Err(error);
                }
                s.cols = cols;
                s.rows = rows;
                Ok(Some(
                    json!({"v":1,"type":"resize","sessionId":id,"seq":s.seq,"cols":cols,"rows":rows}),
                ))
            }
            RemoteTerminalEvent::Exit { id, code } => {
                if let Some(s) = self.sessions.remove(&id) {
                    if s.available {
                        helper.request(
                            json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq}),
                        )?;
                    }
                    Ok(Some(
                        json!({"v":1,"type":"exit","sessionId":id,"seq":s.seq+1,"code":code}),
                    ))
                } else {
                    Ok(None)
                }
            }
        }
    }
    pub fn snapshot(&mut self, id: &str) -> Result<Value, String> {
        let s = self
            .sessions
            .get(id)
            .ok_or("Terminal is no longer running.")?;
        if !s.available {
            return Err("Remote model is unavailable for this terminal.".into());
        }
        let result = self
            .helper
            .as_mut()
            .ok_or("Remote terminal helper unavailable.")?
            .request(json!({"op":"snapshot","sessionId":id,"epoch":s.epoch,"seq":s.seq}));
        let reply = match result {
            Ok(reply) => reply,
            Err(error) if matches!(error.as_str(), "MODEL_LIMIT" | "SNAPSHOT_LIMIT") => {
                self.helper
                    .as_mut()
                    .ok_or("Remote terminal helper unavailable.")?
                    .request(json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq}))?;
                if let Some(s) = self.sessions.get_mut(id) {
                    s.available = false;
                    s.unavailable_reason = unavailable_message(&error);
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        if reply.get("throughSeq").and_then(Value::as_u64) != Some(s.seq)
            || reply.get("cols").and_then(Value::as_u64) != Some(s.cols as u64)
            || reply.get("rows").and_then(Value::as_u64) != Some(s.rows as u64)
        {
            return Err("Remote snapshot watermark mismatch.".into());
        }
        Ok(
            json!({"v":1,"type":"snapshot","seq":s.seq,"cols":s.cols,"rows":s.rows,"state":reply.get("state").ok_or("Invalid Remote snapshot.")?,"suffix":reply.get("suffix").ok_or("Invalid Remote snapshot.")?}),
        )
    }
    pub fn stop(&mut self) {
        self.helper.take();
        self.sessions.clear();
    }
}

struct Helper {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    responses: mpsc::Receiver<Value>,
    deadline: Arc<Mutex<Option<Instant>>>,
    next: u64,
}
impl Drop for Helper {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Helper {
    fn start(app: &tauri::AppHandle) -> Result<Self, String> {
        let (node, bundle) = if cfg!(debug_assertions) {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            (
                root.join(format!(
                    "binaries/lomi-node-{}{}",
                    env!("LOMI_AI_TARGET"),
                    if cfg!(windows) { ".exe" } else { "" }
                )),
                root.join("resources/remote-terminal/index.cjs"),
            )
        } else {
            (
                tauri::utils::platform::current_exe()
                    .map_err(|_| "Application path unavailable.")?
                    .parent()
                    .ok_or("Application path unavailable.")?
                    .join(if cfg!(windows) {
                        "lomi-node.exe"
                    } else {
                        "lomi-node"
                    }),
                app.path()
                    .resource_dir()
                    .map_err(|_| "Remote resources unavailable.")?
                    .join("remote-terminal/index.cjs"),
            )
        };
        Self::from_paths(node, bundle)
    }

    fn from_paths(node: std::path::PathBuf, bundle: std::path::PathBuf) -> Result<Self, String> {
        if !node.is_absolute() || !bundle.is_absolute() || !node.is_file() || !bundle.is_file() {
            return Err("Bundled Remote runtime is unavailable.".into());
        }
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(&bundle).map_err(|_| "Remote runtime unavailable.")?;
        let expected = std::fs::read_to_string(bundle.with_file_name("index.cjs.sha256"))
            .map_err(|_| "Remote runtime verification unavailable.")?;
        if super::hex(&Sha256::digest(&bytes)) != expected.trim() {
            return Err("Remote runtime integrity verification failed.".into());
        }
        let mut cmd = Command::new(node);
        cmd.arg("--no-warnings")
            .arg(bundle)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for name in ["SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR", "LANG"] {
            if let Some(v) = std::env::var_os(name) {
                cmd.env(name, v);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd
            .spawn()
            .map_err(|_| "Cannot start trusted terminal helper.")?;
        let stdin = child
            .stdin
            .take()
            .ok_or("Terminal helper input unavailable.")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Terminal helper output unavailable.")?;
        let child = Arc::new(Mutex::new(child));
        let (send, responses) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader
                    .by_ref()
                    .take((MAX_LINE + 1) as u64)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                };
                if line.len() > MAX_LINE || line.last() != Some(&b'\n') {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    break;
                };
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        let deadline = Arc::new(Mutex::new(None::<Instant>));
        let watch_deadline = deadline.clone();
        let weak = Arc::downgrade(&child);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let Some(child) = weak.upgrade() else {
                break;
            };
            if watch_deadline
                .lock()
                .map(|d| d.is_some_and(|d| Instant::now() > d))
                .unwrap_or(true)
            {
                if let Ok(mut child) = child.lock() {
                    let _ = child.kill();
                }
                break;
            }
        });
        Ok(Self {
            child,
            stdin,
            responses,
            deadline,
            next: 0,
        })
    }
    fn request(&mut self, mut request: Value) -> Result<Value, String> {
        self.next += 1;
        request["id"] = json!(self.next);
        let mut bytes =
            serde_json::to_vec(&request).map_err(|_| "Terminal helper request invalid.")?;
        if bytes.len() > MAX_LINE {
            return Err("Terminal helper request exceeds its budget.".into());
        }
        bytes.push(b'\n');
        let budget = Duration::from_secs(if self.next == 1 { 5 } else { 2 });
        *self
            .deadline
            .lock()
            .map_err(|_| "Terminal helper unavailable.")? = Some(Instant::now() + budget);
        self.stdin
            .write_all(&bytes)
            .and_then(|_| self.stdin.flush())
            .map_err(|_| "Terminal helper pipe closed.")?;
        let reply = self
            .responses
            .recv_timeout(budget)
            .map_err(|_| "Terminal helper deadline exceeded.")?;
        *self
            .deadline
            .lock()
            .map_err(|_| "Terminal helper unavailable.")? = None;
        if reply.get("id").and_then(Value::as_u64) != Some(self.next) {
            return Err("Terminal helper response mismatch.".into());
        }
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(match reply.get("error").and_then(Value::as_str) {
                Some("SESSION_LIMIT") => "SESSION_LIMIT",
                Some("MODEL_LIMIT") => "MODEL_LIMIT",
                Some("INVALID_DIMENSIONS") => "INVALID_DIMENSIONS",
                Some("SNAPSHOT_PENDING") => "SNAPSHOT_PENDING",
                Some("SNAPSHOT_LIMIT") => "SNAPSHOT_LIMIT",
                _ => "Terminal helper rejected the request.",
            }
            .into());
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| "Terminal helper returned an invalid response.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime() -> Runtime {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Runtime {
            sessions: HashMap::new(),
            helper: Some(
                Helper::from_paths(
                    root.join(format!(
                        "binaries/lomi-node-{}{}",
                        env!("LOMI_AI_TARGET"),
                        if cfg!(windows) { ".exe" } else { "" }
                    )),
                    root.join("resources/remote-terminal/index.cjs"),
                )
                .unwrap(),
            ),
        }
    }
    #[test]
    fn remote_helper_native_ipc_isolates_33rd_session_and_model_limit() {
        let mut r = runtime();
        let ids: Vec<_> = (0..33).map(|_| uuid().unwrap()).collect();
        for id in &ids {
            r.apply(RemoteTerminalEvent::Start {
                id: id.clone(),
                cols: 80,
                rows: 24,
            })
            .unwrap();
        }
        assert_eq!(r.sessions.len(), 33);
        assert!(!r.sessions[&ids[32]].available);
        assert!(r.sessions[&ids[32]]
            .unavailable_reason
            .unwrap()
            .contains("32 active desktop terminals"));
        let mut domain = super::super::workspace::Domain::default();
        let epoch = domain.begin().unwrap();
        let workspace_id = uuid().unwrap();
        domain
            .sync(
                &epoch,
                1,
                vec![super::super::workspace::WorkspaceProjection {
                    id: workspace_id.clone(),
                    name: "Refused workspace".into(),
                    terminals: vec![super::super::workspace::TerminalProjection {
                        pane_id: "budget-pane".into(),
                        session_id: Some(ids[32].clone()),
                        title: "Terminal".into(),
                    }],
                }],
            )
            .unwrap();
        assert!(
            super::super::workspace::workspace_unavailable(&domain, &r, &workspace_id)
                .unwrap()
                .contains("32 active desktop terminals")
        );
        assert!(r.sessions[&ids[0]].available);
        r.apply(RemoteTerminalEvent::Output {
            id: ids[0].clone(),
            data: b"survivor".to_vec(),
        })
        .unwrap();
        assert_eq!(r.snapshot(&ids[0]).unwrap()["seq"], 1);
        r.apply(RemoteTerminalEvent::Output {
            id: ids[1].clone(),
            data: b"a".to_vec(),
        })
        .unwrap();
        let data = "\u{0301}".repeat(8000).into_bytes();
        let mut unavailable = false;
        for _ in 0..4 {
            if r.apply(RemoteTerminalEvent::Output {
                id: ids[1].clone(),
                data: data.clone(),
            })
            .unwrap()
            .is_some_and(|v| v["type"] == "unavailable")
            {
                unavailable = true;
                break;
            }
        }
        assert!(unavailable);
        assert!(!r.sessions[&ids[1]].available);
        r.apply(RemoteTerminalEvent::Output {
            id: ids[0].clone(),
            data: b" still alive".to_vec(),
        })
        .unwrap();
        assert_eq!(r.snapshot(&ids[0]).unwrap()["seq"], 2);
        r.apply(RemoteTerminalEvent::Exit {
            id: ids[32].clone(),
            code: Some(0),
        })
        .unwrap();
        r.apply(RemoteTerminalEvent::Exit {
            id: ids[1].clone(),
            code: Some(0),
        })
        .unwrap();
        assert!(r.snapshot(&ids[0]).is_ok());
    }
}

fn unavailable_message(error: &str) -> Option<&'static str> {
    match error {
        "SESSION_LIMIT"=>Some("Remote currently supports independent models for at most 32 active desktop terminals. This workspace cannot be shared completely."),
        "INVALID_DIMENSIONS"=>Some("A workspace terminal exceeds the Remote terminal dimensions limit."),
        "MODEL_LIMIT"|"SNAPSHOT_LIMIT"=>Some("A workspace terminal exceeded the Remote model budget. Restart that terminal before sharing."),
        _=>None,
    }
}
