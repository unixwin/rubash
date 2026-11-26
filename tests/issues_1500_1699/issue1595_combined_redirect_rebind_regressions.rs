//! rubash#1595 — on a function call, `&>f1` followed by `2>f2` did not
//! re-bind fd 2: both streams landed in f1 and f2 stayed empty. The
//! function-call redirect applier propagated the two legs of a combined
//! entry through separate mirror slots, losing the redirect list's
//! left-to-right order (the synthetic stderr leg of `&>` was applied
//! after — therefore over — the genuine later `2>f2`).
//!
//! GNU redir.c do_redirections applies the list strictly in order:
//! `&>f1 2>f2` leaves fd 1 on f1 and fd 2 on f2; the reversed order
//! `2>f2 &>f1` leaves both on f1 (f2 created empty by its own open).
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1595-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_c_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn read_file(dir: &std::path::Path, name: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(dir.join(name)).expect("read output file")).into_owned()
}

const OE: &str = "oe(){ echo OUT; echo ERR >&2; }; ";

#[test]
fn issue1595_combined_then_stderr_redirect_rebinds_fd2() {
    let dir = scratch("combined-first");
    let (_stdout, _stderr, code) = run_c_in(&dir, &format!("{OE} oe &>f1 2>f2"));
    assert_eq!(code, Some(0));
    assert_eq!(read_file(&dir, "f1"), "OUT\n");
    assert_eq!(read_file(&dir, "f2"), "ERR\n");
}

#[test]
fn issue1595_stderr_then_combined_redirect_keeps_both_on_f1() {
    let dir = scratch("stderr-first");
    let (_stdout, _stderr, code) = run_c_in(&dir, &format!("{OE} oe 2>f2 &>f1"));
    assert_eq!(code, Some(0));
    assert_eq!(read_file(&dir, "f1"), "OUT\nERR\n");
    // f2 was opened (created/truncated) by its own redirection, then
    // the combined entry re-bound fd 2 — f2 stays empty.
    assert_eq!(read_file(&dir, "f2"), "");
}

#[test]
fn issue1595_combined_only_binds_both_streams() {
    let dir = scratch("combined-only");
    let (_stdout, _stderr, code) = run_c_in(&dir, &format!("{OE} oe &>f1"));
    assert_eq!(code, Some(0));
    assert_eq!(read_file(&dir, "f1"), "OUT\nERR\n");
}
