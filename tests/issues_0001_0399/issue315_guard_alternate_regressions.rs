//! Issue rubash#315 regressions (wt13/sweepfix lane).
//!
//! Two divergences in the `${arr[@]+"…"}`-guarded expansion family found by
//! the ecosweep2 lane running liquidprompt's own shunit2 suite
//! (tests/test_array.sh, 2 of 3 tests):
//!
//! 1) `${a[@]+"${a[2]+x}"}` reported SET for an UNSET element: the [[ ]]
//! operand context kept the argv splitter's QUOTED_NULL_MARKER carrier
//! (U+E002, the CTLNUL port) as visible value bytes, so `[[ -n … ]]` was
//! true. GNU cond.c cond_expand_word hands the operand to the test with
//! quote removal complete — a quoted-empty alternate is the empty string
//! (remove_quoted_nulls at dequote, subst.c:4795+).
//! 2) `c=( ${b[@]+"${b[@]:0:2}"} )` with b=("" foobar) stored ONE element:
//! the unquoted guard collapsed the quoted slice to a joined string and
//! dropped the empty quoted element. GNU parameter_brace_expand case '+'
//! (subst.c:10348) expands the alternate via expand_string_for_rhs
//! (subst.c:7993); a word-list rhs (`l->next`, subst.c:8023-8027) sets
//! *qdollaratp, so the alternate keeps one field per element — in argv,
//! for-lists AND compound array assignments (arrayfunc.c:557
//! expand_compound_array_assignment -> expand_words_no_vars per word),
//! and a quoted-null rhs stores ONE empty element (subst.c:8036-8044,
//! W_HASQUOTEDNULL).
//!
//! Root causes fixed:
//! - executor/conditional.rs: every [[ ]] operand expansion routes through
//!   expand_conditional_word, which strips the QUOTED_NULL_MARKER carrier.
//! - executor/command_prepare.rs: the multi-word alternate predicate knew
//!   only the exact `[@]}` tail; alternate_contains_braced_at_list_reference
//!   now claims `[@]` followed by `}`/`:`/`#`/`%`/`/`/`^`/`,` (slice and
//!   operator forms) so they take the per-element re-parse path.
//! - executor/command_words.rs: the for-list expander routes whole-word
//!   braced alternates through braced_alternate_word_values like command
//!   words do.
//! - executor/assignment_expansion.rs:
//!   braced_alternate_compound_element_fields claims whole-token
//!   `${var+alt}` elements whose alternate is a quoted at-list/slice/
//!   quoted-null and fans them out per rhs word.
//!
//! Every expectation is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-29; matrices under
//! target/sweepfix/).

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i315-{}-{}",
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

/// The original report 1: is-element-set idiom behind an array guard.
#[test]
fn guard_inner_unset_element_reports_empty() {
    let script = concat!(
        "typeset -a a\n",
        "a[1]=1\n",
        "[[ -n ${a[@]+\"${a[2]+x}\"} ]] && echo NONEMPTY || echo EMPTY\n",
        "[[ -n ${a[@]+\"${a[2]}\"} ]] && echo NONEMPTY || echo EMPTY\n",
        "[[ ${a[@]+\"${a[2]+x}\"} == \"\" ]] && echo EQ || echo NE\n",
        "set -- ${a[@]+\"${a[2]+x}\"}\n",
        "echo argc=$#\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "EMPTY\nEMPTY\nEQ\nargc=1\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// The original report 2: quoted slice inside an unquoted array guard keeps
/// one element per member — the empty quoted element included.
#[test]
fn guard_quoted_slice_keeps_empty_element() {
    let script = concat!(
        "b=( \"\" foobar )\n",
        "c=( ${b[@]+\"${b[@]:0:2}\"} )\n",
        "echo \"count=${#c[@]}\"\n",
        "IFS=/\n",
        "echo \"join=[${c[*]}]\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "count=2\njoin=[/foobar]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Quoted-null alternates store ONE empty element; the self-referential
/// guard idiom and the joined `[*]` forms stay intact.
#[test]
fn guard_quoted_null_and_list_forms() {
    let script = concat!(
        "b=( \"\" foobar \"x y\" )\n",
        "a=(x0)\n",
        "typeset -a t\n",
        "t[1]=1\n",
        "h=( ${b[@]+\"\"} )\n",
        "echo \"empty-alt count=${#h[@]}\"\n",
        "i=( ${b[@]+\"${b[@]}\"} )\n",
        "echo \"whole count=${#i[@]}\"\n",
        "j=( ${t[@]+\"${t[2]+x}\"} )\n",
        "echo \"nested-guard count=${#j[@]}\"\n",
        "k=( ${b[@]+\"${b[*]}\"} )\n",
        "echo \"star count=${#k[@]} val=[${k[0]}]\"\n",
        "m=( ${b[@]+\"x y\"} )\n",
        "echo \"literal count=${#m[@]} val=[${m[0]}]\"\n",
        "n=( ${a[@]-\"${b[@]}\"} )\n",
        "echo \"minus count=${#n[@]}\"\n",
        "u=()\n",
        "o=( ${u[@]+\"${b[@]}\"} )\n",
        "echo \"unset-guard count=${#o[@]}\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    let expected = concat!(
        "empty-alt count=1\n",
        "whole count=3\n",
        "nested-guard count=1\n",
        "star count=1 val=[ foobar x y]\n",
        "literal count=1 val=[x y]\n",
        "minus count=1\n",
        "unset-guard count=0\n",
    );
    assert_eq!(stdout, expected);
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// The guard class fans out per element in argv and for-list contexts too.
#[test]
fn guard_slice_fans_out_in_argv_and_for() {
    let script = concat!(
        "b=( \"\" foobar )\n",
        "set -- ${b[@]+\"${b[@]:0:2}\"}\n",
        "echo \"argc=$#\"\n",
        "printf '[%s]' ${b[@]+\"${b[@]:0:2}\"}\n",
        "echo\n",
        "for x in ${b[@]+\"${b[@]:1:2}\"}; do echo \"iter=[$x]\"; done\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argc=2\n[][foobar]\niter=[foobar]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Assignment-RHS context keeps its single-string shape (the leading
/// empty element joined with its separator).
#[test]
fn guard_slice_assignment_context_unchanged() {
    let script = concat!(
        "b=( \"\" foobar )\n",
        "g=${b[@]+\"${b[@]:0:2}\"}\n",
        "echo \"assign=[${g}]\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "assign=[ foobar]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
