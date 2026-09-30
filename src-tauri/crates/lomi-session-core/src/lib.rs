//! Local session foundation. No renderer, network transport, or remote authority.
#![cfg(unix)]
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd},
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
pub const MAX_INPUT: usize = 16 * 1024;
pub const MAX_HISTORY: usize = 256 * 1024;
pub const MAX_RECORDS: usize = 4096;
pub const MAX_OBSERVERS: usize = 8;
pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}
impl Size {
    pub fn validate(self) -> Result<Self> {
        if self.rows == 0
            || self.cols == 0
            || self.rows > 200
            || self.cols > 400
            || u32::from(self.rows) * u32::from(self.cols) > 40_000
        {
            return Err("unsupported terminal size".into());
        }
        Ok(self)
    }
    fn pty(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Output {
        bytes: Vec<u8>,
        from: u64,
        to: u64,
    },
    Resize {
        size: Size,
    },
    Ended {
        exit_code: Option<u32>,
        output_incomplete: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub seq: u64,
    pub event: Event,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub profile: String,
    pub session_epoch: Uuid,
    pub initial_size: Size,
    pub size: Size,
    pub model_seq: u64,
    pub output_offset: u64,
    pub alternate_screen: bool,
    pub screen_text: String,
    /// Replay all records into a fresh parser at initial_size, including resize.
    pub replay: Vec<Record>,
}
#[derive(Default)]
enum GuardState {
    #[default]
    Ground,
    Escape,
    Intermediate,
    Csi,
    String {
        osc: bool,
    },
}
#[derive(Default)]
struct EscapeGuard {
    state: GuardState,
    length: usize,
}
impl EscapeGuard {
    fn accept(&mut self, bytes: &[u8]) -> bool {
        for &byte in bytes {
            // CAN/SUB cancel every control sequence; ESC starts a new one.
            if byte == 0x18 || byte == 0x1a {
                self.state = GuardState::Ground;
                self.length = 0;
                continue;
            }
            if byte == 0x1b {
                self.state = GuardState::Escape;
                self.length = 0;
                continue;
            }
            match self.state {
                GuardState::Ground => {}
                GuardState::Escape => match byte {
                    // vte retains Escape across C0, DEL and non-dispatch bytes.
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff => {}
                    0x20..=0x2f => self.state = GuardState::Intermediate,
                    b']' => self.state = GuardState::String { osc: true },
                    b'P' | b'X' | b'^' | b'_' => self.state = GuardState::String { osc: false },
                    b'[' => self.state = GuardState::Csi,
                    _ => self.state = GuardState::Ground,
                },
                GuardState::Intermediate => {
                    if (0x30..=0x7e).contains(&byte) {
                        self.state = GuardState::Ground;
                    }
                }
                GuardState::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.state = GuardState::Ground;
                    }
                }
                GuardState::String { osc } => {
                    if osc && byte == 0x07 {
                        self.state = GuardState::Ground;
                        self.length = 0;
                    } else {
                        self.length += 1;
                        if self.length > 4096 {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

struct State {
    guard: EscapeGuard,
    supported: bool,
    parser: vt100::Parser,
    size: Size,
    seq: u64,
    offset: u64,
    history: VecDeque<Record>,
    history_bytes: usize,
    truncated: bool,
    ended: bool,
    observers: Vec<(Uuid, SyncSender<Record>)>,
}
impl State {
    fn publish(&mut self, event: Event) {
        if self.ended {
            return;
        }
        match &event {
            Event::Output { bytes, to, .. } => {
                if self.supported {
                    self.supported = self.guard.accept(bytes);
                    if self.supported {
                        self.parser.process(bytes);
                    }
                }
                self.offset = *to;
            }
            Event::Resize { size } => {
                self.parser.screen_mut().set_size(size.rows, size.cols);
                self.size = *size;
            }
            Event::Ended { .. } => self.ended = true,
        }
        self.seq += 1;
        let record = Record {
            seq: self.seq,
            event,
        };
        self.history_bytes += record_cost(&record);
        self.history.push_back(record.clone());
        while self.history_bytes > MAX_HISTORY || self.history.len() > MAX_RECORDS {
            if let Some(old) = self.history.pop_front() {
                self.history_bytes -= record_cost(&old);
                self.truncated = true;
            }
        }
        // A slow observer loses its subscription, never arbitrary bytes inside it.
        self.observers
            .retain(|(_, sender)| sender.try_send(record.clone()).is_ok());
    }
}
fn record_cost(record: &Record) -> usize {
    match &record.event {
        Event::Output { bytes, .. } => bytes.len() + 64,
        _ => 64,
    }
}
struct Inner {
    epoch: Uuid,
    initial: Size,
    state: Mutex<State>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    input: Mutex<()>,
    fd: File,
}
#[derive(Clone)]
pub struct Session(Arc<Inner>);
pub struct Observer {
    pub snapshot: Snapshot,
    pub events: Receiver<Record>,
    owner: Weak<Inner>,
    id: Uuid,
}
impl Drop for Observer {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            owner
                .state
                .lock()
                .unwrap()
                .observers
                .retain(|(id, _)| *id != self.id);
        }
    }
}
impl Session {
    pub fn spawn(program: &str, args: &[String], cwd: &str, size: Size) -> Result<Self> {
        size.validate()?;
        if program.is_empty()
            || program.len() > 4096
            || args.len() > 64
            || args.iter().map(String::len).sum::<usize>() > MAX_INPUT
            || cwd.len() > 4096
        {
            return Err("command limit".into());
        }
        let pair = native_pty_system()
            .openpty(size.pty())
            .map_err(|e| e.to_string())?;
        let raw = pair
            .master
            .as_raw_fd()
            .ok_or("PTY has no native descriptor")?;
        let duplicated = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicated < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let fd = unsafe { File::from_raw_fd(duplicated) };
        let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let session = Self(Arc::new(Inner {
            epoch: Uuid::new_v4(),
            initial: size,
            state: Mutex::new(State {
                guard: EscapeGuard::default(),
                supported: true,
                parser: vt100::Parser::new(size.rows, size.cols, 100),
                size,
                seq: 0,
                offset: 0,
                history: VecDeque::new(),
                history_bytes: 0,
                truncated: false,
                ended: false,
                observers: vec![],
            }),
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            input: Mutex::new(()),
            fd,
        }));
        let owned = session.clone();
        thread::spawn(move || owned.read_loop());
        Ok(session)
    }
    pub fn epoch(&self) -> Uuid {
        self.0.epoch
    }
    pub fn ended(&self) -> bool {
        self.0.state.lock().unwrap().ended
    }
    fn snapshot_locked(&self, s: &State) -> Result<Snapshot> {
        if !s.supported {
            return Err("unsupported_model: control string exceeds 4096 bytes".into());
        }
        if s.truncated {
            return Err("resync_required: initial replay history evicted; full snapshot serializer not qualified".into());
        }
        Ok(Snapshot {
            profile: "vt100-0.16-replay-v1-unqualified".into(),
            session_epoch: self.epoch(),
            initial_size: self.0.initial,
            size: s.size,
            model_seq: s.seq,
            output_offset: s.offset,
            alternate_screen: s.parser.screen().alternate_screen(),
            screen_text: s.parser.screen().contents(),
            replay: s.history.iter().cloned().collect(),
        })
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        self.snapshot_locked(&self.0.state.lock().unwrap())
    }
    pub fn subscribe(&self) -> Result<Observer> {
        let mut s = self.0.state.lock().unwrap();
        if s.observers.len() >= MAX_OBSERVERS {
            return Err("observer limit".into());
        }
        let snapshot = self.snapshot_locked(&s)?;
        let (tx, rx) = mpsc::sync_channel(32);
        let id = Uuid::new_v4();
        if !s.ended {
            s.observers.push((id, tx));
        }
        Ok(Observer {
            snapshot,
            events: rx,
            owner: Arc::downgrade(&self.0),
            id,
        })
    }
    pub fn replay_after(&self, seq: u64) -> Result<Vec<Record>> {
        let s = self.0.state.lock().unwrap();
        if !s.supported {
            return Err("unsupported_model: control string exceeds 4096 bytes".into());
        }
        if seq > s.seq || s.history.front().is_some_and(|r| seq < r.seq - 1) {
            return Err("resync_required: journal gap".into());
        }
        Ok(s.history.iter().filter(|r| r.seq > seq).cloned().collect())
    }
    pub fn input(&self, bytes: &[u8]) -> Result<usize> {
        if bytes.len() > MAX_INPUT {
            return Err("input limit".into());
        }
        let _mutation = self.0.input.lock().unwrap();
        if self.ended() {
            return Err("session ended".into());
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut written = 0;
        while written < bytes.len() {
            let n = unsafe {
                libc::write(
                    self.0.fd.as_raw_fd(),
                    bytes[written..].as_ptr().cast(),
                    (bytes.len() - written).min(4096),
                )
            };
            if n > 0 {
                written += n as usize;
                continue;
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::WouldBlock && err.kind() != io::ErrorKind::Interrupted {
                return Err(format!(
                    "outcome_unknown: {written} bytes dispatched: {err}"
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "outcome_unknown: {written} bytes dispatched: input timeout"
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(written)
    }
    pub fn resize(&self, size: Size) -> Result<()> {
        size.validate()?;
        let _mutation = self.0.input.lock().unwrap();
        let mut s = self.0.state.lock().unwrap();
        if s.ended {
            return Err("session ended".into());
        }
        self.0
            .master
            .lock()
            .unwrap()
            .resize(size.pty())
            .map_err(|e| e.to_string())?;
        s.publish(Event::Resize { size });
        Ok(())
    }
    /// Requests termination of the owned child. Completion is only an Ended record.
    pub fn close(&self) -> Result<()> {
        let _mutation = self.0.input.lock().unwrap();
        if self.ended() {
            return Ok(());
        }
        self.0
            .child
            .lock()
            .unwrap()
            .kill()
            .map_err(|e| e.to_string())
    }
    fn read_loop(&self) {
        let mut buf = [0u8; 16 * 1024];
        let mut eof = false;
        let mut exit = None;
        let mut drain = None;
        let mut read_error = false;
        loop {
            if exit.is_none() {
                match self.0.child.lock().unwrap().try_wait() {
                    Ok(Some(status)) => {
                        exit = Some(status.exit_code());
                        drain = Some(Instant::now());
                    }
                    Ok(None) => {}
                    Err(_) => {
                        read_error = true;
                    }
                }
            }
            if !eof {
                let n = unsafe {
                    libc::read(self.0.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len())
                };
                if n > 0 {
                    let mut s = self.0.state.lock().unwrap();
                    let from = s.offset;
                    s.publish(Event::Output {
                        bytes: buf[..n as usize].to_vec(),
                        from,
                        to: from + n as u64,
                    });
                } else if n == 0 {
                    eof = true;
                } else {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::EIO) {
                        eof = true;
                    } else if err.kind() != io::ErrorKind::WouldBlock
                        && err.kind() != io::ErrorKind::Interrupted
                    {
                        eof = true;
                        read_error = true;
                    }
                }
            }
            if exit.is_some()
                && (eof || drain.is_some_and(|d| d.elapsed() > Duration::from_secs(2)))
            {
                self.0.state.lock().unwrap().publish(Event::Ended {
                    exit_code: exit,
                    output_incomplete: !eof || read_error,
                });
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    #[test]
    fn escape_c0_and_del_do_not_hide_osc_across_chunks() {
        for byte in [0x00, 0x07, 0x09, 0x0a, 0x19, 0x1f, 0x7f] {
            let mut guard = EscapeGuard::default();
            assert!(guard.accept(&[0x1b, byte]));
            assert!(guard.accept(b"]0;"));
            assert!(!guard.accept(&vec![b'x'; 5000]));
        }
    }
    #[test]
    fn cancelled_strings_and_intermediate_escapes_stay_bounded() {
        let mut guard = EscapeGuard::default();
        assert!(guard.accept(b"\x1b]0;abc\x18"));
        assert!(guard.accept(&vec![b'x'; 5000]));
        for starter in *b"PX^_" {
            let mut guard = EscapeGuard::default();
            assert!(guard.accept(&[0x1b, starter]));
            assert!(!guard.accept(&vec![b'x'; 5000]));
        }
    }
}
