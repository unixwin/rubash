//! Issue rubash#1635: carrier escape pairs leaked into the length/index
//! views of a value. A literal char from the marker registry zone (e.g.
//! U+E301 typed as $'\uE301') is stored as a U+E400 escape pair — ONE data
//! character — but `${v:1}`, `${v#?}` and the positional `${#1}` measured
//! transport units instead of data characters, while `${#v}` (which
//! decodes the pair) counted correctly. Raw-byte pairs (U+E000, $'\xff')
//! were already measured correctly on every view.
//!
//! Root causes fixed:
//! - src/executor/expand_braced_indices.rs: the special/positional branch
//!   of expand_braced_length_parameter used `chars().count()` on the raw
//!   transport text; every other leg already used parameter_char_length.
//! - src/executor/parameter_ops.rs: locale_byte_span only decoded when a
//!   raw-byte sentinel was present, so substring offsets/lengths walked
//!   the U+E400 pair as two units; it now decodes under the same condition
//!   parameter_char_length uses (raw-byte sentinel OR literal escape).
//! - src/executor/conditional/pattern.rs: the pattern-match unit staging
//!   collapsed U+E000 pairs to one unit but staged a U+E400 pair as two,
//!   so `?` could never match the one data character in ${v#?} / ${v%?}
//!   (and [[ == ]] / case). Both flatten functions now collapse the
//!   literal-escape pair to its payload char, mirroring the raw-byte pair.
//!
//! Expectations are the literal GNU bash 5.3.20 outputs for the same
//! scripts under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::process::Command;

fn run(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const PUA: char = '\u{E301}';

#[test]
fn issue1635_pua_pair_measures_one_data_char_on_every_view() {
    let out = run(r#"
v=$'\uE301'
echo "len=${#v} sub0=[${v:0}] sub1=[${v:1}] sub01=[${v:0:1}] sub11=[${v:1:1}] neg=[${v: -1}]"
echo "trimq=[${v#?}] trimqq=[${v##?}] sufq=[${v%?}] sufqq=[${v%%?}] trimstar=[${v#*}]"
"#);
    let expected = format!(
        "len=1 sub0=[{PUA}] sub1=[] sub01=[{PUA}] sub11=[] neg=[{PUA}]\n\
         trimq=[] trimqq=[] sufq=[] sufqq=[] trimstar=[{PUA}]\n"
    );
    assert_eq!(out, expected);
}

#[test]
fn issue1635_positional_and_local_lengths_use_data_units() {
    let out = run(r#"
v=$'\uE301'
w="a"$'\uE301'"b"
set -- "$v" "$w"
echo "pos len1=${#1} len2=${#2} sub=[${1:1}] trim=[${1#?}]"
f() { local x=$1; echo "local len=${#x} sub=[${x:1}] trim=[${x#?}]"; }
f "$v"
arr=("$v" z)
echo "elem len=${#arr[0]} sub=[${arr[0]:1}] trim=[${arr[0]#?}]"
"#);
    assert_eq!(
        out,
        "pos len1=1 len2=3 sub=[] trim=[]\n\
         local len=1 sub=[] trim=[]\n\
         elem len=1 sub=[] trim=[]\n"
    );
}

#[test]
fn issue1635_mixed_text_and_pair_units() {
    let out = run(r#"
v="a"$'\uE301'"b"
echo "len=${#v} sub1=[${v:1}] sub11=[${v:1:1}] sub2=[${v:2}]"
echo "trimq=[${v#?}] sufq=[${v%?}] trimstar=[${v#*}]"
u=$'\u0041\uE301'
echo "two len=${#u} sub1=[${u:1}] trimq=[${u#?}] sufq=[${u%?}]"
"#);
    let expected = format!(
        "len=3 sub1=[{PUA}b] sub11=[{PUA}] sub2=[b]\n\
         trimq=[{PUA}b] sufq=[a{PUA}] trimstar=[a{PUA}b]\n\
         two len=2 sub1=[{PUA}] trimq=[{PUA}] sufq=[A]\n"
    );
    assert_eq!(out, expected);
}

#[test]
fn issue1635_cond_and_case_match_pair_as_one_char() {
    let out = run(r#"
v=$'\uE301'
[[ $v == ? ]] && echo "q T" || echo "q F"
[[ $v == ?? ]] && echo "qq T" || echo "qq F"
case $v in ?) echo "case T";; *) echo "case F";; esac
"#);
    assert_eq!(out, "q T\nqq F\ncase T\n");
}

#[test]
fn issue1635_raw_byte_pair_views_unchanged() {
    // The U+E000 raw-byte encoding already measured as one unit on every
    // view before this fix; pin the byte-exact outputs so the pair
    // handling stays symmetric.
    let out = run(r#"
v=$'\xff'
echo "len=${#v}"
printf '%s' "${v:1}" | od -An -tx1 | tr -d ' \n'; echo " sub1"
printf '%s' "${v#?}" | od -An -tx1 | tr -d ' \n'; echo " trim"
printf '%s' "${v:0:1}" | od -An -tx1 | tr -d ' \n'; echo " sub01"
"#);
    assert_eq!(out, "len=1\n sub1\n trim\nff sub01\n");
}
