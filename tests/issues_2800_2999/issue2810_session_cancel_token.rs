//! rubash#2810: the Session API's cooperative cancellation token —
//! the host-held, signal-free way to end a runaway run.
//!
//! What this cell pins:
//!   * an unbounded `while` loop ends with [`Flow::Cancelled`] a
//!     bounded time after `CancelToken::cancel()` lands from another
//!     thread — and reports the distinct classification fact, never
//!     a spoofed SIGINT outcome;
//!   * the in-process `sleep` wait loop honors the same token
//!     promptly (50ms-slice cadence) — on non-unix, where the
//!     emulation is the dispatch; on unix `sleep` is a real child
//!     (#3488 Yield) and the token must NOT preempt it (see the
//!     unix sleep pin below);
//!   * the `read -t` acquisition and the `select` query abandon
//!     on the token and the run reports the distinct outcome;
//!   * a cancel landing mid-`sleep` / mid-`wait` on a real
//!     external child does not preempt the child — the child
//!     runs to completion and the run still reports Cancelled
//!     at the next safe point (the unit-end conversion);
//!   * the session's state survives the cancelled run and the next
//!     run works once the host resets the token;
//!   * cancellation is not a script-visible event: a `trap INT`
//!     action never fires for it.
//!
//! The `Session` itself is `!Send` — the run executes on this test's
//! thread while the cloneable `CancelToken` (which is `Send + Sync`)
//! rides a helper thread, exactly as an embedding host's console
//! would drive it.

use std::time::{Duration, Instant};

use rubash::api::{CancelToken, Flow, RunOptions, Session};

/// Arm a helper thread that cancels the token after `delay`. The
/// token is the only thing the thread owns — the session never
/// leaves the test thread.
fn cancel_after(token: CancelToken, delay: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        token.cancel();
    });
}

#[test]
fn cancelled_loop_reports_the_distinct_outcome() {
    let mut session = Session::builder().build().expect("session builds");
    let token = session.cancellation_token();
    assert!(!token.is_cancelled(), "a fresh token starts unset");

    cancel_after(token.clone(), Duration::from_millis(150));

    let started = Instant::now();
    let outcome = session
        .run("while true; do :; done", &RunOptions::default())
        .expect("a cancelled run is an outcome, not an error");
    let elapsed = started.elapsed();

    // The poll cadence is per-statement; the bound is generous
    // (seconds, not the loop's unbounded lifetime).
    assert!(
        elapsed < Duration::from_secs(10),
        "cancelled loop did not terminate promptly: {elapsed:?}"
    );
    assert!(
        token.is_cancelled(),
        "the token still reads set after the run"
    );
    assert_eq!(
        outcome.flow,
        Flow::Cancelled,
        "the cancelled run must report the distinct flow"
    );
    assert!(
        outcome.classification.cancelled,
        "the classification fact is populated beside the flow"
    );
    assert_eq!(
        outcome.classification.unwind, None,
        "a cancel is not a script unwind class"
    );
    assert_ne!(
        outcome.status, 130,
        "cancellation must not spoof the SIGINT status"
    );
    assert!(
        !outcome.classification.signal_discard,
        "a cancel is not a signal discard"
    );
}

#[test]
fn cancelled_sleep_returns_promptly() {
    // The engine-side wait this exercises is the in-process sleep
    // emulation's 50ms-slice loop, which is the `sleep` dispatch
    // only on non-unix. On unix `sleep` is a real spawned child
    // (#3488 Yield), so the prompt-return shape does not exist
    // there — see cancelled_sleep_on_unix_does_not_preempt_the_child.
    #[cfg(unix)]
    {
        eprintln!("skipped on unix: sleep dispatches as an external child");
        return;
    }
    #[cfg(not(unix))]
    {
        let mut session = Session::builder().build().expect("session builds");
        let token = session.cancellation_token();
        cancel_after(token.clone(), Duration::from_millis(200));

        let started = Instant::now();
        let outcome = session
            .run("sleep 60", &RunOptions::default())
            .expect("a cancelled sleep is an outcome, not an error");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(10),
            "cancelled sleep did not return promptly: {elapsed:?}"
        );
        assert_eq!(outcome.flow, Flow::Cancelled);
        assert!(outcome.classification.cancelled);
    }
}

