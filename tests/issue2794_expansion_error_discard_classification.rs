//! rubash#2794 — a `${var:?}` expansion error's DISCARD fact never
//! reaches the run classification: the engine adjudicates the error
//! internally (interactive = DISCARD, non-interactive = fatal), but
//! `RunClassification` carries no expansion-error discard face, so a
//! host reading only the classification cannot tell "this line was
//! dropped because of an expansion error" from an ordinary status-1
//! completion (on the discard cells below every classification flag
//! reads false and `unwind` is `None` — the run is
//! classification-silent).
//!
//! GNU law: the `:?` error arm returns
//! `interactive_shell ? &expand_wdesc_error : &expand_wdesc_fatal`
//! (subst.c:10416-10418) — interactive shells DISCARD the line and
//! live (status 1); non-interactive shells die. Under `set -e` the
//! errexit verdict still fires on the discarded unit's status 1
//! (evalstring.c:387-393), so interactive+`-e` ends the session by
//! the errexit break, not by the discard; a non-interactive fatal
//! stays an exit jump even with `-e` armed.
//!
//! This pin locks the FATE matrix through the public Session API —
//! expansion error x {interactive, non-interactive} x {-e, no -e} x
//! {grouped run, line run} — as status / flow / unwind / session
//! survival, so the fix lane that adds the classification face
//! cannot silently change the fate surface while wiring it. The
//! classification field itself does not exist yet and is therefore
//! not referenced here (referencing it would make the pre-fix
//! failure a compile error, not a verdict); the fix lane adds the
//! field assertions in the consumer tests when the field lands.
//! The discard cells additionally assert the two pre-existing
//! discard classes stay silent (`parse_error == false`,
//! `signal_discard == false`): the defect is the MISSING face, not
//! a misclassification into a neighbouring one.
//!
//! Matrix measured on the base tree with a scratch probe (the
//! expectations below are those measurements, cross-checked
//! cell-by-cell against GNU bash 5.3.20 script-file probes: the
//! discard, the same-line tail drop, and the `-e` session end are
//! GNU's; the status/flow/unwind spelling is the engine's contract
//! surface for the same fates).

use rubash::api::{Flow, InvocationShape, RunOptions, RunUnwind, Session};

fn interactive_session() -> Session {
    Session::builder()
        .invocation_shape(InvocationShape::interactive())
        .build()
        .expect("session builds")
}

fn iopts() -> RunOptions {
    let mut options = RunOptions::new();
    options.interactive = true;
    options
}

#[test]
fn issue2794_interactive_grouped_discard_survives() {
    let mut s = interactive_session();
    let outcome = s.run("echo ${q6_undef:?boom}", &iopts()).expect("run");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::Completed);
    assert_eq!(outcome.classification.unwind, None);
    assert!(!outcome.classification.parse_error);
    assert!(!outcome.classification.signal_discard);
    // The session lives: the next run is an ordinary completion.
    let outcome = s.run("true", &iopts()).expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(outcome.flow, Flow::Completed);
}

#[test]
fn issue2794_interactive_line_discard_survives() {
    let mut s = interactive_session();
    let outcome = s.run_line("echo ${q6_undef:?boom}").expect("run_line");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::Completed);
    assert_eq!(outcome.classification.unwind, None);
    assert!(!outcome.classification.parse_error);
    assert!(!outcome.classification.signal_discard);
    let outcome = s.run_line("true").expect("run_line");
    assert_eq!(outcome.status, 0);
    assert_eq!(outcome.flow, Flow::Completed);
}

#[test]
fn issue2794_interactive_grouped_errexit_breaks() {
    let mut s = interactive_session();
    let outcome = s.run("set -e", &iopts()).expect("run");
    assert_eq!(outcome.status, 0);
    let outcome = s.run("echo ${q6_undef:?boom}", &iopts()).expect("run");
    // The discard happened (status 1), and the errexit verdict on
    // that status ends the session: the break, not a fatal jump.
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(1));
    assert_eq!(outcome.classification.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(outcome.classification.errexit_break);
    assert!(!outcome.classification.exit_jump);
}

#[test]
fn issue2794_interactive_line_errexit_breaks() {
    let mut s = interactive_session();
    let outcome = s.run_line("set -e").expect("run_line");
    assert_eq!(outcome.status, 0);
    let outcome = s.run_line("echo ${q6_undef:?boom}").expect("run_line");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(1));
    assert_eq!(outcome.classification.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(outcome.classification.errexit_break);
    assert!(!outcome.classification.exit_jump);
}

#[test]
fn issue2794_noninteractive_grouped_fatal_exit_jump() {
    let mut s = Session::builder().build().expect("build");
    let outcome = s
        .run("echo ${q6_undef:?boom}", &RunOptions::new())
        .expect("run");
    // Non-interactive `:?` is fatal: the run ends as an exit jump
    // carrying the error status.
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ExitRequested(1));
    assert_eq!(outcome.classification.unwind, Some(RunUnwind::ExitJump));
    assert!(outcome.classification.exit_jump);
    assert!(!outcome.classification.errexit_break);
}

#[test]
fn issue2794_noninteractive_grouped_errexit_still_fatal() {
    let mut s = Session::builder().build().expect("build");
    let outcome = s.run("set -e", &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0);
    let outcome = s
        .run("echo ${q6_undef:?boom}", &RunOptions::new())
        .expect("run");
    // With `-e` armed the errexit verdict also fires, but the fatal
    // expansion error stays an exit jump — the jump outranks the
    // break in the unwind decomposition.
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ExitRequested(1));
    assert_eq!(outcome.classification.unwind, Some(RunUnwind::ExitJump));
    assert!(outcome.classification.exit_jump);
    assert!(outcome.classification.errexit_break);
}

#[test]
fn issue2794_interactive_discard_drops_line_tail() {
    let mut s = interactive_session();
    let outcome = s
        .run("echo ${q6_undef:?boom}; marker2794=1", &iopts())
        .expect("run");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::Completed);
    assert_eq!(outcome.classification.unwind, None);
    // Chain: the discard dropped the whole line, so the tail
    // assignment never ran — the marker is still unset next run.
    let outcome = s.run("test \"$marker2794\" = 1", &iopts()).expect("run");
    assert_eq!(outcome.status, 1);
    // And the session itself is unharmed.
    let outcome = s.run("true", &iopts()).expect("run");
    assert_eq!(outcome.status, 0);
}
