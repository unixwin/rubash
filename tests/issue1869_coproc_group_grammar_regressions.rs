//! Regression pins for rubash#1869 — coproc NAME grammar must be the
//! same on every parse route (top level, brace group, function body),
//! and a multi-line group whose first line is a coproc must parse as
//! one group.
//!
//! GNU parse.y:1125-1175 gives coproc three productions —
//! `COPROC shell_command`, `COPROC WORD shell_command`, and
//! `COPROC simple_command`. A WORD is the coproc NAME only before a
//! compound/shell command; before a SIMPLE command there is no name
//! production, so `coproc FC2 cat` runs the simple command `FC2 cat`
//! (the child reports `FC2: command not found`) and the fds bind as
//! COPROC. The #1625 fold/parser fixes (merged in PR #1866) unified
//! the routes; these pins lock the result on shapes measured against
//! GNU bash 5.3.20 (script-file probes, 2026-10-27):
//!
//! - `{ coproc FC2 cat; ... }`, a function body, and the top level
//!   all leave FC2 unbound, bind COPROC, and the child reports
//!   `FC2: command not found` (given a settle window — the report is
//!   asynchronous on both shells).
//! - Multi-line groups with a coproc on the first line parse whole:
//!   no `coproc: command not found`, no stray `}` syntax error.
//!
//! The known remaining divergence in this family — a coproc child's
//! stderr is forwarded through a pipe whose read end dies with the
//! parent, so a diagnostic emitted after a prompt parent exit is
//! lost — is a stderr-lifetime mechanism, not grammar; it is
//! recorded in the wave-3 remainder cluster REPORT (probes
//! p1869_child.sh / p1869_grp.sh) and deliberately not pinned here.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1869-{}-{}",
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

/// The settle window lets the asynchronous child report land on both
/// shells; the assertions are on the BINDINGS (deterministic) plus
/// the child's report text.
fn assert_name_not_stolen(script: &str, label: &str) {
    let (_stdout, stderr, _code) = rubash_file(script);
    assert!(
        stderr.contains("FC2: command not found"),
        "{label}: child must report FC2 as the command: {stderr:?}"
    );
    assert!(
        stderr.contains("declare: FC2: not found"),
        "{label}: FC2 must stay unbound (no name stealing): {stderr:?}"
    );
}

#[test]
fn issue1869_toplevel_simple_coproc_word_is_command_not_name() {
    assert_name_not_stolen("coproc FC2 cat\nsleep 0.6\ndeclare -p FC2\n", "top level");
}

#[test]
fn issue1869_group_simple_coproc_word_is_command_not_name() {
    assert_name_not_stolen(
        "{ coproc FC2 cat; sleep 0.6; declare -p FC2; }\n",
        "brace group",
    );
}

#[test]
fn issue1869_function_body_simple_coproc_word_is_command_not_name() {
    assert_name_not_stolen(
        "f() { coproc FC2 cat; sleep 0.6; declare -p FC2; }\nf\n",
        "function body",
    );
}

#[test]
fn issue1869_multiline_group_named_compound_coproc_first_line() {
    let (stdout, stderr, code) =
        rubash_file("{ coproc CP { cat; }\necho grouped-ok\n}\necho \"after rc=$?\"\n");
    assert_eq!(stdout, "grouped-ok\nafter rc=0\n");
    assert!(
        !stderr.contains("command not found") && !stderr.contains("syntax error"),
        "multi-line group misparse: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1869_multiline_group_unnamed_simple_coproc_first_line() {
    let (stdout, stderr, code) = rubash_file("{\ncoproc cat\necho in-group\n}\necho after\n");
    assert_eq!(stdout, "in-group\nafter\n");
    assert!(
        !stderr.contains("command not found") && !stderr.contains("syntax error"),
        "multi-line group misparse: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}
