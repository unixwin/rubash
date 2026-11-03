//! rubash#2676 — heredoc body expansion applied `${var:=word}`
//! assignments in the wrong ORDER: the embedded-mutation walker
//! hoisted every top-level `:=`/`=` binding ahead of the pass
//! (a pre-scan), so an earlier `${var:-def}` fragment rendered the
//! value a LATER `:=` would bind. GNU expands a heredoc body in one
//! left-to-right pass — redir.c:374 heredoc_expand runs the body
//! through subst.c's expand_string_internal (Q_HERE_DOCUMENT), the
//! same core as ordinary words — so each fragment renders against
//! the environment as it stands when the pass reaches it.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue2676-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn default_before_assign_renders_the_default() {
    // The A1-F01 headline shape: GNU prints `X 1 1` — the first `:-`
    // runs before the `:=` binds. The pre-scan printed `1 1 1`.
    let (stdout, stderr, code) = run_script(
        "default-before-assign",
        "unset a\ncat <<EOT\n${a:-X} ${a:=1} ${a:-Y}\nEOT\n",
    );
    assert_eq!(stdout, "X 1 1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn a1_f01_original_shape() {
    let (stdout, _, code) = run_script(
        "a1-f01",
        "unset undef\ncat <<EOT\n${undef:-def} ${undef:=set} $undef\nEOT\n",
    );
    assert_eq!(stdout, "def set set\n");
    assert_eq!(code, Some(0));
}

#[test]
fn assign_first_control_unchanged() {
    // After `${a:=1}` binds, `${a:-X}` renders the value (1), not
    // the default — GNU prints `1 1 1` here too.
    let (stdout, _, code) = run_script(
        "assign-first",
        "unset a\ncat <<EOT\n${a:=1} ${a:-X} $a\nEOT\n",
    );
    assert_eq!(stdout, "1 1 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn nested_rhs_assignment_applies_at_its_fragment() {
    // `${a:-${b:=2}}`: evaluating the outer fragment's word binds b
    // at that point (GNU subst.c parameter_brace_expand_word), and a
    // later fragment in the same body reads it.
    let (stdout, _, code) = run_script(
        "nested-rhs",
        "unset a b\n: <<EOT\n${a:-${b:=2}}\nEOT\necho \"b=${b-unset}\"\n",
    );
    assert_eq!(stdout, "b=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn comsub_in_body_reads_earlier_assignment() {
    // B2 hd67: a command substitution later in the body observes an
    // earlier fragment's binding (same single pass).
    let (stdout, _, code) = run_script(
        "comsub-reads-earlier",
        "unset a\ncat <<EOT\n${a:=hello} $(echo \"comsub sees $a\")\nEOT\n",
    );
    assert_eq!(stdout, "hello comsub sees hello\n");
    assert_eq!(code, Some(0));
}

#[test]
fn arithmetic_assignment_is_not_hoisted() {
    // B2 hd66 control: arithmetic assignments were never part of the
    // pre-scan; the pass applies `$((b+=1))` once, at its position.
    let (stdout, _, code) = run_script(
        "arith-not-hoisted",
        "unset b\n: <<EOT\n$((b+=1)) ${b:-D}\nEOT\necho \"b=${b-unset}\"\n",
    );
    assert_eq!(stdout, "b=1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn assign_rhs_comsub_runs_once() {
    // The `:=` fragment's word is evaluated once, when the pass
    // reaches the fragment — not once by a pre-scan and again by the
    // render.
    let (stdout, _, code) = run_script(
        "rhs-comsub-once",
        "unset a\ncat <<EOT >/dev/null\n${a:=$(echo ran >> runs.txt)} $a\nEOT\nwc -l < runs.txt\n",
    );
    assert_eq!(stdout, "1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn herestring_word_expands_in_order() {
    // A here-string word expands at the same do_redirections point
    // (redir.c:373, r_reading_string) in the same single pass.
    let (stdout, _, code) = run_script(
        "herestring-order",
        "unset a\ncat <<< \"${a:-X} ${a:=1} ${a:-Y}\"\n",
    );
    assert_eq!(stdout, "X 1 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn assign_then_plain_reads_in_same_body() {
    // Fragments after the `:=` read the bound value in the same
    // pass (the render side of the order guarantee).
    let (stdout, _, code) = run_script(
        "assign-then-read",
        "unset a\ncat <<EOT\n${a:=ok} $a ${a}\nEOT\n",
    );
    assert_eq!(stdout, "ok ok ok\n");
    assert_eq!(code, Some(0));
}
