//! Issue unixwin/rubash#2309: the heredoc eof6 pre-check family.
//!
//! A heredoc body is collected by the delimiter scan and only then
//! expanded as a word, so a pending construct in the body is diagnosed
//! by the EXPANSION layer, not by an input completeness check — and
//! each construct's failure belongs to its own layer:
//!
//! - h5: `$'abc` in a body is literal text (quote processing does not
//!   run in heredoc expansion). The old residual-DFA pre-check claimed
//!   its ansi_single flag and swallowed the body with the eof6 text.
//! - h3: an unterminated backquote fails its extraction in
//!   expand_word_internal (subst.c:11587): `bad substitution: no
//!   closing "`" in …` at the command's line, the command fails.
//! - h4: anything unterminated inside a `$(` — its closer, or a quote
//!   within — fails the parse of the extracted body (subst.c:1262
//!   extract_command_subst hands the body to parse_comsub): the
//!   `command substitution:` EOF diagnostic naming the innermost
//!   pending construct. The stream is seeded at the body's first line
//!   and counts only lines inside the extracted text.
//!
//! All expectations measured against GNU bash 5.3.20 script files
//! (oracle: ~/workspace/rubash-audit/gnu-bash-5.3.20), 2026-10-31.

use std::process::{Command, Stdio};

/// Runs `script` as a file under the built rubash; returns
/// (stdout, stderr, exit code, script path as invoked).
fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>, String) {
    let path = std::env::temp_dir().join(format!(
        "issue2309_{}_{}.sh",
        tag,
        std::process::id()
    ));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
        path.to_string_lossy().into_owned(),
    )
}

// ---------------------------------------------------------------------------
// h5 — `$'` text in a heredoc body is literal.
// ---------------------------------------------------------------------------

#[test]
fn issue2309_h5_ansi_c_text_in_heredoc_body_is_verbatim() {
    let script = "cat <<EOF\nx=$'abc\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, _path) = run_rubash_file(script, "h5");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=$'abc\nafter rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2309_h5_ansi_c_line_before_unclosed_comsub_still_eof6() {
    // Scan order: the `$'` line is literal text; the unclosed `$(` on
    // the next body line is the first failing construct.
    let script = "cat <<EOF\nx=$'abc\ny=$(seq 10\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "h5order");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 3: unexpected EOF while looking for matching `)'\n"
        )
    );
}

// ---------------------------------------------------------------------------
// h3 — unterminated backquote: subst.c:11587 bad substitution.
// ---------------------------------------------------------------------------

#[test]
fn issue2309_h3_unterminated_backquote_is_bad_substitution() {
    let script = "cat <<EOF\nx=`echo abc\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "h3");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    // The reported text runs from the backquote to the end of the body
    // word, trailing newline included — hence the blank line.
    assert_eq!(
        stderr,
        format!("{path}: line 1: bad substitution: no closing \"`\" in `echo abc\n\n")
    );
}

#[test]
fn issue2309_h3_backquote_on_second_body_line_still_reports_command_line() {
    let script = "cat <<EOF\nfirst ok\nx=`echo abc\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "h3line2");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // The whole body is discarded (no `first ok`), and report_error
    // names the executing command's line, not the body line.
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: bad substitution: no closing \"`\" in `echo abc\n\n")
    );
}

// ---------------------------------------------------------------------------
// h4 — unterminated constructs inside `$(`: the body-parse layer.
// ---------------------------------------------------------------------------

#[test]
fn issue2309_h4_unterminated_squote_inside_comsub_names_the_quote() {
    let script = "cat <<EOF\nx=$(echo 'a\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "h4");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 2: unexpected EOF while looking for matching `''\n"
        )
    );
}

#[test]
fn issue2309_h4_unterminated_dquote_inside_comsub_names_the_quote() {
    let script = "cat <<EOF\nx=$(echo \"a\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "h4dq");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 2: unexpected EOF while looking for matching `\"'\n"
        )
    );
}

#[test]
fn issue2309_h4_quote_on_second_extracted_line_reports_body_base_plus_offset() {
    let script = "cat <<EOF\nx=$(echo ok\necho 'a\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "h4line");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 3: unexpected EOF while looking for matching `''\n"
        )
    );
}

