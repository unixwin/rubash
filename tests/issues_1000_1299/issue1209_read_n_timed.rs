//! rubash#1209 regressions: `echo "hello" | { read -n 2 -t 5 c; }` bound
//! the whole line — the timed-read fast path modeled the -t deadline but
//! ignored -n/-N quotas (the options were parsed then skipped). GNU
//! read.def:884-886 stops -n at the quota or the delimiter and reads -N's
//! quota exactly; the fast path now carries the quota through and applies
//! it to the timed value.
//!
//! Expectations verified byte-for-byte against WSL GNU Bash 5.3.20.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1209-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn read_n_with_timeout_takes_quota() {
    let (stdout, _, code) =
        rubash_file("echo \"hello\" | { read -n 2 -t 5 c; echo \"c=[$c]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "c=[he]\n");
}

#[test]
fn read_capital_n_with_timeout_takes_quota() {
    let (stdout, _, code) =
        rubash_file("echo \"hello\" | { read -N 2 -t 5 c; echo \"c=[$c]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "c=[he]\n");
}

#[test]
fn read_n_eof_before_quota_fails() {
    let (stdout, _, code) =
        rubash_file("printf 'hi' | { read -n5 -t5 x; echo \"rc=$? x=[$x]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 x=[hi]\n");
}

#[test]
fn read_capital_n_eof_before_quota_keeps_record_and_fails() {
    let (stdout, _, code) =
        rubash_file("printf 'hi\\n' | { read -N5 -t5 x; echo \"rc=$? x=[$x]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 x=[hi\n]\n");
}
