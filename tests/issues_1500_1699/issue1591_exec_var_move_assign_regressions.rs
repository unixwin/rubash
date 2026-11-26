//! rubash#1591 — `exec {var}>&N-` / `exec {var}<&N-` never assigned
//! var: the {var} dup/move applier gated on the source being open in
//! the SPELLING's direction (`is_open_for_write` for `>&`), so a move
//! from a read-only descriptor (the common case) silently did nothing.
//!
//! GNU redir.c do_redirection_internal, dup/move case with
//! REDIR_VARASSIGN: the descriptor is fcntl(F_DUPFD)-copied wholesale —
//! direction is never consulted — redir_varassign then binds the fresh
//! descriptor to the variable, and a move closes the source.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! (clean `-c` shell: first dynamic fd is 10, the next is 11).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1591-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(dir.join("d"), "line\n").expect("write data file");
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

#[test]
fn issue1591_exec_var_move_from_numbered_read_fd() {
    let dir = scratch("var-move-numbered");
    let script = "exec 3<d; exec {b}>&3-; echo \"b=$b\"; cat <&$b; \
                  cat <&3; echo \"rc3=$?\"";
    let (stdout, _stderr, code) = run_c_in(&dir, script);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("b=10\n"),
        "var must be assigned the fresh fd: {stdout}"
    );
    assert!(
        stdout.contains("line\n"),
        "the moved fd must read: {stdout}"
    );
    assert!(
        stdout.contains("rc3=1\n"),
        "the move must close the source: {stdout}"
    );
}

#[test]
fn issue1591_exec_var_move_from_var_fd() {
    let dir = scratch("var-move-var");
    let script = "exec {a}<d; exec {c}>&$a-; echo \"a=$a c=$c\"; cat <&$c";
    let (stdout, _stderr, code) = run_c_in(&dir, script);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("a=10 c=11\n"),
        "both vars must hold their fds: {stdout}"
    );
    assert!(
        stdout.contains("line\n"),
        "the moved fd must read: {stdout}"
    );
}

#[test]
fn issue1591_exec_var_move_input_spelling_from_write_fd() {
    let dir = scratch("var-move-write-src");
    let script = "exec 3>f; exec {b}<&3-; echo \"b=$b\"";
    let (stdout, _stderr, code) = run_c_in(&dir, script);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("b=10\n"),
        "a write-only source must still dup through `<&`: {stdout}"
    );
    assert!(dir.join("f").exists(), "exec 3>f must create f");
}
