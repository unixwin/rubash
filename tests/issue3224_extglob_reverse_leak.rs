//! Issue unixwin/rubash#3224: the reverse of #3205 — an extglob
//! pattern node lexed while the gate is ON is rejected at execution
//! time when a `shopt -u extglob` earlier in the same read unit has
//! already executed.
//!
//! GNU contract: read legality and execution semantics are decided
//! by two different states. `read_token_word` consults the
//! read-time `extended_glob` flag (parse.y:5466), so a pattern
//! formed with the gate on is a legal word — full stop. At
//! execution the pattern text is only text: pathname expansion
//! asks the LIVE flag whether the word is a glob pattern at all
//! (pathexp.c `unquoted_glob_pattern_p`), and every pattern match
//! (`case`, `${var#pat}`, `${var/pat/repl}`) goes through
//! `strmatch` with `FNMATCH_EXTFLAG` = `(extended_glob ?
//! FNM_EXTMATCH : 0)` (pathexp.h), evaluated with the live global —
//! gate off means the operators degrade to literal text and simply
//! do not match (or match the literal string), never an error.
//! The one face that never degrades is `[[ == ]]`: cond execution
//! forces `extended_glob = 1` around `cond_test`
//! (execute_cmd.c:4109-4111), so conditional RHS patterns keep
//! extglob semantics whatever `shopt` says.
//!
//! Expectations below are byte-verified against GNU bash 5.3.20
//! (script files run with cwd = a fixture directory holding
//! exactly `ab abcx abcz ba bbc cab`).
//!
//! Defect cells (red before the fix): a1, a2, a6, e1 — the live
//! execution state vetoes a legally-read pattern node with
//! `syntax error near unexpected token '('` (rc=2, the rest of the
//! script is abandoned); GNU prints the word literally and runs
//! on (rc=0). The b/c cells pin the matching faces of the same
//! adjudication relation and the d cells pin the conditional
//! exception; the *c cells and a4/d2 are controls (green before
//! and after): a4 keeps the read-time error for a gate that was
//! never on, the *c cells keep full extglob semantics when the
//! gate stays on.

use std::path::PathBuf;
use std::process::Command;

fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash3224-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = dir.join("files");
    std::fs::create_dir_all(&files).expect("create fixture dir");
    for name in ["ab", "abcx", "abcz", "ba", "bbc", "cab"] {
        std::fs::write(files.join(name), "").expect("write fixture file");
    }
    (dir, files)
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let (dir, files) = fixture(tag);
    let file = dir.join("case.sh");
    std::fs::write(&file, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .current_dir(&files)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn assert_literal_fallback(tag: &str, script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = run_script(tag, script);
    assert_eq!(code, Some(0), "{tag}: rc (stderr={stderr})");
    assert_eq!(stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(stderr, "", "{tag}: stderr");
}

#[test]
fn a1_semicolon_segment_falls_back_to_literal() {
    assert_literal_fallback(
        "a1",
        "shopt -s extglob\nshopt -u extglob; echo !(bbc)\necho after\n",
        "!(bbc)\nafter\n",
    );
}

#[test]
fn a2_at_group_falls_back_to_literal() {
    assert_literal_fallback(
        "a2",
        "shopt -s extglob\nshopt -u extglob; echo @(bbc)\necho after\n",
        "@(bbc)\nafter\n",
    );
}

#[test]
fn a6_andor_segment_falls_back_to_literal() {
    assert_literal_fallback(
        "a6",
        "shopt -s extglob\nshopt -u extglob && echo !(bbc)\necho after\n",
        "!(bbc)\nafter\n",
    );
}

#[test]
fn a3c_gate_left_on_still_globs() {
    assert_literal_fallback(
        "a3c",
        "shopt -s extglob\necho !(bbc)\n",
        "ab abcx abcz ba cab\n",
    );
}

#[test]
fn a4_gate_never_on_is_read_error() {
    let (stdout, stderr, code) = run_script("a4", "echo !(bbc)\necho after\n");
    assert_eq!(code, Some(2), "a4: rc (stderr={stderr})");
    assert_eq!(stdout, "", "a4: a read error leaves no stdout");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "a4: stderr names the paren (got {stderr})"
    );
}

#[test]
fn b1_case_pattern_degrades_to_literal_no_match() {
    assert_literal_fallback(
        "b1",
        "shopt -s extglob\n{ shopt -u extglob; case bbc in !(x)) echo NEG;; *) echo STAR;; esac; }\necho after\n",
        "STAR\nafter\n",
    );
}

#[test]
fn b1c_case_pattern_gate_on_negates() {
    assert_literal_fallback(
        "b1c",
        "shopt -s extglob\n{ case bbc in !(x)) echo NEG;; *) echo STAR;; esac; }\necho after\n",
        "NEG\nafter\n",
    );
}

#[test]
fn b2_case_pattern_matches_literal_string() {
    assert_literal_fallback(
        "b2",
        "shopt -s extglob\n{ shopt -u extglob; case \"!(x)\" in !(x)) echo LIT;; *) echo STAR;; esac; }\necho after\n",
        "LIT\nafter\n",
    );
}

#[test]
fn c1_prefix_strip_pattern_degrades_to_literal() {
    assert_literal_fallback(
        "c1",
        "shopt -s extglob\n{ shopt -u extglob; v=aab; echo \"${v#+(a)}\"; }\necho after\n",
        "aab\nafter\n",
    );
}

#[test]
fn c1c_prefix_strip_pattern_gate_on() {
    assert_literal_fallback(
        "c1c",
        "shopt -s extglob\n{ v=aab; echo \"${v#+(a)}\"; }\necho after\n",
        "ab\nafter\n",
    );
}

#[test]
fn c2_substitution_pattern_degrades_to_literal() {
    assert_literal_fallback(
        "c2",
        "shopt -s extglob\n{ shopt -u extglob; v=aab; echo \"${v/+(a)/X}\"; }\necho after\n",
        "aab\nafter\n",
    );
}

#[test]
fn c2c_substitution_pattern_gate_on() {
    assert_literal_fallback(
        "c2c",
        "shopt -s extglob\n{ v=aab; echo \"${v/+(a)/X}\"; }\necho after\n",
        "Xb\nafter\n",
    );
}

#[test]
fn d1_conditional_extglob_survives_shopt_u() {
    assert_literal_fallback(
        "d1",
        "shopt -s extglob\n{ shopt -u extglob; [[ bbc == !(x) ]] && echo MATCH || echo NOMATCH; }\necho after\n",
        "MATCH\nafter\n",
    );
}

#[test]
fn d2_conditional_extglob_without_any_shopt() {
    assert_literal_fallback(
        "d2",
        "[[ bbc == !(x) ]] && echo MATCH || echo NOMATCH\necho after\n",
        "MATCH\nafter\n",
    );
}

#[test]
fn e1_function_body_pattern_falls_back_to_literal() {
    assert_literal_fallback(
        "e1",
        "shopt -s extglob\nf() { echo !(bbc); }\nshopt -u extglob\nf\necho after\n",
        "!(bbc)\nafter\n",
    );
}
