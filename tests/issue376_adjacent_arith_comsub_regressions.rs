//! rubash#376 adjacent `$((...))` span-slicing regressions (wt24/adjcomsub).
//!
//! A word containing two or more `$((...))` separated only by non-alnum
//! material (`:`, space, `.`, `-`, `+`, `/`, `=`, or nothing) was merged
//! into ONE expression: the whole-word fast paths admitted on
//! `strip_prefix("$((") + strip_suffix("))")` — pairing the FIRST `$((`
//! with the LAST `))` of the word — so `echo "$((1+1)):$((2+2))"` evaluated
//! the merged `1+1)):4` (arithmetic syntax error, no output) instead of
//! printing `2:4`. Alnum-adjacent material or a `$param` on either side
//! avoided the fast path and worked, which localized the bug to the
//! whole-word admission.
//!
//! GNU spec (third_party/bash/): parse.y:5493-5523 read_token_word() hands
//! EVERY `$(`-after-`$` to parse.y:4451 parse_comsub(), which on the `(`
//! peek delegates to parse.y:4470 parse_matched_pair(P_ARITH);
//! parse.y:3877 parse_matched_pair() closes the span at the FIRST `))`
//! that returns the paren depth (seeded by the `$(` plus the second `(`)
//! to zero, with quotes and backslash escapes opaque (parse.y:3999-4000,
//! parse.y:4041-4046); the word walk then CONTINUES (parse.y:5521 `goto
//! next_character`), so later `$((`/`$param` units are independent
//! expansion events. GNU has no whole-word `))`-suffix rule.
//!
//! Fix (same invariant, whole class): `whole_word_arithmetic_substitution_body`
//! (src/executor/parameter_ops.rs) — the real span scanner as the
//! whole-word admission — wired into the three fast paths:
//! - src/executor/parameter_core.rs (expand_word_mut_with_context)
//! - src/executor/expand_word.rs (expand_substitution_word)
//! - src/executor/command_prepare.rs (quoted `"$(("` pre-scan)
//!
//! Everything below verified byte-identical against WSL GNU Bash 5.3.0
//! script-file runs (artifacts under target/issue376/).

use std::process::Command;

/// Run a script FILE in its own scratch directory; (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i376-{}-{}",
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

