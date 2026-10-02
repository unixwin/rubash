//! rubash#389: with `shopt -s extglob` in effect, compound-assignment
//! elements carrying extglob patterns were rejected at parse time —
//! `arr=(@(foo|bar) [name]=+(test|bench))` died with `syntax error near
//! unexpected token `('` where GNU 5.3.0 parses and stores the elements.
//! (The extglob-OFF rejection is correct and GNU-identical — that half
//! is owned by the #386 family tests.)
//!
//! GNU spec:
//! - parse.y:5652-5671 read_token_word: `name=(` hands the body to
//!   parse_compound_assignment (parse.y:7104), which re-enters read_token
//!   per element under PST_COMPASSIGN.
//! - parse.y:5466 + syntax.h:90-92 (PATTERN_CHAR `@ * + ? !`): under the
//!   live `extended_glob` the pattern operator plus `(` is consumed as
//!   ONE word via parse_matched_pair — the group interior (spaces
//!   included, parse.y:5468-5475 strcpy) is word data, so GNU's
//!   expand_words_no_vars (arrayfunc.c:610) never field-splits it.
//! - An ESCAPED operator (`\@(`), a quoted one (`'@'(`), or a `(` after
//!   any other character still ends the word and the compound loop's
//!   yyerror names `(' (parse.y:7140+ loop accepts only WORD /
//!   ASSIGNMENT_WORD).
//!
//! Rust owners (one invariant per layer, no symptom guards):
//! 1. Parse admission — find_unquoted_ctrl_op (parser/token_actions.rs)
//!    consults the token's extglob gate (stamped at read time in
//!    finish_word_token, mirroring parse.y:5466's live check) and skips
//!    the balanced group after an unescaped pattern operator.
//! 2. Element integrity — the three compound-body splitters
//!    (split_compound_assignment_words, split_compound_element_words,
//!    StorageWordIter) consume the group as one element; whitespace
//!    inside the group never splits.
//! 3. Storage — token_has_unquoted_whitespace (executor/arrays.rs)
//!    treats the group interior as data, so the element is stored whole.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script files under
//! target/issue-suites/results/wt37-gapfix1/issue389-matrix/ (2026-10-02).

use std::process::Command;

fn rubash(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (output.stdout, output.stderr, output.status.code())
}

#[test]
fn extglob_on_compound_elements_parse_and_store() {
    // The #389 reproducer: patterns parse; unmatched patterns stay
    // literal; quoted/brace elements keep their shape.
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\n\
         arr=(@(foo|bar) [name]=+(test|bench))\n\
         declare -p arr\n\
         arr2=(\"{a,b}\" '@(x|y)' {c,d} @(one|two))\n\
         declare -p arr2\n",
    );
    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "declare -a arr=([0]=\"+(test|bench)\")\n\
         declare -a arr2=([0]=\"{a,b}\" [1]=\"@(x|y)\" [2]=\"c\" [3]=\"d\" [4]=\"@(one|two)\")\n"
    );
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
}

#[test]
fn spaces_inside_extglob_group_never_split_the_element() {
    // parse.y:5468-5475 strcpy's the matched pair into the word: the
    // group interior is data, never a field boundary.
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\n\
         arr=(@(a | b))\n\
         declare -p arr\n\
         arr2=(@(x|y) z)\n\
         declare -p arr2\n\
         arr3=(pre@(a|b)post tail)\n\
         declare -p arr3\n",
    );
    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "declare -a arr=([0]=\"@(a | b)\")\n\
         declare -a arr2=([0]=\"@(x|y)\" [1]=\"z\")\n\
         declare -a arr3=([0]=\"pre@(a|b)post\" [1]=\"tail\")\n"
    );
}

#[test]
fn matched_extglob_element_expands_to_files() {
    // With a file present the pattern fans out to matches (GNU c07/c09).
    let dir = std::env::temp_dir().join("rubash-issue389-glob");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bar"), b"").unwrap();
    let script = format!(
        "shopt -s extglob\ncd {}\narr=(@(foo|bar))\ndeclare -p arr\n",
        dir.to_string_lossy().replace('\\', "/")
    );
    let (stdout, stderr, code) = rubash(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "declare -a arr=([0]=\"bar\")\n"
    );
}

#[test]
fn escaped_or_non_pattern_paren_still_rejects() {
    // `\@(` (LEX_PASSNEXT carried), `r(y)` (prev char is not a pattern
    // operator) and `'@'(` (quote-close) all still hit the compound
    // loop's yyerror naming `(' — GNU-verified shapes.
    for script in [
        "shopt -s extglob\na4=(\\@(x))\ndeclare -p a4\n",
        "shopt -s extglob\na5=(q@(x) r(y))\ndeclare -p a5\n",
        "shopt -s extglob\na2=('@'(x))\ndeclare -p a2\n",
    ] {
        let (stdout, stderr, code) = rubash(script);
        assert_eq!(code, Some(1), "{script}: {code:?}");
        assert!(stdout.is_empty(), "{script}: {stdout:?}");
        let stderr = String::from_utf8_lossy(&stderr);
        assert!(
            stderr.contains("syntax error near unexpected token `('"),
            "{script}: {stderr}"
        );
    }
}

#[test]
fn gate_off_and_mid_script_shopt_u_keep_rejecting() {
    // OFF by default: byte-identical rejection (the #386 family invariant).
    let (stdout, stderr, code) = rubash("arr=(@(foo|bar))\ndeclare -p arr\n");
    assert_eq!(code, Some(1));
    assert!(stdout.is_empty());
    assert!(String::from_utf8_lossy(&stderr).contains("syntax error near unexpected token `('"));

    // ON then OFF mid-script: the earlier line still stores (read-time
    // gate, parse.y:5466), the later line rejects.
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\narr=(@(foo|bar))\ndeclare -p arr\nshopt -u extglob\narr2=(@(foo|bar))\n",
    );
    assert_eq!(code, Some(1), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "declare -a arr=([0]=\"@(foo|bar)\")\n"
    );
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "{stderr}"
    );
}

#[test]
fn multiline_compound_body_and_nested_groups() {
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\n\
         arr=(\n@(foo|bar)\n[name]=+(t|b)\n)\n\
         declare -p arr\n\
         a=(@(foo|@(bar|baz)) tail)\n\
         declare -p a\n\
         b=(x@(a|b)y z@(c|d)w)\n\
         declare -p b\n",
    );
    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "declare -a arr=([0]=\"+(t|b)\")\n\
         declare -a a=([0]=\"@(foo|@(bar|baz))\" [1]=\"tail\")\n\
         declare -a b=([0]=\"x@(a|b)y\" [1]=\"z@(c|d)w\")\n"
    );
}

#[test]
fn subscripted_and_assoc_elements_with_patterns() {
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\n\
         a=([k]=@(x|y))\n\
         declare -p a\n\
         declare -A b=([k]=@(x|y))\n\
         declare -p b\n",
    );
    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&stderr));
    let stdout = String::from_utf8_lossy(&stdout);
    assert!(stdout.contains("declare -a a=([0]=\"@(x|y)\")"), "{stdout}");
    assert!(stdout.contains("declare -A b=([k]=\"@(x|y)\""), "{stdout}");
}
