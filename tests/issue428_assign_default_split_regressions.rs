//! rubash#428 — a value assigned through `${V:=default}` (or `${V=default}`)
//! expanded inside double quotes must bind the DEQUOTED text, so later
//! unquoted `$V` word-splits normally.
//!
//! GNU: subst.c:7966 parameter_brace_expand_rhs() expands the rhs with
//! transient CTLESC protection on quoted whitespace (expand_string_for_rhs,
//! posixexp2 37), but the value BOUND to the parameter is
//! `t1 = dequote_string (temp)` (subst.c:8096) passed to bind_variable
//! (subst.c:8151) / assign_array_element (subst.c:8145); the expansion
//! result is re-derived from the quote context (subst.c:8196
//! `w->word = quote_string (t1)`). The whitespace protection is word-local
//! and must never outlive the expansion inside the stored variable value.
//!
//! Before the fix, rubash baked the double-quoted context's IFS_GLUE
//! (\\x1c) pairs into the stored value, so the whitespace stayed
//! un-splittable forever: after `: "${W:=u s p}"`, `set -- $W` produced one
//! word where GNU bash 5.3.0 produces three.

use std::process::Command;

fn rubash(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The headline: quoted := in a command argument, split on the next command.
#[test]
fn quoted_assign_default_splits_on_later_set_dash_dash() {
    assert_eq!(
        rubash("unset W\n: \"${W:=u s p}\"\nset -- $W\necho $#"),
        "3\n"
    );
}

/// The quoted fragment itself may be the `set --` argument.
#[test]
fn quoted_assign_default_as_set_argument_splits_later() {
    assert_eq!(
        rubash("unset W\nset -- \"${W:=u s p}\"\nset -- $W\necho $#"),
        "3\n"
    );
}

/// Any command argument position binds the same way (echo form).
#[test]
fn quoted_assign_default_via_echo_splits_later() {
    assert_eq!(
        rubash("unset W\necho \"${W:=u s p}\" > /dev/null\nset -- $W\necho $#"),
        "3\n"
    );
}

/// for-in consumption of the assigned value.
#[test]
fn quoted_assign_default_splits_in_for_loop() {
    assert_eq!(
        rubash("unset W\n: \"${W:=u s p}\"\nn=0\nfor w in $W; do n=$((n+1)); done\necho $n"),
        "3\n"
    );
}

/// Unquoted command-argument consumption of the assigned value.
#[test]
fn quoted_assign_default_splits_in_unquoted_argument() {
    assert_eq!(
        rubash("unset W\n: \"${W:=u s p}\"\nprintf '%s|%s|%s' $W"),
        "u|s|p"
    );
}

/// The stored value must be the plain text — no \\x1c carrier pairs. Length
/// 5 = `u s p`; the pre-fix stored form (with \\x1c pairs) is 7 chars.
#[test]
fn quoted_assign_default_stores_dequoted_bytes() {
    assert_eq!(rubash("unset W\n: \"${W:=u s p}\"\necho ${#W}"), "5\n");
    assert_eq!(
        rubash("unset W\n: \"${W:=u s p}\"\nprintf '%s' \"$W\""),
        "u s p"
    );
}

/// The bare `=` operator has the same bind rule (subst.c:8096 op == '=').
#[test]
fn quoted_assign_operator_stores_dequoted_bytes() {
    assert_eq!(
        rubash("unset v\n: \"${v=u s p}\"\nset -- $v\necho $#"),
        "3\n"
    );
}

/// IFS characters other than space must also be restored literally.
#[test]
fn quoted_assign_default_with_colon_ifs_splits_later() {
    assert_eq!(
        rubash("unset W\nIFS=:\n: \"${W:=a:b:c}\"\nset -- $W\necho $#\nunset IFS"),
        "3\n"
    );
}

/// A later plain reassignment keeps splitting (state is not sticky).
#[test]
fn reassignment_after_quoted_assign_default_still_splits() {
    assert_eq!(
        rubash("unset W\n: \"${W:=u s p}\"\nW=\"m n\"\nset -- $W\necho $#"),
        "2\n"
    );
}

/// Guard against over-decoding: the assignment-RHS quoted form (which bound
/// clean even before the fix) must keep splitting.
#[test]
fn assignment_rhs_quoted_assign_default_still_splits() {
    assert_eq!(
        rubash("unset W\nW=\"${W:=u s p}\"\nset -- $W\necho $#"),
        "3\n"
    );
}

/// Escaped whitespace in a double-quoted alternate stays escaped in the
/// stored value (posixexp2 36: `"${v=a\\ b}"` assigns `a\ b`) and later
/// unquoted expansion splits on the space.
#[test]
fn quoted_assign_default_escaped_space_stays_escaped_in_storage() {
    assert_eq!(
        rubash("unset v\n: \"${v:=a\\ b}\"\nprintf '%s' \"$v\""),
        "a\\ b"
    );
}

/// Glob default value: two words, metachar intact for later expansion.
#[test]
fn quoted_assign_default_with_glob_stays_two_words() {
    assert_eq!(
        rubash("unset W\n: \"${W:=x* y}\"\nset -- $W\necho $#"),
        "2\n"
    );
}

/// The wt91 bash-it redline theme pattern (quoted default list, per-prompt
/// segment loop) must iterate per segment.
#[test]
fn redline_theme_powerline_prompt_list_iterates_per_segment() {
    let script = concat!(
        "unset POWERLINE_PROMPT\n",
        "POWERLINE_PROMPT=${POWERLINE_PROMPT:=\"python_venv ruby user_info\"}\n",
        "n=0\nfor segment in $POWERLINE_PROMPT; do n=$((n+1)); done\necho $n"
    );
    assert_eq!(rubash(script), "3\n");
}

/// The wt91 gitline/atomic colon-prefix form (`: "${V:=...}"`).
#[test]
fn gitline_theme_colon_prefix_form_iterates_per_segment() {
    let script = concat!(
        "unset ___ATOMIC_TOP_LEFT\n",
        ": \"${___ATOMIC_TOP_LEFT:=user_info dir scm}\"\n",
        "n=0\nfor segment in $___ATOMIC_TOP_LEFT; do n=$((n+1)); done\necho $n"
    );
    assert_eq!(rubash(script), "3\n");
}
