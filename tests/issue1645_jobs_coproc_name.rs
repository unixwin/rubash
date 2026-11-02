//! Issue rubash#1645: `jobs` prints the auto-name COPROC for a
//! simple-command coproc; GNU omits it (group form agrees).
//!
//! GNU rule (vendored `third_party/bash/print_cmd.c`, `cm_coproc`
//! case, 5.3.20): the coproc name is printed ONLY when the body is
//! not a simple command. So `coproc sleep 5` displays as `coproc
//! sleep 5 &`, while `coproc { sleep 5; }` displays as `coproc
//! COPROC { sleep 5; } &` and `coproc C1 { sleep 5; }` as `coproc
//! C1 { sleep 5; } &`. (There is no `COPROC WORD simple_command`
//! production, so a simple body never carries a real name: `coproc
//! FOO sleep 5` is an unnamed coproc of the simple command
//! [FOO, sleep, 5] and displays as `coproc FOO sleep 5 &`.)
//!
//! The fix is in `coproc_command_source_text`
//! (`src/executor/command_text.rs`): print the name only when
//! `body_kind != SimpleCommand`. That rendering feeds `jobs`,
//! `jobs -l`, and every other command display, matching GNU's
//! single `print_command`.
//!
//! Note: bash 5.2's print code always prints the name, so the
//! local system bash is NOT the oracle here — the vendored 5.3.20
//! `print_cmd.c` and the issue's 5.3.20 measurements are.
#![cfg(unix)]

use std::process::Command;

/// Run a script file under the rubash binary; return (status, stdout, stderr).
fn run(script: &str) -> (i32, String, String) {
    let script_path = std::env::temp_dir().join(format!(
        "rubash-issue1645-probe-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&script_path);
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue1645_jobs_omits_name_for_simple_coproc() {
    let (code, stdout, _) =
        run("coproc sleep 5\njobs\nkill %1 2>/dev/null\nwait 2>/dev/null; true\n");
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("coproc sleep 5 &"), "stdout: {stdout}");
    assert!(
        !stdout.contains("coproc COPROC sleep 5"),
        "auto-name must not print for a simple body, stdout: {stdout}"
    );
}

#[test]
fn issue1645_jobs_l_omits_name_for_simple_coproc() {
    let (code, stdout, _) =
        run("coproc sleep 5\njobs -l\nkill %1 2>/dev/null\nwait 2>/dev/null; true\n");
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("coproc sleep 5 &"), "stdout: {stdout}");
    assert!(
        !stdout.contains("coproc COPROC sleep 5"),
        "stdout: {stdout}"
    );
}

#[test]
fn issue1645_jobs_keeps_name_for_group_coproc() {
    let (code, stdout, _) =
        run("coproc C1 { sleep 5; }\njobs\nkill %1 2>/dev/null\nwait 2>/dev/null; true\n");
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(
        stdout.contains("coproc C1 { sleep 5; } &"),
        "stdout: {stdout}"
    );
}

#[test]
fn issue1645_jobs_keeps_default_name_for_unnamed_group_coproc() {
    let (code, stdout, _) =
        run("coproc { sleep 5; }\njobs\nkill %1 2>/dev/null\nwait 2>/dev/null; true\n");
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(
        stdout.contains("coproc COPROC { sleep 5; } &"),
        "stdout: {stdout}"
    );
}

#[test]
fn issue1645_jobs_word_before_simple_body_is_not_a_name() {
    // `coproc FOO sleep 5` is an unnamed coproc of [FOO, sleep, 5];
    // the display must not invent a COPROC prefix for it.
    let (code, stdout, _) =
        run("coproc FOO sleep 5\njobs\nkill %1 2>/dev/null\nwait 2>/dev/null; true\n");
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("coproc FOO sleep 5 &"), "stdout: {stdout}");
    assert!(!stdout.contains("coproc COPROC FOO"), "stdout: {stdout}");
}

#[test]
fn issue1645_jobs_keeps_explicit_name_for_group_coproc() {
    let (code, stdout, _) =
        run("coproc FOO { sleep 5; }\njobs\nkill %1 2>/dev/null\nwait 2>/dev/null; true\n");
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(
        stdout.contains("coproc FOO { sleep 5; } &"),
        "stdout: {stdout}"
    );
}

#[test]
fn issue1645_disown_clears_coproc_listing() {
    // Control from the issue scope: disown + jobs still agrees
    // (empty listing) after the rendering change.
    let (code, stdout, _) = run("coproc sleep 5\ndisown\njobs\necho done\n");
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(!stdout.contains("coproc"), "stdout: {stdout}");
    assert!(stdout.contains("done"), "stdout: {stdout}");
}
