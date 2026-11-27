//! Issue rubash#861: kvlist self-read miss (write parity, read miss — the
//! #847 CR residual b).
//!
//! GNU bash 5.3.0 baseline (WSL oracle, byte-verified probes 2026-10-19):
//! the strict `name=( [key]=value )` path cooked a kept `$'...'` spelling's
//! quotes into the lexer's E010/E011 data-quote markers and stored them IN
//! the key. The stored cell was `pre$<E010>x<E010>post` while every reader
//! resolved the plain spelling, so:
//!
//!   * the same-spelling read (`${K["pre$'x'post"]}`, the arith reader, and
//!     the variable spelling) MISSED, silently;
//!   * the listing/rendering channels decoded the markers — `${!K[@]}` and
//!     `declare -p` printed byte-identical clean bytes, hiding the face;
//!   * a clean-spelling overwrite added a SECOND cell (GNU: one, value
//!     replaced).
//!
//! GNU: one clean key `pre$'x'post` (70 72 65 24 27 78 27 70 6f 73 74), all
//! reads hit. Fix: `dequote_compound_subscript` restores the data-quote
//! markers to literal bytes in its cooked-text tail (the same law its
//! E309/IFS_GLUE strip already applies), so the kvlist path stores exactly
//! the bytes the assignment entry stores.

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

/// Ticket face: the same-spelling read hits after the kvlist write (GNU
/// `[v]`), and the listed key bytes stay the raw spelling
/// (70 72 65 24 27 78 27 70 6f 73 74).
#[test]
fn kvlist_same_spelling_read_hits() {
    let (stdout, stderr, _) = rubash(
        "declare -A K; K=( [\"pre$'x'post\"]=v )\n\
         printf '%s' \"${!K[@]}\" | od -An -tx1 | tr -d ' \\n'; printf '\\n'\n\
         printf '[%s]\\n' \"${K[\"pre$'x'post\"]}\"",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "70726524277827706f7374\n[v]\n");
}

/// The arith reader over the kvlist-written cell hits too (GNU 7).
#[test]
fn kvlist_arith_reader_hits() {
    let (stdout, stderr, _) = rubash(
        "declare -A K2; K2=( [\"pre$'x'post\"]=7 ); printf '%s\\n' \"$((K2[\"pre$'x'post\"]))\"",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "7\n");
}

/// Same-statement duplicate kvlist write: GNU assoc_insert keeps one slot
/// and replaces the value — the read returns `b` (GNU k3=[b]).
#[test]
fn kvlist_duplicate_write_single_cell() {
    let (stdout, stderr, _) = rubash(
        "declare -A K3; K3=( [\"pre$'x'post\"]=a [\"pre$'x'post\"]=b )\n\
         printf '%s' \"${!K3[@]}\" | od -An -tx1 | tr -d ' \\n'; printf '\\n'\n\
         printf '[%s]\\n' \"${K3[\"pre$'x'post\"]}\"",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "70726524277827706f7374\n[b]\n");
}

/// The variable spelling (k = the exact listed bytes) hits (GNU [w]).
#[test]
fn kvlist_variable_spelling_read_hits() {
    let (stdout, stderr, _) = rubash(
        "k=\"pre\\$'x'post\"\ndeclare -A K4; K4=( [\"pre$'x'post\"]=w )\n\
         printf '[%s]\\n' \"${K4[$k]}\"",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "[w]\n");
}

/// Discriminating row on master (count=2): a clean-spelling overwrite must
/// replace the kvlist cell, not add a hidden-byte twin (GNU count=1).
#[test]
fn kvlist_clean_overwrite_is_one_cell() {
    let (stdout, stderr, _) = rubash(
        "k=\"pre\\$'x'post\"\ndeclare -A K4; K4=( [\"pre$'x'post\"]=w )\n\
         K4[$k]=NEW\n\
         printf '%s\\n' \"${#K4[@]}\"\n\
         printf '[%s]\\n' \"${K4[$k]}\"",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "1\n[NEW]\n");
}

/// Round trip via the listed key: reads back the value (GNU [v]).
#[test]
fn kvlist_round_trip_via_listed_key() {
    let (stdout, stderr, _) = rubash(
        "declare -A K5; K5=( [\"pre$'x'post\"]=v )\n\
         for kk in \"${!K5[@]}\"; do printf '[%s]\\n' \"${K5[$kk]}\"; done",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "[v]\n");
}

/// Non-regression (the E1 sq-piece family, assoc15.sub): a span INSIDE a
/// kvlist value decodes — key `a$bc` rides verbatim (GNU 61 24 62 63) and
/// the same-spelling read hits.
#[test]
fn kvlist_sq_piece_key_unchanged() {
    let (stdout, stderr, _) = rubash(
        "declare -A K6; K6=( ['a$'b'c']=z )\n\
         printf '%s' \"${!K6[@]}\" | od -An -tx1 | tr -d ' \\n'; printf '\\n'\n\
         printf '[%s]\\n' \"${K6['a$'b'c']}\"",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "61246263\n[z]\n");
}

/// Non-regression (the bare-span kvlist spelling): the span DECODES in the
/// bare shape (GNU key `prexpost`, 70 72 65 78 70 6f 73 74). The `tr -d ' \n'`
/// eats od's newline, so the listing has no trailing newline (GNU face).
#[test]
fn kvlist_bare_span_key_decodes() {
    let (stdout, stderr, _) = rubash(
        "declare -A K7; K7=( [pre$'x'post]=y )\n\
         printf '%s' \"${!K7[@]}\" | od -An -tx1 | tr -d ' \\n'",
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "70726578706f7374");
}

/// Non-regression (#857): the plain assignment entry keeps storing and
/// reading the raw dq-wrapped spelling.
#[test]
fn plain_assignment_entry_unchanged() {
    let (stdout, stderr, _) =
        rubash("declare -A M; M[\"pre$'x'post\"]=w\nprintf '[%s]\\n' \"${M[\"pre$'x'post\"]}\"");
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "[w]\n");
}
