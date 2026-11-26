//! Convergence-lock pins for rubash#3256 — the `exec` builtin's
//! option-diagnostic prefixes, asserted byte-for-byte.
//!
//! This is a LOCK, not a fail-first pin: it is green on the base and
//! on the convergence head by design. The convergence moved the
//! three `io::sink` prefix arguments of `parse_options` in
//! `src/builtins/exec.rs` onto the shared derivation chain
//! (`builtins::diagnostic` / `builtins::prefix`); those arguments are
//! unobservable by construction, so what this pin locks is the
//! observable sibling face — `execute_impl`'s real-stderr option
//! parse and the shared `diagnostic_prefix` / `sys_error_prefix`
//! renders it depends on. Any future change to those shared renders
//! (including a migration of the literal fallback arms documented in
//! the #3256 lane report) moves these cells and must be adjudicated
//! against this lock, not slipped in.
//!
//! GNU ownership of the shapes: option diagnostics go through
//! builtin_error (builtins/common.c:83-94 builtin_error_prolog —
//! executing file + line), the bad-interpreter refusal through
//! sys_error (error.c:275-281 — the name alone, no line segment).
//! Expectations were measured from the base binary (master
//! db0c1a40) under LC_ALL=C.UTF-8; the script forms print the file
//! as invoked, the `-c` forms print argv[0] as invoked.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");
const USAGE: &str =
    "exec: usage: exec [-cl] [-a name] [command [argument ...]] [redirection ...]\n";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3256-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run_script(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn run_dash_c(root: &Path, body: &str) -> Outcome {
    let out = Command::new(BIN)
        .arg("-c")
        .arg(body)
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

#[test]
fn script_missing_a_argument() {
    let root = scratch("a-missing");
    let outcome = run_script(&root, "exec -a\n");
    assert_eq!(outcome.stdout, "");
    assert_eq!(
        outcome.stderr,
        format!("case.sh: line 1: exec: -a: option requires an argument\n{USAGE}")
    );
    assert_eq!(outcome.rc, 2);
}

#[test]
fn script_invalid_option() {
    let root = scratch("invalid-opt");
    let outcome = run_script(&root, "exec -z\n");
    assert_eq!(outcome.stdout, "");
    assert_eq!(
        outcome.stderr,
        format!("case.sh: line 1: exec: -z: invalid option\n{USAGE}")
    );
    assert_eq!(outcome.rc, 2);
}

#[test]
fn script_slash_not_found() {
    let root = scratch("not-found");
    let outcome = run_script(&root, "exec ./missing-target-xyz\n");
    assert_eq!(outcome.stdout, "");
    assert_eq!(
        outcome.stderr,
        format!(
            "case.sh: line 1: {}/missing-target-xyz: No such file or directory\n",
            root.display()
        )
    );
    assert_eq!(outcome.rc, 127);
}

#[test]
fn script_bad_interpreter_sys_error() {
    let root = scratch("badinterp");
    let bad = root.join("badinterp.sh");
    std::fs::write(&bad, "#!/nonexistent/interp-xyz\necho unreachable\n").expect("badinterp");
    let mut perms = std::fs::metadata(&bad).expect("meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bad, perms).expect("chmod");
    let outcome = run_script(&root, "exec ./badinterp.sh\n");
    assert_eq!(outcome.stdout, "");
    // sys_error prolog: the executing file alone, no line segment.
    assert_eq!(
        outcome.stderr,
        format!(
            "case.sh: {}: /nonexistent/interp-xyz: bad interpreter: No such file or directory\n",
            bad.display()
        )
    );
    assert_eq!(outcome.rc, 126);
}

#[test]
fn dash_c_missing_a_argument() {
    let root = scratch("c-a-missing");
    let outcome = run_dash_c(&root, "exec -a");
    assert_eq!(outcome.stdout, "");
    assert_eq!(
        outcome.stderr,
        format!("{BIN}: line 1: exec: -a: option requires an argument\n{USAGE}")
    );
    assert_eq!(outcome.rc, 2);
}

#[test]
fn dash_c_invalid_option() {
    let root = scratch("c-invalid-opt");
    let outcome = run_dash_c(&root, "exec -z");
    assert_eq!(outcome.stdout, "");
    assert_eq!(
        outcome.stderr,
        format!("{BIN}: line 1: exec: -z: invalid option\n{USAGE}")
    );
    assert_eq!(outcome.rc, 2);
}

#[test]
fn dash_c_slash_not_found() {
    let root = scratch("c-not-found");
    let outcome = run_dash_c(&root, "exec ./missing-target-xyz");
    assert_eq!(outcome.stdout, "");
    assert_eq!(
        outcome.stderr,
        format!(
            "{BIN}: line 1: {}/missing-target-xyz: No such file or directory\n",
            root.display()
        )
    );
    assert_eq!(outcome.rc, 127);
}
