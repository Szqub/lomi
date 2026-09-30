# Session core foundation

Tauri-free, Unix-only portable-pty ownership with a continuously maintained
vt100 0.16.2 screen. Session handles and observer lifetimes are independent.
Dropping an observer removes only that observer. Slow observers are disconnected
when their 32-record queue fills; callers must obtain a new snapshot before
continuing. Snapshot capture and subscription installation use the same model
lock. Output, resize, and the final exit record share one ordered journal.

The provisional `vt100-0.16-replay-v1-unqualified` snapshot contains the complete
bounded journal from initial size, including resize events, rather than claiming
that formatted visible cells reproduce both buffers, saved cursors, and parser
state. Replay it into a fresh parser at `initial_size`, then apply records after
`model_seq`. This preserves partial UTF-8/escape state without a lossy screen
serialization. If initial history is evicted, snapshot returns `resync_required`.
Replay requests return an explicit gap when their sequence is no longer retained.

Limits: 16 KiB input and output chunks; 256 KiB journal accounting plus 4096
records; eight observers; 32 queued records per observer; 200 rows, 400 columns,
40,000 screen cells; 100 scrollback rows. OSC/DCS/APC/PM/SOS control strings
exceeding 4096 bytes mark the model unsupported and stop feeding the upstream parser (whose OSC
storage is otherwise unbounded); snapshot/replay then return an explicit error.
PTY ownership and exit observation continue. The supported profile still needs
xterm equivalence tests and an independent security review.

Native input is nonblocking, serialized with resize/close, written in <=4 KiB
pieces with a two-second budget. An error states how many bytes were dispatched;
it does not mean the program executed them. No mutation deduplication, remote
lease, remote permission, or durable receipt protocol is implemented here.

An `Ended` record follows confirmed child exit and PTY EOF, or a two-second drain
budget marked `output_incomplete`. Close requests call portable-pty's owned child
kill operation; they do not claim graceful termination, descendant containment,
or completion until the final record is observed. Host death loses this epoch;
there is no persistence, process adoption, or transparent PTY migration.

Run `cargo test --manifest-path Cargo.toml --locked` from this crate directory.
This crate uses an independent workspace until the main workspace integrates it.

## Isolated prototype result and integration work

Verified on macOS arm64: two parser-guard unit tests and seven PTY integration
tests pass, including observer detach, slow-observer isolation, split UTF-8,
alternate screen/resize replay, bounded history, confirmed exit, and split
ESC/NUL OSC rejection. Strict Clippy and Rust formatting checks pass.
These results do not accept S02/S03 or establish the complete xterm profile.

Before integration, remove this independent workspace marker, add the crate to
the parent workspace, and reconcile its lockfile through an explicit dependency
review. Migrate Tauri PTY creation and local MCP/UI mutations to one authoritative
owner; add stable session IDs distinct from process epochs, reconnect/layout
migration, exact snapshot serialization after history eviction, receipt/lease
arbitration, and qualified process-group cleanup. Existing desktop ownership
must remain the active path until those transitions and their tests are ready.
