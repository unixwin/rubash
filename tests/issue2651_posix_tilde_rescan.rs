//! rubash#2651 — in POSIX mode, the tilde result in an `eval`'d
//! assignment was rescanned and re-expanded (double expansion):
//! with HOME='/usr/$x/abc' and x=1234, `eval $h` (h=HOME=~) stored
//! /usr/1234/abc instead of the verbatim /usr/$x/abc.
//!
//! GNU anchors (bash 5.3, upstream tilde2.tests POSIX segment):
//! - expand_word_internal (subst.c:11420-11440): in POSIX mode the
//!   `=`/`:[=]` tilde is NOT expanded during the word walk of an
//!   assignment word unless the word carries W_TILDEEXP; the tilde is
//!   expanded by the assignment path itself, and the expansion result
//!   is inserted as data — expansion results are never rescanned in
//!   the same pass (subst.c:11352-11356 add_string model), so a `$x`
//!   inside the HOME value stays literal.
//! - Outside POSIX mode both shells agree (verbatim result).
//!
//! Oracle: GNU bash 5.3.20 probe scripts (2026-11-08), byte-compared.
//! Harness: script FILES via CARGO_BIN_EXE_rubash, LC_ALL=C.UTF-8.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run_file(script: &str) -> Outcome {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "rubash_issue2651_{}_{}_{}.sh",
        std::process::id(),
        script.len(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn expect(script: &str, want_stdout: &[u8], ctx: &str) {
    let out = run_file(script);
    assert_eq!(
        out.stdout,
        want_stdout,
        "{ctx}: stdout; stderr {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "{ctx}: exit code");
}

#[test]
fn posix_eval_assignment_keeps_tilde_result_verbatim() {
    expect(
        "h=HOME=~\nx=1234\nHOME='/usr/$x/abc'\nset -o posix\neval $h\necho \"[$HOME]\"\n",
        b"[/usr/$x/abc]\n",
        "posix eval assignment",
    );
}

#[test]
fn posix_eval_echo_of_assignment_word_keeps_tilde() {
    // tilde2.tests: `eval echo $h` in POSIX mode prints HOME=~ —
    // the assignment-shaped argument is not tilde-expanded there.
    expect(
        "h=HOME=~\nx=1234\nHOME='/usr/$x/abc'\nset -o posix\neval echo $h\n",
        b"HOME=~\n",
        "posix eval echo $h",
    );
}

#[test]
fn nonposix_eval_assignment_keeps_tilde_result_verbatim() {
    expect(
        "h=HOME=~\nx=1234\nHOME='/usr/$x/abc'\neval $h\necho \"[$HOME]\"\n",
        b"[/usr/$x/abc]\n",
        "non-posix eval assignment (control)",
    );
}

#[test]
fn posix_direct_assignment_keeps_tilde_result_verbatim() {
    expect(
        "x=1234\nHOME='/usr/$x/abc'\nset -o posix\nh2=~\necho \"[$h2]\"\n",
        b"[/usr/$x/abc]\n",
        "posix direct assignment",
    );
}

#[test]
fn posix_direct_assignment_tilde_slash_verbatim() {
    expect(
        "x=1234\nHOME='/usr/$x/abc'\nset -o posix\nh3=~/sub\necho \"[$h3]\"\n",
        b"[/usr/$x/abc/sub]\n",
        "posix direct assignment ~/sub",
    );
}

#[test]
fn posix_word_position_tilde_verbatim() {
    expect(
        "x=1234\nHOME='/usr/$x/abc'\nset -o posix\necho ~\n",
        b"/usr/$x/abc\n",
        "posix plain tilde word",
    );
}

#[test]
fn posix_eval_assignment_user_tilde() {
    // The deferred path must still resolve the tilde itself; only the
    // RESULT is protected from rescanning.
    expect(
        "x=1234\nHOME='/usr/$x/abc'\nh=DEST=~\nset -o posix\neval $h\necho \"[$DEST]\"\n",
        b"[/usr/$x/abc]\n",
        "posix eval assignment to another name",
    );
}
