//! rubash#3268 faces (b) and (c) — in-process path-string resolution
//! from a deleted session cwd: the `cd` builtin's own move and the
//! shell-side redirection opens.
//!
//! GNU: cd moves the process cwd itself (builtins/cd.def cd_builtin ->
//! chdir), and redirections are opened by do_redirections
//! (redir.c:237) inside the forked child (execute_cmd.c
//! execute_disk_command) — in both cases the resolution starts from
//! the cwd INODE, which stays valid after the directory is unlinked.
//! change_to_directory (builtins/cd.def:522) first tries the
//! canonicalized pathname (sh_canonpath with PATH_CHECKDOTDOT |
//! PATH_CHECKEXISTS, lib/sh/pathcanon.c — the deleted component makes
//! it fail) and then falls back to chdir'ing the operand VERBATIM
//! (cd.def tail, step 9 of the POSIX cd algorithm); the verbatim walk
//! from the inode reaches the real parent.
//!
//! Under G-02 rubash's session cwd is a FIELD: cd probed
//! canonicalize(session_cwd + operand) as a STRING (the walk dies at
//! the deleted component), and the shell-side redirect opens joined
//! the target onto the same string. Both faces now spend the cwd
//! anchor (the open handle kept since the directory was entered,
//! rubash#3198) under the same two-state rule as the spawn stamp and
//! the emulated-cat operand read: a RELATIVE operand, a session cwd
//! whose path no longer resolves, and an anchor naming exactly that
//! cwd. Absolute spellings keep the plain string verdict — GNU's
//! child gets the same string and the same ENOENT.
//!
//! Oracle: GNU bash 5.3.20, script-file probes (probe-3268-bc.sh on
//! the dispatch tree), deleted cwd staged by `cd a; (cd ..; rmdir a)`:
//!   cd .. / cd -L .. / cd -P .. -> rc 0, $PWD = the parent, `cat f`
//!     reads hello; cd ../a2 -> rc 0, $PWD = D/a2; cd . -> rc 0 (its
//!     $PWD spelling and getcwd diagnostic are the resetpwd residue,
//!     a getcwd-reporting face — not pinned here); cd $PWD/.. -> rc 1;
//!   cd nonexistent -> rc 1 `cd: nonexistent: No such file or
//!     directory`; cat <../f, /usr/bin/cat <../f, read <../f -> hello
//!     rc 0; cat <../nonexistent -> rc 1; echo >../g and >>../g ->
//!     rc 0, the file lands in the parent; read <>../f -> hello rc 0.
//!     Composite (captain steer, design lane's boundary probe):
//!     x=$(cat <../f) -> x=hello rc 0 — the (c) face projected into a
//!     command substitution: the respawned shell inherits the dead
//!     cwd inode and its input redirect resolves from it; the
//!     missing-target control x=$(cat <../nonexistent) -> x=[] rc 1.
//!     Compound: while read l; done <../f -> the loop reads hello
//!     (the compound fd-0 bind classifies and opens through the
//!     anchor, keeping the live descriptor); the missing-target
//!     control fails the compound with rc 1 before the body runs.
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

/// A canonicalized scratch root D: D/f and D/a2/g are files, D/a is
/// the directory the shell enters and loses. Canonicalized because
/// $PWD after a move is the physical spelling.
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3268-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    std::fs::create_dir_all(dir.join("a2")).expect("create a2");
    std::fs::write(dir.join("f"), "hello\n").expect("write f");
    std::fs::write(dir.join("a2/g"), "sibling\n").expect("write a2/g");
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

/// The deleted-cwd prelude shared by every pin: enter D/a, have a
/// subshell remove it, stay there.
fn prelude() -> &'static str {
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n"
}

// ---------------------------------------------------------------- (b)

