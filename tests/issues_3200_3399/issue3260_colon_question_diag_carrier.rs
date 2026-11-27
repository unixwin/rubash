//! rubash#3260 — a `:?`/`?` diagnostic message word keeps quoted
//! whitespace behind the IFS_GLUE carrier (`\x1c`) for field splitting,
//! and the diagnostic path printed the carrier raw (`q: e\x1c f`).
//! GNU subst.c:8216 parameter_brace_expand_error expands the message
//! word (expand_string, quote removal included) and reports the plain
//! text, so the message must go through the sanctioned visible-text
//! decoder (`locale::decode_to_visible_text`) before it is reported.
//!
//! Expectations verified against GNU bash 5.3.20 with script files
//! (fatal expansion status is 1 in file mode; the pipeline cell keeps
//! the last element's status, 0, exactly like GNU).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3260-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn double_quoted_span_space_restored() {
    let script = "unset q; echo ${q:?\"e f\"}\n";
    let (stdout, stderr, code) = rubash_file("dq", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: e f\n");
    assert_eq!(code, Some(1));
}

#[test]
fn single_quoted_span_space_restored() {
    let script = "unset q; echo ${q:?'e f'}\n";
    let (stdout, stderr, code) = rubash_file("sq", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: e f\n");
    assert_eq!(code, Some(1));
}

#[test]
fn double_quoted_span_tab_restored() {
    let script = "unset q; echo ${q:?\"e\tf\"}\n";
    let (stdout, stderr, code) = rubash_file("tab", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: e\tf\n");
    assert_eq!(code, Some(1));
}

#[test]
fn non_colon_question_span_restored() {
    let script = "unset q; echo ${q?\"e f\"}\n";
    let (stdout, stderr, code) = rubash_file("plain", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: e f\n");
    assert_eq!(code, Some(1));
}

#[test]
fn nested_expansion_inside_double_quoted_span() {
    let script = "v=W; unset q; echo ${q:?\"e $v f\"}\n";
    let (stdout, stderr, code) = rubash_file("nested", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: e W f\n");
    assert_eq!(code, Some(1));
}

#[test]
fn pipeline_element_span_restored() {
    let script = "unset q; echo ${q:?\"e f\"} | cat\n";
    let (stdout, stderr, code) = rubash_file("pipe", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: e f\n");
    assert_eq!(code, Some(0));
}

#[test]
fn heredoc_body_span_restored() {
    let script = "unset q\ncat <<EOF\n${q:?\"e f\"}\nEOF\n";
    let (stdout, stderr, code) = rubash_file("heredoc", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 2: q: e f\n");
    assert_eq!(code, Some(1));
}

#[test]
fn unquoted_word_unchanged_control() {
    let script = "unset q; echo ${q:?e f}\n";
    let (stdout, stderr, code) = rubash_file("unquoted", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: e f\n");
    assert_eq!(code, Some(1));
}

#[test]
fn empty_message_default_unchanged() {
    let script = "unset q; echo ${q:?}\n";
    let (stdout, stderr, code) = rubash_file("empty", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: parameter null or not set\n");
    assert_eq!(code, Some(1));
}

#[test]
fn expansion_value_inside_quotes_unchanged() {
    let script = "v=\"a b\"; unset q; echo ${q:?\"$v\"}\n";
    let (stdout, stderr, code) = rubash_file("value", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: q: a b\n");
    assert_eq!(code, Some(1));
}

#[test]
fn value_context_glue_preserved() {
    // Guard for the fix layer: the IFS_GLUE carrier must survive in
    // VALUE positions (`:-` keeps one quoted word through field
    // splitting, rubash#212) — only the `:?` diagnostic display decodes.
    let script = "unset v; a=(${v:-\"e f\"}); echo ${#a[@]}\n";
    let (stdout, stderr, code) = rubash_file("guard", script);
    assert_eq!(stdout, "1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
