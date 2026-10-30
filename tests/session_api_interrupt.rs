//! The §6.4 signal face of the Session API, in its own
//! integration-test binary on purpose: signal delivery is a
//! process-level queue, and a queued signal must never be
//! serviced by a neighbouring test binary's executor. One
//! process, two sessions, one interrupt.
//!
//! The adjudication this cell pins is the engine's own (and
//! GNU's): the delivery queue dispatches **trapped** signals at
//! command boundaries; an untrapped fatal signal is adjudicated
//! by kernel disposition, so a queued untrapped SIGINT in a
//! non-interactive session is dropped as stale by the boundary
//! poll — its kernel form ends the process itself, which no
//! in-process outcome can represent (and an in-process test
//! must not try). Interactive sessions are the other routed
//! case (the rubash#1662 pin installs the SIGINT handler
//! there); that face is covered by the host PTY suites, not
//! here.
//!
//! The shape: a run is in progress on one session's thread
//! (sessions are one-per-thread, design Q5) with an INT trap
//! installed; a peer session's `interrupt()` — the call a
//! host's console path makes — queues SIGINT through the
//! process delivery queue; the runner's boundary poll dispatches
//! the trap action mid-run, and the run then completes
//! normally (a trap that does not exit lets execution resume,
//! exactly as under GNU).

use std::path::PathBuf;
use std::time::Duration;

use rubash::api::{RunOptions, Session};

#[test]
fn queued_sigint_dispatches_the_trap_at_a_boundary() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-session-api-interrupt-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let started: PathBuf = dir.join("loop-started.txt");
    let trapfile: PathBuf = dir.join("trap-fired.txt");
    let marker: PathBuf = dir.join("loop-done.txt");
    let started_text = started.display().to_string();
    let trap_text = trapfile.display().to_string();
    let marker_text = marker.display().to_string();

    let runner = std::thread::spawn(move || {
        let mut session = Session::builder().build().expect("runner session builds");
        let outcome = session
            .run(
                &format!(
                    "trap 'echo trapped > \"{trap_text}\"' INT; \
                     : > \"{started_text}\"; \
                     for i in {{1..50000}}; do :; done; \
                     echo done > \"{marker_text}\""
                ),
                &RunOptions::default(),
            )
            .expect("the trapped run is an outcome, not an error");
        // The session continues after a dispatched trap.
        let after = session
            .run("echo still-alive", &RunOptions::default())
            .expect("the session is usable after the trap");
        (outcome.status, after.status)
    });

    // Wait until the run has provably started — the trap is the
    // script's first command, so the handshake file's existence
    // also proves the trap is installed — then interrupt from
    // the peer session.
    let mut waited = Duration::ZERO;
    while !started.exists() {
        assert!(
            waited < Duration::from_secs(60),
            "the runner's first commands never executed"
        );
        std::thread::sleep(Duration::from_millis(25));
        waited += Duration::from_millis(25);
    }
    let mut peer = Session::builder().build().expect("peer session builds");
    peer.interrupt();

    let (status, after_status) = runner.join().expect("runner thread");
    assert_eq!(
        status, 0,
        "a trap that does not exit lets the run complete (GNU)"
    );
    assert_eq!(
        std::fs::read_to_string(&trapfile).expect("the trap action ran"),
        "trapped\n",
        "the queued SIGINT dispatched the INT trap mid-run"
    );
    assert!(marker.exists(), "the run completed after the trap");
    assert_eq!(after_status, 0, "the session continues after the trap");
}
