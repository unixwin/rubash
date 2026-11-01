//! Issue 2437: `logout` with a non-numeric argument in a login
//! shell must error and continue, not exit silently.
//!
//! GNU builtins/exit.def:79-90 (logout_builtin) delegates to
//! exit_or_logout, whose argument parse is builtins/common.c:519-554
//! get_exitstat: a bad number is sh_neednumarg ("numeric argument
//! required", common.c:182-185) and get_exitstat returns EX_USAGE
//! (258); exit_or_logout returns any value above EX_SHERRBASE (256)
//! without jumping (exit.def:141-144), so the shell continues with
//! $? = 2 and ~/.bash_logout is never sourced. rubash parsed the
//! word with unwrap_or(0) and exited the whole shell silently.
//!
//! Oracle: GNU bash 5.3.20 — `bash -l -c 'logout foo; echo after
//! rc=$?'` prints `after rc=2` and the shell exits 0.

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
    let dir = std::env::temp_dir().join(format!("rubash-issue2437-{tag}-{}", std::process::id()));
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
fn issue2437_login_logout_non_numeric_arg_errors_and_continues() {
    for (tag, script, word) in [
        ("foo", "logout foo; echo after rc=$?", "foo"),
        ("3x", "logout 3x; echo after rc=$?", "3x"),
    ] {
        let r = run(tag, &["-l", "-c", script]);
        assert_eq!(r.stdout, "after rc=2\n", "shape: {tag}");
        assert!(
            r.stderr
                .contains(&format!("logout: {word}: numeric argument required")),
            "shape {tag} stderr: {}",
            r.stderr
        );
        assert_eq!(r.code, Some(0), "shape: {tag}");
    }
}

#[test]
fn issue2437_login_logout_bad_arg_inside_function_continues() {
    let r = run(
        "fn",
        &[
            "-l",
            "-c",
            "f(){ logout foo; echo in-fn rc=$?; }; f; echo after rc=$?",
        ],
    );
    assert_eq!(r.stdout, "in-fn rc=2\nafter rc=0\n");
    assert!(r.stderr.contains("logout: foo: numeric argument required"));
    assert_eq!(r.code, Some(0));
}

#[test]
fn issue2437_controls() {
    // A numeric argument still exits with that status.
    let r = run("num", &["-l", "-c", "logout 3; echo after"]);
    assert_eq!(r.stdout, "");
    assert_eq!(r.code, Some(3));

    // get_exitstat skips a leading `--` before the status word.
    let r = run("dashdash", &["-l", "-c", "logout -- 5; echo after"]);
    assert_eq!(r.stdout, "");
    assert_eq!(r.code, Some(5));

    // No argument: exits with the last command's status.
    let r = run("bare", &["-l", "-c", "false; logout; echo after"]);
    assert_eq!(r.stdout, "");
    assert_eq!(r.code, Some(1));

    // Non-login shell: the login gate fires before argument parsing.
    let r = run("nonlogin", &["-c", "logout foo; echo after rc=$?"]);
    assert_eq!(r.stdout, "after rc=1\n");
    assert!(r.stderr.contains("logout: not login shell: use `exit'"));
    assert_eq!(r.code, Some(0));

    // CHECK_HELPOPT (exit.def:81): help on stdout, status 2, the
    // shell continues.
    let r = run("help", &["-l", "-c", "logout --help; echo after rc=$?"]);
    assert!(
        r.stdout.starts_with("logout: logout [n]\n    Exit a login shell.\n"),
        "help stdout: {}",
        r.stdout
    );
    assert!(r.stdout.ends_with("after rc=2\n"), "help stdout: {}", r.stdout);
    assert_eq!(r.code, Some(0));

    // Too many arguments: the no_args DISCARD class — under `-c` the
    // rest of the string is abandoned and the shell exits 1.
    let r = run("toomany", &["-l", "-c", "logout 1 2; echo after"]);
    assert_eq!(r.stdout, "");
    assert!(r.stderr.contains("logout: too many arguments"));
    assert_eq!(r.code, Some(1));
}