#[test]
fn cancelled_sleep_on_unix_does_not_preempt_the_child() {
    // Unix-only: `sleep` dispatches as a real external child, and
    // preempting a blocked child is the issue's documented
    // non-goal — so the token must NOT cut the child short. The
    // discrimination is the elapsed time (a preempting token
    // would end the run in ~0.2s; the real child takes the full
    // 2s), plus the boundary semantics: the run still reports
    // Cancelled — the unit-end conversion in execute_ast lifts
    // the still-set token after the last command — with the
    // completed sleep's status 0, and the statement after the
    // sleep never runs. On pristine this pin fails both ways
    // (Completed, `x` set), so it is fail-first, not a guard.
    #[cfg(not(unix))]
    {
        eprintln!("unix-only: the in-process sleep emulation is the prompt-return pin above");
        return;
    }
    #[cfg(unix)]
    {
        let mut session = Session::builder().build().expect("session builds");
        let token = session.cancellation_token();
        cancel_after(token.clone(), Duration::from_millis(150));

        let started = Instant::now();
        let outcome = session
            .run("sleep 2; x=SURVIVED", &RunOptions::default())
            .expect("a cancelled run is an outcome, not an error");
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(1500),
            "the external sleep child must run to completion, not be preempted: {elapsed:?}"
        );
        assert_eq!(outcome.flow, Flow::Cancelled);
        assert!(outcome.classification.cancelled);
        assert_eq!(
            outcome.status, 0,
            "the completed sleep's status stands — no status is synthesized"
        );
        assert!(
            session.variable_value("x").is_none(),
            "the statement after the sleep must not run"
        );
    }
}

#[test]
fn cancelled_read_returns_promptly() {
    // `read -t` blocks in the deadline loop (read_io.rs
    // `read_wait_timed_out`, one token poll per 50ms slice).
    // The procsub pipe stays open with no data and no EOF for
    // 5s, so the read would block the whole deadline without
    // the token; the cancel abandons the acquisition and the
    // boundary poll raises the dedicated unwind. On pristine
    // the same script waits out the procsub and completes,
    // so this pin is fail-first.
    let mut session = Session::builder().build().expect("session builds");
    let token = session.cancellation_token();
    cancel_after(token.clone(), Duration::from_millis(150));

    let started = Instant::now();
    let outcome = session
        .run("read -t 60 x < <(sleep 5)", &RunOptions::default())
        .expect("a cancelled read is an outcome, not an error");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "cancelled read did not return promptly: {elapsed:?}"
    );
    assert_eq!(outcome.flow, Flow::Cancelled);
    assert!(outcome.classification.cancelled);
    assert_ne!(
        outcome.status, 130,
        "cancellation must not spoof the SIGINT status"
    );
    assert!(
        session.variable_value("x").is_none(),
        "the abandoned read assigns nothing"
    );
}

#[test]
fn cancelled_select_returns_promptly() {
    // The `select` query waits in 50ms slices that poll the
    // token (select_exec.rs); on cancel the acquisition
    // abandons as EOF and the unit-end conversion lifts the
    // still-set token to the dedicated unwind. The procsub
    // pipe stays open with no data for 5s, so without the
    // token the query would block until EOF. On pristine the
    // same script waits out the procsub and completes, so
    // this pin is fail-first.
    let mut session = Session::builder().build().expect("session builds");
    let token = session.cancellation_token();
    cancel_after(token.clone(), Duration::from_millis(150));

    let started = Instant::now();
    let outcome = session
        .run(
            "select v in a b c; do echo got-$v; break; done < <(sleep 5)",
            &RunOptions::default(),
        )
        .expect("a cancelled select is an outcome, not an error");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "cancelled select did not return promptly: {elapsed:?}"
    );
    assert_eq!(outcome.flow, Flow::Cancelled);
    assert!(outcome.classification.cancelled);
    assert_ne!(
        outcome.status, 130,
        "cancellation must not spoof the SIGINT status"
    );
}

