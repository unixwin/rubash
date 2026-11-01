//! Issue 2438: a subshell of a login shell is not a login shell.
//!
//! GNU clears login_shell at every subshell entry —
//! execute_cmd.c:1637 (execute_in_subshell) and execute_cmd.c:5475
//! (execute_subshell_builtin_or_function) run
//! `login_shell = interactive = 0` under the comment "Subshells are
//! neither login nor interactive" — so `(logout)` in a login shell
//! takes logout_builtin's not-login arm (builtins/exit.def:83-87):
//! `logout: not login shell: use `exit'`, status 1, and the parent
//! list continues. rubash keyed the builtin on the invocation login
//! flag alone, so the region exited silently as a login shell with
//! status 0.
//!
//! Oracle: GNU bash 5.3.20. The command-substitution control pins
//! the opposite GNU face: a `$( )` child keeps login_shell, so
//! `$(logout)` silently exits the substitution child with the last
//! status (rc 0, empty result, no diagnostic).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run(tag: &str, args: &[&str]) -> Run {
    let dir = std::env::temp_dir().join(format!("rubash-issue2438-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    let output = Command::new(rubash_bin())
        .args(args)
        .stdin(Stdio::null())
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run shell");
    let run = Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    };
    let _ = fs::remove_dir_all(&dir);
    run
}

#[test]
fn issue2438_subshell_of_login_shell_reports_not_login() {
    for (tag, script) in [
        ("plain", "(logout); echo after rc=$?"),
        ("force", "(logout -f); echo after rc=$?"),
        // The login gate precedes argument parsing (exit.def:83
        // before get_exitstat), so a bad argument in the region is
        // still the not-login error, never the numeric one.
        ("badarg", "(logout foo); echo after rc=$?"),
    ] {
        let r = run(tag, &["-l", "-c", script]);
        assert_eq!(r.stdout, "after rc=1\n", "shape: {tag}");
        assert!(
            r.stderr.contains("logout: not login shell: use `exit'"),
            "shape {tag} stderr: {}",
            r.stderr
        );
        assert_eq!(r.code, Some(0), "shape: {tag}");
    }
}

#[test]
fn issue2438_controls() {
    // A subshell `exit` is untouched by the login predicate.
    let r = run("exit5", &["-l", "-c", "(exit 5); echo after rc=$?"]);
    assert_eq!(r.stdout, "after rc=5\n");
    assert_eq!(r.code, Some(0));

    // Non-login subshell: same not-login arm (already correct).
    let r = run("nonlogin", &["-c", "(logout); echo after rc=$?"]);
    assert_eq!(r.stdout, "after rc=1\n");
    assert!(r.stderr.contains("logout: not login shell: use `exit'"));

    // Command substitution keeps login state in GNU: silent,
    // status 0, empty capture.
    let r = run(
        "comsub",
        &["-l", "-c", "x=$(logout); echo \"rc=$? x=[$x]\""],
    );
    assert_eq!(r.stdout, "rc=0 x=[]\n");
    assert_eq!(r.stderr, "");
    assert_eq!(r.code, Some(0));
}
