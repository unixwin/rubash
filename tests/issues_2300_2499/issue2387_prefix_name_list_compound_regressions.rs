//! rubash#2387 — a whole-word `"${!PREFIX@}"` in ARRAY-ASSIGNMENT
//! position collapsed to ONE element holding every matching name
//! joined by a space. ble.sh's teardown captures decoder keymap
//! names exactly this way (`names=("${!_ble_decode_@}")`) and later
//! runs `unset "${names[@]}"`: the single joined word reaches unset
//! as one invalid identifier (`unset: '_ble_decode_..._kmap_...
//! __67108984': not a valid identifier`) and every variable stays
//! set. GNU emits no such diagnostic — verified 2026-11-01 that
//! `unset` itself behaves byte-identically in both shells when
//! handed the joined word directly, so the divergence is born at
//! capture time, in the expansion, not in unset.
//!
//! GNU ground (vendored bash 5.3 tree):
//! - subst.c:9978-10007 parameter_brace_expand — the `${!prefix@}`
//!   arm builds the name list with all_variables_matching_prefix
//!   and returns it via string_list_dollar_at (W_DOLLARAT): one
//!   word per name when quoted, exactly like `"${arr[@]}"`.
//! - variables.c:4250-4262 all_variables_matching_prefix -> the
//!   list is strcmp-sorted (sort_variables).
//! - arrayfunc.c:557 expand_compound_array_assignment — each word
//!   of a compound assignment expands independently, so the list
//!   fans out into that many elements; an empty match contributes
//!   ZERO elements (the `"$@"` rule), never one empty element.
//! - The `*` form is string_list_dollar_star: a single word joined
//!   with IFS[0] in every context, assignment included.
//!
//! All expectations byte-verified against GNU bash 5.3.20
//! (2026-11-01 probes a/c/e/f against master).

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(script: &str) -> Run {
    let out = Command::new(RUBASH)
        .args(["-c", script])
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

// Defined out of alphabetical order on purpose: the fan-out must
// come back strcmp-sorted (zz2387_a, zz2387_b, zz2387_c).
const DEFS: &str = "zz2387_c=1; zz2387_a=1; zz2387_b=1; ";

#[test]
fn issue2387_quoted_prefix_at_in_array_assignment_fans_out() {
    let r = run(&format!(
        "{DEFS} x=(\"${{!zz2387_@}}\"); printf 'n=%s list=[%s]\\n' \"${{#x[@]}}\" \"${{x[*]}}\""
    ));
    assert_eq!(r.stdout, "n=3 list=[zz2387_a zz2387_b zz2387_c]\n");
}

#[test]
fn issue2387_mixed_compound_assignment_fans_out_in_place() {
    let r = run(&format!(
        "{DEFS} x=(pre \"${{!zz2387_@}}\" post); printf 'n=%s list=[%s]\\n' \"${{#x[@]}}\" \"${{x[*]}}\""
    ));
    assert_eq!(r.stdout, "n=5 list=[pre zz2387_a zz2387_b zz2387_c post]\n");
}

#[test]
fn issue2387_empty_prefix_match_contributes_zero_elements() {
    let r = run(&format!(
        "{DEFS} y=(\"${{!zznope2387_@}}\"); printf 'empty n=%s\\n' \"${{#y[@]}}\"; \
         z=(\"${{!zz2387_@}}\" \"${{!zz2387_@}}\"); printf 'twice n=%s\\n' \"${{#z[@]}}\""
    ));
    assert_eq!(r.stdout, "empty n=0\ntwice n=6\n");
}

#[test]
fn issue2387_star_form_stays_one_joined_element() {
    let r = run(&format!(
        "{DEFS} x=(\"${{!zz2387_*}}\"); printf 'n=%s list=[%s]\\n' \"${{#x[@]}}\" \"${{x[*]}}\""
    ));
    assert_eq!(r.stdout, "n=1 list=[zz2387_a zz2387_b zz2387_c]\n");
}

#[test]
fn issue2387_scalar_context_still_joins() {
    // Not an assignment-list context: the scalar expansion keeps
    // string_list_dollar_at's unquoted/scalar join (IFS[0]).
    let r = run(&format!(
        "{DEFS} s=\"${{!zz2387_@}}\"; printf '[%s]\\n' \"$s\""
    ));
    assert_eq!(r.stdout, "[zz2387_a zz2387_b zz2387_c]\n");
}

#[test]
fn issue2387_local_and_declare_compound_forms_fan_out() {
    let r = run(&format!(
        "{DEFS} f() {{ local -a ns=(\"${{!zz2387_@}}\"); printf 'local n=%s\\n' \"${{#ns[@]}}\"; }}; \
         f; declare -a dd=(\"${{!zz2387_@}}\"); printf 'declare n=%s\\n' \"${{#dd[@]}}\""
    ));
    assert_eq!(r.stdout, "local n=3\ndeclare n=3\n");
}

#[test]
fn issue2387_ble_teardown_shape_unsets_without_diagnostic() {
    // The ble.sh shape end to end: capture the names into an array,
    // then unset them through it. GNU: every variable is gone and
    // stderr stays empty.
    let r = run(&format!(
        "{DEFS} names=(\"${{!zz2387_@}}\"); unset \"${{names[@]}}\"; \
         printf 'a=%s b=%s c=%s\\n' \"${{zz2387_a-GONE}}\" \"${{zz2387_b-GONE}}\" \"${{zz2387_c-GONE}}\""
    ));
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout, "a=GONE b=GONE c=GONE\n");
    assert!(
        !r.stderr.contains("not a valid identifier"),
        "teardown diagnostic leaked: {:?}",
        r.stderr
    );
}
