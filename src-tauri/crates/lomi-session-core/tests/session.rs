#![cfg(unix)]
use lomi_session_core::{Event, Session, Size};
use std::{
    thread,
    time::{Duration, Instant},
};
fn session(script: &str) -> Session {
    Session::spawn(
        "/bin/sh",
        &["-c".into(), script.into()],
        "/",
        Size { rows: 24, cols: 80 },
    )
    .unwrap()
}
fn finished(session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !session.ended() {
        assert!(Instant::now() < deadline, "session did not finish");
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn observer_detach_and_slow_observer_do_not_kill_child() {
    let session = session("printf start; sleep 0.2; printf alive; exit 7");
    let observer = session.subscribe().unwrap();
    drop(observer);
    finished(&session);
    let snapshot = session.snapshot().unwrap();
    assert!(snapshot.screen_text.contains("startalive"));
    assert!(matches!(
        snapshot.replay.last().unwrap().event,
        Event::Ended {
            exit_code: Some(7),
            output_incomplete: false
        }
    ));
}
#[test]
fn unicode_alternate_screen_and_resize_replay() {
    let session = session(
        r"printf '\033[?1049h'; printf '界e'; printf '\314'; sleep 0.1; printf '\201'; sleep 0.4; printf '\033[?1049l'; printf 'normal' ",
    );
    thread::sleep(Duration::from_millis(250));
    session
        .resize(Size {
            rows: 30,
            cols: 100,
        })
        .unwrap();
    let snapshot = session.snapshot().unwrap();
    assert!(snapshot.alternate_screen);
    assert!(snapshot.screen_text.contains("界e\u{301}"));
    let mut parser =
        vt100::Parser::new(snapshot.initial_size.rows, snapshot.initial_size.cols, 1000);
    for record in &snapshot.replay {
        match &record.event {
            Event::Output { bytes, .. } => parser.process(bytes),
            Event::Resize { size } => parser.screen_mut().set_size(size.rows, size.cols),
            _ => {}
        }
    }
    assert_eq!(parser.screen().contents(), snapshot.screen_text);
    for record in session.replay_after(snapshot.model_seq).unwrap() {
        assert!(record.seq > snapshot.model_seq);
    }
    finished(&session);
    let suffix = session.replay_after(snapshot.model_seq).unwrap();
    for record in suffix {
        match record.event {
            Event::Output { bytes, .. } => parser.process(&bytes),
            Event::Resize { size } => parser.screen_mut().set_size(size.rows, size.cols),
            _ => {}
        }
    }
    assert!(!parser.screen().alternate_screen());
    assert!(parser.screen().contents().contains("normal"));
}
#[test]
fn inherited_descriptor_drain_finishes_with_confirmed_child_exit() {
    let session = session("trap '' HUP; sleep 3 & printf done; exit 0");
    finished(&session);
    assert!(matches!(
        session.snapshot().unwrap().replay.last().unwrap().event,
        Event::Ended {
            exit_code: Some(0),
            output_incomplete: _
        }
    ));
}
#[test]
fn history_and_input_limits_are_explicit() {
    let session = session("head -c 300000 /dev/zero; exit 0");
    finished(&session);
    assert!(session.snapshot().unwrap_err().contains("evicted"));
    assert!(session.replay_after(0).unwrap_err().contains("gap"));
    assert!(session
        .input(&vec![0; 16 * 1024 + 1])
        .unwrap_err()
        .contains("limit"));
    assert!(Size { rows: 0, cols: 80 }.validate().is_err());
}

#[test]
fn slow_observer_is_detached_and_idle_detach_releases_slot() {
    let session =
        session("i=0; while [ $i -lt 60 ]; do printf x; sleep 0.015; i=$((i+1)); done; exit 0");
    for _ in 0..20 {
        drop(session.subscribe().unwrap());
    }
    let observer = session.subscribe().unwrap();
    finished(&session);
    let records: Vec<_> = observer.events.try_iter().collect();
    assert_eq!(records.len(), 32);
    assert!(observer.events.recv().is_err());
    assert!(session.snapshot().unwrap().screen_text.contains("xxxxxxxx"));
}

#[test]
fn oversized_osc_marks_model_unsupported_without_unbounded_parser_storage() {
    let session =
        session(r"printf '\033]0;'; head -c 5000 /dev/zero | tr '\000' x; printf '\007'; exit 0");
    finished(&session);
    assert!(session
        .snapshot()
        .unwrap_err()
        .contains("unsupported_model"));
    assert!(session
        .replay_after(0)
        .unwrap_err()
        .contains("unsupported_model"));
}

#[test]
fn split_escape_nul_osc_marks_snapshot_and_replay_unsupported() {
    let session = session(
        r"printf '\033\000'; sleep 0.05; printf ']0;'; head -c 5000 /dev/zero | tr '\000' x; printf '\007'; exit 0",
    );
    finished(&session);
    assert!(session
        .snapshot()
        .unwrap_err()
        .contains("unsupported_model"));
    assert!(session
        .replay_after(0)
        .unwrap_err()
        .contains("unsupported_model"));
}
