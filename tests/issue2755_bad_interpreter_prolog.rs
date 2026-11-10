//! Regression pins for rubash#2755 — the bad-interpreter exec
//! diagnostic carried a `line 1:` segment GNU omits.
//!
//! GNU ownership: execute_cmd.c:6184 reports the unresolvable-#!
//! refusal through `sys_error ("%s: %s: %s", command, interp,
//! "bad interpreter")`, and error.c sys_error is the one reporter
//! built on `error_prolog (0)` — the prolog is get_name_for_error
//! alone, never a line segment, in any context. The sibling
//! shell_execve classifications go through internal_error /
//! file_error (error_prolog (1)) and DO carry the line segment;
//! that contrast is pinned here as a guard (`./somedir`).
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2755-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
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

fn write(path: &Path, content: &str, mode: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(path, content).expect("fixture file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("fixture mode");
}

fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    write(
        &root.join("badinterp"),
        "#!/nonexistent-interp\necho hi\n",
        0o755,
    );
    write(
        &root.join("bin2").join("badinterp"),
        "#!/nonexistent-interp\necho hi\n",
        0o755,
    );
    std::fs::create_dir_all(root.join("somedir")).expect("somedir");
    root
}

#[test]
fn bad_interpreter_prolog_has_no_line_segment() {
    let root = fixture("prolog");
    let out = run(&root, "./badinterp; echo \"rc=$?\"\n");
    assert_eq!(out.stdout, "rc=126\n");
    assert_eq!(
        out.stderr,
        "case.sh: ./badinterp: /nonexistent-interp: bad interpreter: No such file or directory\n"
    );
    assert_eq!(out.rc, 0);
}

#[test]
fn bad_interpreter_via_path_has_no_line_segment() {
    let root = fixture("path");
    let out = run(
        &root,
        "PATH=./bin2:/usr/bin:/bin\nbadinterp; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "rc=126\n");
    assert!(
        out.stderr
            .contains("badinterp: /nonexistent-interp: bad interpreter: No such file or directory"),
        "stderr={:?}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("line"),
        "line segment leaked: stderr={:?}",
        out.stderr
    );
    assert_eq!(out.rc, 0);
}

#[test]
fn sibling_is_a_directory_keeps_its_line_segment() {
    let root = fixture("isdir");
    let out = run(&root, "./somedir; echo \"rc=$?\"\n");
    assert_eq!(out.stdout, "rc=126\n");
    assert_eq!(out.stderr, "case.sh: line 1: ./somedir: Is a directory\n");
    assert_eq!(out.rc, 0);
}