#[test]
fn issue3268_cd_dotdot_from_deleted_cwd() {
    let dir = scratch("b1");
    let script = format!("{}cd ..; echo cdrc=$?; cat f; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "cdrc=0\nhello\nrc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_cd_logical_dotdot_from_deleted_cwd() {
    let dir = scratch("b2");
    let script = format!(
        "{}cd -L ..; echo cdrc=$?; echo \"PWD=$PWD\"; cat f; echo rc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    let expected = format!("cdrc=0\nPWD={}\nhello\nrc=0\n", dir.0.display());
    assert_eq!(stdout, expected, "stderr:\n{stderr}");
}

#[test]
fn issue3268_cd_physical_dotdot_from_deleted_cwd() {
    let dir = scratch("b3");
    let script = format!(
        "{}cd -P ..; echo cdrc=$?; echo \"PWD=$PWD\"; cat f; echo rc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    let expected = format!("cdrc=0\nPWD={}\nhello\nrc=0\n", dir.0.display());
    assert_eq!(stdout, expected, "stderr:\n{stderr}");
}

#[test]
fn issue3268_cd_sibling_from_deleted_cwd() {
    let dir = scratch("b5");
    let script = format!(
        "{}cd ../a2; echo cdrc=$?; echo \"PWD=$PWD\"; cat g; echo rc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    let expected = format!("cdrc=0\nPWD={}/a2\nsibling\nrc=0\n", dir.0.display());
    assert_eq!(stdout, expected, "stderr:\n{stderr}");
}

#[test]
fn issue3268_cd_dot_from_deleted_cwd_stays_functional() {
    // GNU: `cd .` succeeds (verbatim chdir of "." from the inode).
    // Its $PWD spelling and stderr getcwd diagnostic are the
    // change_to_directory resetpwd residue — a getcwd-reporting face
    // outside this pin; what is pinned is the verdict and that the
    // session stays usable in the still-deleted directory.
    let dir = scratch("b4");
    let script = format!("{}cd .; echo cdrc=$?; cat ../f; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "cdrc=0\nhello\nrc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_cd_absolute_dead_spelling_still_fails() {
    // Control (both shells, oracle-measured): an absolute spelling
    // that textually traverses the deleted directory dies — the
    // anchor route is for relative operands only.
    let dir = scratch("b6");
    let script = format!("{}cd $PWD/..; echo cdrc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "cdrc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_cd_missing_from_deleted_cwd_still_fails() {
    // Control: the anchor walk's own ENOENT is the verdict — the fix
    // must not manufacture success for a name the inode lacks.
    let dir = scratch("b7");
    let script = format!("{}cd nonexistent; echo cdrc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "cdrc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cd: nonexistent: No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_cd_dotdot_twice_climbs_out() {
    let dir = scratch("b8");
    let parent = dir.0.parent().expect("scratch parent").to_path_buf();
    let script = format!(
        "{}cd ..; echo cdrc=$?; cd ..; echo cdrc2=$?; echo \"PWD=$PWD\"\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    let expected = format!("cdrc=0\ncdrc2=0\nPWD={}\n", parent.display());
    assert_eq!(stdout, expected, "stderr:\n{stderr}");
}

// ---------------------------------------------------------------- (c)

#[test]
fn issue3268_input_redirect_emulated_cat() {
    let dir = scratch("c1");
    let script = format!("{}cat <../f; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "hello\nrc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_input_redirect_external_cat() {
    let dir = scratch("c2");
    let script = format!("{}/usr/bin/cat <../f; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "hello\nrc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_input_redirect_read_builtin() {
    let dir = scratch("c3");
    let script = format!("{}read line <../f; echo \"line=$line rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "line=hello rc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_input_redirect_missing_still_fails() {
    // Control: GNU fails the open in the child with the target as
    // spelled; the anchor walk's ENOENT is the same verdict.
    let dir = scratch("c4");
    let script = format!("{}cat <../nonexistent; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("../nonexistent: No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_output_redirect_creates_in_parent() {
    let dir = scratch("c5");
    let script = format!(
        "{}echo hello2 >../g; echo rc=$?; /usr/bin/cat ../g; echo crc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\nhello2\ncrc=0\n", "stderr:\n{stderr}");
    // The file physically lands in the parent directory.
    let landed = std::fs::read_to_string(dir.0.join("g")).expect("g landed in parent");
    assert_eq!(landed, "hello2\n");
}

#[test]
fn issue3268_output_redirect_appends_in_parent() {
    let dir = scratch("c6");
    let script = format!(
        "{}echo one >>../g; echo rc=$?; echo two >>../g; echo rc=$?; /usr/bin/cat ../g\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\nrc=0\none\ntwo\n", "stderr:\n{stderr}");
    let landed = std::fs::read_to_string(dir.0.join("g")).expect("g landed in parent");
    assert_eq!(landed, "one\ntwo\n");
}

#[test]
fn issue3268_readwrite_redirect_read_builtin() {
    let dir = scratch("c7");
    let script = format!("{}read line <>../f; echo \"line=$line rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "line=hello rc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_comsub_input_redirect_from_deleted_cwd() {
    // Composite cell (captain steer from the (a)(d)(e) design lane):
    // the (c) face inside a command substitution — the respawned
    // comsub shell inherits the dead cwd inode and its input
    // redirect resolves relative to it (GNU: do_redirections in the
    // forked child, execute_cmd.c). Oracle-measured on GNU 5.3.20:
    // x=hello rc 0; base rubash: x= rc 1.
    let dir = scratch("c8");
    let script = format!("{}x=$(cat <../f); echo \"x=$x rc=$?\"\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=hello rc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_compound_loop_input_redirect_from_deleted_cwd() {
    // The compound fd-0 bind: GNU opens the loop's `< file` before
    // the body runs (redir.c do_redirection_internal) in the forked
    // child — from the inode. Oracle-measured on GNU 5.3.20:
    // got:hello rc 0; base rubash rejected the open in the shell
    // process (rc 1, `../f: No such file or directory`).
    let dir = scratch("c10");
    let script = format!(
        "{}while read l; do echo \"got:$l\"; done <../f; echo rc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "got:hello\nrc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3268_compound_loop_input_redirect_missing_still_fails() {
    // Control (both shells, oracle-measured): the compound is
    // abandoned with the redirection's diagnostic and status 1.
    let dir = scratch("c11");
    let script = format!(
        "{}while read l; do echo \"got:$l\"; done <../nonexistent; echo rc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("../nonexistent: No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3268_comsub_input_redirect_missing_still_fails() {
    // Control (both shells, oracle-measured): inside the comsub the
    // anchor walk's ENOENT is still the verdict.
    let dir = scratch("c9");
    let script = format!(
        "{}x=$(cat <../nonexistent); echo \"x=[$x] rc=$?\"\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x=[] rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("../nonexistent: No such file or directory"),
        "stderr:\n{stderr}"
    );
}
