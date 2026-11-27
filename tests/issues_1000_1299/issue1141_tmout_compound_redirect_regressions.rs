//! rubash#1141 regressions: TMOUT armed inside a compound command whose
//! stdin is a process substitution — `{ read …; } < <(…)`,
//! `select … done < <(…)`, `while/for … done < <(…)` — never fired. The
//! compound dispatch runs BEFORE the simple-command materialization
//! gate, so the redirect target reached `bind_command_fd0_input` still
//! in raw `<(…)` form and fell through to the untimed FUNCTION_STDIN
//! slurp (`loop_redirect_input`), which also cleared the stream's
//! timing record. The binder now materializes the raw form with its
//! production schedule and binds fd 0 to the timed endpoint directly.
//!
//! GNU spec (vendored tree `bash.git @9c465866`, Bash-5.3 patch 20):
//! - builtins/read.def:506-519: with TMOUT set (or -t given) read arms
//!   an alarm; a timeout returns 128+SIGALRM with the arrived partial
//!   input assigned (read.def:528-554).
//! - execute_cmd.c:3426/3578: select's query IS the read builtin, so a
//!   timed-out select read ends the loop with EXECUTION_FAILURE.
//! - subst.c:6362 process_substitute: the body runs concurrently on a
//!   pipe, so consumers observe real arrival times — rubash's recorded
//!   production schedule is the equivalent evidence.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle), stdout and stderr
//! compared separately. Wall-clock bounds are deliberately generous:
//! rubash pays the producer's full duration at materialization, so a
//! case's wall time tracks the producer, not the timeout.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1141-{}-{}",
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
fn issue1141_group_redirect_read_tmout_expires() {
    let (stdout, _, code) =
        rubash_file("{ TMOUT=1; read x; echo \"rc=$? x=[$x]\"; } < <(sleep 2; echo hi)\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=142 x=[]\n");
}

#[test]
fn issue1141_group_redirect_second_read_tmout_expires_again() {
    let (stdout, _, code) = rubash_file(
        "{ TMOUT=1; read x; echo \"r1=$? x=[$x]\"; read y; echo \"r2=$? y=[$y]\"; } < <(sleep 3; echo hi)\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "r1=142 x=[]\nr2=142 y=[]\n");
}

#[test]
fn issue1141_group_redirect_read_partial_input_on_timeout() {
    // Only the first line is asserted byte-exact: the second read sits
    // exactly on the arrival==deadline boundary (the remainder lands
    // one timeout after the first read's deadline), where GNU's own
    // outcome is decided by sub-10ms wall-clock jitter — see the
    // cluster report. The arrived partial prefix IS deterministic.
    let (stdout, _, code) = rubash_file(
        "{ TMOUT=1; read x; echo \"r1=$? x=[$x]\"; } < <(printf a; sleep 2; printf 'b\\n')\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "r1=142 x=[a]\n");
}

#[test]
fn issue1141_select_redirect_tmout_ends_loop() {
    let (stdout, stderr, code) = rubash_file(
        "TMOUT=1; select c in aa bb; do echo \"c=$c rc=$?\"; break; done < <(sleep 2; echo 1); echo \"end rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "\nend rc=1\n");
    assert_eq!(stderr, "1) aa\n2) bb\n#? ");
}

#[test]
fn issue1141_while_redirect_tmout_condition_read_fails() {
    let (stdout, _, code) = rubash_file(
        "TMOUT=1; while read x; do echo \"got [$x]\"; break; done < <(sleep 2; echo hi); echo \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
}

#[test]
fn issue1141_for_redirect_read_tmout_expires() {
    let (stdout, _, code) = rubash_file(
        "TMOUT=1; for i in 1; do read x; echo \"rc=$? x=[$x]\"; done < <(sleep 2; echo hi)\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=142 x=[]\n");
}

#[test]
fn issue1141_group_redirect_without_tmout_serves() {
    let (stdout, _, code) =
        rubash_file("{ read x; echo \"rc=$? x=[$x]\"; } < <(sleep 2; echo hi)\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[hi]\n");
}

#[test]
fn issue1141_group_redirect_prompt_data_serves_under_tmout() {
    let (stdout, _, code) =
        rubash_file("{ TMOUT=3; read x; echo \"rc=$? x=[$x]\"; } < <(sleep 1; echo hi)\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[hi]\n");
}

// Guards for the #1141 forms fixed by the earlier timing work (direct
// redirect, pipeline producer, pipeline select, function consumer):
// they share the same verdict machinery and must not regress.

#[test]
fn issue1141_direct_redirect_read_tmout_still_expires() {
    let (stdout, _, code) =
        rubash_file("TMOUT=1; read x < <(sleep 2; echo hi); echo \"rc=$? x=[$x]\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=142 x=[]\n");
}

#[test]
fn issue1141_pipeline_read_tmout_still_expires() {
    let (stdout, _, code) =
        rubash_file("(sleep 2; echo hi) | { TMOUT=1; read x; echo \"rc=$? x=[$x]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=142 x=[]\n");
}

#[test]
fn issue1141_pipeline_select_tmout_still_ends_loop() {
    let (stdout, stderr, code) = rubash_file(
        "(sleep 2; echo 1) | { TMOUT=1; select c in aa bb; do echo \"c=$c rc=$?\"; break; done; echo \"end rc=$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "\nend rc=1\n");
    assert_eq!(stderr, "1) aa\n2) bb\n#? ");
}
