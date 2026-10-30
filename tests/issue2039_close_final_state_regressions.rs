//! rubash#2039: `Session::close()` had no final-state return
//! face — it consumed the session and returned only the `i32`
//! status, so a host whose contract reads session state *after*
//! close (the niu #668 cwd funnel: the `--cwd-state` recorder
//! must read the final directory after the EXIT trap, because a
//! trap's `cd` is the session's last cwd write) was forced to
//! snapshot before closing, and a pre-close snapshot is
//! structurally blind to the EXIT trap's own effects.
//!
//! The fix: `Session::close_with_report` runs the identical
//! exit path and conveys the close boundary as a
//! [`rubash::api::CloseReport`] value (`status` + `cwd`, read
//! after the exit path ran); `Session::close` keeps its exact
//! signature and delegates to it. These pins are written
//! against the public `rubash::api` surface only, the way an
//! embedding host consumes it.

use std::path::PathBuf;

use rubash::api::{CloseReport, RunOptions, Session};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2039-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn opts() -> RunOptions {
    RunOptions::default()
}

/// The close-boundary cwd is read after the EXIT trap ran: a
/// trap whose `cd` moves the session reports the moved-to
/// directory through [`CloseReport::cwd`], while the pre-close
/// [`Session::cwd`] face still reports the pre-trap directory —
/// exactly the read a pre-close snapshot could not get right
/// (the structural premise of rubash#668).
#[test]
fn close_with_report_conveys_the_exit_trap_cwd() {
    let dir = temp_dir("trap-cwd");
    let from = dir.join("from");
    let into = dir.join("into");
    std::fs::create_dir_all(&from).expect("from dir");
    std::fs::create_dir_all(&into).expect("into dir");
    let into_text = into.to_string_lossy().into_owned();

    // Engine-canonical form of the moved-to directory, straight
    // from the cd machinery on a trap-free control session —
    // the pins compare the engine to itself, not to a host path
    // spelling.
    let mut control = Session::builder()
        .working_directory(&from)
        .env("INTO", into_text.clone())
        .build()
        .expect("control session builds");
    let control_cd = control
        .run("cd \"$INTO\"", &opts())
        .expect("control cd runs");
    assert_eq!(control_cd.status, 0);
    let into_canonical = control.cwd().to_path_buf();
    let start_canonical = {
        // A session that never cds: the pre-trap directory form.
        let plain = Session::builder()
            .working_directory(&from)
            .build()
            .expect("plain session builds");
        plain.cwd().to_path_buf()
    };

    let mut session = Session::builder()
        .working_directory(&from)
        .env("INTO", into_text)
        .build()
        .expect("session builds");
    session
        .run("trap 'cd \"$INTO\"' EXIT", &opts())
        .expect("trap armed");
    // Pre-close, the trap has not run: the session cwd face
    // still reports the start directory. This is the only read
    // the pre-#2039 API allowed, and it cannot see the trap.
    assert_eq!(session.cwd(), start_canonical);
    assert_ne!(session.cwd(), into_canonical);

    let report = session.close_with_report();
    assert_eq!(report.status, 0);
    assert_eq!(report.cwd, into_canonical);
    assert_ne!(report.cwd, start_canonical);
}

/// The status half of the report is the close contract's final
/// status, unchanged: an EXIT trap whose body is a bare
/// `exit N` resolves the close to N (GNU trap.c run_exit_trap's
/// trap_saved_exit_value rule), and without a trap override the
/// close returns the entry status.
#[test]
fn close_with_report_status_is_the_exit_path_resolution() {
    // Bare `exit 7` inside the EXIT trap overrides to 7.
    let mut overridden = Session::builder().build().expect("session builds");
    overridden
        .run("trap 'exit 7' EXIT", &opts())
        .expect("trap armed");
    let report: CloseReport = overridden.close_with_report();
    assert_eq!(report.status, 7);

    // No trap override: the entry status survives the close.
    let mut plain = Session::builder().build().expect("session builds");
    let outcome = plain.run("false", &opts()).expect("false runs");
    assert_eq!(outcome.status, 1);
    let report = plain.close_with_report();
    assert_eq!(report.status, 1);
    assert_eq!(report.cwd, plain_cwd_marker());
}

/// `Session::close` keeps its exact 1.5.0 contract: same
/// signature, same status value the exit path resolves — it is
/// `close_with_report().status`. Out-of-tree hosts compiled
/// against the pre-#2039 face keep working unchanged.
#[test]
fn close_keeps_the_i32_status_face() {
    let mut overridden = Session::builder().build().expect("session builds");
    overridden
        .run("trap 'exit 7' EXIT", &opts())
        .expect("trap armed");
    let status: i32 = overridden.close();
    assert_eq!(status, 7);

    // The full finish duties stay inside close: the EXIT trap
    // runs (session_api's close pin covers trap + procsub
    // delivery; here the trap's file side effect pins it again
    // through the retained face).
    let dir = temp_dir("close-delegate");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = Session::builder()
        .env("OUT", dir_text)
        .build()
        .expect("session builds");
    session
        .run("trap 'echo trapped > \"$OUT/trap.txt\"' EXIT", &opts())
        .expect("trap armed");
    assert_eq!(session.close(), 0);
    let written =
        std::fs::read_to_string(dir.join("trap.txt")).expect("trap file written by close");
    assert_eq!(written, "trapped\n");
}

/// The pre-close cwd read on a fresh session (shared by the
/// status pin above so its assertion set stays self-contained).
fn plain_cwd_marker() -> PathBuf {
    let session = Session::builder().build().expect("session builds");
    session.cwd().to_path_buf()
}
