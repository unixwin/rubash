//! rubash#3495 — unquoted digit-leading compound LHS is a GNU parse-time
//! syntax error, not a builtin diagnostic.
//!
//! GNU ground (bash 5.3.20, script-file probes, probe3495/):
//! - `declare -a 9bad=(x)` (also `9bad+=(x)`, `9bad[0]=(x)`, bare `9bad=(x)`,
//!   `echo 9bad=(x)`, `declare`/`typeset`/`export`/`readonly` without `-a`,
//!   `local` inside a function body, `if`-wrapped): `syntax error near
//!   unexpected token \`('`, rc=2, the whole script dies — nothing at or
//!   after the line runs, and a later operand on the same line never binds.
//! - Bare `9bad=(x)` names the NEXT token instead: `syntax error near
//!   unexpected token \`x'` (the `(` shifts as a function-head candidate
//!   after the single word, parse.y:1054).
//! - `eval 'declare -a 9bad=(x)'` is contained: eval rc=2, the script
//!   continues after it.
//! - Guards that must NOT change: `declare -a 9bad=x` (no paren) and
//!   quoted `declare -a '9bad=(x)'` stay builtin `not a valid identifier`
//!   (rc=1, script continues); `declare -a 9bad[0]=x` stays InvalidId;
//!   `a9=(x y)` and `_9=(x)` still bind; bare `9bad=x` stays a
//!   `command not found` (rc=127).
//!
//! Root cause: GNU gates the `=(` compound branch on token_is_assignment
//! (parse.y:5653 -> parse.y:5270 -> general.c:480 assignment()), which
//! requires a legal-variable-starter first character (general.c:498-502).
//! The lexer's compound_assignment_start (src/lexer/word.rs) used a
//! weaker all-alnum-plus-any-alpha test, so it folded `9bad=(x)` into one
//! atomic word and the declare builtin reported InvalidId (rc=1). The fix
//! requires the starter character there, matching the sibling validators
//! classification::is_assignment and token_actions::valid_compound_assignment_lhs
//! (both already enforced it since #3464); the parser's stray-`(` arm then
//! reports the error. The #3464 nail that locked the InvalidId interim now
//! expects the parse error (tests/issue3186_*).
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle; the
//! script path is normalized to SCRIPT, mirroring tests/issue2462_* and
//! tests/issue3186_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3495-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Every spelling under which `path` may surface in rubash's output:
/// the native spelling, the forward-slash spelling, and — on Windows —
/// the translated `/<drive>/...` form rubash prints in diagnostics.
/// On unix the extra spellings coincide with the native one.
fn path_variants(path: &Path) -> Vec<String> {
    let native = path.to_string_lossy().into_owned();
    let mut variants = vec![native.clone()];
    let forward = native.replace('\\', "/");
    if forward != native {
        variants.push(forward.clone());
    }
    if forward.len() >= 2
        && forward.as_bytes()[1] == b':'
        && forward.as_bytes()[0].is_ascii_alphabetic()
    {
        let rest = &forward[2..];
        let drive = forward.as_bytes()[0] as char;
        variants.push(format!("/{}{rest}", drive.to_ascii_lowercase()));
        variants.push(format!("/{drive}{rest}"));
    }
    variants
}

/// Run `script` as a script FILE; return (stdout, stderr, exit code) with
/// the scratch path normalized to WORK and the script path to SCRIPT.
fn run_script(dir: &Path, script: &str) -> (String, String, i32) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        let mut out = text.to_string();
        for (path, token) in [(script_path.as_path(), "SCRIPT"), (dir, "WORK")] {
            for variant in path_variants(path) {
                out = out.replace(&variant, token);
            }
        }
        out
    };
    (
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
        output.status.code().unwrap_or(-1),
    )
}

fn assert_parse_error(stdout: &str, stderr: &str, code: i32) {
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
}

#[test]
fn declare_a_digit_leading_compound_is_parse_error() {
    let dir = scratch("decl-a");
    let (stdout, stderr, code) = run_script(&dir, "declare -a 9bad=(x)\necho \"rc=$?\"\n");
    assert_eq!(stdout, "");
    assert_eq!(code, 2, "stderr: {stderr}");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: syntax error near unexpected token `('\nSCRIPT: line 1: `declare -a 9bad=(x)'\n"
    );
}

#[test]
fn declare_a_digit_leading_compound_plus_equals_is_parse_error() {
    let dir = scratch("decl-plus");
    let (stdout, stderr, code) = run_script(&dir, "declare -a 9bad+=(x)\necho \"rc=$?\"\n");
    assert_eq!(stdout, "");
    assert_eq!(code, 2, "stderr: {stderr}");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: syntax error near unexpected token `('\nSCRIPT: line 1: `declare -a 9bad+=(x)'\n"
    );
}

#[test]
fn bare_digit_leading_compound_names_next_token() {
    // GNU names `x`, not `(`: the `(` shifts as a function-head candidate
    // after the single word `9bad=` (parse.y:1054).
    let dir = scratch("bare");
    let (stdout, stderr, code) = run_script(&dir, "9bad=(x)\necho \"rc=$?\"\n");
    assert_eq!(stdout, "");
    assert_eq!(code, 2, "stderr: {stderr}");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: syntax error near unexpected token `x'\nSCRIPT: line 1: `9bad=(x)'\n"
    );
}

