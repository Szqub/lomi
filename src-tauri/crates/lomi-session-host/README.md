# Local session host foundation

Run `lomi-session-host --socket-dir /absolute/canonical/private-directory`.
The directory is created 0700, `host.sock` is 0600, and `owner.lock` is 0600.
The lock is never unlinked. A nonblocking exclusive flock precedes stale socket
cleanup. Symlink paths, unexpected socket entries, unsafe directory permissions,
and invalid owner lock metadata are rejected. Unix peer credentials must match
the host effective UID. No TCP listener or remote control is implemented.

IPC uses newline-delimited JSON, version 1. For example:

```json
{"version":1,"command":"status"}
{"version":1,"command":"create","program":"/bin/sh","args":[],"cwd":"/","size":{"rows":24,"cols":80}}
{"version":1,"command":"list"}
{"version":1,"command":"snapshot","session_epoch":"UUID from create"}
{"version":1,"command":"replay","session_epoch":"UUID from create","after":0}
{"version":1,"command":"input","session_epoch":"UUID from create","bytes":[108,115,10]}
{"version":1,"command":"resize","session_epoch":"UUID from create","size":{"rows":30,"cols":100}}
{"version":1,"command":"close","session_epoch":"UUID from create"}
```

Responses contain `version`, `ok`, and either `result` or `error`. Input success
means bytes dispatched only. Close on a running session requests child
termination and reports pending; close on an ended session removes the retained
record and releases its slot. Client EOF or write failure never closes a PTY.
There is no IPC live subscription yet; the core provides independent bounded
observers, and IPC clients can poll snapshot/replay.

Limits: 16 clients, 32 retained sessions, 128 KiB request frames, 2 MiB reply
frames, ten-second read inactivity timeout and two-second write timeout. Core
bounds and snapshot qualifications are documented in the sibling crate.

This development endpoint trusts every process with the same UID for local
terminal operations. It does not authenticate an approved application identity,
provide challenge-bound process proofs, or implement account/key administration.
It must not expose enrollment, grants, secrets, or remote transport. Raw terminal
replay is untrusted; a future renderer adapter must disable unsafe OSC effects.
No LaunchAgent, SMAppService approval, background opt-in, auth migration,
key-store ownership, lock-screen policy, shutdown service management, or durable
host/session identity is implemented. S02/S03 are not accepted by this foundation.

Run `cargo test --manifest-path Cargo.toml --locked`. Tests start actual host
subprocesses and demonstrate reconnect after IPC disconnect, ownership lock,
private permissions, and rejection of unsafe paths. This crate uses an
independent workspace until the main workspace integrates it.

## Isolated prototype result and integration work

Verified on macOS arm64: three actual host subprocess tests pass for reconnect
following IPC disconnect, retained PTY output/exit, private permissions, exclusive
ownership, unsafe directory/lock rejection, and local input/resize/close
completion. Strict Clippy and Rust formatting checks pass. Cross-UID rejection
uses platform peer credentials but was not exercised with a second OS user.
These results do not accept S03 or authorize background/remote operation.

Before integration, remove the standalone workspace marker and reconcile the
lockfile through the parent workspace. Add authenticated application roles and
challenge-bound process proof, live bounded IPC subscriptions, graceful service
shutdown, approved macOS service installation and background opt-in, durable
host/session identity, safe auth/key-store ownership migration, and lock-screen
policy. Connect the Tauri desktop only after its current PTY lifecycle is
explicitly migrated and validated. Do not expose this same-UID local command
surface to a renderer plugin or relay as an administrative proxy.
