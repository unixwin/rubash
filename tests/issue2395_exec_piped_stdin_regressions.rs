//! rubash#2395 — `exec` over a virtual fd 0 (pipeline-stage text,
//! heredoc, here-string) leaked its stdin carrier and, in a pipeline
//! stage, dropped the command's stderr redirection.
//!
//! Two halves of one boundary — GNU has exactly one set of
//! descriptors for the exec'd image: execute_cmd.c:4244/4268
//! performs the command's redirections, then exec_builtin
//! (builtins/exec.def:231) calls shell_execve (execute_cmd.c:6126)
//! with the shell's fds as bound. There is no stdin materialization
//! (GNU's only materialized input, the heredoc temp of redir.c:423
//! here_document_to_fd, is an unnamed fd) and no second stderr
//! channel.
//!
//! 1. CARRIER. rubash's virtual fd-0 text has no OS handle, so
//!    exec_inherited_stdin writes it to a temp file for the child.
//!    The carrier was created with fs::write + File::open and the
//!    path was dropped on the floor: every such exec left a
//!    `rubash-process-subst-*.tmp` in $TMPDIR forever. The carrier
//!    is now anonymous from birth (unlinked after open on Unix,
//!    FILE_FLAG_DELETE_ON_CLOSE on Windows), so it dies with the
//!    last descriptor.
//!
//! 2. STDERR. A pipeline stage's clone strips its fd-1/fd-2
//!    redirects (the driver owns them and applies them to the
//!    stage's captured streams), and arms `stderr_capture` for the
//!    clone's fd-2 bytes. The exec spawn wrote the child's piped
//!    stderr — and its own diagnostics — straight to the process
//!    stderr, bypassing the capture and therefore the driver's
//!    redirect application: `... | exec bash -c 'echo e >&2'
//!    2>/dev/null` printed `e` to the terminal. The spawn's stderr
//!    destination now follows the executor's armed capture (see
//!    ExecDiagStderr in trap_exec.rs).
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (the issue's Regression seed plus the
//! heredoc / here-string / file-redirect variants).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue2395-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let tmp = dir.join("tmp");
    fs::create_dir_all(&tmp).expect("create scratch tmp");
    fs::write(dir.join("seed.txt"), "seed-line\n").expect("write seed");
    (dir, tmp)
}

/// Run `script` as a script FILE under rubash, cwd = dir, TMPDIR =
/// tmp, stdin null. Returns (stdout, stderr).
fn rubash_script(dir: &Path, tmp: &Path, name: &str, body: &str) -> (String, String) {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .env("TMPDIR", tmp)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The issue's residue rule: no `rubash-process-subst-*.tmp` entry
/// may exist under TMPDIR after the run (names carry pid + random,
/// only the wildcard existence is compared).
fn assert_no_carrier_residue(tmp: &Path) {
    let residue: Vec<String> = fs::read_dir(tmp)
        .expect("read scratch tmp")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("rubash-process-subst-") && name.ends_with(".tmp"))
        .collect();
    assert!(
        residue.is_empty(),
        "exec left stdin carrier residue in TMPDIR: {residue:?}"
    );
}

#[test]
fn issue2395_pipeline_exec_reads_stdin_and_leaves_no_carrier() {
    let (dir, tmp) = scratch("pipe-stdin");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "case.sh",
        "printf 'hello-stdin\\n' | exec bash -c 'cat' 2>/dev/null\n",
    );
    assert_eq!(stdout, "hello-stdin\n");
    assert_eq!(stderr, "");
    assert_no_carrier_residue(&tmp);
}

#[test]
fn issue2395_pipeline_exec_stderr_redirect_is_not_dropped() {
    let (dir, tmp) = scratch("pipe-stderr-null");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "case.sh",
        "printf '' | exec bash -c 'echo to-err >&2' 2>/dev/null; echo done\n",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "");
    assert_no_carrier_residue(&tmp);
}

#[test]
fn issue2395_pipeline_exec_stderr_redirect_to_file_receives_bytes() {
    let (dir, tmp) = scratch("pipe-stderr-file");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "case.sh",
        "printf 'x\\n' | exec bash -c 'echo to-err >&2' 2>err.txt; echo done\n",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "");
    let routed = fs::read_to_string(dir.join("err.txt")).expect("read routed stderr file");
    assert_eq!(routed, "to-err\n");
    assert_no_carrier_residue(&tmp);
}

#[test]
fn issue2395_exec_heredoc_stdin_leaves_no_carrier() {
    let (dir, tmp) = scratch("heredoc");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "case.sh",
        "exec bash -c 'cat' <<HD\nheredoc-body\nHD\n",
    );
    assert_eq!(stdout, "heredoc-body\n");
    assert_eq!(stderr, "");
    assert_no_carrier_residue(&tmp);
}

#[test]
fn issue2395_exec_herestring_stdin_leaves_no_carrier() {
    let (dir, tmp) = scratch("herestring");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "case.sh",
        "exec bash -c 'cat' <<<hello-hs\n",
    );
    assert_eq!(stdout, "hello-hs\n");
    assert_eq!(stderr, "");
    assert_no_carrier_residue(&tmp);
}

#[test]
fn issue2395_exec_file_stdin_still_works() {
    let (dir, tmp) = scratch("file-stdin");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "case.sh",
        "exec bash -c 'cat' < seed.txt\n",
    );
    assert_eq!(stdout, "seed-line\n");
    assert_eq!(stderr, "");
    assert_no_carrier_residue(&tmp);
}

#[test]
fn issue2395_pipeline_exec_failure_diagnostic_follows_stage_stderr() {
    // GNU: the stage subshell's exec failure diagnostic is the
    // stage's stderr — routed by the stage's own `2>` like any
    // other stage byte, and visible on the shell's stderr without
    // one. The exec spawn's diagnostics must ride the same channel.
    let (dir, tmp) = scratch("diag-routed");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "routed.sh",
        "printf '' | exec rubash-no-such-command-2395 2>/dev/null; echo done\n",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "");
    let (stdout, stderr) = rubash_script(
        &dir,
        &tmp,
        "unrouted.sh",
        "printf '' | exec rubash-no-such-command-2395; echo done\n",
    );
    assert_eq!(stdout, "done\n");
    assert!(
        stderr.contains("rubash-no-such-command-2395"),
        "exec failure diagnostic missing from stage stderr: {stderr:?}"
    );
    assert_no_carrier_residue(&tmp);
}
