//! Issue unixwin/rubash#1915: a process-substitution body containing a
//! single-quoted `$...` (classically an awk program) tripped the PARENT
//! shell's `set -u` check — `mapfile -t x < <(printf '10\n' | awk
//! '$1 == 10 {print $1}')` died with `$1: unbound variable`.
//!
//! GNU owner: subst.c:11366-11385 (expand_word_internal '<'/'>' cases)
//! extracts the body verbatim via extract_process_subst (subst.c:1311)
//! and hands it to process_substitute (subst.c:6362); the parent word
//! walk never expands parameters inside the body — the child shell
//! parses it. Rubash's parent-side nounset pre-scans treated
//! `$( ... )` and backtick bodies as opaque child text (rubash#307)
//! but walked straight into `<( ... )` / `>( ... )` span text, so the
//! body's quoted `$1` read as a live parent reference. The pre-scans
//! (simple-command words/assignments and the redirect-target gate)
//! now mask out exactly the spans the parser recorded for the
//! command — verbatim child text — before judging, so look-alike
//! `<(...)`-shaped text that is NOT a recorded span (e.g. inside
//! double quotes, where `$` stays a live parent reference) is still
//! judged in full.
//!
//! Verified against GNU Bash 5.3.20 (script-file probes): every
//! `should_pass` shape below is byte-identical (stdout/stderr/rc) and
//! every `still_fires` shape keeps its GNU fatal.
//!
//! The `[[ ]]` face (t7 group below): the conditional operand scans
//! are separate scan points from the simple-command pre-scan —
//! `expand_conditional_word`'s expansion-error scan (the `-n`/`-z`
//! and binary operand path) and `regex_operands_nounset_fatal` (the
//! `=~` operands). `execute_conditional_command` publishes the spans
//! the parser recorded for each operand (its `arg_metadata`), and
//! both scan points mask exactly the spans recorded for the word
//! under scan before judging it — the same structural criterion, so
//! a quoted look-alike span in a sibling operand still fires.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1915_procsub_setu_pipeline_external_body() {
    // The issue's probe 29 verbatim: GNU prints `10`, rc 0, no stderr.
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nmapfile -t x < <(printf '10\\n' | awk '$1 == 10 {print $1}')\nprintf '%s\\n' \"${x[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_procsub_setu_single_external_body_no_pipeline() {
    // No pipeline: the body is one external command whose quoted
    // program text carries the `$1`.
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nmapfile -t x < <(awk '$1 == 10 {print $1}' <<< '10')\nprintf '%s\\n' \"${x[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_procsub_setu_word_position() {
    // The span as a command word (not a redirect target) is the same
    // parent walk: GNU materializes it without a nounset diagnostic.
    let (stdout, stderr, code) =
        rubash_stdin("set -u\necho <(printf '10\\n' | awk '$1 == 10 {print $1}')\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(!stdout.trim().is_empty(), "stdout: {stdout}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_procsub_setu_output_span_word() {
    let (stdout, stderr, code) = rubash_stdin("set -u\necho >(awk '$1 == 10 {print $1}')\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(!stdout.trim().is_empty(), "stdout: {stdout}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_procsub_setu_quoted_dollar_in_builtin_argument() {
    // The quoted `$1` rides a builtin's argument inside the body; it is
    // data for printf, not a parent reference.
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nmapfile -t x < <(printf '%s\\n' '$1' | awk '{print}')\nprintf '%s\\n' \"${x[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "$1\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_procsub_setu_quoted_unbound_name_in_body() {
    // A quoted NAME that is unbound everywhere is still the child's
    // text: GNU runs the body and the script continues.
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nmapfile -t x < <(awk '$NOPE == 1 {print}' <<< '10')\nprintf 'done\\n'\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_procsub_setu_quoted_paren_and_subshell_body() {
    // A `)` inside the body's quotes is part of the recorded span's
    // verbatim text, and a parenthesized body command nests inside it;
    // neither may leak a `$` verdict into the parent scan.
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nmapfile -t x < <(awk '{print \")\" $1}' <<< '10')\nprintf '%s\\n' \"${x[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, ")10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) =
        rubash_stdin("set -u\nmapfile -t x < <((echo 10))\nprintf '%s\\n' \"${x[0]}\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_nounset_still_fires_for_plain_unbound_word() {
    let (stdout, stderr, code) = rubash_stdin("set -u\necho $NOPE\necho after\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1915_nounset_still_fires_after_a_procsub_word() {
    // Masking a span must not swallow the rest of the word list.
    let (stdout, stderr, code) = rubash_stdin("set -u\necho <(echo hi) $NOPE\necho after\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1915_nounset_still_fires_inside_arithmetic_group() {
    // Arithmetic text carries no recorded procsub span (a `<(` there
    // is comparison plus a group), so masking leaves it untouched and
    // a real unbound name in the group is still a parent-side fatal.
    let (stdout, stderr, code) = rubash_stdin("set -u\necho $(( 1 < ($NOPE + 2) ))\necho after\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1915_arith_comparison_parens_no_false_hit() {
    let (stdout, stderr, code) = rubash_stdin("set -u\necho $(( (3) < (2) ))\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_nounset_still_fires_for_redirect_word() {
    // GNU: the diagnostic prints, cat never runs, and because cat is
    // external the fatal is contained in its fork — the script
    // continues and exits 0.
    let (stdout, stderr, code) = rubash_stdin("set -u\ncat < $NOPE\necho after\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after\n", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1915_procsub_setu_assignment_value_span() {
    // A span on an assignment RHS is child text as well: the body's
    // `$1` is never judged by the parent, the assignment completes,
    // and the script continues. (The materialized /dev/fd path's
    // lifetime is a separate surface: reading it from a later command
    // fails in GNU 5.3.20 too, so this test pins only the nounset
    // claim — no `unbound variable` diagnostic, rc 0.)
    let (stdout, stderr, code) =
        rubash_stdin("set -u\nx=<(awk '$1 == 10 {print $1}' <<< '10')\ncat \"$x\"\necho end\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "end\n", "stderr: {stderr}");
    assert!(!stderr.contains("unbound variable"), "stderr: {stderr}");
}

#[test]
fn issue1915_nounset_still_fires_for_double_quoted_fake_span_word() {
    // `"<(awk '$1')"` is NOT a process substitution: inside double
    // quotes the `$1` stays a live parent reference, so GNU raises
    // the nounset fatal. Only parser-recorded spans are masked; this
    // look-alike text must still be judged.
    let (stdout, stderr, code) = rubash_stdin("set -u\necho \"<(awk '$1')\"\necho after\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(stderr.contains("$1: unbound variable"), "stderr: {stderr}");
}

#[test]
fn issue1915_nounset_still_fires_for_double_quoted_fake_span_assignment() {
    // Same look-alike on an assignment RHS: the quoted `$1` is live.
    let (stdout, stderr, code) = rubash_stdin("set -u\nx=\"<(awk '$1')\"\necho after\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(stderr.contains("$1: unbound variable"), "stderr: {stderr}");
}

#[test]
fn issue1915_nounset_still_fires_for_fake_span_beside_real_span() {
    // One line, both shapes: the recorded span is masked, the
    // double-quoted look-alike next to it still raises the fatal.
    let (stdout, stderr, code) =
        rubash_stdin("set -u\necho <(awk '$1' <<< '10') \"<(awk '$1')\"\necho after\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(stderr.contains("$1: unbound variable"), "stderr: {stderr}");
}

#[test]
fn issue1915_cond_nounset_span_operand_unary() {
    // t7, the residual this group closes: a procsub span as a `[[ ]]`
    // unary operand. GNU extracts the span verbatim and tests the
    // resulting /dev/fd word; the body's `$1` is never a parent
    // reference, so `-n` is true and the script runs on.
    let (stdout, stderr, code) =
        rubash_stdin("set -u\nif [[ -n <(awk '$1' <<< '10') ]]; then echo yes; fi\necho end\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "yes\nend\n", "stderr: {stderr}");
    assert_eq!(stderr, "");

    // `-z` of the same operand is false, still with no diagnostic.
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nif [[ -z <(awk '$1' <<< '10') ]]; then echo yes; else echo no; fi\necho end\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "no\nend\n", "stderr: {stderr}");
    assert_eq!(stderr, "");

    // An output span `>(...)` as the operand, same criterion.
    let (stdout, stderr, code) =
        rubash_stdin("set -u\nif [[ -n >(awk '{print $1}') ]]; then echo yes; fi\necho end\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "yes\nend\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_cond_nounset_span_operand_regex_rhs() {
    // The span as the `=~` right operand: the nounset scan of the
    // pattern operand must not judge the body either. (GNU matches
    // the subject against the materialized /dev/fd path; "abc" does
    // not match it, so the observable verdict is `no`, rc 0.)
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nif [[ abc =~ <(awk '$1' <<< '10') ]]; then echo yes; else echo no; fi\necho end\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "no\nend\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1915_cond_nounset_still_fires_for_unary_operand() {
    // Guards: a bare unbound operand keeps its GNU fatal on both the
    // `-n` and `-z` paths.
    for op in ["-n", "-z"] {
        let (stdout, stderr, code) = rubash_stdin(&format!(
            "set -u\nif [[ {op} $NOPE ]]; then echo yes; fi\necho end\n"
        ));
        assert_eq!(code, Some(1), "op: {op}");
        assert_eq!(stdout, "", "op: {op}, stderr: {stderr}");
        assert!(
            stderr.contains("NOPE: unbound variable"),
            "op: {op}, stderr: {stderr}"
        );
    }
}

#[test]
fn issue1915_cond_nounset_still_fires_for_regex_operands() {
    // Guards: unbound references on either side of `=~` keep the
    // fatal (subject side and pattern side, unquoted and quoted).
    let (stdout, stderr, code) =
        rubash_stdin("set -u\nx=abc\nif [[ $x =~ $NOPE ]]; then echo yes; fi\necho end\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );

    let (stdout, stderr, code) =
        rubash_stdin("set -u\nif [[ $NOPE =~ x ]]; then echo yes; fi\necho end\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );

    let (stdout, stderr, code) =
        rubash_stdin("set -u\nif [[ abc =~ \"$NOPE\" ]]; then echo yes; fi\necho end\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1915_cond_nounset_still_fires_for_quoted_fake_span() {
    // The q3 guard on the conditional face: `"<(awk '$NOPE')"` is
    // not a process substitution — inside double quotes the `$NOPE`
    // is a live parent reference, so GNU raises the fatal. Only the
    // spans recorded for a word are masked; this operand recorded
    // none.
    let (stdout, stderr, code) =
        rubash_stdin("set -u\nif [[ -n \"<(awk '$NOPE')\" ]]; then echo yes; fi\necho end\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );

    // The same precision across operands of one conditional: the
    // quoted look-alike on the left records no span, so the fatal
    // fires even though the right operand is a real span whose own
    // body text is identical.
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nif [[ \"<(awk '$NOPE')\" == <(awk '$NOPE') ]]; then echo yes; fi\necho end\n",
    );
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1915_cond_nounset_still_fires_beside_a_span() {
    // Masking one word must not disarm the scan anywhere else: a
    // live reference BEFORE a real span in the same word still
    // fires...
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nif [[ -n $NOPE<(awk '$1' <<< '10') ]]; then echo yes; fi\necho end\n",
    );
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );

    // ...and a live reference in the NEXT term of the same
    // conditional is the one reported (not the masked span's `$1`).
    let (stdout, stderr, code) = rubash_stdin(
        "set -u\nif [[ -n <(awk '$1' <<< '10') && -n $NOPE ]]; then echo yes; fi\necho end\n",
    );
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("$1"), "stderr: {stderr}");

    // The file-test operand path keeps its pre-existing parity.
    let (stdout, stderr, code) =
        rubash_stdin("set -u\nif [[ -e $NOPE ]]; then echo yes; fi\necho end\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("NOPE: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1915_comsub_prefix_still_runs_before_later_unbound() {
    // rubash#319 ordering, unchanged by the span masking: the comsub
    // to the left of the unbound reference produces its side effect
    // first.
    let (_stdout, stderr, code) = rubash_stdin("set -u\necho $(echo hi >&2) $NOPE\n");
    assert_eq!(code, Some(1));
    let hi = stderr.find("hi").expect("comsub side effect printed");
    let unbound = stderr
        .find("NOPE: unbound variable")
        .expect("unbound diagnostic printed");
    assert!(hi < unbound, "stderr: {stderr}");
}
