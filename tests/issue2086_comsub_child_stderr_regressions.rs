//! Issue rubash#2086: a spawned child's stderr inside a command
//! substitution was swallowed wholesale — GNU forwards it to the
//! shell's stderr, rubash dropped it (the exit status survived, so the
//! divergence was silent).
//!
//! Root cause: the word-level direct-spawn route for a lone external
//! command (`run_external_command_substitution`) configured the child
//! with `Stdio::piped()` for stderr and let `wait_with_output` collect
//! bytes nobody ever routed onward. GNU subst.c:7143
//! command_substitute() forks the child sharing the parent's fd 2 —
//! only fd 1 is replaced by the capture pipe; fd 2 is never captured
//! (subst.c:7149).
//!
//! Fix: with no body-level stderr redirect, the child inherits fd 2
//! live when this executor's fd 2 is the ambient process stderr. When
//! fd 2 is rebound (a file, a stdout snapshot), closed, or captured by
//! an enclosing pipeline stage, the route declines to the
//! full-execution path, whose stdio planner realizes fd 2 from the fd
//! table. A body-level `2>file` / `2>&1` keeps its existing handling.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20 and GNU
//! coreutils 9.4 (self-compiled oracle, script-file probes 2026-10-29).
//! Unix-gated like the S5 sync corpus: the pinned diagnostic texts are
//! coreutils' own, and the Windows face of the same shapes is carried
//! by the three-way difftest battery.

#![cfg(unix)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2086-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, body: &str, extra_path: Option<&Path>) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(bin) = extra_path {
        let path = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("{}:{path}", bin.display()));
    }
    let mut child = command.spawn().expect("spawn rubash");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash hung (>60s) running probe script:\n{body}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out_thread.join().expect("stdout drain")).into_owned();
    let stderr = String::from_utf8_lossy(&err_thread.join().expect("stderr drain")).into_owned();
    (stdout, stderr, status.code())
}

/// Writes an executable helper script into `dir/bin` and returns the
/// bin dir for PATH prepending. The helper is a spawned descendant
/// (its interpreter is a grandchild of the shell), so its stderr can
/// only arrive through the passthrough this issue is about.
fn helper(dir: &Path, name: &str, body: &str) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("create bin dir");
    let file = bin.join(name);
    std::fs::write(&file, body).expect("write helper");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("chmod helper");
    bin
}

// ---- The four corpus shapes (S5 registered material) -------------------

