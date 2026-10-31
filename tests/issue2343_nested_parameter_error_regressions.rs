//! rubash#2343 — a `${inner:?msg}` failure inside another expansion's
//! word is fatal and is reported for the INNER parameter. GNU expands
//! the word of an operator exactly when the operator selects it
//! (subst.c parameter_brace_expand / parameter_brace_expand_rhs), and
//! parameter_brace_expand_error (subst.c:8216) expands the message word
//! before reporting, so a nested failure pre-empts the outer report.
//! The message text is never a value, and an unevaluated word (operator
//! not selecting it) stays silent.
//!
//! Expectations verified against GNU bash 5.3.20 with script files
//! (fatal expansion status is 1 in file mode).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2343-{tag}-{}", std::process::id()));
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
fn nested_error_in_message_names_inner_variable() {
    let script = "unset a b; echo \"[${a:?${b:?inner}}]\"; echo rc=$?\n";
    let (stdout, stderr, code) = rubash_file("msg", script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "t.sh: line 1: b: inner\n");
    assert_eq!(code, Some(1));
}

#[test]
fn nested_error_under_default_operator_is_fatal() {
    for (tag, script) in [
        (
            "colon",
            "unset a b; echo \"[${a:-${b:?inner}}]\"; echo rc=$?\n",
        ),
        (
            "plain",
            "unset a b; echo \"[${a-${b:?inner}}]\"; echo rc=$?\n",
        ),
        (
            "assign",
            "unset a b; echo \"[${a:=${b:?inner}}]\"; echo rc=$?\n",
        ),
    ] {
        let (stdout, stderr, code) = rubash_file(tag, script);
        assert_eq!(stdout, "", "shape {tag}");
        assert_eq!(stderr, "t.sh: line 1: b: inner\n", "shape {tag}");
        assert_eq!(code, Some(1), "shape {tag}");
    }
}

#[test]
fn nested_error_under_alternate_operator_fires_only_when_word_used() {
    let script = "\
unset a b; echo \"[${a:+${b:?inner}}]\"; echo \"rc=$? after\"
a=X; unset b; echo \"[${a:+${b:?inner}}]\"; echo rc=$?
";
    let (stdout, stderr, code) = rubash_file("alt", script);
    assert_eq!(stdout, "[]\nrc=0 after\n");
    assert_eq!(stderr, "t.sh: line 2: b: inner\n");
    assert_eq!(code, Some(1));
}

#[test]
fn nested_error_stays_silent_when_word_not_evaluated() {
    let script = "\
unset b; a2=set; echo \"[${a2:-${b:?inner}}]\"; echo rc=$?
a=ok; unset b; echo \"[${a:?${b:-X}}]\"; echo rc=$?
unset a b; echo \"[${a:-${b:-inner}}]\"; echo rc=$?
";
    let (stdout, stderr, code) = rubash_file("lazy", script);
    assert_eq!(stdout, "[set]\nrc=0\n[ok]\nrc=0\n[inner]\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn nested_error_inside_command_substitution_is_contained() {
    let script = "\
unset b; echo \"[$(echo ${b:?inner})]\"; echo \"rc=$? after\"
unset a b; echo \"[${a:-$(echo ${b:?inner})}]\"; echo \"rc=$? after\"
";
    let (stdout, stderr, code) = rubash_file("comsub", script);
    assert_eq!(stdout, "[]\nrc=0 after\n[]\nrc=0 after\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: b: inner\n\
         t.sh: line 2: b: inner\n"
    );
    assert_eq!(code, Some(0));
}
