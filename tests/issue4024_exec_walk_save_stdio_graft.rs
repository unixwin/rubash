//! Issue unixwin/rubash#4024 — captain-grafted pins complementing
//! tests/issue4024_exec_walk_save_stdio_source.rs.
//!
//! These shapes come from the captain lane's probe set for #4024
//! (probes p02–p08/p11) and cover the faces the PR's own pin file
//! does not: two stdio saves claiming slots in order (p02), a save
//! dup'd to a numbered fd (p03), the stdin save read from a FILE
//! with continuation reads (p08 file form), multi-line service
//! from a dup'd-back stream (p04), the save slot's disposal seen
//! from the same shell with its diagnostic text (p06), the full
//! diagnostic text of the two-stdio shape (p11), and the two
//! guards that already passed before the fix (p05 plain-dup
//! control, p07 function walk). Every expectation was byte-checked
//! against GNU bash 5.3.20 (add_undo_redirect's F_DUPFD save,
//! redir.c:1296/1303) on the captain's oracle before grafting.
//!
//! The mechanism under test is this PR's
//! `FdTable::exec_walk_saved_entry`: the exec walk's undo save of a
//! symbolic stdio arm (`Stdout` / `Stderr` /
//! `InheritedProcessStdin`) is upgraded to a real descriptor dup,
//! so the same list's close entry retires only the redirector's
//! number and a later `N>&<slot>` dup binds a live endpoint.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4024-graft-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Run `body` as a script inside `dir` (fed on stdin).
fn run_in(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    // Single-quote the path: on Windows it carries backslashes the
    // script's own quoting rules would otherwise strip.
    rubash_stdin(&format!("cd '{}' || exit 1\n{body}", dir.display()))
}

/// Run `body` as the script file `probe.sh` inside `dir`, with
/// the shell's stdin redirected from the prepared file `data`.
/// The script-file layout keeps diagnostic prefixes in the
/// `probe.sh: line N:` form, and fd 0 arrives as an inherited
/// descriptor (the symbolic InheritedProcessStdin arm), which is
/// the arm the stdin save shape exercises.
fn run_script_with_stdin_file(dir: &Path, body: &str, data: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("probe.sh"), body).expect("write probe.sh");
    std::fs::write(dir.join("data"), data).expect("write data");
    let stdin_file = std::fs::File::open(dir.join("data")).expect("open data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::from(stdin_file))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash script");
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

#[test]
fn issue4024_exec_stderr_save_dup_back() {
    // Probe p02: both stdio saves in one list claim slots in
    // order (fd 1 at 10, fd 2 at 11) and each dups back to its own
    // stream.
    let dir = test_dir("stderr");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec >&- 2>&- 1>&10 2>&11\n\
         echo out\n\
         echo err >&2\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "out\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "err\n", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_save_dup_to_numbered_fd() {
    // Probe p03: the save may be dup'd to a different fd than the
    // one it saved — fd 5 takes fd 1's saved description while
    // fd 1 itself stays closed (and reports its write error).
    let dir = test_dir("numbered");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec >&- 5>&10\n\
         echo via5 >&5\n\
         echo to-stdout\n\
         echo stdout-rc=$? >&5\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "via5\nstdout-rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("echo: write error: Bad file descriptor"),
        "stderr must carry the closed-stdout write error; got {stderr:?}"
    );
}

#[test]
fn issue4024_exec_dup_back_multiline() {
    // Probe p04: the dup'd-back stream keeps serving line after
    // line, not just the first write.
    let dir = test_dir("multiline");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec >&- 1>&10\n\
         echo one\n\
         echo two\n\
         printf 'three\\n'\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "one\ntwo\nthree\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_stdin_save_dup_back_reads() {
    // Probe p08 (file form): fd 0's save dups back and `read`
    // keeps consuming the inherited FILE stream where it left
    // off — the second read must see the second line (shared
    // offset of the dup'd description).
    let dir = test_dir("stdin");
    let (stdout, stderr, code) = run_script_with_stdin_file(
        &dir,
        "exec 0<&- 0<&10\n\
         read line\n\
         echo \"got:$line\"\n\
         read line2\n\
         echo \"got2:$line2\"\n",
        "l1\nl2\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "got:l1\ngot2:l2\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_save_slot_disposed_dup_back_lives() {
    // Probe p06 (same-shell form): the save slot itself is
    // disposed when the list stands — writing to fd 10 fails with
    // its diagnostic — while the descriptor dup'd back from it is
    // independent and lives.
    let dir = test_dir("disposed");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec >&- 1>&10\n\
         echo via10 >&10\n\
         echo rc10=$?\n\
         echo hi\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc10=1\nhi\n", "stderr: {stderr}");
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "stderr must name the disposed save slot; got {stderr:?}"
    );
}

#[test]
fn issue4024_exec_save_dup_to_stderr_slot() {
    // Probe p11 (full-text form): `2>&10` dups fd 1's SAVE (slot
    // 10), not fd 1's closed state — fd 2 lands on stdout's
    // description while fd 1 stays closed, so fd 1's write errors
    // are reported through fd 2 onto stdout's capture, with the
    // complete diagnostic text GNU prints.
    let dir = test_dir("twostdio");
    let (stdout, stderr, code) = run_script_with_stdin_file(
        &dir,
        "exec >&- 2>&10\n\
         echo out\n\
         echo err >&2\n\
         echo rc=$?\n",
        "",
    );
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "probe.sh: line 2: echo: write error: Bad file descriptor\n\
         err\n\
         probe.sh: line 4: echo: write error: Bad file descriptor\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_plain_dup_control_unchanged() {
    // Probe p05 guard: a dup from a REAL dup (fd 9, bound by an
    // earlier exec) already survived the close before this fix
    // and must keep working.
    let dir = test_dir("control");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>&1\n\
         exec >&- 1>&9\n\
         echo hi\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_function_walk_dup_from_save_unchanged() {
    // Probe p07 guard: the scoped (function-call) walk never arms
    // kernel closes, so its dup-from-save already worked before
    // this fix; the exec-scoped change must not disturb it.
    let dir = test_dir("fnwalk");
    let (stdout, stderr, code) = run_in(
        &dir,
        "f() { echo inner; }\n\
         f >&- 1>&10\n\
         echo outer\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inner\nouter\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}
