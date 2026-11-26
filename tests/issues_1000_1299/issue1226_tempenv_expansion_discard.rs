//! Issue unixwin/rubash#1226 regression: a failing assignment-prefix
//! expansion DISCARDs the whole command (GNU subst.c:13092-13106
//! shell_expand_word_list -> exp_jump_to_top_level(DISCARD)) — the command
//! is never run, the rest of the list is abandoned, and a non-posix script
//! continues on the next line. POSIX mode is FORCE_EOF instead
//! (subst.c:10881-10888 expand_wdesc_fatal).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn tempenv_arith_failure_skips_command_nonposix() {
    // GNU: prints the diagnostic, skips `echo hi`, continues on the next
    // line — `after rc=1 x=[]`; final status is the echo's 0.
    let (stdout, stderr, code) = rubash("x=$((1/0)) echo hi\necho \"after rc=$? x=[$x]\"\n");
    assert_eq!(stdout, "after rc=1 x=[]\n");
    assert!(stderr.contains("division by 0"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn tempenv_arith_failure_abandons_rest_of_line() {
    // Same command list on one line: the DISCARD abandons the tail.
    let (stdout, _, _) = rubash("x=$((1/0)) echo hi; echo \"after rc=$?\"\n");
    assert_eq!(stdout, "");
}

#[test]
fn tempenv_arith_failure_posix_exits() {
    // POSIX mode: a tempenv assignment expansion error is expand_wdesc_fatal
    // — no `hi`, no `after`, exit 1 in script mode (FORCE_EOF remaps to 127
    // only at the `-c` top-level catch, shell.c:1471).
    let script = std::env::temp_dir().join("issue1226_posix.sh");
    std::fs::write(
        &script,
        "set -o posix\nx=$((1/0)) echo hi\necho \"after rc=$?\"\n",
    )
    .expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "");
    assert!(stderr.contains("division by 0"), "stderr: {stderr}");
    assert_eq!(output.status.code(), Some(1));
}
