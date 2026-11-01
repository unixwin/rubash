//! rubash#2410 — cross-command process-substitution VALUE leak.
//!
//! GNU recognizes a process substitution only while scanning the
//! word's own characters at expansion time: expand_word_internal's
//! '<'/'>' cases (subst.c:11365-11385) hand the in-word text to
//! extract_process_subst (subst.c:1311) / process_substitute
//! (subst.c:6362). A value that arrives through parameter expansion is
//! appended as data and never re-scanned, so `x='<(echo hi)'` followed
//! by `echo "$x"` prints the literal `<(echo hi)` — in ANY session
//! state. Rubash's post-expansion materializer re-scanned the expanded
//! word text whenever word metadata had been dropped (the common case:
//! expand_command_words preserves metadata only for a small command
//! set), so the data word was materialized as if it were source
//! syntax: the echo printed a /dev/fd/N path and `>(...)` values
//! actually EXECUTED their bodies. The fix gates the standalone
//! materialization arm on parse provenance (the node's substitution
//! aggregate, cloned through expansion) and records provenance for
//! alias-introduced words (alias values are re-lexed by GNU at parse
//! time, parse.y push_string, so their substitutions are source).
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! (2026-11-01). Unix-only: the materialized path form is /dev/fd/N.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2410-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_script(case: &Case, script: &str) -> (String, String, Option<i32>) {
    let path = case.dir.join("case.sh");
    std::fs::write(&path, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&case.dir)
        .arg("case.sh")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Seed shape: even after a real substitution was materialized earlier
/// in the session, a quoted variable holding the literal text stays
/// literal (GNU prints `<(echo hi)`).
#[test]
fn issue2410_quoted_value_stays_literal_after_real_procsub() {
    let case = make_case("seed");
    let script = r#"cat <(echo real)
x='<(echo hi)'
echo "$x"
"#;
    let (stdout, stderr, code) = run_script(&case, script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "real\n<(echo hi)\n");
}

/// Position and reference forms of the same data word: all literal.
#[test]
fn issue2410_value_literal_in_all_word_positions() {
    let case = make_case("positions");
    let script = r#"x='<(echo hi)'
echo head "$x"
echo "$x" tail
echo "${x}"
f() { echo "arg=$1"; }
f "$x"
/bin/echo "$x"
"#;
    let (stdout, stderr, code) = run_script(&case, script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "head <(echo hi)\n<(echo hi) tail\n<(echo hi)\narg=<(echo hi)\n<(echo hi)\n"
    );
}

/// An output-substitution VALUE must not execute its body: GNU runs
/// nothing and creates no file; the buggy materializer spawned the
/// body (creating the capture file) and printed a /dev/fd/N path.
#[test]
fn issue2410_output_value_does_not_execute() {
    let case = make_case("output-value");
    let script = r#"y='>(cat > side.txt)'
echo "$y"
if test -e side.txt; then echo SIDE-EFFECT; else echo no-side-effect; fi
"#;
    let (stdout, stderr, code) = run_script(&case, script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, ">(cat > side.txt)\nno-side-effect\n");
}

/// `cat "$x"` treats the value as a file NAME (and fails), it does not
/// read a materialized body.
#[test]
fn issue2410_cat_of_value_is_a_filename() {
    let case = make_case("cat-value");
    let script = r#"x='<(echo hi)'
cat "$x"
echo "rc=$?"
"#;
    let (stdout, stderr, code) = run_script(&case, script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("<(echo hi)"),
        "cat should report the literal name, stderr: {stderr}"
    );
}

/// Real source substitutions in the same session keep working on the
/// word, redirect and diff faces, and an alias-introduced substitution
/// (re-lexed source in GNU) still materializes.
#[test]
fn issue2410_real_procsub_faces_still_materialize() {
    let case = make_case("real-faces");
    let script = r#"shopt -s expand_aliases
cat <(echo word-face)
cat < <(echo redirect-face)
diff <(echo same) <(echo same)
echo "diff-rc=$?"
read line < <(echo feed-face)
echo "read=$line"
alias show='cat <(echo alias-face)'
show
for f in <(echo loop-face); do cat "$f"; done
"#;
    let (stdout, stderr, code) = run_script(&case, script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "word-face\nredirect-face\ndiff-rc=0\nread=feed-face\nalias-face\nloop-face\n"
    );
}

/// Nested command substitution carrying a real substitution still
/// resolves (control for the provenance gate's blast radius).
#[test]
fn issue2410_nested_comsub_procsub_still_works() {
    let case = make_case("nested");
    let script = r#"echo "$(cat <(echo nested-face))"
x='<(echo hi)'
echo "$x"
"#;
    let (stdout, stderr, code) = run_script(&case, script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "nested-face\n<(echo hi)\n");
}
