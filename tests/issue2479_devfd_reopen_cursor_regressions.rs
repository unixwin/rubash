//! Issue unixwin/rubash#2479: the surviving #2256 residual — a
//! `/dev/fd/N` alias OPEN whose binding feeds a same-list `<&N` dup
//! (or a persistent `exec` bind) must still be a fresh open of the
//! alias target, not a dup of the ambient descriptor.
//!
//! GNU semantics (bash 5.3 sources): on a HAVE_DEV_FD build the
//! `/dev/fd/N` spelling is a plain open() of the magic link
//! (redir.c:725, find_user_tempname/open_file path in
//! do_redirection_internal; the RF_DEVFD F_DUPFD special case at
//! redir.c:603/629 compiles only `#if !defined(HAVE_DEV_FD)`). A
//! regular-file target therefore yields a NEW open file description
//! at offset 0 — the source descriptor's cursor never moves — while
//! the `<&N` instruction itself (make_cmd.c r_dup) is a dup2 that
//! shares whatever description the same-list binding created. A pipe
//! target cannot be re-opened; the alias serves the pipe itself, so
//! repeated alias serves share the stream.
//!
//! The pre-fix rubash bound BOTH faces as dups of the ambient fd:
//! the emulated-builtin stdin serve resolved the `<&3` winner
//! against fd 6's own stream (draining it), and the persistent
//! `exec 3< /dev/fd/6` arm classified the alias spelling through the
//! dup-target helper before the fresh-open ladder could see it.
//!
//! Expectations are the GNU Bash 5.3.20 outputs of the same scripts
//! (oracle binary run side by side in the cluster workdir). Pins run
//! rubash as a subprocess and observe stdout/stderr only.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SEED: &str = "LINE1\nLINE2\n";

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2479-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn collect(child: std::process::Child) -> (String, String, Option<i32>) {
    let mut child = child;
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

/// Run `script` as rubash's stdin inside `dir` (seed pre-written as
/// the two-line file `seed2`).
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("seed2"), SEED).expect("write seed");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write script");
    drop(child.stdin.take());
    collect(child)
}

/// The issue's headline shape: the same-list alias open feeds a
/// `<&3` dup, twice. GNU prints the seed twice; pre-fix rubash
/// printed it once (the first serve drained fd 6 through the dup
/// classification). fd 6 itself must stay at offset 0.
#[test]
fn issue2479_numbered_alias_open_dup_reads_fresh_twice() {
    let dir = test_dir("fresh-twice");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed2\ncat 3< /dev/fd/6 <&3\ncat 3< /dev/fd/6 <&3\nread -r -u 6 line\necho \"fd6:$line\"\n",
    );
    assert_eq!(out, "LINE1\nLINE2\nLINE1\nLINE2\nfd6:LINE1\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// The /proc/self/fd spelling of the same shape is the same open.
#[test]
fn issue2479_proc_self_fd_spelling_reads_fresh_twice() {
    let dir = test_dir("proc-spelling");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed2\ncat 3< /proc/self/fd/6 <&3\ncat 3< /proc/self/fd/6 <&3\n",
    );
    assert_eq!(out, "LINE1\nLINE2\nLINE1\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Persistent face: `exec 3< /dev/fd/6` is an alias OPEN, not a dup —
/// binding it twice yields two independent fresh descriptions and
/// the ambient fd 6 never moves.
#[test]
fn issue2479_persistent_exec_alias_bind_is_fresh_open() {
    let dir = test_dir("exec-bind");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed2\nexec 3< /dev/fd/6\ncat <&3\nexec 3< /dev/fd/6\ncat <&3\nread -r -u 6 line\necho \"fd6:$line\"\n",
    );
    assert_eq!(out, "LINE1\nLINE2\nLINE1\nLINE2\nfd6:LINE1\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// The read-write spelling of the persistent alias bind follows the
/// same fresh-open rule.
#[test]
fn issue2479_persistent_exec_alias_bind_readwrite_is_fresh_open() {
    let dir = test_dir("exec-bind-rw");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed2\nexec 3<> /dev/fd/6\nhead -1 <&3\nexec 3<> /dev/fd/6\nhead -1 <&3\n",
    );
    assert_eq!(out, "LINE1\nLINE1\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Control: a TRUE dup binding (`3<&6`) shares the ambient stream —
/// the serve drains fd 6 exactly like GNU's dup2. This guards the
/// fix's binding-kind distinction: only alias-OPEN bindings serve
/// fresh, dup bindings keep sharing.
#[test]
fn issue2479_true_dup_binding_still_shares_ambient_stream() {
    let dir = test_dir("true-dup");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed2\ncat 3<&6 <&3\nread -r -u 6 line\necho \"fd6:[$line] rc=$?\"\n",
    );
    assert_eq!(out, "LINE1\nLINE2\nfd6:[] rc=1\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Control: a pipe endpoint cannot be re-opened, so repeated alias
/// serves of a procsub-bound fd share the pipe — first serve drains
/// it, second sees EOF.
#[test]
fn issue2479_pipe_alias_still_shares_the_pipe() {
    let dir = test_dir("pipe-share");
    let (out, err, code) = run_script(
        &dir,
        "exec 6< <(printf 'P1\\nP2\\n')\ncat 3< /dev/fd/6 <&3\ncat 3< /dev/fd/6 <&3\necho end\n",
    );
    assert_eq!(out, "P1\nP2\nend\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Partial consumption through an alias open leaves the ambient fd
/// untouched as well: two `head -1` serves both see the first line.
#[test]
fn issue2479_partial_consume_alias_open_leaves_source_at_top() {
    let dir = test_dir("partial");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed2\nhead -1 3< /dev/fd/6 <&3\nhead -1 3< /dev/fd/6 <&3\n",
    );
    assert_eq!(out, "LINE1\nLINE1\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// A closed alias target keeps GNU's open-failure outcome: the
/// diagnostic names the spelling, the command status is 1, and the
/// shell stays alive for the next command.
#[test]
fn issue2479_closed_alias_target_reports_open_failure() {
    let dir = test_dir("closed-alias");
    let (out, err, code) = run_script(&dir, "exec 3< /dev/fd/9\necho \"alive=$?\"\n");
    assert_eq!(out, "alive=1\n");
    assert!(
        err.contains("/dev/fd/9: No such file or directory"),
        "stderr should carry the GNU open diagnostic, got: {err:?}"
    );
    assert_eq!(code, Some(0));
}
