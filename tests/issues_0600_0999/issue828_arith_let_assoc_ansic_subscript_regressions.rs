//! Issue rubash#828: the fourth subscript write entry — arithmetic `((...))`
//! assignments (and their `let "A[...]=v"` form) — stored the raw `$'...'`
//! spelling instead of the decoded key the other entries resolve.
//!
//! GNU bash 5.3.0 baseline (WSL, UTF-8 locale, byte-verified probes
//! 2026-10-13; the full differential is on the PR):
//!
//! 1. `((A[$'x\ty']=1))` stores the clean decoded bytes `78 09 79` — the
//!    arithmetic text reaches `expand_subscript_string` raw and its
//!    expand_word_internal arm ANSI-C-decodes the span with the product
//!    quoted, so decoded bytes (and backslashes: `((D[$'a\\b']=2))` stores
//!    `61 5c 62`) land in the key. rubash kept the raw spelling
//!    (`24 27 78 5c 74 79 27`), so `${A[$k]}` printed empty.
//! 2. `let` is the scrambled sibling: its operand was word-expanded once
//!    (let.def passes EXP_EXPANDED), so `$'` is NOT re-recognized as
//!    ANSI-C — the `$` stays literal data, the span quotes drop as shell
//!    quotes and the raw spelling rides verbatim: `let "B[$'x\ty']=1"`
//!    stores `$x\ty` (`24 78 5c 74 79`). GNU's own quirk, pinned here as
//!    parity — rubash used to keep the quotes too (`$'x\ty'`).
//! 3. `unset 'A[$'"'"'x\ty'"'"']'` — the CR-Nit-2 data point: the operand
//!    spelling that decodes to the stored key. GNU's unset does not
//!    ANSI-C-decode the operand and KEEPS the cell; rubash used to decode
//!    it and DELETE. Landed by the parked unset-operand follow-up
//!    (rubash#847, `quote_remove_ansi_c_spans`), pinned below as parity.
//!
//! Fix: `expand_assoc_subscript_once` rewrites the literal `$'...'` spans
//! per entry (`ArithAnsiSpanMode::Decode` for the `((...))`/`$((...))`
//! reader family, `ArithAnsiSpanMode::LetOperand` for `let`), leaving bare
//! (non-`$'`) spellings untouched — the `(( ))` body reaches the evaluator
//! after the lexer's own escape pass, so bare-escape handling stays as is.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
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

/// Ticket headline: the arithmetic write entry stores the decoded key, so a
/// variable holding the same bytes reads it back (GNU: `[1]`).
#[test]
fn arith_write_ansi_c_read_variable() {
    let (stdout, stderr, _) =
        rubash("k=$'x\\ty'; declare -A A; ((A[$'x\\ty']=1)); printf '[%s]\\n' \"${A[$k]}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[1]\n");
}

/// The stored key is the clean decoded bytes `x TAB y` (GNU 78 09 79).
#[test]
fn arith_write_tab_key_storage_bytes() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; ((A[$'x\\ty']=1)); for k in \"${!A[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "780979\n");
}

/// The arithmetic reader decodes the same way: `$((A[$'x\ty']))` hits the
/// clean cell, and `declare -p` renders GNU's `[$'x\ty']` form.
#[test]
fn arith_reader_and_declarep_agree() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; ((A[$'x\\ty']=1)); printf '%s\\n' \"$((A[$'x\\ty']))\"; declare -p A",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "1\ndeclare -A A=([$'x\\ty']=\"1\" )\n");
}