#[test]
fn cancellation_does_not_preempt_an_external_wait() {
    // Unix-only: the `wait` builtin's wait on a real child is
    // not a safe point (the issue's non-goal, same class as
    // the unix sleep pin above). The child runs to completion
    // (elapsed proves it) and the run still reports
    // Cancelled at the next boundary. The 500ms cancel delay
    // leaves ample margin for the background spawn to finish
    // before the token lands, so the boundary poll before
    // `wait` cannot trip first. On pristine the script runs
    // to completion with `x` set, so this pin is fail-first.
    #[cfg(not(unix))]
    {
        eprintln!("unix-only: the waited-for child is a real external child");
        return;
    }
    #[cfg(unix)]
    {
        let mut session = Session::builder().build().expect("session builds");
        let token = session.cancellation_token();
        cancel_after(token.clone(), Duration::from_millis(500));

        let started = Instant::now();
        let outcome = session
            .run("sleep 3 & wait; x=SURVIVED", &RunOptions::default())
            .expect("a cancelled run is an outcome, not an error");
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(2500),
            "the waited-for child must run to completion: {elapsed:?}"
        );
        assert_eq!(outcome.flow, Flow::Cancelled);
        assert!(outcome.classification.cancelled);
        assert_eq!(
            outcome.status, 0,
            "the completed wait's status stands — no status is synthesized"
        );
        assert!(
            session.variable_value("x").is_none(),
            "the statement after the wait must not run"
        );
    }
}

#[test]
fn session_state_survives_and_reset_re_arms() {
    let mut session = Session::builder().build().expect("session builds");
    let token = session.cancellation_token();

    // State the cancelled run must not lose.
    session
        .run("x=42; f() { echo fn; }", &RunOptions::default())
        .expect("setup run");

    cancel_after(token.clone(), Duration::from_millis(150));
    let outcome = session
        .run("while true; do :; done", &RunOptions::default())
        .expect("cancelled run is an outcome");
    assert_eq!(outcome.flow, Flow::Cancelled);

    // Variables and functions are intact; the token is one-way, so
    // while it still stands a further run ends at its first safe
    // point — the explicit-reset contract in action.
    assert_eq!(session.variable_value("x").as_deref(), Some("42"));
    let still_set = session
        .run("f", &RunOptions::default())
        .expect("a run against the standing token is still an outcome");
    assert_eq!(
        still_set.flow,
        Flow::Cancelled,
        "the un-reset token keeps ending runs at their first safe point"
    );

    token.reset();
    assert!(!token.is_cancelled());

    let after = session
        .run("f; y=$x", &RunOptions::default())
        .expect("post-reset run");
    assert_eq!(after.flow, Flow::Completed);
    assert_eq!(after.status, 0);
    assert!(!after.classification.cancelled);
    assert_eq!(session.variable_value("y").as_deref(), Some("42"));
}

#[test]
fn cancellation_never_fires_the_int_trap() {
    let mut session = Session::builder().build().expect("session builds");
    let token = session.cancellation_token();

    // `t=99` is the script-visible marker the INT trap action would
    // leave behind if cooperative cancellation ever rode the signal
    // machinery.
    cancel_after(token.clone(), Duration::from_millis(150));
    let outcome = session
        .run(
            "trap 't=99' INT; while true; do :; done",
            &RunOptions::default(),
        )
        .expect("cancelled run is an outcome");
    assert_eq!(outcome.flow, Flow::Cancelled);

    assert!(
        session.variable_value("t").is_none(),
        "the INT trap action must not run for a cooperative cancel"
    );
}

#[test]
fn no_token_use_means_no_behavioral_difference() {
    let mut session = Session::builder().build().expect("session builds");
    // The token exists (the session always carries one) but nothing
    // ever cancels it — plain runs classify exactly as before.
    let _unused = session.cancellation_token();
    let outcome = session
        .run("echo hi; true", &RunOptions::default())
        .expect("ordinary run");
    assert_eq!(outcome.flow, Flow::Completed);
    assert_eq!(outcome.status, 0);
    assert!(!outcome.classification.cancelled);
}
