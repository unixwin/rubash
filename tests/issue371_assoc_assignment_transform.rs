//! rubash#371: `${assoc[*]@A}` must print the whole `declare -A assoc=(...)`
//! assignment as GNU bash 5.3 does.
//!
//! GNU owners (third_party/bash @b4608166, Bash-5.3 patch 15):
//! - subst.c:8923 parameter_brace_transform dispatches VT_ARRAYVAR to
//!   array_transform (subst.c:8835), whose 'A' arm calls
//!   array_var_assignment (subst.c:8680): assoc_to_assign (assoc.c:414) /
//!   array_to_assign (array.c:948) render the `=(...)` body unless the
//!   variable is invisible/unset.
//! - chk_atstar (subst.c:7642-7649): a QUOTED `[*]` subscript never marks
//!   the word for the final list_string split (subst.c:12134) — one word.
//!   A quoted `[@]` (subst.c:7630-7641) does, and the plain
//!   `declare -<flags> name` prefix breaks on IFS while the value body —
//!   CTLESC-carried by quote_string inside array_var_assignment
//!   (subst.c:8703) — survives as one field.
//! - setifs (subst.c:12322): IFS unset defaults to " \t\n"; a set-but-null
//!   IFS hits `*ifs_chars ? ifs_chars : " "` (subst.c:12134) — a space.
//! - assign_array_element (arrayfunc.c:815) allocates a real array cell,
//!   so an element assignment after `declare -A` makes the variable set
//!   and the body render.
//!
//! Every expectation below was verified byte-for-byte against WSL GNU Bash
//! 5.3.0 from a script file (target/issue371-matrix.sh and
//! target/issue371-ifs*.sh in the lane worktree).

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::fs;

fn run_capture(input: &str) -> String {
    let output_path = "target/rubash-issue371-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("{input} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    fs::read_to_string(output_path).unwrap()
}

#[test]
fn assoc_star_at_a_quoted_prints_full_assignment_body() {
    // The exact rubash#371 reproducer: the body must survive and the
    // quoted result must be ONE word.
    assert_eq!(
        run_capture(
            "declare -A assoc; assoc[one]=alpha; assoc[two]=beta; \
             printf '<%s>\\n' \"${assoc[*]@A}\""
        ),
        "<declare -A assoc=([two]=\"beta\" [one]=\"alpha\" )>\n"
    );
}

#[test]
fn indexed_star_at_a_quoted_is_one_word() {
    assert_eq!(
        run_capture("arr=(alpha beta); printf '<%s>\\n' \"${arr[*]@A}\""),
        "<declare -a arr=([0]=\"alpha\" [1]=\"beta\")>\n"
    );
}

#[test]
fn element_assigned_indexed_array_keeps_at_a_body() {
    assert_eq!(
        run_capture("declare -a ei; ei[0]=z; ei[3]=w; printf '<%s>\\n' \"${ei[*]@A}\""),
        "<declare -a ei=([0]=\"z\" [3]=\"w\")>\n"
    );
}

#[test]
fn at_sign_at_a_quoted_splits_prefix_on_ifs_with_body_intact() {
    // GNU: quoted ${arr[@]@A} behaves like "$@" for the SPLIT decision
    // only — `declare`, `-a`, and `arr=(...)` become three words under the
    // default IFS, and the value body stays inside the last word.
    assert_eq!(
        run_capture("arr=(alpha beta); printf '<%s>\\n' \"${arr[@]@A}\""),
        "<declare>\n<-a>\n<arr=([0]=\"alpha\" [1]=\"beta\")>\n"
    );
}

#[test]
fn at_sign_at_a_quoted_with_colon_ifs_does_not_split() {
    // list_string (subst.c:12134) splits on the IFS characters: a colon
    // IFS finds no separator in the transform result, so it is one word.
    assert_eq!(
        run_capture("arr=(alpha beta); IFS=:; printf '<%s>\\n' \"${arr[@]@A}\""),
        "<declare -a arr=([0]=\"alpha\" [1]=\"beta\")>\n"
    );
}

#[test]
fn star_at_a_quoted_with_colon_ifs_is_one_word() {
    assert_eq!(
        run_capture("declare -A a=( [k]=v ); IFS=:; printf '<%s>\\n' \"${a[*]@A}\""),
        "<declare -A a=([k]=\"v\" )>\n"
    );
}

#[test]
fn at_sign_at_a_quoted_with_null_ifs_splits_on_space() {
    // subst.c:12134 `*ifs_chars ? ifs_chars : " "`: a null IFS still splits
    // the prefix on a literal space (GNU 5.3 probe target/issue371-ifs3.sh).
    assert_eq!(
        run_capture("arr=(alpha beta); IFS=''; printf '<%s>\\n' \"${arr[@]@A}\""),
        "<declare>\n<-a>\n<arr=([0]=\"alpha\" [1]=\"beta\")>\n"
    );
}

#[test]
fn assoc_member_and_bare_at_a_forms_stay_one_word() {
    // VT_ARRAYMEMBER / VT_VARIABLE go through string_var_assignment
    // (subst.c:8645); chk_atstar never applies, so no split.
    assert_eq!(
        run_capture(
            "declare -A assoc=( [one]=alpha ); \
             printf '<%s>\\n' \"${assoc[one]@A}\" \"${assoc@A}\" \"${assoc[*]@a}\""
        ),
        "<declare -A assoc='alpha'>\n<declare -A assoc>\n<A>\n"
    );
}

#[test]
fn scalar_at_a_at_a_at_q_quoted_unchanged() {
    // Guard for the already-correct scalar forms sharing the transform
    // path (must not regress from the array-split fix).
    assert_eq!(
        run_capture("v='two words'; printf '<%s>\\n' \"${v@A}\" \"${v@a}\" \"${v@Q}\""),
        "<v='two words'>\n<>\n<\'two words\'>\n"
    );
}
