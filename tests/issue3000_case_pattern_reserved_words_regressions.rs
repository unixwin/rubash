//! rubash#3000 regressions: a `case` pattern list containing escaped
//! reserved words (`\!`, `\{`, `\}`, `\esac`) and split across a
//! backslash continuation failed to parse on rubash while GNU parsed it
//! (the modernish `lib/modernish/mdl/sys/cmd/harden.mm:161-163` shape).
//!
//! Root cause (GNU parse.y anchors): the lexer's `record_token`
//! (`src/lexer/scanner.rs`) terminated the case-reader state on the
//! DEQUOTED value `esac`, ignoring quoting — but CHECK_FOR_RESERVED_WORD
//! (parse.y:3168-3177) recognizes a reserved word only when it is
//! unquoted (`!quoted`), and inside PST_CASEPAT only `esac` counts, not
//! after `|` (Posix grammar rule 4, parse.y:3181-3182) or `(` (parse.y:
//! 3183-3184). The premature termination corrupted the token stamps:
//! following pattern words (`fi`/`for`/`while`...) were stamped
//! `reserved_position`, so the gather's keyword-stack fold pushed phantom
//! `done`/`fi` frames, and the clause-closing `)` lost its
//! `case_pattern_close` stamp — the fold then read the `)` as a stray
//! close, the group gather broke the function definition after the
//! pattern's second line, and the truncated group failed with
//! `syntax error near unexpected token 'fi'`. The pattern word `case`
//! also wrongly opened a nested case frame (no PST_CASEPAT guard),
//! inflating `case_stmt_depth`.
//!
//! The fix (same file, `record_token` only): the `esac` terminator arm
//! fires only for an unquoted `esac` outside the rule-4 positions, plus
//! the empty case's Word `esac` directly after the case's `in`
//! (parse.y:3433-3441 special_case_tokens — `case x in esac` lexes
//! `esac` as a Word because reserved_word_position is false after `in`);
//! the `case` opener arm is guarded by `!case_pattern`; the
//! `reserved_position` stamp gains the `(` rule-4 refusal.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3000-{}-{}",
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

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script failed: {script}\nstderr: {stderr}");
    assert_eq!(stderr, "", "script {script} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "script: {script}");
}

fn check_error(script: &str, expected_stderr: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(2),
        "script did not fail: {script}\nstdout: {stdout}"
    );
    assert_eq!(stdout, "", "script {script} wrote to stdout");
    assert_eq!(stderr, expected_stderr, "script: {script}");
}

/// The issue's minimal form, verbatim (harden.mm:161-163 shape): an
/// escaped-reserved-word pattern list split by a `\` continuation.
#[test]
fn issue3000_minimal_harden_shape() {
    check(
        r#"f() {
	case $1 in
	(\!|\{|\}|case|do|done|elif|else|\esac|fi|for|if|in|then|until|while \
	|break|:|continue|.|eval|exec|exit)
		echo reserved ;;
	(*) echo other ;;
	esac
}
f fi
f ls
"#,
        "reserved\nother\n",
    );
}

/// The same harden.mm list used as a classifier: every word in the list
/// matches `reserved`, non-members match `other` (GNU-verified).
#[test]
fn issue3000_harden_pattern_words_all_match() {
    check(
        r#"f() {
	case $1 in
	(\!|\{|\}|case|do|done|elif|else|\esac|fi|for|if|in|then|until|while \
	|break|:|continue|.|eval|exec|exit)
		echo reserved ;;
	(*) echo other ;;
	esac
}
for w in '!' '{' '}' case do done elif else esac fi for if in then until while break : continue . eval exec exit ls foo ''; do
  printf '%s => ' "$w"; f "$w"
done
"#,
        "! => reserved\n{ => reserved\n} => reserved\ncase => reserved\ndo => reserved\ndone => reserved\nelif => reserved\nelse => reserved\nesac => reserved\nfi => reserved\nfor => reserved\nif => reserved\nin => reserved\nthen => reserved\nuntil => reserved\nwhile => reserved\nbreak => reserved\n: => reserved\ncontinue => reserved\n. => reserved\neval => reserved\nexec => reserved\nexit => reserved\nls => other\nfoo => other\n => other\n",
    );
}

/// The same list at top level, without the function wrapper: the
/// function-definition fold is not part of the bug.
#[test]
fn issue3000_toplevel_case_no_function_wrapper() {
    check(
        r#"case fi in
(\!|\{|\}|case|do|done|elif|else|\esac|fi|for|if|in|then|until|while \
|break|:|continue|.|eval|exec|exit)
	echo reserved ;;
(*) echo other ;;
esac
"#,
        "reserved\n",
    );
}

