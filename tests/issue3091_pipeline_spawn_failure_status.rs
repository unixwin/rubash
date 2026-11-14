//! Regression pins for rubash#3091 — a pipeline member whose spawn
//! fails (file exists but is not executable, or does not exist) must
//! land the same execve-errno classification and status as the simple
//! command form, and its diagnostic must name the command the way GNU
//! names it.
//!
//! Base behaviour (master f17644d7 era, probed on 919562bf-era and
//! 8a28d350 builds): `./pnoexec | cat` reports the member status as
//! empty / 0 in PIPESTATUS (GNU: 126) and the diagnostic drops the
//! as-typed `./` spelling (`pnoexec: Permission denied`); the plain
//! PATH-found member likewise reports 0 and the bare name instead of
//! GNU's resolved pathname. The simple-command form of the same file
//! already classifies 126 correctly — only pipeline members collapse.
//!
//! GNU ownership: every pipeline element is executed through
//! execute_simple_command (execute_cmd.c:2620), so a member's spawn
//! failure is classified by shell_execve's errno table
//! (execute_cmd.c:6126-6159: EACCES on an existing file => 126
//! "Permission denied", ENOENT => 127 "No such file or directory")
//! and the per-member statuses are recorded into PIPESTATUS
//! (execute_cmd.c:1210-1224). Diagnostics name a slash-containing
//! member by its as-typed bytes and a PATH-found member by the
//! resolved pathname shell_execve actually attempted.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts (script-file form, LC_ALL=C.UTF-8).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3091-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write(path: &Path, content: &str, mode: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(path, content).expect("fixture file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("fixture mode");
}

/// Fixture: `sub/pnoexec` (0644 plain file) and `sub/pexec` (0755
/// runnable script), mirroring the issue's repro fixture.
fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    write(&root.join("sub").join("pnoexec"), "not a program\n", 0o644);
    write(
        &root.join("sub").join("pexec"),
        "#!/bin/sh\necho RAN\n",
        0o755,
    );
    root
}

struct Outcome {
    stdout: String,
    stderr: String,
    #[allow(dead_code)]
    rc: i32,
}

/// Run one probe script (written to `<root>/case.sh`) with the
/// fixture's `sub` directory as cwd, exactly like the issue repro's
/// `cd "$FIX/sub"`.
fn run(root: &Path, body: &str) -> Outcome {
    let script = root.join("case.sh");
    std::fs::write(&script, body).expect("case script");
    let out = Command::new(BIN)
        .arg(&script)
        .current_dir(root.join("sub"))
        .env("LC_ALL", "C.UTF-8")
        .env("FIX", root)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn script_prefix(root: &Path) -> String {
    format!("{}: line 1: ", root.join("case.sh").display())
}

// ---- member spawn failure: status + diagnostic spelling ----

#[test]
fn first_member_eacces_lands_126_with_as_typed_name() {
    let root = fixture("first");
    let out = run(
        &root,
        "./pnoexec | cat; echo \"rc=${PIPESTATUS[0]} all=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(out.stdout, "rc=126 all=126 0\n");
    assert_eq!(
        out.stderr,
        format!("{}./pnoexec: Permission denied\n", script_prefix(&root))
    );
}

#[test]
fn last_member_eacces_lands_126_with_as_typed_name() {
    let root = fixture("last");
    let out = run(
        &root,
        "echo hi | ./pnoexec; echo \"rc=${PIPESTATUS[1]} all=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(out.stdout, "rc=126 all=0 126\n");
    assert_eq!(
        out.stderr,
        format!("{}./pnoexec: Permission denied\n", script_prefix(&root))
    );
}

#[test]
fn middle_member_eacces_lands_126_with_as_typed_name() {
    let root = fixture("middle");
    let out = run(
        &root,
        "echo hi | ./pnoexec | cat; echo \"all=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(out.stdout, "all=0 126 0\n");
    assert_eq!(
        out.stderr,
        format!("{}./pnoexec: Permission denied\n", script_prefix(&root))
    );
}

#[test]
fn path_found_member_eacces_lands_126_with_resolved_name() {
    let root = fixture("plain");
    let out = run(
        &root,
        "export PATH=\"$FIX/sub:/usr/bin:/bin\"\npnoexec | cat; echo \"rc=${PIPESTATUS[0]}\"\n",
    );
    assert_eq!(out.stdout, "rc=126\n");
    // GNU names a PATH-found member by the pathname execve attempted.
    assert_eq!(
        out.stderr,
        format!(
            "{}: line 2: {}/sub/pnoexec: Permission denied\n",
            root.join("case.sh").display(),
            root.display()
        )
    );
}

#[test]
fn missing_member_lands_127_with_as_typed_name() {
    let root = fixture("enoent");
    let out = run(&root, "./nosuchfile | cat; echo \"rc=${PIPESTATUS[0]}\"\n");
    assert_eq!(out.stdout, "rc=127\n");
    assert_eq!(
        out.stderr,
        format!(
            "{}./nosuchfile: No such file or directory\n",
            script_prefix(&root)
        )
    );
}

// ---- chain: the recorded member status feeds the next command ----

#[test]
fn pipestatus_of_failed_member_feeds_a_branch() {
    let root = fixture("chain");
    let out = run(
        &root,
        "./pnoexec | cat; s=${PIPESTATUS[0]}; if [ \"$s\" -eq 126 ]; then echo CHAIN-126; else echo \"CHAIN-OTHER-$s\"; fi\n",
    );
    assert_eq!(out.stdout, "CHAIN-126\n");
    assert_eq!(
        out.stderr,
        format!("{}./pnoexec: Permission denied\n", script_prefix(&root))
    );
}

// ---- controls (expected green on the base: regression locks) ----

#[test]
fn simple_command_form_classifies_126() {
    let root = fixture("simple");
    let out = run(&root, "./pnoexec; echo \"rc=$?\"\n");
    assert_eq!(out.stdout, "rc=126\n");
    // The simple form's classification text is already correct on the
    // base; its name spelling rides a different face (see REPORT), so
    // this lock asserts the status and the classification text only.
    assert!(
        out.stderr.contains("Permission denied"),
        "stderr={:?}",
        out.stderr
    );
}

#[test]
fn healthy_member_still_runs_and_lands_0() {
    let root = fixture("healthy");
    let out = run(&root, "./pexec | cat; echo \"rc=${PIPESTATUS[0]}\"\n");
    assert_eq!(out.stdout, "RAN\nrc=0\n");
    assert_eq!(out.stderr, "");
}