/// Decoded backslashes survive on the arithmetic entry (GNU: `61 5c 62`).
/// The `${...}` reader keeps its pinned write/read asymmetry (it re-decodes
/// the raw text unquoted and the backslash drops), so its self-read misses
/// exactly like GNU.
#[test]
fn arith_backslash_span_bytes_and_reader_asymmetry() {
    let (stdout, stderr, _) = rubash(
        "declare -A D; ((D[$'a\\\\b']=2))
         for k in \"${!D[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done
         printf 'arith:%s\\n' \"$((D[$'a\\\\b']))\"
         printf 'var:%s\\n' \"${D[$'a\\\\b']}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "615c62\narith:2\nvar:\n");
}

/// Unrecognized ANSI-C escapes keep their backslash and self-read hits
/// (GNU: `705c71`, read `1`).
#[test]
fn arith_unrecognized_escape_keeps_backslash() {
    let (stdout, stderr, _) = rubash(
        "declare -A F; ((F[$'p\\\\q']=1))
         for k in \"${!F[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done
         printf '%s\\n' \"$((F[$'p\\\\q']))\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "705c71\n1\n");
}

/// rubash#735 interplay on the arithmetic entry: a user-typed `$'\u{E401}'`
/// key survives the decode as plain bytes (GNU: `ee 90 81`).
#[test]
fn arith_user_pua_e401_key_survives() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; ((A[$'\\uE401']=g)); for k in \"${!A[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "ee9081\n");
}

/// The `++` entry shares the lvalue path: `((A[$'x\ty']++))` stores clean
/// (GNU: `78 09 79`, value readable through the variable spelling).
#[test]
fn arith_increment_entry_normalizes() {
    let (stdout, stderr, _) = rubash(
        "k=$'x\\ty'; declare -A N; ((N[$'x\\ty']++)); printf '%s' \"${!N[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; printf '[%s]\\n' \"${N[$k]}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "780979\n[1]\n");
}

/// The plain arithmetic-command read form agrees (GNU: `1`).
#[test]
fn arith_command_read_form() {
    let (stdout, stderr, _) =
        rubash("declare -A A; ((A[$'x\\ty']=1)); (( z = A[$'x\\ty'] )); echo $z");
    assert_clean(&stderr);
    assert_eq!(stdout, "1\n");
}

/// Cross-spelling agreement: a variable-spelling write is readable through
/// the arithmetic ANSI-C spelling, both reader forms (GNU: `1` / `[1]`).
#[test]
fn var_write_arith_read_cross_spelling() {
    let (stdout, stderr, _) = rubash(
        "k=$'x\\ty'; declare -A C; C[$k]=1; printf '%s\\n' \"$((C[$'x\\ty']))\"; printf '[%s]\\n' \"${C[$'x\\ty']}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "1\n[1]\n");
}

/// `$'x\ty'` and a dq literal-tab write collapse onto ONE cell through the
/// arithmetic entry (GNU: single key `78 09 79`).
#[test]
fn arith_tab_spellings_collapse_to_one_cell() {
    let (stdout, stderr, _) = rubash(
        "declare -A E; ((E[$'x\\ty']=first)); ((E[\"x\ty\"]=second)); printf '%s' \"${!E[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "780979\n");
}

/// Membership agrees across spellings after an arithmetic write
/// (GNU: set / set / set).
#[test]
fn membership_agrees_across_spellings() {
    let (stdout, stderr, _) = rubash(
        "k=$'x\\ty'; declare -A I; ((I[$'x\\ty']=1))
         [[ -n \"${I[$k]+x}\" ]] && echo set || echo unset
         [[ -v I[$k] ]] && echo set || echo unset
         [[ -n \"${I[$'x\\ty']+x}\" ]] && echo set || echo unset",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "set\nset\nset\n");
}

/// `unset "A[$k]"` (variable spelling) removes the arithmetic-written cell
/// (GNU: `A=()`).
#[test]
fn unset_via_variable_removes_arith_cell() {
    let (stdout, stderr, _) =
        rubash("k=$'x\\ty'; declare -A U; ((U[$'x\\ty']=1)); unset \"U[$k]\"; declare -p U");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A U=()\n");
}

/// CR Nit 2 data point, now GNU PARITY (rubash#847): the unset operand
/// spelling carrying a quoted-source `$'...'` span. GNU 5.3.0 does not
/// ANSI-C-decode the operand — its deferred pass quote-removes it (quotes
/// drop, the span dollar stays literal, content verbatim) — so the
/// `$'...'`-written cell KEEPS (`declare -A V=([$'x\ty']="1" )`, probes
/// 2026-10-13/2026-10-19). rubash decoded the span, resolved the stored
/// clean key and deleted the cell; `quote_remove_ansi_c_spans` mirrors
/// GNU's deferred-pass law.
#[test]
fn unset_ansi_c_operand_spelling_matches_gnu() {
    let (stdout, stderr, _) =
        rubash("declare -A V; ((V[$'x\\ty']=1)); unset 'V[$'\"'\"'x\\ty'\"'\"']'; declare -p V");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A V=([$'x\\ty']=\"1\" )\n");
}

/// The `let` double-quoted form stores GNU's scrambled key: `$` literal,
/// span quotes dropped, raw spelling verbatim — `$x\ty` (`24 78 5c 74 79`).
/// Every clean-byte spelling misses it; the `let` reader scrambles the same
/// way and hits (GNU: `[]` / `1`).
#[test]
fn let_dq_form_stores_gnu_scrambled_key() {
    let (stdout, stderr, _) = rubash(
        "k=$'x\\ty'; declare -A B; let \"B[$'x\\ty']=1\"
         printf '%s' \"${!B[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'
         printf '[%s]\\n' \"${B[$k]}\"
         let \"v=B[$'x\\ty']\"; printf '[%s]\\n' \"$v\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "24785c7479\n[]\n[1]\n");
}

/// `declare -p` renders the scrambled key as plain data (GNU:
/// `(["\$x\\ty"]="1" )`).
#[test]
fn let_scrambled_key_declarep() {
    let (stdout, stderr, _) = rubash("declare -A B; let \"B[$'x\\ty']=1\"; declare -p B");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A B=([\"\\$x\\\\ty\"]=\"1\" )\n");
}

/// The single-quoted operand spelling reaches `let` as the same text and
/// scrambles identically (GNU: `24 78 5c 74 79`).
#[test]
fn let_sq_operand_matches_dq_form() {
    let (stdout, stderr, _) = rubash(
        "declare -A B2; let 'B2[$'\"'\"'x\\ty'\"'\"']=1'; printf '%s' \"${!B2[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "24785c7479\n");
}

/// CR Nit 1's row: `let "O[$'a\\b']=2"` — the dq operand collapses `\\`
/// before `let`, and the scrambled key keeps `$` plus the verbatim
/// backslash (GNU: `$a\b`, `24 61 5c 62`); the `let` self-read hits.
#[test]
fn let_backslash_dq_form_stores_verbatim() {
    let (stdout, stderr, _) = rubash(
        "declare -A O; let \"O[$'a\\\\b']=2\"
         printf '%s' \"${!O[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'
         let \"w=O[$'a\\\\b']\"; printf '[%s]\\n' \"$w\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "24615c62\n[2]\n");
}

/// The single-quoted operand preserves BOTH backslashes (no dq collapse on
/// the way in), and the key keeps them verbatim (GNU: `$a\\b`,
/// `24 61 5c 5c 62`).
#[test]
fn let_sq_backslash_keeps_both() {
    let (stdout, stderr, _) = rubash(
        "declare -A O2; let 'O2[$'\"'\"'a\\\\b'\"'\"']=2'; printf '%s' \"${!O2[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "24615c5c62\n");
}

/// Text around the span rides the same rule (GNU: `$x\typost`, `pre$x\ty`).
#[test]
fn let_mixed_text_around_span() {
    let (stdout, stderr, _) = rubash(
        "declare -A M; let \"M[$'x\\ty'post]=1\"
         printf '%s' \"${!M[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'
         declare -A P; let \"P[pre$'x\\ty']=1\"
         printf '%s' \"${!P[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "24785c7479706f7374\n70726524785c7479\n");
}

/// The entry boundary that makes the quirk: a `let` READ of a clean cell
/// scrambles and misses, and the `$(( ))` reader of a `let`-written
/// scrambled cell misses too (GNU: `0` / `0`).
#[test]
fn clean_and_scrambled_cells_miss_across_entries() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; ((A[$'x\\ty']=1)); let \"y=A[$'x\\ty']\"; echo $y
         declare -A B; let \"B[$'x\\ty']=1\"; echo $((B[$'x\\ty']))",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "0\n0\n");
}

/// Regression guard: `let` with a VARIABLE subscript still expands the
/// variable into the clean key — bare tab and dq literal-tab too
/// (GNU: `78 09 79`, readable back).
#[test]
fn let_variable_subscript_still_expands() {
    let (stdout, stderr, _) = rubash(
        "k=$'x\\ty'; declare -A J; let \"J[$k]=9\"
         printf '[%s]\\n' \"${J[$k]}\"
         declare -A K; let \"K[x\ty]=7\"; printf '%s' \"${!K[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "[9]\n780979\n");
}

/// Boundary: under `shopt -s array_expand_once` the EXP_EXPANDED `let`
/// operand takes GNU's AV_NOEXPAND — the subscript text is the key
/// verbatim, quotes included (GNU: `24 27 78 5c 74 79 27`), while the
/// `(( ))` write under the same shopt still decodes.
#[test]
fn shopt_array_expand_once_let_stores_verbatim() {
    let (stdout, stderr, _) = rubash(
        "shopt -s array_expand_once
         declare -A W; let \"W[$'x\\ty']=1\"
         printf '%s' \"${!W[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'
         declare -A W2; ((W2[$'x\\ty']=1))
         printf '%s' \"${!W2[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "2427785c747927\n780979\n");
}

/// cr-839 BLOCKER regression, E3 shape: a `$'...'` spelling INSIDE a
/// double-quoted subscript segment is data, not an ANSI-C span. The span
/// scanner must track quote state — GNU and master both store the raw
/// spelling `pre$'x'post` (`70 72 65 24 27 78 27 70 6f 73 74`); the
/// unguarded scanner decoded the span and stored `prexpost`.
#[test]
fn dq_quoted_dollar_quote_stays_data() {
    let (stdout, stderr, _) = rubash(
        "declare -A E3; ((E3[\"pre$'x'post\"]=1)); printf '%s' \"${!E3[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "70726524277827706f7374\n");
}

/// cr-839 BLOCKER regression, E1 shape: in `'a$'b'c'` the `$'` sits at the
/// single-quote CLOSE boundary — the `$` is that span's data dollar and the
/// `'` its closer, so it must not open an ANSI-C span (the unguarded
/// scanner consumed `'b'` and stored `abc`, losing the `$` bytes). The
/// arithmetic entry keeps the master-known raw shape `'a$'b'c'`
/// (`27 61 24 27 62 27 63 27`): GNU quote-removes the subscript word and
/// stores `a$bc` (`61 24 62 63`, probed 2026-10-13 on the `(( ))` and
/// assignment entries alike), while the rubash arithmetic path does not run
/// quote removal — a pre-existing divergence of this entry, unchanged here
/// (full parity needs quote removal on this path; parked). The let form
/// shares the scanner and the same raw shape (see `L2` below).
#[test]
fn sq_close_boundary_dollar_quote_not_a_span() {
    let (stdout, stderr, _) = rubash(
        "declare -A E1; ((E1['a$'b'c']=1)); printf '%s' \"${!E1[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "2761242762276327\n");
}

/// The let operand carries the same shapes through the shared scanner and
/// keeps its rows: `L1["pre$'x'post"]` is GNU parity (`pre$'x'post`, the
/// single quotes ride inside the surviving dquotes), `L2['a$'b'c']` keeps
/// the master-known raw shape (GNU quote-removes to `a$bc` — the same
/// pre-existing divergence as the `(( ))` entry above).
#[test]
fn let_entry_quote_state_shapes_unchanged() {
    let (stdout, stderr, _) = rubash(
        "declare -A L1; let \"L1[\\\"pre\\$'x'post\\\"]=1\"
         printf '%s' \"${!L1[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'
         declare -A L2; let 'L2['\"'\"'a$'\"'\"'b'\"'\"'c'\"'\"']=1'
         printf '%s' \"${!L2[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "70726524277827706f7374\n2761242762276327\n");
}

// =====================================================================
// rubash#847 — the two remaining faces of the span-state family.
//
// GNU bash 5.3.0 baselines (WSL, UTF-8 locale, byte-verified probes
// 2026-10-19; the full differential lives on the PR):
//
// 1. E3b — the #822 assignment/reader ANSI-C span scanner tracked only
//    single-quote state, so a span INSIDE double quotes was decoded
//    anyway: `M["pre$'x'post"]=w` stored the merged `prexpost`. GNU's
//    expand_word_internal never recognizes ANSI-C quoting inside dquotes
//    — the whole spelling is data and the key stays `pre$'x'post`
//    (`70726524277827706f7374`), on the assignment entry, the `((...))`
//    entry and the `${...}` reader alike.
// 2. u-family — unset's deferred operand pass quote-removes a
//    quoted-source `$'...'` spelling instead of decoding it
//    (`quote_remove_ansi_c_spans`): u1/u2/u3 below keep the cell.
//
// Both fixes mirror the #839 dual-state law: `$'` opens a span only
// outside both quotes, `'`/`"` toggle only outside the other quote.

/// E3b on the assignment entry: the dq-wrapped span stays raw data
/// (GNU: `pre$'x'post`, `70726524277827706f7374`).
#[test]
fn e3b_assignment_dq_span_kept_raw() {
    let (stdout, stderr, _) = rubash(
        "declare -A M; M[\"pre$'x'post\"]=w
         printf '%s' \"${!M[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "70726524277827706f7374\n");
}

/// E3b on the reader: the same spelling self-reads the `((...))`-written
/// cell (GNU: `gr=1`).
#[test]
fn e3b_reader_self_read_hits() {
    let (stdout, stderr, _) = rubash(
        "declare -A G; ((G[\"pre$'x'post\"]=1)); printf 'gr=%s\\n' \"${G[\"pre$'x'post\"]}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "gr=1\n");
}

/// E3b on the arithmetic write: the stored key is the clean spelling, so a
/// variable holding the same bytes reads it back (GNU: `[1]`).
#[test]
fn e3b_arith_write_variable_read_hits() {
    let (stdout, stderr, _) = rubash(
        "k=\"pre$'x'post\"; declare -A G; ((G[\"pre$'x'post\"]=1)); printf '[%s]\\n' \"${G[$k]}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "[1]\n");
}

/// Dquote escape rules still apply to the kept spelling's surroundings:
/// the dq `\\t` stays a literal backslash pair, no ANSI-C decode fires
/// (GNU: `abcd$'e\\tf'gh`, `616263642427655c7466276768`).
#[test]
fn e3b_dq_adjacent_text_span_kept_raw() {
    let (stdout, stderr, _) = rubash(
        "declare -A W5; W5[ab\"cd$'e\\tf'\"gh]=1
         printf '%s' \"${!W5[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "616263642427655c7466276768\n");
}

/// The dq `\\\\` collapse happens at dquote level, not in the span: the
/// kept spelling shows one backslash (GNU: `x$'a\\b'y`, `782427615c622779`).
#[test]
fn e3b_dq_escaped_backslash_span_kept_raw() {
    let (stdout, stderr, _) = rubash(
        "declare -A W6; W6[\"x$'a\\\\b'y\"]=1
         printf '%s' \"${!W6[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "782427615c622779\n");
}

/// E3 non-regression: a bare `$x` INSIDE the dquotes still expands (GNU:
/// `pre'post`, `70726527706f7374`).
#[test]
fn e3_bare_expansion_non_regression() {
    let (stdout, stderr, _) = rubash(
        "declare -A E3; ((E3[\"pre$x'post\"]=1))
         printf '%s' \"${!E3[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "70726527706f7374\n");
}

/// E1 non-regression: the sq-piece spelling keys on `a$bc` (GNU:
/// `61246263`).
#[test]
fn e1_sq_pieces_non_regression() {
    let (stdout, stderr, _) = rubash(
        "declare -A E1; E1['a$'b'c']=1
         printf '%s' \"${!E1[@]}\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "61246263\n");
}

/// u1 — the CR Nit 2 spelling (sq-embedded operand): the cell survives
/// (GNU: `declare -A A1=([$'x\\ty']=\"1\" )`).
#[test]
fn unset_u1_sq_embedded_keeps_cell() {
    let (stdout, stderr, _) = rubash(
        "declare -A A1; ((A1[$'x\\ty']=1)); unset 'A1[$'\"'\"'x\\ty'\"'\"']'; declare -p A1",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A A1=([$'x\\ty']=\"1\" )\n");
}

/// u2 — the plain double-quoted operand: same keep (GNU parity).
#[test]
fn unset_u2_double_quoted_keeps_cell() {
    let (stdout, stderr, _) =
        rubash("declare -A A2; ((A2[$'x\\ty']=1)); unset \"A2[$'x\\ty']\"; declare -p A2");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A A2=([$'x\\ty']=\"1\" )\n");
}

/// u3 — the dq-escaped-dollar operand: same keep (GNU parity).
#[test]
fn unset_u3_dq_escaped_dollar_keeps_cell() {
    let (stdout, stderr, _) =
        rubash("declare -A A3; ((A3[$'x\\ty']=1)); unset \"A3[\\$'x\\ty']\"; declare -p A3");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A A3=([$'x\\ty']=\"1\" )\n");
}

/// The resolved key is `$x\ty` — literal span dollar plus the quote-removed
/// content — so a LITERAL `$x` cell is the one that gets hit (GNU gnu9
/// probe: `unset "C[\$'x']"` deletes the `$x` cell and keeps `x`).
#[test]
fn unset_resolves_to_literal_dollar_prefix_key() {
    let (stdout, stderr, _) =
        rubash("declare -A C; C[x]=a; C[\\$x]=i; unset \"C[\\$'x']\"; declare -p C");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A C=([x]=\"a\" )\n");
}

/// Bare (unquoted) ANSI-C spelling non-regression: it decoded at word
/// expansion long before unset, so the cell is still removed (GNU: `()`).
#[test]
fn unset_bare_spelling_still_deletes() {
    let (stdout, stderr, _) =
        rubash("declare -A G1; ((G1[$'x\\ty']=1)); unset G1[$'x\\ty']; declare -p G1");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A G1=()\n");
}

/// Variable-spelling operand non-regression: `$k` still expands on the
/// deferred pass and removes the cell (GNU: `()`).
#[test]
fn unset_variable_spelling_still_deletes() {
    let (stdout, stderr, _) =
        rubash("k=$'x\\ty'; declare -A A5; ((A5[$'x\\ty']=1)); unset \"A5[$k]\"; declare -p A5");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A A5=()\n");
}

/// A `$k` with NO span in the operand still expands even from a
/// single-quoted source (GNU probe: `unset 'P3[$k]'` with k=Q deletes the
/// `Q` cell) — the transform touches only `$'...'` spellings.
#[test]
fn unset_param_expansion_without_span_still_runs() {
    let (stdout, stderr, _) =
        rubash("k=Q; declare -A P3; P3[Q]=cell; unset 'P3[$k]'; declare -p P3");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A P3=()\n");
}
