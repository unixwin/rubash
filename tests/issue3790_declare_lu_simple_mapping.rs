//! rubash#3790: `declare -l`/`-u` (and `local -l`/`-u`) applied Rust's
//! FULL Unicode case mappings instead of GNU's locale-aware SIMPLE
//! (single-character) mappings, and converted even in the C locale
//! where GNU converts nothing.
//!
//! GNU owner: make_variable_value (variables.c:2970-2986, CASEMOD_ATTRS
//! branch) runs the value through sh_modcase, whose per-character
//! towupper/towlower (lib/sh/casemod.c:46-47) are 1:1 mappings — `ß`
//! has no simple uppercase and stays `ß`, the `ﬀ` ligature stays,
//! U+0130 `İ` lowercases to plain `i` (its simple mapping; the full
//! mapping is `i` + U+0307). In a single-byte locale glibc's
//! towupper/towlower map ASCII only, so no non-ASCII character
//! converts at all.
//!
//! Expectations verified against GNU bash 5.2.21 (probe; the issue's
//! own evidence was gathered against GNU bash 5.3) with
//! LC_ALL=C.utf8 and LC_ALL=C. The `${x,,}`/`${x^^}` expansion path
//! already follows GNU (rubash#1063) and is covered here only as a
//! control — it must not change.

use std::process::Command;

fn rubash(locale: &str, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", locale)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const UTF8: &str = "C.utf8";
const C_LOCALE: &str = "C";

// --- UTF-8 locale: multi-character mappings must NOT apply ---

#[test]
fn issue3790_declare_lower_dotted_capital_i_is_plain_i() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -l a=İ; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=i\n");
}

#[test]
fn issue3790_declare_upper_sharp_s_stays() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -u b=ß; echo \"b=$b\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "b=ß\n");
}

#[test]
fn issue3790_declare_upper_ff_ligature_stays() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -u b=ﬀ; echo \"b=$b\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "b=ﬀ\n");
}

#[test]
fn issue3790_local_lower_dotted_capital_i() {
    let (stdout, stderr, code) = rubash(UTF8, "f() { local -l a=İÉ; echo \"a=$a\"; }; f");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=ié\n");
}

#[test]
fn issue3790_local_upper_sharp_s_dotless_i() {
    let (stdout, stderr, code) = rubash(UTF8, "f() { local -u b=ßı; echo \"b=$b\"; }; f");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "b=ßI\n");
}

#[test]
fn issue3790_declare_p_shows_simple_mapped_value() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -l a=İ; declare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "declare -l a=\"i\"\n");
}

// --- UTF-8 locale: single-character mappings are SAME, must not move ---

#[test]
fn issue3790_declare_lower_single_char_mapping() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -l a=É; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=é\n");
}

#[test]
fn issue3790_declare_upper_single_char_mapping() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -u b=é; echo \"b=$b\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "b=É\n");
}

#[test]
fn issue3790_declare_lower_ascii() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -l a=ABC; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=abc\n");
}

#[test]
fn issue3790_expansion_lower_control_unaffected() {
    // The ${x,,} path already follows GNU (rubash#1063): control only.
    let (stdout, stderr, code) = rubash(UTF8, "x=İ; echo \"${x,,}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "i\n");
}

// --- UTF-8 locale: capcase shares the same sh_modcase mechanism ---

#[test]
fn issue3790_declare_capcase_ff_ligature_first_char_stays() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -c a=ﬀoo; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=ﬀoo\n");
}

#[test]
fn issue3790_declare_capcase_ascii_still_capitalizes() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -c a=hello; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=Hello\n");
}

#[test]
fn issue3790_declare_capcase_dotted_capital_first_char() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -c a=İx; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=İx\n");
}

// --- UTF-8 locale: array elements run through the same conversion ---

#[test]
fn issue3790_array_lower_elements_simple_mapping() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -la a=(İÉ ß); echo \"${a[0]}|${a[1]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "ié|ß\n");
}

#[test]
fn issue3790_array_upper_elements_simple_mapping() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -ua a=(ß ﬀ); echo \"${a[0]}|${a[1]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "ß|ﬀ\n");
}

#[test]
fn issue3790_assoc_upper_element_value_simple_mapping() {
    let (stdout, stderr, code) = rubash(UTF8, "declare -uA h; h[k]=ß; echo \"${h[k]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "ß\n");
}

// --- C locale: no conversion at all ---

#[test]
fn issue3790_c_locale_declare_lower_no_conversion() {
    let (stdout, stderr, code) = rubash(C_LOCALE, "declare -l a=İ; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=İ\n");
}

#[test]
fn issue3790_c_locale_declare_upper_no_conversion() {
    let (stdout, stderr, code) = rubash(C_LOCALE, "declare -u b=ß; echo \"b=$b\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "b=ß\n");
}

#[test]
fn issue3790_c_locale_declare_upper_ff_ligature_no_conversion() {
    let (stdout, stderr, code) = rubash(C_LOCALE, "declare -u b=ﬀ; echo \"b=$b\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "b=ﬀ\n");
}

#[test]
fn issue3790_c_locale_declare_lower_eacute_no_conversion() {
    let (stdout, stderr, code) = rubash(C_LOCALE, "declare -l a=É; echo \"a=$a\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=É\n");
}

#[test]
fn issue3790_c_locale_array_lower_no_conversion() {
    let (stdout, stderr, code) = rubash(C_LOCALE, "declare -la a=(İÉ); echo \"${a[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "İÉ\n");
}
