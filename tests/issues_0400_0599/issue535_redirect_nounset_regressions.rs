//! Issue rubash#535 — `set -u` applies to redirect words and here-string
//! bodies.
//!
//! GNU redir.c:298 redirection_expand runs expand_words_no_vars, whose
//! nounset check covers every redirect word: `cat < $nope` reports
//! `nope: unbound variable` (the BARE name) and dies with status 1. The
//! blast radius follows where do_redirections runs — a forked child for
//! external targets (the script continues), the main shell for
//! builtin/function and null-command targets (the noninteractive script
//! ends, `-c` exits 127 via the shell.c:1471 remap). Here-string bodies
//! expand at application time too: an unquoted or double-quoted `<<<
//! $nope` reports, while `<<< '$nope'` stays the literal text.
//!
//! rubash used to expand unbound redirect words silently (`<<< $nope`)
//! or misclassify them as `ambiguous redirect` (`< $nope`).
//!
//! Every case is byte-verified against WSL GNU Bash 5.3.0 (script-file
//! oracle, 2026-10-12).

use std::process::Command;

/// Script-mode runner: the GNU oracle probes for this issue were taken in
/// script mode (where a main-shell fatal exits 1); `-c` mode remaps the
/// same fatal to 127 (see dash_c_mode_remaps_fatal_to_127).
fn rubash(script: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("rubash535_{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// #535 repro: the input-redirect word reports the BARE unbound name (not
/// `ambiguous redirect`, not `$nope`), the command dies with status 1, and
/// the script continues with the next line.
#[test]
fn input_redirect_word_reports_bare_unbound_name() {
    let (stdout, stderr, code) = rubash("set -u; cat < $nope\necho AFTER");
    assert_eq!(stdout, "AFTER\n");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    assert!(!stderr.contains("ambiguous"), "{stderr:?}");
    assert!(!stderr.contains("$nope:"), "{stderr:?}");
    assert_eq!(code, Some(0));
}

/// #535: an unquoted here-string body reports the same way and the script
/// continues.
#[test]
fn unquoted_here_string_body_reports_unbound() {
    let (stdout, stderr, _) = rubash("set -u; cat <<< $nope; echo SAME");
    assert_eq!(stdout, "SAME\n");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
}

/// #535: a double-quoted here-string body expands parameters and still
/// reports (GNU probe 2026-10-12).
#[test]
fn double_quoted_here_string_body_reports_unbound() {
    let (stdout, stderr, _) = rubash("set -u; cat <<< \"$nope\"; echo SAME");
    assert_eq!(stdout, "SAME\n");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
}

/// #535 boundary kept: a single-quoted here-string body is literal data —
/// no diagnostic, no abort.
#[test]
fn single_quoted_here_string_body_stays_literal() {
    let (stdout, stderr, code) = rubash("set -u; cat <<< '$nope'; echo SAME");
    assert_eq!(stdout, "$nope\nSAME\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// #535: an OUTPUT redirect word on a BUILTIN target is fatal — the
/// script ends there (GNU: do_redirections runs in the main shell for
/// builtins, execute_cmd.c:5606).
#[test]
fn builtin_output_redirect_word_is_fatal() {
    let (stdout, stderr, code) = rubash("set -u; echo x > $nope\necho NEXTLINE");
    assert_eq!(stdout, "");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    assert_eq!(code, Some(1));
}

/// #535: an OUTPUT redirect word on an EXTERNAL target is contained — the
/// forked child dies with status 1 and the script continues (GNU probe:
/// NEXTLINE prints, script rc 0).
#[test]
fn external_output_redirect_word_is_contained() {
    let (stdout, stderr, code) = rubash("set -u; ls > $nope\necho NEXTLINE");
    assert_eq!(stdout, "NEXTLINE\n");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    assert_eq!(code, Some(0));
}

/// #535: a null command's redirect word is fatal too (GNU
/// execute_null_command applies do_redirections in the main shell; probe:
/// `set -u; > $nope` ends the script).
#[test]
fn null_command_redirect_word_is_fatal() {
    let (stdout, stderr, code) = rubash("set -u; > $nope\necho AFTER_NULL");
    assert_eq!(stdout, "");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    assert_eq!(code, Some(1));
}

/// #535: a builtin INPUT redirect word is fatal (main-shell
/// do_redirections); GNU probe: no next line, rc 1.
#[test]
fn builtin_input_redirect_word_is_fatal() {
    let (stdout, stderr, code) = rubash("set -u; read x < $nope\necho AFTER");
    assert_eq!(stdout, "");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    assert_eq!(code, Some(1));
}

/// #535: `-c` mode remaps the fatal to 127 (shell.c:1471 run_one_command
/// FORCE_EOF arm — the same containment the plain word-nounset path uses).
#[test]
fn dash_c_mode_remaps_fatal_to_127() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -u; read x < $nope")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    assert_eq!(output.status.code(), Some(127));
}

/// #535 ordering: the diagnostic routes through the fd 2 redirects applied
/// BEFORE the failing redirect word — `2>/dev/null` after it suppresses
/// nothing (GNU probe: the error still prints), while `2>/dev/null` before
/// it swallows the report (issue #250 routing).
#[test]
fn diagnostic_routes_through_earlier_stderr_redirects() {
    let (stdout, stderr, _) = rubash("set -u; cat < $nope 2>/dev/null; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    let (stdout, stderr, _) = rubash("set -u; cat 2>/dev/null < $nope; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "");
}

/// #535 boundary kept: a pipeline member's redirect word is contained by
/// the member boundary — the member dies, the script continues (GNU
/// probe: `after` runs, rc 0).
#[test]
fn pipeline_member_redirect_word_is_contained() {
    let (stdout, stderr, code) = rubash("set -u; echo hi | read x < $nope; echo after; echo rc=$?");
    assert_eq!(stdout, "after\nrc=0\n");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
    assert_eq!(code, Some(0));
}

/// #535 boundary kept: quoted heredoc bodies stay literal; unquoted
/// heredoc bodies already reported before this fix (pinned).
#[test]
fn heredoc_bodies_keep_their_behavior() {
    let (stdout, stderr, code) = rubash("set -u; cat <<'E'\n$nope\nE\necho after");
    assert_eq!(stdout, "$nope\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let (stdout, stderr, _) = rubash("set -u; cat <<E\n$nope\nE\necho after");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("nope: unbound variable"), "{stderr:?}");
}