/// Minimal trigger: `\esac` followed by a structural keyword (`for`)
/// and a tail word — the second trigger element the bisect isolated.
#[test]
fn issue3000_minimal_escaped_esac_plus_for() {
    check(
        r#"f() {
	case $1 in
	(\esac|for|eval) echo hit ;;
	(*) echo other ;;
	esac
}
f for
f eval
f ls
"#,
        "hit\nhit\nother\n",
    );
}

/// Rule 4, `(` face: an UNQUOTED `esac` directly after `(` is pattern
/// text (parse.y:3183-3184), even before a structural keyword.
#[test]
fn issue3000_unescaped_esac_after_paren_is_pattern_text() {
    check(
        r#"f() {
	case $1 in
	(esac|for|eval) echo hit ;;
	(*) echo other ;;
	esac
}
f for
f eval
f esac
f ls
"#,
        "hit\nhit\nhit\nother\n",
    );
}

/// Rule 4, `|` face: an UNQUOTED `esac` directly after `|` is pattern
/// text (parse.y:3181-3182).
#[test]
fn issue3000_unescaped_esac_after_pipe_is_pattern_text() {
    check(
        r#"case esac in
(a|esac|b) echo hit ;;
(*) echo other ;;
esac
"#,
        "hit\n",
    );
}

/// A quoted `esac` is pattern text in both quoting styles.
#[test]
fn issue3000_quoted_esac_is_pattern_text() {
    check(
        r#"case esac in
(a|"esac"|b) echo hit ;;
(*) echo other ;;
esac
"#,
        "hit\n",
    );
    check(
        r#"case esac in
(a|'esac'|b) echo hit ;;
(*) echo other ;;
esac
"#,
        "hit\n",
    );
}

/// Pattern `in` followed by an unquoted `esac`: the `esac` lexes as a
/// Word (reserved_word_position is false after `in`) but is pattern text
/// — the empty-case Word rule must not fire on a pattern `in`.
#[test]
fn issue3000_pattern_in_then_unquoted_esac_stays_pattern() {
    check(
        r#"f() {
	case $1 in
	(a|in|esac) echo hit ;;
	(*) echo other ;;
	esac
}
f esac
f in
f a
f ls
"#,
        "hit\nhit\nhit\nother\n",
    );
    check(
        r#"f() {
	case $1 in
	(in|esac) echo hit ;;
	(*) echo other ;;
	esac
}
f esac
f in
f ls
"#,
        "hit\nhit\nother\n",
    );
}

/// A Word `esac` in a clause body is an argument, never a terminator:
/// `echo in esac` must print it and the case must survive to `;;`.
#[test]
fn issue3000_word_esac_in_clause_body_is_argument() {
    check(
        r#"case a in
a) echo in esac ;;
b) echo hit-b ;;
(*) echo other ;;
esac
"#,
        "in esac\n",
    );
    check(
        r#"case b in
a) echo in esac ;;
b) echo hit-b ;;
(*) echo other ;;
esac
"#,
        "hit-b\n",
    );
}

/// The empty case still closes, on one line and across a newline
/// (parse.y:3433-3441 special_case_tokens; parse.y:1037).
#[test]
fn issue3000_empty_case_still_closes() {
    check(
        r#"case $1 in esac
echo after:$?
"#,
        "after:0\n",
    );
    check(
        r#"case $1 in
esac
echo after:$?
"#,
        "after:0\n",
    );
}

/// A nested case in a clause body, including an empty inner case.
#[test]
fn issue3000_nested_case_in_clause_body() {
    check(
        r#"case a in
a) case x in esac; echo inner-done ;;
b) echo hit-b ;;
(*) echo other ;;
esac
"#,
        "inner-done\n",
    );
}

/// `case in esac` is NOT an empty case: the subject is `in`, the header
/// `in` never arrives — GNU reports the `esac` as unexpected.
#[test]
fn issue3000_case_in_esac_is_error() {
    check_error(
        "case in esac\necho after:$?\n",
        "case.sh: line 1: syntax error near unexpected token `esac'\ncase.sh: line 1: `case in esac'\n",
    );
}

/// An `esac` as a clause-body command terminates the case (it is a
/// reserved word there), so the following `;;` is GNU's syntax error.
#[test]
fn issue3000_esac_as_clause_command_terminates_case() {
    check_error(
        "case $1 in\na) esac ;;\n(*) echo other ;;\nesac\n",
        "case.sh: line 2: syntax error near unexpected token `;;'\ncase.sh: line 2: `a) esac ;;'\n",
    );
}