/// The #376 reproducer: quoted colon-separated pair.
#[test]
fn quoted_two_arith_comsubs_separated_by_colon() {
    let (stdout, stderr, code) = rubash_file("echo \"$((1+1)):$((2+2))\"\n");
    assert_eq!(stdout, "2:4\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Unquoted form of the same word.
#[test]
fn unquoted_two_arith_comsubs_separated_by_colon() {
    let (stdout, stderr, code) = rubash_file("echo $((1+1)):$((2+2))\n");
    assert_eq!(stdout, "2:4\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The whole separator class from the issue matrix: `:`, space, `.`, `-`,
/// `+`, `/`, `=`, and direct adjacency.
#[test]
fn separator_class_between_two_arith_comsubs() {
    let (stdout, stderr, code) = rubash_file(
        "echo \"$((1+1)) $((2+2))\"\necho \"$((1+1)).$((2+2))\"\necho \"$((1+1))-$((2+2))\"\necho \"$((1+1))+$((2+2))\"\necho \"$((1+1))/$((2+2))\"\necho \"$((1+1))=$((2+2))\"\necho \"$((1+1))$((2+2))\"\n",
    );
    assert_eq!(stdout, "2 4\n2.4\n2-4\n2+4\n2/4\n2=4\n24\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Three `$((...))` in one word (triple colon chain).
#[test]
fn triple_arith_comsub_chain() {
    let (stdout, stderr, code) = rubash_file("echo \"$((1+1)):$((2+2)):$((3+3))\"\n");
    assert_eq!(stdout, "2:4:6\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Variable operands in both comsubs.
#[test]
fn variable_operands_in_both_comsubs() {
    let (stdout, stderr, code) = rubash_file("v=9\necho \"$((v)):$((v))\"\n");
    assert_eq!(stdout, "9:9\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// `$param` on either side of an arith comsub still expands (guard the
/// neighboring fast-path admissions).
#[test]
fn param_and_arith_comsub_mix() {
    let (stdout, stderr, code) =
        rubash_file("H=ab\necho \"$((1+1)):$H\"\nset -- x\necho \"${1}:$((1+1))\"\n");
    assert_eq!(stdout, "2:ab\nx:2\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Alnum-adjacent material keeps working (the previously-correct route).
#[test]
fn alnum_adjacent_material_still_works() {
    let (stdout, stderr, code) = rubash_file("echo \"a$((1+1))b$((2+2))c\"\n");
    assert_eq!(stdout, "a2b4c\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Balanced parens inside the first body: the depth counter must keep the
/// span open past the inner `))`-lookalike and still decline the merged
/// read (the closer is not the word's end).
#[test]
fn balanced_inner_parens_keep_first_span_independent() {
    let (stdout, stderr, code) = rubash_file("echo \"$(( (1+2) )):$((3+4))\"\n");
    assert_eq!(stdout, "3:7\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Assignment RHS position (walker route, must not regress).
#[test]
fn assignment_rhs_two_arith_comsubs() {
    let (stdout, stderr, code) = rubash_file("v=$((1+1)):$((2+2))\necho \"$v\"\n");
    assert_eq!(stdout, "2:4\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// `case` pattern, `[[ ]]`, heredoc body, substring offset, array
/// subscript: the class boundary positions from the issue.
#[test]
fn class_boundary_positions() {
    let (stdout, stderr, code) = rubash_file(
        "case 2:4 in \"$(echo 2):4\") echo case-ok;; *) echo case-miss;; esac\nif [[ \"$((1+1)):$((2+2))\" == \"2:4\" ]]; then echo cond-ok; fi\ncat <<EOF\n$((1+1)):$((2+2))\nEOF\nb=0123456789\necho \"${b:$((1+1)):$((2+2))}\"\na=(); a[$((1+1))+$((2+2))]=x\necho \"idx=${!a[@]}\"\n",
    );
    assert_eq!(stdout, "case-ok\ncond-ok\n2:4\n2345\nidx=6\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// `$((` adjacent to a `$( ` command substitution and a quoted parameter
/// expansion in the same word.
#[test]
fn arith_comsub_adjacent_to_comsub_and_quoted_param() {
    let (stdout, stderr, code) = rubash_file(
        "echo \"$(echo hi)$((1+1))\"\necho \"$(echo $((1+1))):x\"\necho \"$((1+1)) \"${x:=ok}\" $((2+2))\"\necho \"x=$x\"\n",
    );
    assert_eq!(stdout, "hi2\n2:x\n2 ok 4\nx=ok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Single-comsub words must STAY on the fast path (guard the admission's
/// true-positive side, including quoted whole-word and nested parens).
#[test]
fn single_arith_comsub_words_stay_on_fast_path() {
    let (stdout, stderr, code) = rubash_file(
        "echo \"$((1+1))\"\necho $((1+1))\necho \"$(( (1+2) ))\"\necho $((1+1))x\necho \"head:$((1+1))\"\necho \"$((1+1)):tail\"\n",
    );
    assert_eq!(stdout, "2\n2\n3\n2x\nhead:2\n2:tail\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// An arithmetic failure inside a genuine single-comsub word still reports
/// exactly once (the fast path's error contract, kept; GNU-verified
/// 2026-09-28: diagnostic then the script continues, exit 0).
#[test]
fn single_comsub_arithmetic_error_reports_once() {
    let (stdout, stderr, code) = rubash_file("echo \"$((7<=))\"\necho after\n");
    assert_eq!(stdout, "after\n");
    assert_eq!(
        stderr,
        "case.sh: line 1: 7<=: arithmetic syntax error: operand expected (error token is \"<=\")\n"
    );
    assert_eq!(code, Some(0));
}

/// `for`-body and `printf` argument positions (both hit the quoted
/// pre-scan in command_prepare).
#[test]
fn for_body_and_printf_positions() {
    let (stdout, stderr, code) = rubash_file(
        "for i in 1; do echo \"$((1+1)):$((2+2))\"; done\nprintf '%s\\n' \"$((1+1)):$((2+2))\"\n",
    );
    assert_eq!(stdout, "2:4\n2:4\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
