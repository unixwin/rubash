//! Issue rubash#3453 regression: printf's fd-1 write failure aborts the
//! builtin at the END of the failing rendering pass — GNU printf.def:819
//! checks `if (vflag == 0 && ferror (stdout)) PRETURN (EXECUTION_FAILURE)`
//! once per pass, so the failing pass's diagnostics still reach fd 2 while
//! later passes' renderings and diagnostics die. The ordered-output route
//! carries pass-boundary marker chunks (fd u8::MAX) from the builtin to
//! the routing walk, and the pipeline-stage delivery rebuilds the stderr
//! leg from the surviving prefix; a failed fd-2 delivery is lost silently
//! like GNU's builtin_error writing to the descriptor that just failed
//! (rubash#699 R1.3).

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    #[cfg(unix)]
    cmd.arg0("bash");
    cmd.arg("-c").arg(script);
    cmd.env("LC_ALL", "C.UTF-8");
    let output = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn pipeline_stage_later_pass_diagnostics_die() {
    // `printf '%d\n' 5 bad 7 2>&1 1>&- | cat`: fd 2 rides the pipe, so
    // the write error lands in cat's stream; the failing pass is the
    // first (the "5" write), so the pass-2 `bad` diagnostic and the
    // pass-3 "7" rendering die — GNU emits the write error alone.
    let (stdout, _, code) = rubash("printf '%d\\n' 5 bad 7 2>&1 1>&- | cat; echo rc=$?");
    assert_eq!(
        stdout,
        "bash: line 1: printf: write error: Bad file descriptor\nrc=0\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn pipeline_stage_both_fds_closed_stays_silent() {
    // `printf ... 1>&- 2>&- | cat`: the fd-1 write error's report goes to
    // fd 2, which is itself closed — GNU loses it silently end to end.
    let (stdout, stderr, code) = rubash("printf '%d\\n' 5 bad 7 1>&- 2>&- | cat; echo done");
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn closed_fd1_drops_later_pass_diagnostics() {
    // `printf '%d\n' 5 bad 7 1>&-`: `bad` is pass 2, after the pass-1 fd-1
    // write failure — GNU reports the write error alone, status 1.
    let (stdout, stderr, code) = rubash("printf '%d\\n' 5 bad 7 1>&-");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "bash: line 1: printf: write error: Bad file descriptor\n"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn closed_fd1_keeps_same_pass_diagnostics() {
    // `printf '%d\n%d\n' 5 bad 2>errf 1>&-`: `bad`'s conversion lives in
    // the SAME pass as the first fd-1 write, so its diagnostic still
    // reaches the redirected fd 2 — then the write error at the pass
    // boundary (printf.def:819).
    let (stdout, _, code) = rubash("printf '%d\\n%d\\n' 5 bad 2>errf 1>&-; cat errf; rm -f errf");
    assert_eq!(
        stdout,
        "bash: line 1: printf: bad: invalid number\nbash: line 1: printf: write error: Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn open_fd1_pipeline_keeps_full_rendering() {
    // Control: with fd 1 open the whole render rides the pipe and the
    // diagnostic order is the production order (rubash#3324) — no abort.
    let (stdout, stderr, code) = rubash("printf '%d\\n' 5 bad 7 | cat");
    assert_eq!(stdout, "5\n0\n7\n");
    assert_eq!(stderr, "bash: line 1: printf: bad: invalid number\n");
    assert_eq!(code, Some(0));
}
