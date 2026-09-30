#![cfg(unix)]
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::UnixStream,
    },
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Host(Child);
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(path: &std::path::Path) -> Host {
    Host(
        Command::new(env!("CARGO_BIN_EXE_lomi-session-host"))
            .args(["--socket-dir", path.to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}
fn connect(path: &std::path::Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(stream) = UnixStream::connect(path.join("host.sock")) {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            return stream;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
}
fn request(stream: &mut UnixStream, value: Value) -> Value {
    let mut encoded = serde_json::to_vec(&value).unwrap();
    encoded.push(b'\n');
    stream.write_all(&encoded).unwrap();
    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}
#[test]
fn host_owns_session_across_disconnect_and_enforces_single_owner() {
    let temp = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(temp.path()).unwrap().join("host");
    let _host = start(&path);
    let mut stream = connect(&path);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(path.join("host.sock")).unwrap().mode() & 0o777,
        0o600
    );
    let second = Command::new(env!("CARGO_BIN_EXE_lomi-session-host"))
        .args(["--socket-dir", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("another session host"));
    let status = request(&mut stream, json!({"version":1,"command":"status"}));
    assert_eq!(status["result"]["remote_control"], false);
    let created = request(
        &mut stream,
        json!({"version":1,"command":"create","program":"/bin/sh","args":["-c","printf before; sleep 0.2; printf after; exit 9"],"cwd":"/","size":{"rows":24,"cols":80}}),
    );
    assert_eq!(created["ok"], true, "{created}");
    let id = created["result"]["session_epoch"].as_str().unwrap();
    drop(stream);
    thread::sleep(Duration::from_millis(600));
    let mut stream = connect(&path);
    let snapshot = request(
        &mut stream,
        json!({"version":1,"command":"snapshot","session_epoch":id}),
    );
    assert_eq!(snapshot["ok"], true, "{snapshot}");
    assert!(snapshot["result"]["screen_text"]
        .as_str()
        .unwrap()
        .contains("beforeafter"));
    let records = snapshot["result"]["replay"].as_array().unwrap();
    assert_eq!(records.last().unwrap()["event"]["exit_code"], 9);
    let removed = request(
        &mut stream,
        json!({"version":1,"command":"close","session_epoch":id}),
    );
    assert_eq!(removed["result"]["removed"], true);
    assert_eq!(
        request(&mut stream, json!({"version":99,"command":"list"}))["ok"],
        false
    );
}
#[test]
fn refuses_untrusted_directory_and_lock_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(temp.path()).unwrap().join("host");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let mut host = start(&path);
    assert!(!host.0.wait().unwrap().success());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink("/dev/null", path.join("owner.lock")).unwrap();
    let mut host = start(&path);
    assert!(!host.0.wait().unwrap().success());
}

#[test]
fn local_input_resize_and_close_have_distinct_completion() {
    let temp = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(temp.path()).unwrap().join("host");
    let _host = start(&path);
    let mut stream = connect(&path);
    let created = request(
        &mut stream,
        json!({"version":1,"command":"create","program":"/bin/sh","args":["-c","read line; printf 'got:%s' \"$line\"; sleep 30"],"cwd":"/","size":{"rows":24,"cols":80}}),
    );
    let id = created["result"]["session_epoch"].as_str().unwrap();
    let resized = request(
        &mut stream,
        json!({"version":1,"command":"resize","session_epoch":id,"size":{"rows":30,"cols":100}}),
    );
    assert_eq!(resized["ok"], true);
    let input = request(
        &mut stream,
        json!({"version":1,"command":"input","session_epoch":id,"bytes":b"hello\n".to_vec()}),
    );
    assert_eq!(input["result"]["receipt"], "dispatched");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = request(
            &mut stream,
            json!({"version":1,"command":"snapshot","session_epoch":id}),
        );
        if snapshot["result"]["screen_text"]
            .as_str()
            .unwrap()
            .contains("got:hello")
        {
            assert_eq!(snapshot["result"]["size"]["rows"], 30);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let close = request(
        &mut stream,
        json!({"version":1,"command":"close","session_epoch":id}),
    );
    assert_eq!(close["result"]["completion"], "pending");
    loop {
        let list = request(&mut stream, json!({"version":1,"command":"list"}));
        if list["result"][0]["ended"] == true {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let snapshot = request(
        &mut stream,
        json!({"version":1,"command":"snapshot","session_epoch":id}),
    );
    assert_eq!(
        snapshot["result"]["replay"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["event"]["kind"],
        "ended"
    );
}
