//! rubash#3198 — external spawns from a deleted session cwd.
//!
//! GNU: cd moves the process cwd itself (builtins/cd.def cd_builtin ->
//! chdir), and execute_cmd.c's execute_disk_command forks a child that
//! inherits that cwd — no path is ever re-resolved at spawn. A
//! directory unlinked while it is the cwd therefore stays a valid cwd:
//! absolute-path commands run, `.` lists empty, `..` still names the
//! real parent. Under G-02 rubash's session cwd is a FIELD, and every
//! spawn used to stamp it with `Command::current_dir(path)` — the
//! child-side chdir re-resolves the string, fails ENOENT once the
//! directory is gone, and EVERY foreground external spawn died with
//! `cannot execute: required file not found`, rc 127 (measured on the
//! base tree: wc/ls/grep/id all 127; the in-process and pipeline faces
//! were unaffected).
//!
//! The fix (stamp_spawn_cwd in external_inner.rs) keeps an open
//! directory handle — the cwd anchor — from the moment the directory
//! is entered, and fchdir's the child onto that inode when the path
//! no longer resolves. These pins assert COMPLETION FORMS (output
//! bytes + exit status), never timing.
//!
//! Oracle: GNU bash 5.3.20, script-file probes on the base tree:
//!   wc "$D/f" -> `1 1 6 <D>/f` rc 0; bare `ls` -> empty rc 0;
//!   grep -q -> rc 0; id -> rc 0; `cat ../f` -> `hello` rc 0;
//!   `cat f` (a parent file, not in the deleted dir) -> rc 1;
//!   `x=$(/usr/bin/id -u)` -> the uid, rc 0; pipeline `wc -l` -> 1.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3198-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

fn tool_path() -> String {
    if cfg!(windows) {
        std::env::var("PATH").unwrap_or_default()
    } else {
        "/usr/bin:/bin".to_string()
    }
}

/// Spawn rubash on `script` inside `dir` (the harness cwd is the
/// scratch root D); returns (stdout, stderr, status code).
fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", tool_path())
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
        for name in [
            "SystemRoot",
            "SystemDrive",
            "COMSPEC",
            "PATHEXT",
            "OS",
            "TEMP",
            "TMP",
        ] {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
    }
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

/// Build the deleted-cwd stage in `dir`: D/f is a one-line file, the
/// shell cd's into D/a and a subshell removes it. Returns the script
/// prelude shared by every pin.
fn prelude(dir: &Path) -> String {
    std::fs::write(dir.join("f"), "hello\n").expect("write f");
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n".to_string()
}

#[test]
fn issue3198_foreground_absolute_command_runs() {
    let dir = scratch("wc");
    let script = format!("{}wc \"$HOME/f\"; echo rc=$?\n", prelude(&dir.0));
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    let expected = format!("1 1 6 {}/f\nrc=0\n", dir.0.display());
    assert_eq!(stdout, expected, "stderr:\n{stderr}");
}

#[test]
fn issue3198_bare_ls_lists_the_deleted_empty_dir() {
    let dir = scratch("ls");
    let script = format!("{}ls; echo rc=$?\n", prelude(&dir.0));
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // The deleted directory is empty: GNU prints nothing, rc 0. A
    // fallback that births the child in some OTHER directory (the
    // process cwd, a parent) lists that directory's entries instead.
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3198_grep_and_id_run() {
    let dir = scratch("grep-id");
    let script = format!(
        "{}grep -q hello \"$HOME/f\"; echo grep=$?; id >/dev/null; echo id=$?\n",
        prelude(&dir.0)
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "grep=0\nid=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3198_relative_parent_still_resolves() {
    let dir = scratch("dotdot");
    let script = format!("{}cat ../f; echo rc=$?\n", prelude(&dir.0));
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // `..` from the deleted directory is its real parent: only a
    // child born in the deleted inode itself (the anchor fchdir)
    // resolves this; a child born anywhere else cannot.
    assert_eq!(stdout, "hello\nrc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3198_relative_missing_file_fails_like_gnu() {
    let dir = scratch("missing");
    let script = format!("{}cat f 2>/dev/null; echo rc=$?\n", prelude(&dir.0));
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // `f` lives in the PARENT, not in the deleted directory: GNU's
    // child gets ENOENT (cat rc 1) — it must not "find" the file by
    // being born in the parent or the process cwd.
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
}

#[test]
fn issue3198_comsub_external_runs() {
    let dir = scratch("comsub");
    let script = format!(
        "{}x=$(id -u); echo \"uid:$x rc=$?\"\n",
        prelude(&dir.0)
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert!(
        stdout.starts_with("uid:") && stdout.trim_end().ends_with("rc=0") && stdout.len() > 9,
        "stdout: {stdout:?} stderr:\n{stderr}"
    );
}

#[test]
fn issue3198_pipeline_stage_still_runs() {
    // Regression lock: the pipeline face already worked on the base
    // tree (its stages are born through a different spawn shape) and
    // must keep working through the shared stamp.
    let dir = scratch("pipe");
    let script = format!(
        "{}printf 'x\\n' | wc -l; echo rc=$?\n",
        prelude(&dir.0)
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "1\nrc=0\n", "stderr:\n{stderr}");
}