#[test]
fn declare_a_digit_leading_compound_subscript_is_parse_error() {
    let dir = scratch("decl-sub");
    let (stdout, stderr, code) = run_script(&dir, "declare -a 9bad[0]=(x)\necho \"rc=$?\"\n");
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn declare_without_flag_digit_leading_compound_is_parse_error() {
    let dir = scratch("decl-noflag");
    let (stdout, stderr, code) = run_script(&dir, "declare 9bad=(x)\necho \"rc=$?\"\n");
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn typeset_a_digit_leading_compound_is_parse_error() {
    let dir = scratch("typeset");
    let (stdout, stderr, code) = run_script(&dir, "typeset -a 9bad=(x)\necho \"rc=$?\"\n");
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn export_a_digit_leading_compound_is_parse_error() {
    let dir = scratch("export");
    let (stdout, stderr, code) = run_script(&dir, "export -a 9bad=(x)\necho \"rc=$?\"\n");
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn readonly_a_digit_leading_compound_is_parse_error() {
    let dir = scratch("readonly");
    let (stdout, stderr, code) = run_script(&dir, "readonly -a 9bad=(x)\necho \"rc=$?\"\n");
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn echo_digit_leading_compound_is_parse_error() {
    // Even as an ordinary command's argument: the `(` is a shellbreak once
    // the compound branch refuses the digit-leading LHS.
    let dir = scratch("echo");
    let (stdout, stderr, code) = run_script(&dir, "echo 9bad=(x)\necho \"rc=$?\"\n");
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn local_digit_leading_compound_in_function_is_parse_error() {
    // The error is at parse of the function definition: the whole script
    // dies, the function never runs.
    let dir = scratch("local");
    let (stdout, stderr, code) = run_script(
        &dir,
        "g() { local -a 9bad=(x); echo \"rc=$?\"; }\ng\necho AFTER\n",
    );
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn if_wrapped_digit_leading_compound_is_parse_error() {
    let dir = scratch("ifwrap");
    let (stdout, stderr, code) =
        run_script(&dir, "if true; then declare -a 9bad=(x); fi\necho AFTER\n");
    assert_parse_error(&stdout, &stderr, code);
}

#[test]
fn later_operand_on_same_line_never_binds() {
    let dir = scratch("later-op");
    let (stdout, stderr, code) = run_script(&dir, "declare -a 9bad=(x) good=(y)\necho \"rc=$?\"\n");
    assert_parse_error(&stdout, &stderr, code);
    assert!(
        stderr.contains("`declare -a 9bad=(x) good=(y)'"),
        "stderr: {stderr}"
    );
}

#[test]
fn eval_digit_leading_compound_is_contained() {
    // The eval re-parse fails the same way, but only the eval dies: the
    // enclosing script continues (GNU probe s18).
    let dir = scratch("eval");
    let (stdout, stderr, code) = run_script(
        &dir,
        "eval 'declare -a 9bad=(x)'\necho \"rc=$?\"\necho AFTER\n",
    );
    assert_eq!(stdout, "rc=2\nAFTER\n");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
}

#[test]
fn declare_a_digit_leading_scalar_stays_invalid_id() {
    // No paren, no compound branch: still the builtin diagnostic, rc=1,
    // and the script continues.
    let dir = scratch("scalar");
    let (stdout, stderr, code) =
        run_script(&dir, "declare -a 9bad=x\necho \"rc=$?\"\necho AFTER\n");
    assert_eq!(stdout, "rc=1\nAFTER\n");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `9bad=x': not a valid identifier\n"
    );
}

#[test]
fn declare_a_digit_leading_compound_quoted_stays_invalid_id() {
    // Quoted: the `(` is data, never a compound branch — builtin
    // diagnostic, rc=1, script continues.
    let dir = scratch("quoted");
    let (stdout, stderr, code) =
        run_script(&dir, "declare -a '9bad=(x)'\necho \"rc=$?\"\necho AFTER\n");
    assert_eq!(stdout, "rc=1\nAFTER\n");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `9bad=(x)': not a valid identifier\n"
    );
}

#[test]
fn declare_a_digit_leading_subscript_scalar_stays_invalid_id() {
    let dir = scratch("sub-scalar");
    let (stdout, stderr, code) =
        run_script(&dir, "declare -a 9bad[0]=x\necho \"rc=$?\"\necho AFTER\n");
    assert_eq!(stdout, "rc=1\nAFTER\n");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `9bad[0]=x': not a valid identifier\n"
    );
}

#[test]
fn declare_a_letter_leading_compound_still_binds() {
    let dir = scratch("valid-a9");
    let (stdout, stderr, code) =
        run_script(&dir, "declare -a a9=(x y)\necho \"rc=$?\"\ndeclare -p a9\n");
    assert_eq!(stdout, "rc=0\ndeclare -a a9=([0]=\"x\" [1]=\"y\")\n");
    assert_eq!(stderr, "");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn declare_a_underscore_digit_compound_still_binds() {
    let dir = scratch("valid-u9");
    let (stdout, stderr, code) =
        run_script(&dir, "declare -a _9=(x)\necho \"rc=$?\"\ndeclare -p _9\n");
    assert_eq!(stdout, "rc=0\ndeclare -a _9=([0]=\"x\")\n");
    assert_eq!(stderr, "");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn declare_a_valid_compound_still_binds() {
    let dir = scratch("valid-good");
    let (stdout, stderr, code) = run_script(
        &dir,
        "declare -a 'good=(x y)'\necho \"rc=$?\"\ndeclare -p good\n",
    );
    assert_eq!(stdout, "rc=0\ndeclare -a good=([0]=\"x\" [1]=\"y\")\n");
    assert_eq!(stderr, "");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn bare_digit_leading_scalar_stays_command_not_found() {
    // No `= (` involved: `9bad=x` is not an assignment word (GNU
    // general.c:498 first-character gate) and runs as a command name.
    let dir = scratch("bare-scalar");
    let (stdout, stderr, code) = run_script(&dir, "9bad=x\necho \"rc=$?\"\necho AFTER\n");
    assert_eq!(stdout, "rc=127\nAFTER\n");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("9bad=x: command not found"),
        "stderr: {stderr}"
    );
}
