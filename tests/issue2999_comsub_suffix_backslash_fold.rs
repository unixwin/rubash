//! rubash#2999 — a comsub/backquote suffix folds a backslash out of a
//! preceding single-quoted segment.
//!
//! Root cause: the lexer cooks `'a\$b c'` to `a\` + DATA_DOLLAR (`$` as the
//! U+001F carrier) + `b c`, and the argument-word expansion path runs the
//! MUTABLE walker (`command_prepare.rs` → `expand_word_mut_with_context` →
//! `expand_embedded_parameters_mut_inner` in
//! `src/executor/embedded_mutations.rs`). That walker never got the rubash#2216
//! twin the non-mut walker has: bare `\` rides as ASSIGN_EXPANSION_BACKSLASH
//! (U+E30D) through the protecting walk and is restored after the final
//! unescape pass. Instead, two fold sites in
//! `expand_embedded_parameters_ordered_mut` re-cooked the literal data:
//! (1) the `\\` arm collapsed a `\\` pair to one `\` — for a cooked word
//! the pair can only be literal single-quote data, since the lexer already
//! resolved every live `\\` into DATA_BACKSLASH; (2) the `if ch != '$'`
//! fall-through pushed a lone `\` verbatim, and the walker decodes
//! DATA_DOLLAR to `$` mid-walk, so the assembled string held a fake `\$`
//! pair which the `$(`/backquote-gated trailing
//! `unescape_remaining_shell_escapes` pass folded to `$`. The non-mut twin
//! (`expand_embedded_parameters_inner`, `src/executor/embedded_parameters.rs`)
//! already carries bare `\` as U+E30D in its protecting walk.
//!
//! The fix gives the mut walker the same twin: the `\\` arm and the lone-`\`
//! fall-through ride as U+E30D when the walk protects RHS data (heredoc
//! excluded — heredoc `\\` is live escape syntax collapsing to one `\`),
//! the single-quote arm rides its `\`s when the walk protects data or is an
//! alternate word half, and the alternate entry restores U+E30D at its own
//! boundary (its consumers — the whole-word `${}` path via the non-mut
//! `expand_word`, the `=` RHS store, the compound store — never restore
//! it; a comsub-suffixed alternate half is re-protected by the enclosing
//! walker's `${` splice before the enclosing trailing pass).
//!
//! GNU anchors: subst.c:11882-11886 `case '\''` → `string_extract_single_quoted`
//! → `add_quoted_string` (subst.c:11862) → `quote_string` (subst.c:4773)
//! CTLESC-protects the span; subst.c:4807 `dequote_string` strips only
//! CTLESC bytes; parse.y:5694-5706 `got_escaped_character`. Oracle evidence:
//! GNU bash 5.3.20 vs rubash master 30ddc122, byte tables in the probe
//! scripts under `~/workspace/fix-2999/` (probe1/2/4/5).
//!
//! Known residuals (NOT pinned — they still fold on head, same family, need
//! their own design): expansion-result backslashes inside alternate word
//! halves (`${x:-$y$(...)}` with `y='a\$b'`) and nested alternates
//! (`${x:-${y:-'a\$b'}$(printf "")}`) — the alternate walker splices
//! results bare (protect_rhs_data=false) and its own trailing pass folds
//! them; the inner alternate's restore undoes the protection before the
//! outer trailing pass. Also the redirect-target IFS gap (`'a b c'$(...)`
//! as a redirect target reports "ambiguous redirect" while GNU creates the
//! file) — pre-existing, separate mechanism.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the pin is
//! hermetic under any ambient locale.

use std::process::{Command, Stdio};

fn run(script: &str) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(out, expected, "{ctx}: got {out:?}, want {expected:?}");
}

#[test]
fn comsub_suffix_single_backslash_preserved() {
    expect(
        "printf '[%s]\\n' 'a\\$b c'$(printf \"\")",
        b"[a\\$b c]\n",
        "comsub suffix must not fold \\$ out of a single-quoted segment",
    );
}

