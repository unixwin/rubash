//! Issue unixwin/rubash#3905: a compound assignment opened inside a
//! command substitution (`echo $(a=(1`) is a parse_compound_assignment
//! matched pair, not a nested subshell — EOF reports the list's line
//! with exit 1, like the top-level form.
//!
//! GNU anchor: read_token_word hands a `name=(` word to
//! parse_compound_assignment when the word is token_is_assignment
//! (parse.y:5270, parse.y:5653) and assignment_acceptable(last_read_token)
//! (parse.y:3163) or PST_ASSIGNOK (declare/local/export/readonly/typeset)
//! holds. EOF inside the list takes parse_compound_assignment's own
//! parser_error at parse.y:7151-7152 ("unexpected EOF while looking for
//! matching `)'" at the list's orig_line_number) with
//! set_exit_status (EXECUTION_FAILURE) at parse.y:7167-7169 — exit 1, or
//! 2 when errexit converts it (error.c:324-327). A plain unclosed `$(`
//! instead takes the yyerror route: EOF line, exit 2.
//!
//! The rubash defect was in the close-char scans
//! (src/lexer/continuation.rs): the nested `(` arm pushed every body `(
//! as a command subshell frame, because the scans do not feed their word
//! machine inside a delimiter body, so the `name=` shape was invisible.
//! The arm now recognizes the compound-assignment opener from the raw
//! text (nested_compassign_opener) and pushes an array_list frame,
//! mirroring the top-level arm; the EOF status split (#3903) then reads
//! the aggregated array_list bit as before.
//!
//! Every message/line/code below is measured against GNU bash 5.3.20
//! (script-file oracle probes, 2026-11-18). The argument-position and
//! in-list shapes belong to the token-error family (#3904 A29): they are
//! pinned here by exit code only, as controls that this fix must not
//! flip.

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("i3905_cc_{}_{}.sh", tag, std::process::id()));
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
        out.status.code().unwrap_or(-1),
    )
}

/// Compound assignments inside `$(...)` report the list's `)` at the
/// list's line with exit 1 (exit 2 under errexit), in every position
/// GNU accepts an assignment word.
#[test]
fn comsub_compound_assignment_eof_fidelity() {
    // (tag, script, expected report line, expected exit)
    for (tag, script, line, code) in [
        ("single", "echo $(a=(1", 1usize, 1i32),
        ("multi_elem", "echo $(a=(1 2", 1, 1),
        ("multiline", "echo $(a=(\n1\n", 1, 1),
        ("plus_assign", "echo $(a+=(1", 1, 1),
        ("ident_chars", "echo $(_a9=(1", 1, 1),
        ("declare_arg", "echo $(declare a=(1", 1, 1),
        ("local_arg", "echo $(local a=(1", 1, 1),
        ("subscript", "echo $(a[1]=(1", 1, 1),
        ("chain", "echo $(x=1 a=(1", 1, 1),
        ("if_cond", "echo $(if a=(1", 1, 1),
        ("time_prefix", "echo $(time a=(1", 1, 1),
        ("after_closed", "echo $(a=(1) a=(2", 1, 1),
        ("in_array_comsub", "v=($(a=(1", 1, 1),
        // The name is split by a line continuation: the joined token is
        // one assignment word and the pair opens on the second line.
        ("continuation_name", "echo $(a\\\n=(1", 2, 1),
        // errexit converts EXECUTION_FAILURE to 2 (error.c:324-327).
        ("errexit", "set -e\necho $(a=(1", 2, 2),
    ] {
        let (_stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, code, "{tag}: {script:?} -> {stderr:?}");
        let needle = format!("line {line}: unexpected EOF while looking for matching `)'");
        assert!(stderr.contains(&needle), "{tag}: {stderr:?}");
    }
}

/// Neighbouring shapes keep their verdicts: a plain unclosed `$(` takes
/// the yyerror route (EOF line, exit 2), the arithmetic body keeps its
/// own route, and the token-error family (#3904 A29) keeps its codes.
#[test]
fn comsub_compound_assignment_neighbour_controls() {
    // Plain comsub: EOF line (line 2), exit 2.
    let (_stdout, stderr, got) = run_file("echo $(echo hi", "plain_comsub");
    assert_eq!(got, 2, "plain_comsub -> {stderr:?}");
    assert!(
        stderr.contains("line 2: unexpected EOF while looking for matching `)'"),
        "plain_comsub: {stderr:?}"
    );
    // A line continuation inside the body still counts as a physical
    // line: the inner comsub's EOF report names line 3, not 2. (This
    // pins the escape-arm line count in the close-char scans; before
    // it, an escaped newline inside a delimiter body was consumed
    // without advancing the counter.)
    let (_stdout, stderr, got) = run_file("echo $(foo \\\nbar $(echo hi", "continuation_eof_line");
    assert_eq!(got, 2, "continuation_eof_line -> {stderr:?}");
    assert!(
        stderr.contains("line 3: unexpected EOF while looking for matching `)'"),
        "continuation_eof_line: {stderr:?}"
    );
    // Top-level compound assignment: unchanged (list line, exit 1).
    let (_stdout, stderr, got) = run_file("a=(1", "toplevel");
    assert_eq!(got, 1, "toplevel -> {stderr:?}");
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching `)'"),
        "toplevel: {stderr:?}"
    );
    // Arithmetic body: exit 2 at the pair's line, not the compassign split.
    let (_stdout, stderr, got) = run_file("echo $((a=(1", "arith");
    assert_eq!(got, 2, "arith -> {stderr:?}");
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching `)'"),
        "arith: {stderr:?}"
    );
    // Token-error family, exit codes only (wording is #3904 A29's face):
    // an argument-position `a=(` is not an assignment word, a
    // digit-led name is not an identifier, a spaced `=` is not an
    // assignment word, and a `(` inside an array list is element text.
    for (tag, script, code) in [
        ("arg_position", "echo $(foo a=(1", 2i32),
        ("digit_start", "echo $(9a=(1", 2),
        ("spaced_eq", "echo $(a =(", 2),
        ("in_array_element", "v=(a=(1", 1),
    ] {
        let (_stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, code, "{tag}: {script:?} -> {stderr:?}");
    }
}

/// The closed form still parses and runs: the assignment inside the
/// substitution is a real array assignment.
#[test]
fn comsub_compound_assignment_closed_form_still_parses() {
    let (stdout, stderr, got) = run_file("echo $(a=(1 2); declare -p a)", "closed_form");
    assert_eq!(got, 0, "closed_form -> {stderr:?}");
    assert!(
        stdout.contains("declare -a a=([0]=\"1\" [1]=\"2\")"),
        "closed_form: stdout {stdout:?}"
    );
}
