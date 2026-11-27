//! Issue rubash#1661: on the plain (untimed) dispatch path, a null
//! command produced by a command substitution — `$(false)` as a whole
//! simple command — never fired the ERR trap under `set -e`, and the
//! redirect-carrying form lost the substitution's status entirely.
//!
//! GNU execute_cmd.c: a failed simple command is adjudicated by
//! run_error_trap BEFORE the ERREXIT jump, and execute_null_command's
//! status is the last command substitution status from the command's
//! expansion. The assignment/redirect forms already fired via the
//! rubash#1171 tail of execute_empty_words_command; the bare
//! vanished-words arm in execute_command converted to
//! Err(ExitCode) without firing, and the empty-words tail started its
//! status at 0, clobbering the substitution status for
//! `$(false) >/dev/null`.
//!
//! Consolidation follow-up (same issue): both conversion sites
//! initially fired without the rubash#1611 timed-dispatch deferral,
//! so under `time` the bare form fired twice and the assignment form
//! converted unmarked (exit 127 via the genuine-unwind arm). Both
//! sites now follow the #1611 mechanism — depth-guarded fire plus
//! the errexit_status_converted mark — leaving the ast_exec time
//! branch as the single adjudication site for timed commands.
//!
//! Timed-assignment follow-up (same issue): the timed
//! assignment-only form also could not reach that conversion from
//! the parse side — `time` occupied the simple command's name slot
//! while tokens were classified, so the assignment token stayed a
//! plain word and the stripped inner command dispatched it as a
//! command name (127, the assignment never applied at all). The
//! parser fold and the executor's `time` name-dispatch now restore
//! the classification (parser::reclassify_leading_assignment_words)
//! from the recorded token kinds; the scalar pins at the end cover
//! application, status passthrough, prefix temporariness, and the
//! quoted-word control.
//!
//! Expectations verified against GNU bash 5.3.20 (self-compiled
//! oracle), script-file mode authoritative.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1661-{}-{}",
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
fn issue1661_bare_null_command_fires_err_trap_under_errexit() {
    // The issue's Regression seed, verbatim in shape.
    let (stdout, _, code) = rubash_file("set -e\ntrap 'echo ERRFIRED' ERR\n$(false)\necho done\n");
    assert_eq!(stdout, "ERRFIRED\n");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1661_bare_null_command_fires_err_trap_once_without_errexit() {
    let (stdout, _, code) = rubash_file("trap 'echo ERRFIRED' ERR\n$(false)\necho done\n");
    assert_eq!(stdout, "ERRFIRED\ndone\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1661_redirected_null_command_fires_err_trap_under_errexit() {
    let (stdout, _, code) =
        rubash_file("set -e\ntrap 'echo ERRFIRED' ERR\n$(false) >/dev/null\necho done\n");
    assert_eq!(stdout, "ERRFIRED\n");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1661_null_command_keeps_substitution_status() {
    let (stdout, _, code) = rubash_file(
        "$(exit 3) >/dev/null\necho \"st=$?\"\n\
         $(false) >/dev/null\necho \"st2=$?\"\n\
         x=$(false)\necho \"st3=$?\"\n\
         \"$(false)\"\necho \"st4=$?\"\n\
         `exit 3` >/dev/null\necho \"st5=$?\"\n",
    );
    assert_eq!(stdout, "st=3\nst2=1\nst3=1\nst4=127\nst5=3\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1661_seed_does_not_leak_across_commands() {
    // The substitution-status channel is only attributable when the
    // null command's own words carried a substitution: a plain
    // assignment+redirect null command after a substitution-bearing
    // command keeps status 0 (the pre-fix behavior for that shape).
    let (stdout, _, code) =
        rubash_file("echo $(false) >/dev/null\nx=1 >/dev/null\necho \"st=$?\"\n");
    assert_eq!(stdout, "st=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1661_assignment_only_form_still_fires_err_trap() {
    // Control: the rubash#1171 arm — assignment-only null commands
    // already fired before this fix and must keep firing exactly once.
    let (stdout, _, code) =
        rubash_file("set -e\ntrap 'echo ERRFIRED' ERR\nx=$(false)\necho done\n");
    assert_eq!(stdout, "ERRFIRED\n");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1661_timed_bare_null_command_fires_err_trap_exactly_once() {
    // The consolidation stop: the bare arm fired unguarded, and the
    // ast_exec time branch fired again for the same command, so
    // `time $(false)` printed ERRFIRED twice (GNU: once). The time
    // report must still print and the exit status stays 1.
    let (stdout, stderr, code) =
        rubash_file("set -e\ntrap 'echo ERRFIRED' ERR\ntime $(false)\necho done\n");
    assert_eq!(stdout, "ERRFIRED\n");
    assert!(stderr.contains("real"), "time report missing: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1661_timed_assignment_null_command_fires_once_and_exits_1() {
    // The assignment-only tail converted without the rubash#1611
    // conversion mark, so the timed frame mistook the conversion
    // for a genuine unwind and the script exited 127. GNU: the
    // trap fires exactly once, the report prints, exit 1.
    let (stdout, stderr, code) =
        rubash_file("set -e\ntrap 'echo ERRFIRED' ERR\ntime x=$(false)\necho done\n");
    assert_eq!(stdout, "ERRFIRED\n");
    assert!(stderr.contains("real"), "time report missing: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1661_timed_plain_failure_fires_err_trap_exactly_once() {
    // Control: an ordinary failing command under `time` — the
    // dispatch-level conversion sites already defer to the time
    // branch; the null-command forms above must match this count.
    let (stdout, stderr, code) =
        rubash_file("set -e\ntrap 'echo ERRFIRED' ERR\ntime false\necho done\n");
    assert_eq!(stdout, "ERRFIRED\n");
    assert!(stderr.contains("real"), "time report missing: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1661_timed_scalar_assignment_applies_and_reports_status() {
    // Classification-restore pins: before the restore, the timed
    // inner command dispatched `x=1` as a command name (127) and the
    // assignment was never applied. GNU: a timed simple command's
    // prefix assignments are its whole effect — applied to the
    // shell, and the status is the substitution's status.
    let (stdout, stderr, code) = rubash_file("time x=1\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "X=1 S=0\n");
    assert!(stderr.contains("real"), "time report missing: {stderr}");
    assert_eq!(code, Some(0));

    let (stdout, _, code) = rubash_file("time x=$(exit 3)\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=3\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1661_timed_assignment_prefix_is_temporary_for_a_command() {
    // With a command name present, the restored assignments are
    // prefix (temporary) assignments, exactly like the untimed form.
    let (stdout, _, code) = rubash_file("x=outer\ntime x=inner true\necho \"AFTER=$x S=$?\"\n");
    assert_eq!(stdout, "AFTER=outer S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1661_timed_quoted_assignment_word_stays_a_command_name() {
    // Classification gate control: a quoted 'x=1' is a plain word
    // (TokenKind::Word), not an assignment token — GNU looks it up
    // as a command name and fails 127 in both shells.
    let (stdout, stderr, code) = rubash_file("time 'x=1'\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("command not found"),
        "lookup diagnostic missing: {stderr}"
    );
    assert_eq!(code, Some(0));
}