#[test]
fn issue2309_h4_comsub_on_later_body_line_keeps_body_first_line_base() {
    // The comsub stream is seeded at the body's first line; the
    // opener's own body line does not shift the report.
    let script = "cat <<EOF\nfirst ok\nx=$(echo 'a\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "h4base");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 2: unexpected EOF while looking for matching `''\n"
        )
    );
}

// ---------------------------------------------------------------------------
// The genuine eof6 shape keeps its diagnostic — with the corrected
// line geometry (stream base = body first line + extracted-text lines).
// ---------------------------------------------------------------------------

#[test]
fn issue2309_eof6_genuine_unclosed_comsub_keeps_diagnostic() {
    let script = "read foo <<EOF\n$(seq 10\nEOF\necho \"foo=<$foo>\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "eof6");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "foo=<>\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 3: unexpected EOF while looking for matching `)'\n"
        )
    );
}

#[test]
fn issue2309_eof6_genuine_unclosed_comsub_fails_the_command() {
    let script = "read foo <<EOF\n$(seq 10\nEOF\necho \"read rc=$? foo=<$foo>\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "eof6rc");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "read rc=1 foo=<>\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 3: unexpected EOF while looking for matching `)'\n"
        )
    );
}

#[test]
fn issue2309_eof6_unclosed_comsub_on_last_body_line_reports_extracted_lines() {
    // Three body lines, the `$(` on the last: GNU reports body-first
    // line + the extracted text's own lines (3), not the delimiter
    // line (5).
    let script = "cat <<EOF\nfirst text\nsecond text\nx=$(seq 10\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "eof6last");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 3: unexpected EOF while looking for matching `)'\n"
        )
    );
}

#[test]
fn issue2309_eof6_unclosed_comsub_spanning_body_reports_eof_line() {
    let script = "cat <<EOF\nx=$(seq 10\nmid text\nlast text\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "eof6span");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 5: unexpected EOF while looking for matching `)'\n"
        )
    );
}

#[test]
fn issue2309_eof6_mixed_good_and_bad_constructs_discard_whole_body() {
    let script = "cat <<EOF\na=$(echo ok) b=$(seq 10\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, path) = run_rubash_file(script, "eof6mixed");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: command substitution: line 3: unexpected EOF while looking for matching `)'\n"
        )
    );
}

// ---------------------------------------------------------------------------
// Controls — well-formed bodies stay byte-identical.
// ---------------------------------------------------------------------------

#[test]
fn issue2309_control_well_formed_body_expands_normally() {
    let script = "cat <<EOF\nx=`echo abc` y=$(echo def) z=$'lit' w=${UNSET:-dflt} v=$((1+2))\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, _path) = run_rubash_file(script, "ctlmixed");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=abc y=def z=$'lit' w=dflt v=3\nafter rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2309_control_closed_comsub_with_quotes_and_backquote() {
    let script = "cat <<EOF\nx=$(echo 'a' \"b\" `echo c`) y=$(echo $(echo inner))\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, _path) = run_rubash_file(script, "ctlnest");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=a b c y=inner\nafter rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2309_control_bare_apostrophe_in_body_text_is_inert() {
    let script = "cat <<EOF\nit's $(echo fine) and \"quoted\" text\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, _path) = run_rubash_file(script, "ctlquote");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "it's fine and \"quoted\" text\nafter rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2309_control_quoted_delimiter_body_is_verbatim() {
    let script = "cat <<'EOF'\nx=$'abc `echo no` $(echo no)\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, _path) = run_rubash_file(script, "ctlquoted");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=$'abc `echo no` $(echo no)\nafter rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2309_control_closed_parameter_with_backquote_default() {
    let script = "cat <<EOF\nx=${UNSET:-`echo inner`}\nEOF\necho \"after rc=$?\"\n";
    let (stdout, stderr, code, _path) = run_rubash_file(script, "ctlparam");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=inner\nafter rc=0\n");
    assert_eq!(stderr, "");
}
