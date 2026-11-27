//! Issue rubash#298 and rubash#298 regression: field splitting of unquoted
//! parameter expansions in compound array assignments (#298) and dynamic
//! variables (RANDOM) in array-subscript arithmetic (#299).
//!
//! #298: oh-my-bash half-life theme / lib/utils.sh `_omb_util_split` relies
//! on `set -- a.b.c; IFS=.; A=($1)` storing 3 elements. GNU
//! arrayfunc.c:557 expand_compound_array_assignment ->
//! parse_string_to_word_list (585) hands every compound word to
//! expand_words_no_vars (610) -> subst.c:12590 expand_word_list_internal:
//! "Words with the W_QUOTED or W_NOSPLIT bits set, or for which no
//! expansion is done, do not undergo word splitting" — every other word is
//! field-split on the CURRENT IFS, including non-whitespace IFS
//! characters. Rubash's positional-parameter fast path stored the expanded
//! `$N` (and the unquoted `$@`/`${@}` join) as ONE element.
//!
//! #299: the random theme indexes with `${a[RANDOM%83]}`. GNU
//! arrayfunc.c:1370 array_expand_index -> evalexp -> expr.c:1183
//! expr_streval resolves operand names through find_variable, which sees
//! dynamic variables — RANDOM via variables.c:1432 get_random (registered
//! at variables.c:1857 INIT_DYNAMIC_VAR), drawing from the same RNG state
//! as `$((RANDOM))`. Rubash's `&self` subscript evaluators passed no RNG
//! state, so RANDOM resolved empty and the whole expression failed with
//! "arithmetic syntax error: operand expected".
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28). The seeded RANDOM
//! cases pin the exact Park-Miller draw sequence (GNU lib/sh/random.c:57
//! intrand32), so they double as an RNG-compatibility check.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

// ---------------------------------------------------------------------------
// rubash#298: unquoted positional parameters field-split on the current IFS
// ---------------------------------------------------------------------------

/// The reporter's exact shape: bare unquoted `$1` splits on a
/// non-whitespace IFS (`set -- a.b.c; IFS=.; A=($1)` -> 3 elements).
#[test]
fn bare_positional_splits_on_non_whitespace_ifs() {
    let (stdout, stderr, _) = rubash(
        "set -- a.b.c; IFS=.; A=($1); printf '%s:%s|' \"${#A[@]}\" \"${A[0]}|${A[1]}|${A[2]}\"; echo",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "3:a|b|c|\n");
}

