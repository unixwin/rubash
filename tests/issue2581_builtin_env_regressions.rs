//! rubash#2581, as reversed by rubash#3951 — `env` is NOT a shell
//! builtin, so `builtin env` must fail like every other non-builtin
//! name.
//!
//! #2581 (closed 2026-11-02) found `env` in every builtin face
//! (BUILTIN_NAMES, the command registry row, `enable -n env`,
//! `compgen -b`) but `execute_builtin_direct_command` had no `env`
//! arm, so `builtin env` failed with "not a shell builtin" rc=1.
//! Its fix added the arm ("add the arm (one line) or remove env
//! from builtin faces"). #3951 shows that choice kept the
//! self-contradiction alive on the other faces: GNU has NO env
//! builtin (there is no builtins/env.def in bash 5.3; env is a
//! coreutils program), so the faces — not the dispatch — were the
//! wrong half. #3951 removes `env` from the builtin registration
//! (it becomes an emulated external command), and `builtin env`
//! fails again, this time for the right reason: GNU's
//! `builtin env` fails the same way (builtins/builtin.def: the
//! name is not in `shell_builtins`).
//!
//! The plain-`env` execution this file used to pin through the
//! `builtin` prefix is pinned in `issue3951_env_not_a_builtin`
//! instead (plain `env` still runs the in-process emulation).

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2581-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_2581_builtin_env_is_not_a_shell_builtin() {
    let (stdout, stderr, _code) = run_script("builtin env; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("not a shell builtin"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue_2581_builtin_env_assignment_form_fails() {
    // `builtin env NAME=value <prog>` fails before any execution:
    // the name is not a builtin, so no assignment is applied and
    // no program runs.
    let (stdout, stderr, _code) =
        run_script("builtin env ADDED2581=zzz printenv ADDED2581; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("not a shell builtin"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue_2581_enable_n_env_is_refused() {
    // `enable -n env` cannot disable a name that is not a builtin
    // (GNU: `enable: env: not a shell builtin`, rc 1) — and the
    // refusal leaves plain `env` working: the in-process emulation
    // still answers the simple-command face.
    let (stdout, stderr, _code) = run_script(
        "enable -n env; echo \"envn_rc=$?\"\nbuiltin env; echo \"builtin_rc=$?\"\nFOO2581=bar env | grep '^FOO2581='; echo \"plain_rc=$?\"\n",
    );
    assert_eq!(
        stdout, "envn_rc=1\nbuiltin_rc=1\nFOO2581=bar\nplain_rc=0\n",
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("not a shell builtin"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue_2581_builtin_echo_control() {
    let (stdout, stderr, _code) = run_script("builtin echo hi\n");
    assert_eq!(stdout, "hi\n", "stderr: {stderr}");
}
