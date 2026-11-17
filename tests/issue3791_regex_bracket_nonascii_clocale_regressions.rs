//! rubash#3791 — in the C locale `[[ =~ ]]` rejected every bracket
//! expression holding a non-ASCII character with "invalid regular
//! expression": the regex crate in bytes mode (unicode(false))
//! refuses any non-ASCII char inside a class ("Unicode not allowed
//! here").
//!
//! GNU owner: lib/sh/shmatch.c:79-86 sh_regmatch() hands the pattern
//! to libc regcomp with REG_EXTENDED only; in the C locale glibc reads
//! pattern and subject as bytes, so a literal multibyte char inside a
//! bracket is the set of its UTF-8 bytes ([é] is {0xC3, 0xA9},
//! [é-ÿ] is the byte range 0xA9-0xC3 plus 0xC3/0xBF). A lone byte
//! 0xC3 matches [é] and ^[é]$ alike; the two-byte é matches [é] but
//! not ^[é]$.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! LC_ALL=C and C.utf8).
//!
//! Residual (out of scope, not #3791): a lone invalid byte as the
//! SUBJECT (`[[ $'\xc3' =~ [é] ]]`, GNU m1) still does not match —
//! the subject reaches the byte-mode matcher as a U+E000 raw-byte
//! marker pair, not the raw byte. That is the subject-side marker
//! decoding face, adjacent to #3545; it needs its own fix + nails.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3791-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, body: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .env("LC_ALL", lc_all)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const C_PROBE: &str = "\
[[ é =~ [é] ]]; echo \"bracket=$?\"\n\
[[ é =~ [é-ÿ] ]]; echo \"range=$?\"\n\
[[ a =~ [^é] ]]; echo \"neg=$?\"\n\
[[ é =~ ^[é]$ ]]; echo \"anchored=$?\"\n\
[[ x =~ [à-ÿ] ]]; echo \"agrave=$?\"\n\
[[ 😀 =~ ^[😀]$ ]]; echo \"emoji=$?\"\n\
[[ é =~ [[:alpha:]] ]]; echo \"alpha=$?\"\n\
[[ aé =~ é ]]; echo \"literal=$?\"\n\
[[ é =~ ^.$ ]]; echo \"dot=$?\"\n\
[[ é =~ [à-ÿ] ]]; echo \"agrave-eacute=$?\"\n\
[[ ÿ =~ [é-ÿ] ]]; echo \"range-y=$?\"\n\
p='\\é'; [[ é =~ $p ]]; echo \"bslash=$?\"\n\
[[ é =~ [é]{2} ]]; echo \"repeat=$?\"\n\
[[ é =~ ([é]) ]]; echo \"group=$?\"\n\
shopt -s nocasematch; [[ É =~ [é] ]]; echo \"nocase=$?\"\n";

const UTF8_PROBE: &str = "[[ é =~ ^[é]$ ]]; echo \"utf8=$?\"\n";

#[test]
fn cond_regex_bracket_nonascii_c_locale() {
    let (stdout, stderr, code) = run_script("c", C_PROBE, "C");
    assert_eq!(
        stdout,
        "bracket=0\nrange=0\nneg=0\nanchored=1\nagrave=1\nemoji=1\nalpha=1\nliteral=0\ndot=1\nagrave-eacute=0\nrange-y=0\nbslash=0\nrepeat=0\ngroup=0\nnocase=0\n",
    );
    assert_eq!(stderr, "", "unexpected stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn cond_regex_bracket_nonascii_utf8_control() {
    let (stdout, stderr, code) = run_script("utf8", UTF8_PROBE, "C.utf8");
    assert_eq!(stdout, "utf8=0\n");
    assert_eq!(stderr, "", "unexpected stderr: {stderr}");
    assert_eq!(code, Some(0));
}
