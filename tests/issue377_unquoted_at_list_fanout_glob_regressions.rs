//! rubash#377: the UNQUOTED compound-assignment fan-out must pathname-
//! expand its fields — the inverse transport of the #369 quoted-element
//! fix.
//!
//! GNU pipeline: `expand_compound_array_assignment` (arrayfunc.c:557)
//! tokenizes the body (parse_string_to_word_list, quoting intact), then
//! `expand_words_no_vars` (arrayfunc.c:610 -> subst.c:12592 ->
//! subst.c:13219 expand_word_list_internal) expands every word: an
//! unquoted `${a[@]}` rides `string_list_pos_params` (subst.c:3030, the
//! `pchar == '@' && quoted == 0` arms) as the members joined with the
//! first IFS character, the standard field splitter runs on the joined
//! string, and `glob_expand_word_list` (subst.c:13264) pathname-expands
//! every surviving field — `a=('*' '*'); arr=(${a[@]})` stores the
//! matches, and an unquoted expansion field carries no CTLESC protection
//! (only quoted spans get `add_quoted_string`, subst.c:11862).
//!
//! Root cause on the rubash side (three sites, one invariant):
//! 1. The compound walker's whole-word `[@]` arm
//!    (`expand_compound_positional_at_assignment_impl`) transported EVERY
//!    member as `quote_array_value` storage words — the QUOTED transport,
//!    whose quote span the storage glob gate re-encodes as CTLESC-protected
//!    data. Unquoted fan-outs now ride the joined-string field-split model
//!    (`field_split_positional_values_with_ifs`, IFS[0] join) and each
//!    field takes `ARRAY_FIELD_SPLIT_MARKER` (the unquoted field-split
//!    product form the storage marker arm globs).
//! 2. `expand_declare_compound_elements` (subscript_expansion.rs) tagged
//!    EVERY bare-element product — quoted ones included — with
//!    ARRAY_FIELD_SPLIT_MARKER. A quoted product arrives CTLESC-protected
//!    (\x11, the add_quoted_string port) and `quote_array_value` strips
//!    those pairs at serialization, so the tag destroyed the quoting state
//!    exactly where the marker arm dequotes-then-globs; quoted products now
//!    keep the plain quoted storage word (no marker).
//! 3. The declare storage marker arm (append_array_value,
//!    declare/storage/array.rs) globbed the raw quote-wrapped token, which
//!    `pathname_expand_word` rejects on its leading quote; it now globs the
//!    dequoted field — GNU globs the expanded word AFTER quote removal
//!    (subst.c:13264), so `declare -a d=($x)` with x='*' stores the
//!    matches.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes (target/probe377 matrices,
//! 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    output.stdout
}

/// Elements rendered as `[%s]` joined, the m377 matrix form.
fn elements(script_head: &str) -> Vec<u8> {
    rubash(&format!(
        "{script_head}\nprintf '[%s]' \"${{arr[@]}}\"; echo",
    ))
}

