//! Regression pins for rubash#3148 — the `exec` builtin face did not
//! classify a bad interpreter: it reported the raw OS error text with
//! a `line 1:` segment and an `exec:` infix at 127, where GNU reports
//! the shell_execve sys_error classification at 126.
//!
//! GNU ownership: builtins/exec.def:172 rebinds the target with
//! `full_pathname` (general.c:931 — cwd join with MP_RMDOT, so the
//! report name carries no `/./` residue) before shell_execve
//! (execute_cmd.c:6126). A failed execve on an executable `#!` file is
//! classified at execute_cmd.c:6184 through
//! `sys_error ("%s: %s: %s", command, interp, "bad interpreter")` for
//! ANY errno (the sample check is errno-agnostic; only the directory,
//! non-executable, and E2BIG/ENOMEM arms pre-empt it) and returns
//! EX_NOEXEC (126). sys_error is error.c:275 -> error_prolog (0): the
//! prolog is get_name_for_error alone — no line segment, and no
//! builtin-name infix (this is not a builtin_error report). The
//! sibling classifications (file_error, error.c:126) go through
//! error_prolog (1) and keep the line segment; the missing-slash-name
//! and lookup-failure locks below pin that contrast.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same probe
//! scripts under LC_ALL=C.UTF-8, measured against the oracle binary.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3148-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    // The report name is built from the shell's (physical) cwd, so the
    // fixture root is canonicalized once and used in that form for the
    // child cwd, the fixture shebangs, and every expectation.
    std::fs::canonicalize(&dir).expect("canonical root")
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
    // An interpreter that exists but is not executable: the kernel
    // refuses the interpreter with EACCES, and GNU's errno-agnostic
    // sample check still classifies it as a bad interpreter
    // (execute_cmd.c:6166-6194) — sys_error prolog, EX_NOEXEC.
    write(&root.join("plaininterp"), "echo plain\n", 0o644);
    write(
        &root.join("badinterp_eacces"),
        &format!("#!{}/plaininterp\necho hi\n", root.display()),
        0o755,
    );
    root
}

fn bad_interpreter_line(root: &Path) -> String {
    format!(
        "case.sh: {}/badinterp: /nonexistent-interp: bad interpreter: No such file or directory\n",
        root.display()
    )
}

#[test]
fn exec_bad_interpreter_top_level_is_sys_error_126() {
    let root = fixture("toplevel");
    let out = run(&root, "exec ./badinterp\n");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, bad_interpreter_line(&root));
    assert_eq!(out.rc, 126);
}

#[test]
fn exec_bad_interpreter_via_path_reports_full_pathname() {
    let root = fixture("path");
    let out = run(&root, "PATH=./bin2:/usr/bin:/bin\nexec badinterp\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!(
            "case.sh: {}/bin2/badinterp: /nonexistent-interp: bad interpreter: No such file or directory\n",
            root.display()
        )
    );
    assert_eq!(out.rc, 126);
}

#[test]
fn exec_bad_interpreter_in_command_substitution() {
    let root = fixture("comsub");
    let out = run(&root, "x=$(exec ./badinterp); echo \"rc=$?\"\n");
    assert_eq!(out.stdout, "rc=126\n");
    assert_eq!(out.stderr, bad_interpreter_line(&root));
    assert_eq!(out.rc, 0);
}

#[test]
fn exec_bad_interpreter_in_subshell() {
    let root = fixture("subshell");
    let out = run(&root, "(exec ./badinterp); echo \"rc=$?\"\n");
    assert_eq!(out.stdout, "rc=126\n");
    assert_eq!(out.stderr, bad_interpreter_line(&root));
    assert_eq!(out.rc, 0);
}

#[test]
fn exec_bad_interpreter_in_pipeline_stage() {
    let root = fixture("pipeline");
    let out = run(
        &root,
        "exec ./badinterp | cat; echo \"st=${PIPESTATUS[0]}\"\n",
    );
    assert_eq!(out.stdout, "st=126\n");
    assert_eq!(out.stderr, bad_interpreter_line(&root));
    assert_eq!(out.rc, 0);
}

#[test]
fn exec_bad_interpreter_eacces_interpreter_is_still_bad_interpreter() {
    let root = fixture("eacces");
    let out = run(&root, "exec ./badinterp_eacces\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!(
            "case.sh: {}/badinterp_eacces: {}/plaininterp: bad interpreter: Permission denied\n",
            root.display(),
            root.display()
        )
    );
    assert_eq!(out.rc, 126);
}

#[test]
fn exec_missing_slash_name_keeps_line_segment_and_127() {
    let root = fixture("missing");
    let out = run(&root, "exec ./missing\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!(
            "case.sh: line 1: {}/missing: No such file or directory\n",
            root.display()
        )
    );
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_not_found_keeps_builtin_error_form() {
    let root = fixture("notfound");
    let out = run(&root, "exec nosuchcmd3148\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "case.sh: line 1: exec: nosuchcmd3148: not found\n"
    );
    assert_eq!(out.rc, 127);
}
