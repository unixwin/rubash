//! Issue rubash#2930 — a `<<` inside `$(( ))` appearing in a larger word
//! (quoted, prefixed, assignment RHS, mid-word) was mis-lexed as a
//! here-document start: the rest of the line was swallowed into the word
//! (or the enclosing compound command died with an EOF syntax error).
//!
//! GNU ground truth (vendored bash.git @9c465866, Bash-5.3 patch 20):
//! parse.y:4451 `parse_comsub()` peeks `(` after `$(` and, per POSIX
//! interp 217 (parse.y:4463-4470), hands `$((` to
//! `parse_matched_pair(..., P_ARITH)` (parse.y:3832) — the whole body is
//! consumed by the matched-pair scanner, which tracks only nesting,
//! quotes and dollar-constructs. The token-level redirect scanner
//! (parse.y:3693+) never sees the `<<`, so it can never queue a
//! here-document (here-documents are queued only from a real REDIRECT
//! token, parse.y:3116 `redir_stack[need_here_doc++]`). The
//! arithmetic-vs-subshell verdict is parse.y:4970 `parse_arith_cmd`:
//! arithmetic only when the group opened by the second `(` closes right
//! before a `)`.
//!
//! Rust semantic owner: the lexer's arithmetic classification gate
//! `dollar_double_paren_is_arithmetic` (src/lexer/skip.rs), whose
//! documented contract is "text starting at the two-character `((`
//! opener". The token-start site (src/lexer/scanner.rs) and the
//! expansion site (src/executor/parameter_ops.rs) honour it; the six
//! embedded-position sites (skip_word_inner, skip_double, skip_cmd_subst,
//! the two skip_braced arms, run_skip_brace) passed the text starting at
//! the SECOND `(`, so the gate could never answer true there and every
//! embedded `$((...))` fell to the command-substitution scanner, whose
//! heredoc arm treated the body's `<<` as a here-document start.
//!
//! Expectations were verified byte-for-byte against GNU Bash 5.3.20
//! (the oracle binary) on 2026-11-07; the negative controls pin the
//! rubash#2096 / rubash#512 subshell-or-list classification, which must
//! not move.

use std::process::Command;