#[test]
fn comsub_cp_missing_operand_diagnostic_passes_through() {
    let dir = scratch("cp");
    let (stdout, stderr, code) = run_script(&dir, "x=$(cp); echo \"rc=$? [$x]\"\n", None);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 []\n");
    assert_eq!(
        stderr, "cp: missing file operand\nTry 'cp --help' for more information.\n",
        "the spawned cp's diagnostic must reach the shell's stderr (GNU)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_ls_missing_path_diagnostic_passes_through() {
    let dir = scratch("ls");
    let (stdout, stderr, code) = run_script(&dir, "x=$(ls /nonexist); echo \"rc=$? [$x]\"\n", None);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=2 []\n");
    assert_eq!(
        stderr, "ls: cannot access '/nonexist': No such file or directory\n",
        "the spawned ls's diagnostic must reach the shell's stderr (GNU)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_basename_extra_operand_diagnostic_passes_through() {
    let dir = scratch("basename");
    let (stdout, stderr, code) =
        run_script(&dir, "x=$(basename a b c); echo \"rc=$? [$x]\"\n", None);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 []\n");
    assert_eq!(
        stderr, "basename: extra operand 'c'\nTry 'basename --help' for more information.\n",
        "the spawned basename's diagnostic must reach the shell's stderr (GNU)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_dirname_missing_operand_diagnostic_passes_through() {
    let dir = scratch("dirname");
    let (stdout, stderr, code) = run_script(&dir, "x=$(dirname); echo \"rc=$? [$x]\"\n", None);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 []\n");
    assert_eq!(
        stderr, "dirname: missing operand\nTry 'dirname --help' for more information.\n",
        "the spawned dirname's diagnostic must reach the shell's stderr (GNU)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Passthrough mechanics: descendants, capture integrity, interleave --

#[test]
fn comsub_descendant_stderr_passes_and_status_survives() {
    let dir = scratch("descendant");
    let bin = helper(
        &dir,
        "failcmd",
        "#!/bin/sh\necho DESCENDANT-ERR >&2\necho out\nexit 3\n",
    );
    let (stdout, stderr, code) =
        run_script(&dir, "x=$(failcmd); echo \"rc=$? [$x]\"\n", Some(&bin));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=3 [out]\n", "capture and status stay GNU's");
    assert_eq!(stderr, "DESCENDANT-ERR\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_stderr_interleave_keeps_stdout_capture_intact() {
    let dir = scratch("interleave");
    let bin = helper(&dir, "mixcmd", "#!/bin/sh\necho O1\necho E1 >&2\necho O2\n");
    let (stdout, stderr, code) = run_script(&dir, "x=$(mixcmd); echo \"cap=[$x]\"\n", Some(&bin));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // The stderr line is live on the shell's stderr (it precedes the
    // result line in real time); the capture holds only the stdout
    // lines, in order.
    assert_eq!(stderr, "E1\n");
    assert_eq!(stdout, "cap=[O1\nO2]\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_nested_substitution_stderr_passes_through() {
    let dir = scratch("nested");
    let (stdout, stderr, code) = run_script(
        &dir,
        "x=$(echo $(ls /nonexist)); echo \"rc=$? [$x]\"\n",
        None,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 []\n");
    assert_eq!(
        stderr, "ls: cannot access '/nonexist': No such file or directory\n",
        "the inner substitution's child diagnostic must reach the shell's stderr (GNU)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_body_level_stderr_redirects_unchanged() {
    let dir = scratch("bodyredir");
    let bin = helper(
        &dir,
        "failcmd",
        "#!/bin/sh\necho DESCENDANT-ERR >&2\necho out\nexit 3\n",
    );
    // 2>&1 merges the diagnostic into the capture, in write order.
    let (stdout, stderr, code) =
        run_script(&dir, "x=$(failcmd 2>&1); echo \"rc=$? [$x]\"\n", Some(&bin));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=3 [DESCENDANT-ERR\nout]\n");
    assert_eq!(stderr, "");
    // 2>/dev/null silences it by the command's own redirection.
    let (stdout, stderr, code) = run_script(
        &dir,
        "x=$(failcmd 2>/dev/null); echo \"rc=$? [$x]\"\n",
        Some(&bin),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=3 [out]\n");
    assert_eq!(stderr, "");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Ambient fd-2 bindings: the decline arm hands them to full execution

#[test]
fn comsub_stderr_follows_exec_installed_fd2_file() {
    let dir = scratch("execfd2");
    let bin = helper(
        &dir,
        "failcmd",
        "#!/bin/sh\necho DESCENDANT-ERR >&2\necho out\nexit 3\n",
    );
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 2>err.txt\nx=$(failcmd)\necho \"val=[$x]\"\necho \"errfile=[$(cat err.txt)]\"\n",
        Some(&bin),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "", "fd 2 is the file; nothing reaches the terminal");
    assert_eq!(stdout, "val=[out]\nerrfile=[DESCENDANT-ERR]\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_stderr_passes_inside_pipeline_stage() {
    let dir = scratch("stage");
    let (stdout, stderr, code) = run_script(&dir, "echo $(ls /nonexist) | cat\n", None);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "\n");
    assert_eq!(
        stderr, "ls: cannot access '/nonexist': No such file or directory\n",
        "a substitution expanded inside a pipeline stage keeps GNU's passthrough"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_stderr_closed_fd2_stays_silent() {
    let dir = scratch("closed");
    let bin = helper(
        &dir,
        "failcmd",
        "#!/bin/sh\necho DESCENDANT-ERR >&2\necho out\nexit 3\n",
    );
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 2>&-\nx=$(failcmd)\necho \"val=[$x] rc=$?\"\n",
        Some(&bin),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stderr, "",
        "a closed fd 2 drops the diagnostic, as GNU's does"
    );
    assert_eq!(stdout, "val=[out] rc=3\n");
    let _ = std::fs::remove_dir_all(&dir);
}