/// Two unquoted `$1` elements each split; adjacent IFS delimiters keep the
/// empty middle field (GNU field splitting, subst.c split-on-IFS rules).
#[test]
fn repeated_positionals_and_adjacent_delimiters() {
    let (stdout, stderr, _) =
        rubash("set -- a.b.c; IFS=.; A=($1 $1); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "6:a.b.c.a.b.c\n");

    let (stdout, stderr, _) =
        rubash("set -- a.b..b; IFS=.; A=($1); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "4:a.b..b\n");
}

/// Unquoted `$@` / `${@}` join with the first IFS character and the join
/// is field-split (subst.c:2957 string_list_dollar_at, quoted == 0);
/// the QUOTED spellings keep one element per parameter.
#[test]
fn unquoted_at_word_join_and_split_shapes() {
    // Unquoted: `set -- "a.b" c` joins to `a.b.c`, splits to [a][b][c].
    let (stdout, stderr, _) =
        rubash("set -- 'a.b' c; IFS=.; A=($@); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "3:a.b.c\n");

    // Same through the braced spelling.
    let (stdout, stderr, _) =
        rubash("set -- 'a.b' c; IFS=.; A=(${@}); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "3:a.b.c\n");

    // Quoted "$@" / "${@}": W_DOLLARAT keeps one word per parameter —
    // the dotted parameter stays a single element.
    let (stdout, stderr, _) =
        rubash("set -- 'a.b' c; IFS=.; A=(\"$@\"); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "2:a.b.c\n");

    let (stdout, stderr, _) = rubash(
        "set -- 'a.b' c; IFS=.; A=(\"${@}\"); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "2:a.b.c\n");
}

/// An unquoted expansion that produces nothing contributes no field
/// (subst.c:13219 expand_word_list_internal).
#[test]
fn empty_and_unset_positional_contribute_no_field() {
    let (stdout, stderr, code) = rubash("set -- ''; IFS=.; A=($1); echo ${#A[@]}");
    assert_clean(&stderr);
    assert_eq!(stdout, "0\n");
    assert_eq!(code, Some(0));

    let (stdout, stderr, _) = rubash("set --; IFS=.; A=($1); echo ${#A[@]}");
    assert_clean(&stderr);
    assert_eq!(stdout, "0\n");
}

/// Dynamic variables take the same treatment: `$EPOCHREALTIME` contains a
/// `.` and splits into two fields under IFS=.
#[test]
fn dynamic_epochrealtime_splits_on_dot_ifs() {
    let (stdout, stderr, _) = rubash("IFS=.; A=($EPOCHREALTIME); printf 'n=%s' \"${#A[@]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "n=2\n");
}

/// A `[N]=$1` element word is W_ASSIGNMENT|W_NOSPLIT (parse.y:5786-5796):
/// the whole expanded value lands at the subscript, unsplit.
#[test]
fn subscripted_positional_element_stays_unsplit() {
    let (stdout, stderr, _) =
        rubash("set -- a.b.c; IFS=.; A=([2]=$1); printf '%s:%s' \"${#A[@]}\" \"${A[2]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "1:a.b.c\n");
}

/// Regression guards for shapes that were already correct: the braced
/// `${1}` and mixed-word `x$1y` spellings go through the general splitter.
#[test]
fn braced_and_mixed_word_forms_still_split() {
    let (stdout, stderr, _) =
        rubash("set -- a.b.c; IFS=.; A=(${1}); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "3:a.b.c\n");

    let (stdout, stderr, _) =
        rubash("set -- a.b.c; IFS=.; A=(x$1y); printf '%s:%s' \"${#A[@]}\" \"${A[*]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "3:xa.b.cy\n");
}

// ---------------------------------------------------------------------------
// rubash#299: dynamic variables (RANDOM) in array-subscript arithmetic
// ---------------------------------------------------------------------------

/// Seeded RANDOM draws in `${a[RANDOM%3]}` follow GNU's exact Park-Miller
/// sequence: `RANDOM=1` then two reads pick `y` then `x` (byte-verified
/// against GNU Bash 5.3.0). Also proves two reads in one word each draw
/// (the RNG state advances per read).
#[test]
fn seeded_random_subscript_draws_match_gnu() {
    let (stdout, stderr, _) =
        rubash("a=(x y z); RANDOM=1; printf '%s %s\n' \"${a[RANDOM%3]}\" \"${a[RANDOM%3]}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "y x\n");
}

/// The `$((...))` subscript spelling evaluates through the same
/// RNG-aware evaluator (arrayfunc.c:1353 -> evalexp).
#[test]
fn seeded_random_arith_subscript_matches_gnu() {
    let (stdout, stderr, _) = rubash("a=(x y z); RANDOM=1; printf '%s\n' \"${a[$((RANDOM%3))]}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "y\n");
}

/// The length form `${#a[RANDOM%3]}` resolves the subscript as well.
#[test]
fn seeded_random_length_subscript_matches_gnu() {
    let (stdout, stderr, _) = rubash("a=(x y z); RANDOM=1; printf 'len=%s\n' \"${#a[RANDOM%3]}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "len=1\n");
}

/// Unseeded `${a[RANDOM%3]}` yields a valid element and — the actual bug —
/// never emits `arithmetic syntax error: operand expected`.
#[test]
fn unseeded_random_subscript_resolves_without_error() {
    for _ in 0..8 {
        let (stdout, stderr, code) = rubash("a=(x y z); printf '%s' \"${a[RANDOM%3]}\"; echo");
        assert_clean(&stderr);
        assert_eq!(code, Some(0));
        assert!(
            stdout == "x\n" || stdout == "y\n" || stdout == "z\n",
            "not an array element: {stdout:?}"
        );
    }
}

/// A bare `${a[RANDOM]}` indexes past the array and yields the empty
/// element, silently (GNU: a[RANDOM] is out of range for a 3-element
/// array), while SRANDOM resolves through the same dynamic layer.
#[test]
fn bare_random_index_and_srandom_resolve() {
    let (stdout, stderr, code) = rubash("a=(x y z); printf '[%s]' \"${a[RANDOM]}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "[]\n");
    assert_eq!(code, Some(0));

    let (stdout, stderr, _) = rubash("a=(x y z); printf '%s' \"${a[SRANDOM%3]}\"; echo");
    assert_clean(&stderr);
    assert!(
        stdout == "x\n" || stdout == "y\n" || stdout == "z\n",
        "SRANDOM did not resolve: {stdout:?}"
    );
}
