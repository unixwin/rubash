//! rubash#1228 — the shell must initialize `$0` from argv[0] as
//! exec'd, not from its own binary name. Filed against the niu host
//! path; the zero-queue campaign's x6 cluster falsified it for the
//! engine itself: spawning the rubash binary with argv[0] `hello` /
//! `-hello` yields `$0` `hello` / `-hello`, exactly like GNU. This pin
//! keeps the engine-side contract in CI (host-side delivery is the
//! niubash repo's own surface).

#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

fn rubash_with_arg0(arg0: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg0(arg0)
        .args(["-c", "echo $0"])
        .env_clear()
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1228_dollar_zero_comes_from_argv0() {
    let (stdout, stderr, code) = rubash_with_arg0("hello");
    assert_eq!(stdout, "hello\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1228_dollar_zero_keeps_login_dash_prefix() {
    let (stdout, stderr, code) = rubash_with_arg0("-hello");
    assert_eq!(stdout, "-hello\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
