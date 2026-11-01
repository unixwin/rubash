//! Issue 2456: a process-substitution child keeps the parent's
//! login-ness.
//!
//! GNU's procsub fork (subst.c:6412-6445 process_substitute) never
//! passes through the two `login_shell = 0` reset sites —
//! execute_cmd.c:1636-1637 (execute_in_subshell) and
//! execute_cmd.c:5474-5475 (execute_subshell_builtin_or_function) —
//! so the child inherits the parent's CURRENT login_shell global.
//! In a login shell `<(logout; echo hi)` therefore exits the child
//! silently at `logout` (builtins/exit.def:83-88 takes the login
//! arm) and `hi` never prints; no ~/.bash_logout is sourced either
//! (exit.def:160 requires subshell_environment == 0). rubash's
//! procsub body rides a respawn transport that was born
//! invocation_login=false, so it printed the not-login diagnostic
//! and `hi`.
//!
//! The inherited value is the parent's global AT CONSTRUCTION: a
//! procsub built inside a `( )` region inherits the cleared global
//! (diagnostic + `hi`), while one built inside a command
//! substitution (subst.c:7143 — also not a reset boundary) keeps
//! login-ness.
//!
//! Oracle: GNU bash 5.3.20, byte-compared on every shape below.

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
    let dir = std::env::temp_dir().join(format!("rubash-issue2456-{tag}-{}", std::process::id()));
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
fn issue2456_input_procsub_child_of_login_shell_logs_out_silently() {
    let r = run(
        "input-login",
        &["-l", "-c", "cat <(logout; echo hi); echo after rc=$?"],
    );
    assert_eq!(r.stdout, "after rc=0\n");
    assert_eq!(r.stderr, "");
    assert_eq!(r.code, Some(0));
}

#[test]
fn issue2456_output_procsub_child_of_login_shell_logs_out_silently() {
    // The producer is asynchronous: the script sleeps so the child
    // has run before the marker is checked. GNU leaves the marker
    // absent (the child exits at logout, `hi` never runs).
    let dir = std::env::temp_dir().join(format!("rubash-issue2456-out-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    let marker = dir.join("marker");
    let script = format!(
        "tee >(logout; echo hi >{}) </dev/null >/dev/null; sleep 1",
        marker.display()
    );
    let output = Command::new(rubash_bin())
        .args(["-l", "-c", &script])
        .stdin(Stdio::null())
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run shell");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(
        !marker.exists(),
        "producer wrote past logout: {}",
        fs::read_to_string(&marker).unwrap_or_default()
    );
    // Positive control: the same transport without logout delivers.
    let marker2 = dir.join("marker2");
    let script2 = format!(
        "tee >(echo hi >{}) </dev/null >/dev/null; sleep 1",
        marker2.display()
    );
    let output2 = Command::new(rubash_bin())
        .args(["-l", "-c", &script2])
        .stdin(Stdio::null())
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run shell");
    assert_eq!(output2.status.code(), Some(0));
    assert_eq!(
        fs::read_to_string(&marker2).expect("positive control marker"),
        "hi\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2456_procsub_logout_does_not_source_bash_logout() {
    // exit.def:160: bash_logout runs only when
    // subshell_environment == 0; the procsub child's is nonzero, so
    // a successful logout there leaves no trace.
    let dir = std::env::temp_dir().join(format!("rubash-issue2456-lo-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    let marker = dir.join("logout-ran");
    fs::write(
        dir.join(".bash_logout"),
        format!("echo ran > {}\n", marker.display()),
    )
    .expect("write .bash_logout");
    let output = Command::new(rubash_bin())
        .args(["-l", "-c", "cat <(logout; echo hi); echo after"])
        .stdin(Stdio::null())
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run shell");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "after\n");
    assert!(!marker.exists(), "~/.bash_logout ran in a procsub child");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2456_procsub_controls() {
    // Non-login parent: no login-ness to inherit — diagnostic + hi.
    let r = run(
        "nonlogin",
        &["-c", "cat <(logout; echo hi); echo after rc=$?"],
    );
    assert_eq!(r.stdout, "hi\nafter rc=0\n");
    assert!(
        r.stderr.contains("logout: not login shell: use `exit'"),
        "stderr: {}",
        r.stderr
    );

    // Constructed inside a `( )` region of a login shell: the
    // parent global was cleared at the region entry, so the child
    // inherits the cleared value — diagnostic + hi.
    let r = run(
        "in-parens",
        &["-l", "-c", "( cat <(logout; echo hi) ); echo after rc=$?"],
    );
    assert_eq!(r.stdout, "hi\nafter rc=0\n");
    assert!(
        r.stderr.contains("logout: not login shell: use `exit'"),
        "stderr: {}",
        r.stderr
    );

    // Constructed inside a command substitution of a login shell:
    // comsub is not a reset boundary either, so login-ness crosses
    // both forks — silent, empty capture.
    let r = run(
        "in-comsub",
        &["-l", "-c", "x=$(cat <(logout; echo hi)); echo \"x=[$x]\""],
    );
    assert_eq!(r.stdout, "x=[]\n");
    assert_eq!(r.stderr, "");

    // Nested process substitution: the grandchild inherits the
    // state the child was born with.
    let r = run(
        "nested",
        &["-l", "-c", "cat <(cat <(logout; echo hi); echo outer)"],
    );
    assert_eq!(r.stdout, "outer\n");
    assert_eq!(r.stderr, "");
}
