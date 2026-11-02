//! rubash#1651 — coproc ignores here-string / here-doc stdin
//! redirects; the coproc keeps stdin on the coproc pipe and
//! blocks forever (GNU honors the redirection: the coproc
//! consumes the body and exits on its own).
//!
//! Root cause: apply_coproc_redirects only handled the
//! single-field redirects (redirect_in/out/append/err), and a
//! memory-backed body never lands in redirect_in — a
//! here-string lives in cmd.redirects + cmd.here_string, a
//! heredoc in cmd.redirects + cmd.heredoc_redirects — so the
//! child stayed on the coproc pipe. GNU redir.c
//! do_redirections binds fd 0 to the body before the child
//! runs. The fix resolves the winning fd-0 body (same
//! last-wins ordering as stdin_string_for_command_mut, whose
//! here-string expansion also strips quotes segment-wise)
//! and feeds it to the child through a pre-filled pipe
//! (writer thread, so a body larger than the pipe buffer
//! cannot block the spawn; the parent's coproc write end
//! stays registered as ${COPROC[1]}, useless but present,
//! like GNU's rebound pipe end).
//!
//! Only HereDoc/HereString winners take this path: `< file`
//! keeps the real-file path, and `<&N` dups keep the pipe
//! (streaming a dup through a string buffer would change
//! blocking behavior).
//!
//! Expectations verified against GNU bash 5.3.20. Stdin is
//! nulled so a regression blocks on nothing.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1651-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const ALIVE_PROBE: &str = "sleep 0.3\nif kill -0 $COPROC_PID 2>/dev/null; then echo \"STILL-ALIVE\"; kill $COPROC_PID 2>/dev/null; else echo \"DEAD\"; fi\nwait $COPROC_PID 2>/dev/null; echo \"wait_rc=$?\"";

#[test]
fn coproc_herestring_stdin_consumed_and_exits() {
    let script = format!("coproc cat <<< \"hello\"\n{ALIVE_PROBE}\n");
    let (stdout, _, code) = rubash_script(&script);
    assert_eq!(stdout, "DEAD\nwait_rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_heredoc_stdin_consumed_and_exits() {
    let script = format!("coproc cat << \"HDONE\"\nhello\nHDONE\n{ALIVE_PROBE}\n");
    let (stdout, _, code) = rubash_script(&script);
    assert_eq!(stdout, "DEAD\nwait_rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_herestring_content_matches_gnu() {
    let (stdout, _, code) =
        rubash_c("coproc cat <<< \"hello\"; cat <&${COPROC[0]}; wait $COPROC_PID; echo \"rc=$?\"");
    assert_eq!(stdout, "hello\nrc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_herestring_expansion_and_quotes() {
    let (stdout, _, code) =
        rubash_c("v=WORLD; coproc cat <<< \"hi $v\"; cat <&${COPROC[0]}; wait $COPROC_PID");
    assert_eq!(stdout, "hi WORLD\n");
    assert_eq!(code, Some(0));
    // Single quotes are literal, double quotes inside survive.
    let (stdout, _, _) =
        rubash_c("coproc cat <<< 'double\"quote'; cat <&${COPROC[0]}; wait $COPROC_PID");
    assert_eq!(stdout, "double\"quote\n");
}

#[test]
fn coproc_heredoc_content_and_expansion() {
    let (stdout, _, code) =
        rubash_c("v=W; coproc cat << HDONE\nhi $v\nHDONE\ncat <&${COPROC[0]}; wait $COPROC_PID");
    assert_eq!(stdout, "hi W\n");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_file_stdin_still_works() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1651f-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    std::fs::write(dir.join("in.txt"), "FROMFILE\n").expect("seed file");
    let script = format!(
        "coproc cat < {}\n{ALIVE_PROBE}\n",
        dir.join("in.txt").display()
    );
    let (stdout, _, code) = rubash_script(&script);
    assert_eq!(stdout, "DEAD\nwait_rc=0\n");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coproc_last_fd0_redirect_wins_both_directions() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1651w-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    std::fs::write(dir.join("in.txt"), "FROMFILE\n").expect("seed file");
    let file = dir.join("in.txt").display().to_string();
    let (stdout, _, _) = rubash_c(&format!(
        "coproc cat < {file} <<< \"FROMHS\"; cat <&${{COPROC[0]}}; wait $COPROC_PID"
    ));
    assert_eq!(stdout, "FROMHS\n");
    let (stdout, _, _) = rubash_c(&format!(
        "coproc cat <<< \"FROMHS\" < {file}; cat <&${{COPROC[0]}}; wait $COPROC_PID"
    ));
    assert_eq!(stdout, "FROMFILE\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coproc_compound_body_herestring() {
    let (stdout, _, code) = rubash_c(
        "coproc { cat; } <<< \"grp\"; cat <&${COPROC[0]}; wait $COPROC_PID; echo \"rc=$?\"",
    );
    assert_eq!(stdout, "grp\nrc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_large_heredoc_no_deadlock() {
    let mut script = String::from("coproc cat << \"BIG\"\n");
    for i in 0..500 {
        script.push_str(&format!("line{i:04}\n"));
    }
    script.push_str("BIG\ncat <&${COPROC[0]} > /tmp/rubash-1651-big-out.txt; wait $COPROC_PID; echo \"rc=$?\"\n");
    let (stdout, _, code) = rubash_script(&script);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(code, Some(0));
    let out = std::fs::read_to_string("/tmp/rubash-1651-big-out.txt").expect("read out");
    assert_eq!(out.lines().count(), 500);
    assert!(out.starts_with("line0000\n"));
    let _ = std::fs::remove_file("/tmp/rubash-1651-big-out.txt");
}

#[test]
fn coproc_numbered_and_empty_herestring() {
    let (stdout, _, code) =
        rubash_c("coproc cat 0<<< \"num\"; cat <&${COPROC[0]}; wait $COPROC_PID; echo \"rc=$?\"");
    assert_eq!(stdout, "num\nrc=0\n");
    assert_eq!(code, Some(0));
    let (stdout, _, code) =
        rubash_c("coproc cat <<< \"\"; cat <&${COPROC[0]}; echo \"cat_rc=$?\"; wait $COPROC_PID");
    assert_eq!(stdout, "\ncat_rc=0\n");
    assert_eq!(code, Some(0));
}
