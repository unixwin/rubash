//! rubash#435 — multi-line `(( ))` arithmetic commands (the ble.sh family).
//!
//! GNU reads the arithmetic-command body through parse_matched_pair
//! (parse.y:4963 parse_arith_cmd via parse.y:4904 parse_dparen): the scan
//! crosses physical newlines, `read_token` hands the whole balanced
//! group to ARITH_CMD as raw text, and expr.c's lexer treats `\n` as
//! whitespace (parse.y:1203 arith_command). Newlines inside `(( ... ))`
//! are body characters, never command separators — comma-newline chains,
//! `&&`/`||`-newline continuations, nested ternary parens and `;` data
//! all parse and evaluate.
//!
//! These tests pin the six verbatim ble.sh constructs from the ecosystem
//! harvest (akinomyoga/ble.sh: src/color.sh:538, lib/init-term.sh:313,
//! lib/keymap.vi.sh:3487, src/canvas.sh:333, src/edit.sh:5329,
//! lib/core-syntax.sh:3975) against WSL GNU Bash 5.3.0 outputs captured
//! 2026-10-06 (script files, byte-for-byte). The engine already accepts
//! every shape on this base; the tests lock the class against
//! regression. The canvas wrapper (t4) is rejected by GNU AND rubash
//! identically — my truncated extraction broke the ternary chain's
//! balance — so it pins PARITY (rc 2, no output), not acceptance; the
//! full canvas.sh file passes `bash -n` on both engines.

use std::process::Command;

fn rubash(script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

/// src/color.sh:538 — no-space `((h1=...`, comma-newline chain, nested
/// ternary parens inside the arithmetic body.
#[test]
fn color_hxx2color_multiline_comma_chain() {
    let script = "\
f() {
  local H=$1 Min=$2 Range=$3 Unit=$4
  local h1 h2 x=$Min y=$Min z=$Min
  ((h1=H%120,h2=120-h1,
    x+=Range*(h2<60?h2:60)/60,
    y+=Range*(h1<60?h1:60)/60))
  ((x=x*255/Unit,
    y=y*255/Unit,
    z=z*255/Unit))
  echo \"$h1 $x $y $z\"
}
f 130 0 200 255";
    let (out, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(out, "10 200 33 0\n");
}

/// lib/init-term.sh:313 — nested-ternary group spanning four newlines.
#[test]
fn init_term_nested_ternary_multiline() {
    let script = "\
i=8 i1=1
((j1=(i1==3?6:
      (i1==6?3:
       (i1==1?4:
        (i1==4?1:i1))))))
echo \"$j1 $((i-i1+j1))\"";
    let (out, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(out, "4 11\n");
}

/// lib/keymap.vi.sh:3487 — comma-newline chain with `&&` groups, a
/// parenthesized assignment and a postfix decrement; `${COLUMNS:-eol}`
/// expansion inside the body.
#[test]
fn keymap_vi_comma_chain_with_paren_assignments() {
    let script = "\
index=3 bol=0 eol=10 COLUMNS=80
((index=(bol+${COLUMNS:-eol})/2,
  index>eol&&(index=eol),
  bol<eol&&index==eol&&(index--)))
echo \"$index $?\"";
    let (out, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(out, "9 0\n");
}

/// src/edit.sh:5329 — comma-newline chain into a ternary whose branches
/// are parenthesized assignment groups.
#[test]
fn edit_mark_ternary_paren_branches() {
    let script = "\
_ble_edit_ind=5 ibeg=1 iend=9 _ble_edit_mark=3 insert=xy
((_ble_edit_ind+=${#insert},
  _ble_edit_mark>ibeg&&(
    _ble_edit_mark<iend?(
      _ble_edit_mark=_ble_edit_ind
    ):(
      _ble_edit_mark=ibeg
    ))))
echo \"$_ble_edit_ind $_ble_edit_mark $?\"";
    let (out, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(out, "7 7 0\n");
}

/// lib/core-syntax.sh:3975 — `((` at command position inside a function
/// body, `${#...}` expansions and an array-element assignment inside the
/// multi-line body.
#[test]
fn core_syntax_function_body_multiline_chain() {
    let script = "\
f() {
  rematch1=a rematch2='(x'
  i=5
  local CTX_CMDXC=9 CTX_ARGX0=8
  ((_ble_bash>=990000)) || true
  if ((i>1)); then
    ((v1=i+1,v2=i+2,
      v3=v1+v2))
    ((${#rematch2}>=2&&(w[i+1]=CTX_CMDXC),
      i+=2))
  fi
  echo \"$v1 $v2 $v3 ${w[6]} $i\"
}
f";
    let (out, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(out, "6 7 13 9 7\n");
}

/// src/canvas.sh:333 shape (truncated wrapper): GNU and rubash reject the
/// unbalanced extraction identically — parse parity, rc 2, no stdout.
#[test]
fn canvas_truncated_wrapper_parity_rejection() {
    let script = "\
code=0x3200 ret=0
((
  0x3100<=code&&code<0xA4D0||0xAC00<=code&&code<0xD7A4?(
    ret=2
  ):(0x2000<=code&&code<0x2700?(
    ret=1
  ):(ret=0)))
))
echo \"$ret $?\"";
    let (out, code) = rubash(script);
    assert_eq!(code, Some(2));
    assert_eq!(out, "");
}

/// The harvest's headline shape: `&&`-joined parenthesized assignment and
/// `+=` across the newline, comma-separated (mission reproducer).
#[test]
fn mission_reproducer_and_comma_newline() {
    let (out, code) =
        rubash("a=1\n(( a&&(b=5),\nc+=${a} ))\necho \"rc=$? b=${b:-unset} c=${c:-unset}\"");
    assert_eq!(code, Some(0));
    assert_eq!(out, "rc=0 b=5 c=1\n");
}

/// Multi-line `(( ))` inside `eval` (ble.sh builds these programmatically).
#[test]
fn multiline_dparen_through_eval() {
    let (out, code) =
        rubash("a=1\neval '(( a&&(b=5),\nc+=${a} ))'\necho \"b=${b:-unset} c=${c:-unset}\"");
    assert_eq!(code, Some(0));
    assert_eq!(out, "b=5 c=1\n");
}

/// `-n` (noexec) accepts every shape — the harvest's rejection face.
#[test]
fn noexec_accepts_multiline_dparen_file() {
    let dir = std::env::temp_dir().join("rubash435pin");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("m435.sh");
    std::fs::write(
        &file,
        "a=1\n(( a&&(b=5),\nc+=${a} ))\n((h1=H%120,h2=120-h1,\n  x+=1))\n",
    )
    .expect("write temp script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg(&file)
        .output()
        .expect("run rubash -n");
    assert!(
        output.status.success(),
        "bash -n must accept a multi-line (( )) script: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
