//! rubash#2810: the Session API's cooperative cancellation token —
//! the host-held, signal-free way to end a runaway run.
//!
//! What this cell pins:
//!   * an unbounded `while` loop ends with [`Flow::Cancelled`] a
//!     bounded time after `CancelToken::cancel()` lands from another
//!     thread — and reports the distinct classification fact, never
//!     a spoofed SIGINT outcome;
//!   * the in-process `sleep` wait loop honors the same token
//!     promptly (50ms-slice cadence);
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
    // emulation's 50ms-slice loop. On unix `sleep` can dispatch as a
    // real external child, and preempting a blocked child is the
    // issue's documented non-goal — so the prompt-return pin lives
    // on the platforms where the builtin stays in-process.
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