#[test]
fn comsub_suffix_double_backslash_preserved() {
    expect(
        "printf '[%s]\\n' 'a\\\\b c'$(printf \"\")",
        b"[a\\\\b c]\n",
        "comsub suffix must not fold \\\\ out of a single-quoted segment",
    );
}

#[test]
fn comsub_suffix_nonempty_output_preserved() {
    expect(
        "printf '[%s]\\n' 'a\\$b c'$(printf \"Z\")",
        b"[a\\$b cZ]\n",
        "comsub suffix with output must keep the quoted backslash",
    );
}

#[test]
fn backquote_suffix_single_backslash_preserved() {
    expect(
        "printf '[%s]\\n' 'a\\$b c'`printf \"\"`",
        b"[a\\$b c]\n",
        "backquote suffix must not fold \\$ out of a single-quoted segment",
    );
}

#[test]
fn mixed_suffix_param_then_comsub_preserved() {
    expect(
        "V=; printf '[%s]\\n' 'a\\$b c'${V}$(printf \"\")",
        b"[a\\$b c]\n",
        "parameter + comsub suffix mix must keep the quoted backslash",
    );
}

#[test]
fn mixed_suffix_comsub_then_param_preserved() {
    expect(
        "V=; printf '[%s]\\n' 'a\\$b c'$(printf \"\")${V}",
        b"[a\\$b c]\n",
        "comsub + parameter suffix mix must keep the quoted backslash",
    );
}

#[test]
fn mixed_suffix_backquote_then_param_preserved() {
    expect(
        "V=; printf '[%s]\\n' 'a\\$b c'`printf \"\"`${V}",
        b"[a\\$b c]\n",
        "backquote + parameter suffix mix must keep the quoted backslash",
    );
}

#[test]
fn alternate_half_comsub_suffix_preserved() {
    expect(
        "x=; printf '[%s]\\n' ${x:-'a\\$b c'$(printf \"\")}",
        b"[a\\$b c]\n",
        "alternate word half with comsub suffix must keep the quoted backslash",
    );
}

#[test]
fn alternate_half_dash_comsub_suffix_preserved() {
    expect(
        "unset x; printf '[%s]\\n' ${x-'a\\$b c'$(printf \"\")}",
        b"[a\\$b c]\n",
        "${x-...} half with comsub suffix must keep the quoted backslash",
    );
}

#[test]
fn alternate_half_double_backslash_comsub_suffix_preserved() {
    expect(
        "x=; printf '[%s]\\n' ${x:-'a\\\\b c'$(printf \"\")}",
        b"[a\\\\b c]\n",
        "alternate word half must keep a doubled backslash with comsub suffix",
    );
}

#[test]
fn alternate_half_without_comsub_no_carrier_leak() {
    expect(
        "x=; printf '[%s]\\n' ${x:-'a\\$b c'}",
        b"[a\\$b c]\n",
        "alternate half without a suffix must not leak the protection carrier",
    );
}

#[test]
fn assign_rhs_alternate_comsub_suffix_preserved() {
    expect(
        "x=; z=${x:-'a\\$b'$(printf \"\")}; printf '[%s]\\n' \"$z\"",
        b"[a\\$b]\n",
        "= RHS alternate half with comsub suffix must keep the quoted backslash",
    );
}

#[test]
fn compound_element_comsub_suffix_preserved() {
    expect(
        "A=('a\\$b c'$(printf \"\")); printf '[%s]\\n' \"${A[0]}\"",
        b"[a\\$b c]\n",
        "compound element with comsub suffix must keep the quoted backslash",
    );
}

#[test]
fn assignment_suffix_control_still_preserved() {
    expect(
        "v='a\\$b c'$(printf \"\"); printf '[%s]\\n' \"$v\"",
        b"[a\\$b c]\n",
        "assignment path (already correct) must stay correct",
    );
}

#[test]
fn unquoted_heredoc_double_backslash_still_collapses() {
    expect(
        "cat <<EOF\na\\\\b\nEOF",
        b"a\\b\n",
        "heredoc \\\\ is live escape syntax and must still collapse to one \\",
    );
}
