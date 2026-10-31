//! rubash#2309 (filed as the #2274 residual face): the completeness
//! scanner had no notion of the case region. Inside a case arm body —
//! command position again once the pattern list's `)` is read (GNU
//! parse.y:1227-1234 `pattern_list: pattern ')' compound_list`) — a
//! multi-line array assignment opening the body was misjudged as
//! closed: the reader split the group per line and the run died with
//! `syntax error near unexpected token ')'` at the array's closing
//! line.
//!
//! Root cause, three parts, all in the compound-assignment residual
//! machinery of src/lexer/continuation.rs plus its checkpoint driver:
//!
//! 1. `compound_residuals_advance` tracked no case state, so the
//!    pattern word's flush left `seen_command_word` set and the arm
//!    body's first `name=(` failed the assignment gate (the #2274
//!    position model, parse.y:3157-3164). The scan now carries the
//!    PST_CASEPAT region (parse.y:3379/3396 sets it at the case `in`,
//!    parse.y:3438 clears it at `esac`): pattern words are data, the
//!    list's `)` re-opens command position, `;;`/`;&`/`;;&`
//!    (parse.y:1237-1247) re-arm the region, and extglob group parens
//!    nest inside the pattern without ending it.
//! 2. The feeder commits `case WORD in` as its own logical line and
//!    resets every residual checkpoint — the region would have been
//!    lost at that boundary even with (1). The compound checkpoint
//!    now carries its line-boundary state across a commit whenever a
//!    case is open (`compound_line_boundary_state`).
//! 3. A word made only of quoted/expansion spans (`echo "a b"; G=(`)
//!    left the word buffer empty with `word_pure` false, and the
//!    impurity leaked into the next word, closing its `name=(` gate.
//!    The separator arms now reset purity unconditionally: a
//!    separator completes the current word, so the next word starts
//!    bare (one word per read_token).
//!
//! All expectations verified against GNU bash 5.3.20 via script files.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2309-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = scratch(tag);
    let file = dir.join("case.sh");
    fs::write(&file, script).expect("write script");
    let output = Command::new(RUBASH)
        .arg(&file)
        .output()
        .expect("run rubash on script file");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_case_ok(tag: &str, script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = run_script(tag, script);
    assert_eq!(code, Some(0), "{tag}: rc, stderr={stderr}");
    assert_eq!(stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(stderr, "", "{tag}: stderr must be empty");
}

/// The issue's headline reproducer: header on its own line, the
/// multi-line array is the first command of the arm body.
#[test]
fn issue2309_headline_array_opens_case_arm_body() {
    assert_case_ok(
        "headline",
        "case x in\n(x) A=(\n1\n2\n)\necho \"rc=$? n=${#A[@]}\";;\nesac\necho \"done rc=$?\"\n",
        "rc=0 n=2\ndone rc=0\n",
    );
}

/// Header and arm on one physical line (control for the region model
/// inside a single logical line).
#[test]
fn issue2309_array_arm_same_line_as_header() {
    assert_case_ok(
        "same-line-header",
        "case x in (x) A=(\n1\n2\n)\necho \"n=${#A[@]}\";;\nesac\necho done\n",
        "n=2\ndone\n",
    );
}

/// Pattern list with `|` alternatives, several arms closed by `;;`,
/// and a second case in the same script.
#[test]
fn issue2309_multiple_arms_and_pattern_alternatives() {
    assert_case_ok(
        "multiarm",
        "case a in\n(a|b) B=(\np\nq\n)\necho \"B n=${#B[@]}\";;\n(c) C=(\nz\n)\necho \"C n=${#C[@]}\";;\nesac\ncase c in\n(a|b) B2=(\np\n)\necho \"B2 n=${#B2[@]}\";;\n(c) C2=(\nz\nw\n)\necho \"C2 n=${#C2[@]}\";;\nesac\necho \"done\"\n",
        "B n=2\nC2 n=2\ndone\n",
    );
}

/// The `;&` fallthrough terminator: the region re-arms and the next
/// arm body runs without a pattern test.
#[test]
fn issue2309_fallthrough_terminator() {
    assert_case_ok(
        "fallthrough",
        "case m in\n(m) D=(\n1\n)\necho \"D n=${#D[@]}\" ;&\n(n) echo \"fell through\";;\nesac\necho \"done\"\n",
        "D n=1\nfell through\ndone\n",
    );
}

/// The `;;&` test-next terminator: after the array body, the next
/// pattern is tested and its body runs too.
#[test]
fn issue2309_test_next_terminator() {
    assert_case_ok(
        "test-next",
        "case b in\n(b) I=(\n1\n)\necho \"I n=${#I[@]}\";;&\n(b) echo \"retested\";;\nesac\necho \"done\"\n",
        "I n=1\nretested\ndone\n",
    );
}

/// A nested case inside an arm body: two region levels, each with its
/// own pattern list and arm-body array.
#[test]
fn issue2309_nested_case_in_arm_body() {
    assert_case_ok(
        "nested",
        "case o in\n(o) case i in\n     (i) E=(\n     7\n     8\n     )\n     echo \"E n=${#E[@]}\";;\n     esac;;\nesac\necho \"done\"\n",
        "E n=2\ndone\n",
    );
}

/// A case inside a command substitution: the region model must hold
/// while the substitution unit is scanned.
#[test]
fn issue2309_case_inside_command_substitution() {
    assert_case_ok(
        "comsub",
        "out=$(case s in\n(s) F=(\naa\nbb\n)\necho \"F=${#F[@]}\";;\nesac)\necho \"out=[$out]\"\necho \"done\"\n",
        "out=[F=2]\ndone\n",
    );
}

/// The array is NOT the first command of the arm body — a quoted
/// command precedes it on the previous line (the word-purity leak
/// shape: the quoted word must not poison the assignment's gate).
#[test]
fn issue2309_array_not_first_in_arm_body() {
    assert_case_ok(
        "not-first",
        "case t in\n(t) echo \"arm start\"\n    G=(\n    u\n    v\n    )\n    echo \"G n=${#G[@]}\";;\nesac\necho \"done\"\n",
        "arm start\nG n=2\ndone\n",
    );
}

/// An extglob pattern: the group's `)` is pattern text, only the
/// second `)` closes the list — and the array after it still opens.
#[test]
fn issue2309_extglob_pattern_then_array() {
    assert_case_ok(
        "extglob",
        "shopt -s extglob\ncase foo in\n@(foo|bar)) H=(\n1\n)\necho \"H n=${#H[@]}\";;\nesac\necho \"done\"\n",
        "H n=1\ndone\n",
    );
}

/// The purity invariant without any case: a quoted word, a separator,
/// and a multi-line array on one line at top level.
#[test]
fn issue2309_quoted_word_then_array_same_line() {
    assert_case_ok(
        "quote-then-array",
        "echo \"ab\"; G=(\n1\n)\necho \"n=${#G[@]}\"\n",
        "ab\nn=1\n",
    );
}

/// Negative control: a pattern containing parens with a plain arm
/// body — the pattern's parens must not open command position (no
/// phantom assignment region) and the body runs as ordinary commands.
#[test]
fn issue2309_pattern_parens_do_not_open_command_position() {
    assert_case_ok(
        "pattern-parens-plain",
        "shopt -s extglob\ncase q in\n@(q|w)) echo plain;;\nesac\necho \"done\"\n",
        "plain\ndone\n",
    );
}

/// Negative control: an unclosed array in an arm body must still
/// read as incomplete — the EOF diagnostic, not a silent split.
#[test]
fn issue2309_unclosed_array_in_arm_body_still_errors() {
    let (stdout, stderr, code) = run_script("neg-unclosed", "case x in\n(x) A=(\n1\n2\n");
    assert_eq!(code, Some(1), "neg-unclosed: rc, stdout={stdout}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `)'"),
        "neg-unclosed: stderr must carry the EOF diagnostic, got: {stderr}"
    );
    assert_eq!(stdout, "", "neg-unclosed: stdout must be empty");
}
