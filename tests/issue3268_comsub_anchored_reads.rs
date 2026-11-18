//! rubash#3268 face (a) — the word-level command-substitution `cat`
//! shortcut arms read their file operand IN the shell process.
//!
//! GNU has no such shortcut: command_substitute (subst.c:7143)
//! forks and the body runs in the child, whose cwd is the session's
//! inode — a relative operand like `../f` is walked by the kernel
//! from that inode and stays readable after the directory is
//! unlinked. The `$(< file)` shortcut is optimize_cat_file
//! (subst.c:6665) over open_redir_file (builtins/evalstring.c:759),
//! a plain open(fn, O_RDONLY) in the same forked context; a
//! directory operand opens fine and its first zread fails EISDIR,
//! so read_comsub (subst.c:6700-6712) yields empty output with
//! EXECUTION_SUCCESS — silent, rc 0.
//!
//! Under G-02 the two rubash shortcut arms resolved the operand by
//! joining it onto the session cwd FIELD and opening the string:
//! once that directory is deleted, the joined spelling dies at its
//! dead component. The arms are the shared byte ladder
//! `comsub_cat_operand_bytes` (the word-level `cat` arm and the
//! `time` arm — the timed arm's failure policy is silent skip with
//! status 0, so its red shape was silent data loss) and
//! `comsub_cat_file_substitute`'s own cwd join + read. Both now
//! spend the cwd anchor through read_session_anchored (rubash#3198),
//! the same two-state rule as the spawn stamp and the emulated-cat
//! operand read: a RELATIVE operand, a session cwd whose path no
//! longer resolves (identity-gated, rubash#3275), and an anchor
//! naming exactly that cwd. Absolute spellings keep the plain
//! string verdict — GNU's child gets the same string and the same
//! ENOENT.
//!
//! Oracle: GNU bash 5.3.20, script-file probes (probe-gnu-a.sh on
//! the dispatch tree), deleted cwd staged by `cd a; (cd ..; rmdir
//! a)` with D/f = "hello" and D/sub a directory:
//!   $(cat ../f), `cat ../f`, $(cat ../f ../f), $(<../f) ->
//!     hello content, rc 0, no diagnostic;
//!   $(time cat ../f) -> x=hello rc 0 (time report on stderr);
//!   $(<../sub) -> empty, rc 0, silent;
//!   $(cat ../f; true) -> hello rc 0 (full-execution control);
//!   $(cat $PWD/../f) -> empty, rc 1 (absolute dead spelling);
//!   live cwd $(cat f) / $(<f) -> hello.
//!
//! These pins assert COMPLETION FORMS (output bytes + exit status),
//! never timing. Unix only: the cwd anchor is a Unix directory
//! handle; off Unix there is no anchor and the faces keep their
//! pre-fix shape.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A canonicalized scratch root D: D/f is a file, D/sub a directory
/// with a file in it, D/a the directory the shell enters and loses.
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3268a-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    std::fs::create_dir_all(dir.join("sub")).expect("create sub");
    std::fs::write(dir.join("f"), "hello\n").expect("write f");
    std::fs::write(dir.join("sub/sf"), "subfile\n").expect("write sub/sf");
    let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    ScratchDir(canonical)
}

/// Spawn rubash on `script` inside `dir`; returns (stdout, stderr,
/// status code).
fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

/// The deleted-cwd prelude shared by the deleted-cwd pins: enter
/// D/a, have a subshell remove it, stay there.
fn prelude() -> &'static str {
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n"
}

// ------------------------------------------------------- fail-first

#[test]
fn issue3268_a_cat_dollar() {
    let dir = scratch("cat-dollar");
    let script = format!("{}x=$(cat ../f); echo \"x=$x rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=hello rc=0\n", "stderr:\n{stderr}");
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_a_cat_backquote() {
    let dir = scratch("cat-backquote");
    let script = format!("{}x=`cat ../f`; echo \"x=$x rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=hello rc=0\n", "stderr:\n{stderr}");
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_a_cat_two_operands() {
    let dir = scratch("cat-two");
    let script = format!("{}x=$(cat ../f ../f); echo \"x=$x rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=hello\nhello rc=0\n", "stderr:\n{stderr}");
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_a_catfile() {
    let dir = scratch("catfile");
    let script = format!("{}x=$(<../f); echo \"x=$x rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=hello rc=0\n", "stderr:\n{stderr}");
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_a_timed_cat() {
    // The timed arm's failure policy is silent skip with status 0,
    // so on the base tree this cell's red shape is silent data loss
    // (`x=` with rc 0 and no diagnostic). The pin asserts the
    // captured value and the status only; the `time` report itself
    // goes to stderr with timing digits and is not asserted.
    let dir = scratch("timed-cat");
    let script = format!("{}x=$(time cat ../f); echo \"x=$x rc=$?\"\n", prelude());
    let (stdout, _stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=hello rc=0\n");
}

#[test]
fn issue3268_a_catfile_dir_silent() {
    // GNU read_comsub on a directory: the open succeeds, the first
    // zread fails EISDIR, the substitution is empty with
    // EXECUTION_SUCCESS (subst.c:6700-6712). The anchor route
    // surfaces the same EISDIR from the read, and the arm maps it
    // back to this silent shape.
    let dir = scratch("catfile-dir");
    let script = format!("{}x=$(<../sub); echo \"x=[$x] rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=[] rc=0\n", "stderr:\n{stderr}");
    assert!(stderr.is_empty(), "stderr:\n{stderr}");
}

// --------------------------------------------------- regression locks

#[test]
fn issue3268_a_full_exec_stays() {
    // Control: a multi-command body takes the full-execution clone
    // path (the anchor crosses in ShellState::clone) and was
    // already green; the shortcut convergence must not disturb it.
    let dir = scratch("full-exec");
    let script = format!("{}x=$(cat ../f; true); echo \"x=$x rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=hello rc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_a_abs_dead_stays_dead() {
    // Over-fix guard (both shells, oracle-measured): an absolute
    // spelling that textually traverses the deleted directory
    // stays dead — the anchor route serves relative operands only.
    let dir = scratch("abs-dead");
    let script = format!("{}x=$(cat $PWD/../f); echo \"x=[$x] rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=[] rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_a_live_cwd_normal() {
    // Control: with a live cwd the anchor never engages and both
    // shortcut arms read exactly as before.
    let dir = scratch("live");
    let script = "x=$(cat f); y=$(<f); echo \"x=$x y=$y rc=$?\"\n";
    let (stdout, stderr, code) = run_rubash(&dir.0, script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=hello y=hello rc=0\n", "stderr:\n{stderr}");
}
