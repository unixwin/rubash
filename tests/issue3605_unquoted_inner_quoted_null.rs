//! rubash#3605 (run-dollars) — quoting bytes that live INSIDE a word's own
//! `${...}`/`$(...)` constructs mark `had_quoted_null` for the INNER
//! expansion only; the OUTER word's quoting state is what decides whether
//! an all-null expansion survives as an empty field. For an UNQUOTED
//! outer word the result is discarded.
//!
//! GNU: subst.c:12026-12048 (expand_word_internal end-of-word) — when
//! `*istring == '\0'`, the word keeps one CTLNUL field only under
//! `had_quoted_null || (quoted_dollar_at == 0 && quoted_state ==
//! PARTIALLY_QUOTED)`; UNQUOTED state or a quoted `"$@"` discards it.
//! `recho ${v=''}` binds nothing and expands to one quoted-null producer,
//! but the outer word carries no quoting of its own, so it vanishes —
//! while `recho "${v=''}"` keeps one empty argument.

use std::process::Command;

fn rubash(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Unquoted word whose expansion is only an inner quoted null drops.
#[test]
fn unquoted_brace_alternate_quoted_null_drops() {
    assert_eq!(rubash("unset v; set -- ${v=''}; echo $#"), "0\n");
    assert_eq!(rubash("unset v; set -- ${v= ''}; echo $#"), "0\n");
}

/// `${v=$*}` with a single empty positional is the same UNQUOTED discard.
#[test]
fn unquoted_brace_alternate_empty_star_drops() {
    assert_eq!(rubash("unset v; set -- ''; set -- ${v=$*}; echo $#"), "0\n");
}

/// Any quoting on the OUTER word keeps the empty field.
#[test]
fn quoted_or_partial_outer_word_keeps_empty_field() {
    assert_eq!(rubash("unset v; set -- \"${v=''}\"; echo $#"), "1\n");
    assert_eq!(rubash("unset v; set -- ''${v=''}; echo $#"), "1\n");
    assert_eq!(rubash("unset v; set -- \"${v= ''}\"; echo $#"), "1\n");
}

/// `[[` operands keep their own word semantics: the operand must not
/// vanish under the unquoted rule.
#[test]
fn conditional_operand_inner_quoted_null_survives() {
    assert_eq!(
        rubash("unset v; if [[ ${v=''} = \"\" ]]; then echo yes; else echo no; fi"),
        "yes\n"
    );
}

/// Stripping inner null markers must not eat real text around them.
#[test]
fn inner_null_marker_leaves_literal_text() {
    assert_eq!(rubash("unset v; set -- x${v=''}y; echo $#:$1"), "1:xy\n");
}