fn run(script: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("rubash runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// The issue's regression seed, verbatim in shape: quoted `$(( << ))`
/// inside a command substitution capture, plus the assignment form.
#[test]
fn seed_quoted_shift_and_assignment() {
    let script = r#"out=$(echo "a=$(( 1 << 2 ))"; echo hi)
want='a=4
hi'
[ "$out" = "$want" ] || { echo "FAIL: got [$out]"; exit 1; }
x=$(( 1 << 3 )); [ "$x" = 8 ] || { echo "FAIL assign: x=[$x]"; exit 1; }
echo PASS"#;
    let (stdout, stderr, rc) = run(script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(rc, 0, "stderr: {stderr}");
}

/// mA1: double-quoted mid-word shift, `;` separator survives.
#[test]
fn quoted_mid_word_shift_semicolon() {
    let (stdout, stderr, rc) = run(r#"echo "a=$(( 1 << 2 ))"; echo hi"#);
    assert_eq!(stdout, "a=4\nhi\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// The assignment form silently dropped the following command entirely.
#[test]
fn assignment_shift_keeps_following_command() {
    let (stdout, stderr, rc) = run("x=$(( 1 << 3 )); echo $x");
    assert_eq!(stdout, "8\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// mA2: two arithmetic expansions on one line.
#[test]
fn two_expansions_one_line() {
    let (stdout, stderr, rc) = run("echo x$(( 1 << 2 )) $(( 3 << 1 )); echo hi");
    assert_eq!(stdout, "x4 6\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// mA3/mA4/mA5: `&&`, `||` and pipe separators after the word survive.
#[test]
fn and_or_pipe_separators_survive() {
    let (stdout, stderr, rc) = run("echo a$(( 1 << 2 )) && echo and-ok");
    assert_eq!(stdout, "a4\nand-ok\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("false || echo a$(( 1 << 2 ))");
    assert_eq!(stdout, "a4\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("echo a$(( 1 << 2 )) | tr a-z A-Z");
    assert_eq!(stdout, "A4\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// mA6: a `>` redirection after the word still applies to the command,
/// not to swallowed text.
#[test]
fn redirect_after_word_applies() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2930-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("out.txt");
    let script = format!("echo a$(( 1 << 2 )) > {}; echo hi", file.to_string_lossy());
    let (stdout, stderr, rc) = run(&script);
    assert_eq!(stdout, "hi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
    let body = std::fs::read_to_string(&file).expect("redirect target written");
    assert_eq!(body, "a4\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// mA7: the whole word is the quoted arithmetic expansion.
#[test]
fn quoted_whole_word_shift() {
    let (stdout, stderr, rc) = run(r#"echo "$(( 1 << 2 ))"; echo hi"#);
    assert_eq!(stdout, "4\nhi\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// mA8/mA9/mC10: mid-word, unquoted and quoted, spaced and spaceless.
#[test]
fn mid_word_shift_spaced_and_spaceless() {
    let (stdout, stderr, rc) = run("echo pre$(( 1 << 2 ))post; echo hi");
    assert_eq!(stdout, "pre4post\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("echo a$((1<<2))b; echo hi");
    assert_eq!(stdout, "a4b\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("echo a$(( (3) << 1 ))b; echo hi");
    assert_eq!(stdout, "a6b\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("echo a$(( 8 >> 1 << 2 ))b; echo hi");
    assert_eq!(stdout, "a16b\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// mA12: a variable operand, and the `<<=` shift-assign form.
#[test]
fn variable_shift_and_shift_assign() {
    let (stdout, stderr, rc) = run("y=3; echo a$(( y << 2 ))b; echo hi");
    assert_eq!(stdout, "a12b\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("x=1; echo a$(( x <<= 2 ))b; echo x=$x");
    assert_eq!(stdout, "a4b\nx=4\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// mA13/mA14: inside `if` / `for` the master build died with
/// `syntax error: unexpected end of file`.
#[test]
fn if_and_for_compound_bodies() {
    let (stdout, stderr, rc) = run(r#"if [ "$(( 1 << 2 ))" = 4 ]; then echo yes; fi; echo after"#);
    assert_eq!(stdout, "yes\nafter\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("for i in $(( 1 << 2 )); do echo i=$i; done; echo after");
    assert_eq!(stdout, "i=4\nafter\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// mA15: the swallow is same-line only; the next line runs, and the
/// first line's command list is whole again.
#[test]
fn next_line_still_executes() {
    let (stdout, stderr, rc) = run("echo a$(( 1 << 2 )); echo same\necho next");
    assert_eq!(stdout, "a4\nsame\nnext\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// An arithmetic expansion nested inside a command substitution, bare
/// and inside double quotes: master reported a spurious
/// `command substitution: 1 unterminated here-document` warning and an
/// EOF error respectively.
#[test]
fn comsub_nested_arith_shift() {
    let (stdout, stderr, rc) = run("echo $(echo $(( 1 << 2 ))); echo hi");
    assert_eq!(stdout, "4\nhi\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run(r#"echo "$(echo $(( 1 << 2 )))"; echo hi"#);
    assert_eq!(stdout, "4\nhi\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// Backtick substitution carrying an arithmetic shift: the value was
/// right on master but the run carried the spurious unterminated
/// here-document warning.
#[test]
fn backtick_arith_shift_no_spurious_warning() {
    let (stdout, stderr, rc) = run("echo `echo $(( 1 << 2 ))`; echo hi");
    assert_eq!(stdout, "4\nhi\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// Nested arithmetic inside an embedded arithmetic expansion, and an
/// arithmetic expansion inside a `${...}` default (the braced scanner
/// already classified this one correctly — positive control).
#[test]
fn nested_and_braced_arith_shift() {
    let (stdout, stderr, rc) = run("echo a$(( 1 + $(( 2 << 1 )) ))b; echo hi");
    assert_eq!(stdout, "a5b\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("echo ${x:-$(( 1 << 2 ))}; echo hi");
    assert_eq!(stdout, "4\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// Negative control (rubash#2096/#512): a `$((` whose inner group does
/// NOT close right before `)` is a command substitution of a
/// subshell-or-list, at every word position — the classification gate
/// must keep answering "not arithmetic" for it.
#[test]
fn subshell_or_list_classification_unchanged() {
    let (stdout, stderr, rc) = run("echo a$((echo hi) 2>&1)b; echo done");
    assert_eq!(stdout, "ahib\ndone\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run(r#"echo "a$((echo hi) 2>&1)b"; echo done"#);
    assert_eq!(stdout, "ahib\ndone\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    // GNU itself rejects the spaceless `$(( (cmd) || cmd ))` reading as
    // an arithmetic error; both shells agree byte-for-byte modulo the
    // program-name prefix.
    let (stdout, stderr, rc) = run("echo a$(( (echo hi) || echo lo ))b; echo done");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(stderr.contains("missing `)'"), "stderr: {stderr}");
    assert_eq!(rc, 1);
}

/// Positive controls: bare whole-word arithmetic, `>>`, and a real
/// here-document all behave exactly as before.
#[test]
fn bare_arith_rightshift_and_real_heredoc_unchanged() {
    let (stdout, stderr, rc) = run("echo $(( 1 << 2 )); echo hi");
    assert_eq!(stdout, "4\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("echo a$(( 1 >> 2 )); echo hi");
    assert_eq!(stdout, "a0\nhi\n", "stderr: {stderr}");
    assert_eq!(rc, 0);

    let (stdout, stderr, rc) = run("cat <<EOF; echo n=$(( 1 << 2 ))\nbody\nEOF");
    assert_eq!(stdout, "body\nn=4\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

/// A real here-document inside a command substitution whose BODY is an
/// arithmetic-shift line: the body is heredoc text, and the enclosing
/// substitution must still close on its own line.
#[test]
fn comsub_heredoc_body_with_arith_shift_line() {
    let script = "echo $(cat <<EOF\nv=$(( 1 << 2 ))\nEOF\n); echo done";
    let (stdout, stderr, rc) = run(script);
    assert_eq!(stdout, "v=4\ndone\n", "stderr: {stderr}");
    assert_eq!(rc, 0);
}

// ---------------------------------------------------------------------------
// Second face of rubash#2930 (captain takeover, 2026-11-07): the same
// arithmetic-verdict-before-heredoc-scan rule at the EXECUTOR-side
// collectors and the remaining lexer unit scans. The pushed lexer fix
// classified `$((...))` at token positions, but the collectors that copy
// a command-substitution body verbatim (embedded_mutations::
// collect_command_substitution_source_ex, the embedded_parameters
// collector, command_input_scope's span scan), the heredoc-header scans
// (command_substitution_pipelines::heredoc_header_*), and the lexer unit
// scans (skip_parenthesized_unit_corrected, command_substitutions_balanced)
// still walked the body as command text, so a shift `<<` inside a nested
// `$((...))` reached a heredoc arm: the closing `))` was swallowed as
// heredoc body (unexpected-EOF) or a spurious "unterminated
// here-document" warning was emitted. GNU ground truth is the same
// parse.y:4463-4470 + parse.y:4970 pair cited in the header: a
// classified-arithmetic unit is opaque everywhere it is scanned.
// Fail-first record (pushed head f332892d, captain probe 2026-11-07):
// five of the shapes below died with "unexpected EOF while looking for
// matching `)'", two printed the right value with a spurious
// unterminated-here-document warning on stderr; all seven are byte-equal
// to GNU Bash 5.3.20 once the collectors consult the gate.
// ---------------------------------------------------------------------------

/// A bare arithmetic shift inside a command substitution: the collector
/// must copy the `$((...))` unit verbatim, not heredoc-scan its body.
#[test]
fn second_face_comsub_arith_shift() {
    let (stdout, stderr, rc) = run("echo $(echo $((1<<2)))");
    assert_eq!(stdout, "4\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// Same unit in an assignment capture: right value, and NO spurious
/// unterminated-here-document warning (the header scan took the shift
/// `<<` as a heredoc operator on the unfixed tree).
#[test]
fn second_face_comsub_arith_shift_assignment_no_warning() {
    let (stdout, stderr, rc) = run("x=$(printf %s $((3<<1))); echo $x");
    assert_eq!(stdout, "6\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// Nested command substitutions around the arithmetic unit
/// (command_substitutions_balanced must also classify before scanning).
#[test]
fn second_face_nested_comsub_arith_shift() {
    let (stdout, stderr, rc) = run("echo $(echo $(echo $((2<<3))))");
    assert_eq!(stdout, "16\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// An arithmetic shift EARLIER in the same substitution header must not
/// hide a real here-document that follows it: the header scan finds the
/// first OPERATOR `<<`, skipping the arithmetic unit.
#[test]
fn second_face_arith_shift_then_real_heredoc_in_comsub() {
    let script = "echo $(a=$((1<<1)); cat <<MARKER\nhi\nMARKER\n)";
    let (stdout, stderr, rc) = run(script);
    assert_eq!(stdout, "hi\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// Spaced shift inside a command substitution.
#[test]
fn second_face_comsub_arith_shift_spaced() {
    let (stdout, stderr, rc) = run("echo $(echo $(( 1 << 2 )))");
    assert_eq!(stdout, "4\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// Backtick substitution around the arithmetic unit: right value, no
/// spurious unterminated-here-document warning.
#[test]
fn second_face_backtick_arith_shift_no_warning() {
    let (stdout, stderr, rc) = run("echo `echo $((1<<3))`");
    assert_eq!(stdout, "8\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}

/// Two sibling substitutions, each with an arithmetic shift.
#[test]
fn second_face_double_comsub_arith_shift() {
    let (stdout, stderr, rc) = run("echo $(echo $((5<<1))) $(echo $((1<<1)))");
    assert_eq!(stdout, "10 2\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(rc, 0);
}