/// The fixture lives in the Rust temp dir so the glob has a known match
/// set (only377a / only377b). The script is built by concatenation so the
/// caller's `{`/`}` never enter a format string.
fn fixture(script_body: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join("rubash377fx");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    std::fs::write(dir.join("only377a"), b"").expect("touch fixture a");
    std::fs::write(dir.join("only377b"), b"").expect("touch fixture b");
    let script = format!("cd '{}'; ", dir.display()) + "a=('*' '*'); x='*'; " + script_body;
    let out = rubash(&script);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn unquoted_at_list_fanout_globs_per_member() {
    // The #377 reproducer: GNU stores the glob matches, one run per member.
    assert_eq!(
        fixture("arr=(${a[@]}); printf '[%s]' \"${arr[@]}\"; echo"),
        b"[only377a][only377b][only377a][only377b]\n",
        "subst.c:13264 glob_expand_word_list runs on every unquoted field"
    );
}

#[test]
fn unquoted_at_list_fanout_splits_members_on_ifs() {
    // The join-then-split model: members joined with IFS[0], then the
    // standard field splitter (subst.c:3030 string_list_pos_params).
    assert_eq!(
        elements("a3=('p q' r); arr=(${a3[@]})"),
        b"[p][q][r]\n",
        "an unquoted member's IFS whitespace splits (default IFS)"
    );
    assert_eq!(
        elements("IFS=.; a4=('p.q' r); arr=(${a4[@]})"),
        b"[p][q][r]\n",
        "a non-whitespace IFS splits unquoted members too"
    );
}

#[test]
fn unquoted_at_list_empty_member_boundary() {
    // Default IFS: an empty member between neighbors vanishes (whitespace
    // collapse); under a non-whitespace IFS it is an empty FIELD (the
    // IFS[0] join makes it an adjacent separator pair).
    assert_eq!(
        elements("a2=('' x); arr=(${a2[@]})"),
        b"[x]\n",
        "default-IFS empty member contributes no field"
    );
    assert_eq!(
        elements("IFS=.; a9=('' x); arr=(${a9[@]})"),
        b"[][x]\n",
        "dot-IFS empty member is an empty field (string_list join)"
    );
    assert_eq!(
        elements("IFS=.; a10=('' '' y); arr=(${a10[@]})"),
        b"[][][y]\n",
        "two empty dot-IFS members are two empty fields"
    );
    assert_eq!(
        rubash("a8=('' ''); arr=(${a8[@]}); printf 'count=%d' \"${#arr[@]}\"; echo"),
        b"count=0\n",
        "all-empty members under the default IFS store zero elements"
    );
}

#[test]
fn declare_unquoted_scalar_parameter_globs() {
    // `declare -a d=($x)` — the storage marker arm must glob the dequoted
    // field (declare/storage/array.rs; GNU globs after quote removal).
    assert_eq!(
        fixture("declare -a d=($x); printf '[%s]' \"${d[@]}\"; echo"),
        b"[only377a][only377b]\n",
        "an unquoted $x field is a pattern word (subst.c:13264)"
    );
}

#[test]
fn declare_unquoted_at_list_fanout_globs() {
    assert_eq!(
        fixture("declare -a d2=(${a[@]}); printf '[%s]' \"${d2[@]}\"; echo"),
        b"[only377a][only377b][only377a][only377b]\n",
        "declare's compound expansion globs the unquoted fan-out fields"
    );
}

#[test]
fn declare_quoted_string_rhs_at_list_globs_like_gnu() {
    // `declare -a e1='(x ${a[@]})'`: GNU re-parses the dequoted string and
    // expands the now-unquoted at-list (arrayfunc.c:557 comment).
    assert_eq!(
        fixture("declare -a e1='(x ${a[@]})'; printf '[%s]' \"${e1[@]}\"; echo"),
        b"[x][only377a][only377b][only377a][only377b]\n",
        "the string-RHS at-list is unquoted after argument quote removal"
    );
}

#[test]
fn declare_string_rhs_mixed_quoting_controls() {
    // Byte-verified vs GNU 5.3.0 (target/probe377/v1.sh): a quoted member
    // inside the string RHS stays literal while an unquoted member globs.
    assert_eq!(
        fixture("declare -a e2='(y \"$x\")'; printf '[%s]' \"${e2[@]}\"; echo"),
        b"[y][*]\n",
        "a quoted $x member of a string RHS is data"
    );
    assert_eq!(
        fixture("declare -a e3='(z \"$x\" $x)'; printf '[%s]' \"${e3[@]}\"; echo"),
        b"[z][*][only377a][only377b]\n",
        "quoted and unquoted members of one string RHS diverge correctly"
    );
}

#[test]
fn quoted_fanout_controls_stay_literal() {
    // Must NOT be over-expanded (rubash#369 class, both transports):
    // a quoted at-list member keeps its glob characters as data.
    assert_eq!(
        elements("a=('*' '*'); arr=(\"${a[@]}\")"),
        b"[*][*]\n",
        "quoted at-list members are CTLESC-protected data"
    );
    assert_eq!(
        rubash("x='*'; declare -a d3=(\"$x\"); printf '[%s]' \"${d3[@]}\"; echo"),
        b"[*]\n",
        "a quoted $x element through declare stays literal"
    );
    assert_eq!(
        rubash("x='* p'; declare -a d4=(\"$x\"); printf '[%s]' \"${d4[@]}\"; echo"),
        b"[* p]\n",
        "a quoted multi-word value stays one literal element"
    );
}

#[test]
fn unquoted_plain_scalar_control_still_globs() {
    assert_eq!(
        fixture("arr2=($x); printf '[%s]' \"${arr2[@]}\"; echo"),
        b"[only377a][only377b]\n",
        "the plain unquoted scalar element keeps globbing (control)"
    );
}

#[test]
fn append_fanout_globs() {
    assert_eq!(
        fixture("arr14=(q); arr14+=(${a[@]}); printf '[%s]' \"${arr14[@]}\"; echo"),
        b"[q][only377a][only377b][only377a][only377b]\n",
        "arr+=(${{a[@]}}) rides the same unquoted fan-out transport"
    );
}

#[test]
fn nullglob_drops_unmatched_fanout_fields() {
    assert_eq!(
        fixture("shopt -s nullglob; a5=('zz-*'); arr12=(${a5[@]}); printf 'count=%d' \"${#arr12[@]}\"; echo"),
        b"count=0\n",
        "an unmatched pattern under nullglob contributes no element"
    );
}

#[test]
fn unmatched_pattern_without_nullglob_stays_literal() {
    assert_eq!(
        fixture("a5=('zz-*'); arr12=(${a5[@]}); printf '[%s]' \"${arr12[@]}\"; echo"),
        b"[zz-*]\n",
        "default shell keeps an unmatched pattern literal"
    );
}
