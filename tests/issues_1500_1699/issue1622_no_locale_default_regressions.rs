//! rubash#1622 — with NO locale variable set at all (or all of them
//! empty), GNU bash runs in the C locale: its startup setlocale(LC_ALL,
//! "") finds nothing to activate (locale.c set_default_locale), so
//! character semantics are byte semantics. rubash used to keep a
//! default UTF-8 model for the unset case (#1464 fixed the
//! explicit-locale faces only), so `${#é}` counted 1 character and the
//! quoting forms emitted raw UTF-8 where GNU escapes octal bytes.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! `env -i` and `LANG=` runs). Byte-form inputs keep this file ASCII.

use std::process::Command;

fn rubash_env(script: &str, envs: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script).env_clear();
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const PROBE: &str = "v=$'\\xc3\\xa9'; echo \"len=${#v}\"; printf '%q\\n' \"$v\"; echo \"Q=${v@Q}\"";

#[test]
fn unset_locale_defaults_to_c_locale_byte_semantics() {
    let (stdout, stderr, code) = rubash_env(PROBE, &[]);
    assert_eq!(stdout, "len=2\n$'\\303\\251'\nQ=$'\\303\\251'\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn empty_lang_defaults_to_c_locale_byte_semantics() {
    let (stdout, stderr, code) = rubash_env(PROBE, &[("LANG", "")]);
    assert_eq!(stdout, "len=2\n$'\\303\\251'\nQ=$'\\303\\251'\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn explicit_utf8_locale_keeps_character_semantics() {
    let (stdout, stderr, code) = rubash_env(PROBE, &[("LC_ALL", "C.UTF-8")]);
    assert_eq!(stdout, "len=1\n\u{e9}\nQ='\u{e9}'\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
