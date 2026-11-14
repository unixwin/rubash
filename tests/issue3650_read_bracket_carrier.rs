//! rubash#3650 — a `read`-filled value containing an assignment-carrier
//! byte (0x0c, 0x11, 0x13–0x1f) compares unequal to the identical literal
//! under `[`/`test` in the C locale, while `[[ ]]` agrees with GNU for
//! most of them and the value's bytes are identical on both sides.
//!
//! Root cause (same as #3489): the `read` builtin's single-byte-locale
//! byte domain (read_split.rs `read_byte_domain`, rubash#2819). A record
//! served from a byte transport is first carrier-tagged
//! (`tag_read_carrier_text`: every ASCII carrier byte rides as a raw-byte
//! marker pair). Those pairs are non-ASCII, so under `LC_ALL=C` the record
//! took the byte domain, and `byte_domain_value` turned the fields back
//! into text by emitting every byte < 0x80 as a bare char — undoing the
//! tagging. `[` (builtins/test.rs `left == right`) compares the internal
//! forms literally, so the bare read value no longer matches the tagged
//! `$'...'` literal; `[[ ]]` decodes both sides first, so it agrees
//! except for the four bytes (0x11/0x13/0x14/0x16) the lexer claims
//! elsewhere too. GNU: read.def:838-845 escapes CTLESC/CTLNUL in the
//! record and read.def:1127/:1061 `dequote_string` undoes it, so the
//! bound value is byte-identical to the input in every locale.
//!
//! This pin covers the `[` comparison face; the formatter face of the
//! same root cause is pinned by #3547 (tests/issue3489_*.rs), whose
//! one-line `byte_domain_value` fix also resolves every cell here.
//! Expectations are the GNU 5.3.20 oracle's (script file, `LC_ALL=C`).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_file(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3650-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    let script_path: PathBuf = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash script file");
    let _ = fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// One trigger byte under `[ = ]`: scalar `read` from a pipe, C locale.
fn assert_bracket_byte(hex: &str) {
    let script = format!(
        "export LC_ALL=C\nprintf 'A\\x{hex}B\\n' | {{ read x; [ \"$x\" = $'A\\x{hex}B' ] && echo PASS || echo FAIL; }}\n"
    );
    let (stdout, stderr, status) = rubash_file(&format!("bracket-{hex}"), &script);
    assert_eq!(stdout, "PASS\n", "byte 0x{hex}: stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// One trigger byte under `[[ == ]]` (only the four bytes that break it).
fn assert_double_bracket_byte(hex: &str) {
    let script = format!(
        "export LC_ALL=C\nprintf 'A\\x{hex}B\\n' | {{ read x; [[ \"$x\" == $'A\\x{hex}B' ]] && echo PASS || echo FAIL; }}\n"
    );
    let (stdout, stderr, status) = rubash_file(&format!("double-bracket-{hex}"), &script);
    assert_eq!(stdout, "PASS\n", "byte 0x{hex}: stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

#[test]
fn bracket_byte_0c() {
    assert_bracket_byte("0c");
}
#[test]
fn bracket_byte_11() {
    assert_bracket_byte("11");
}
#[test]
fn bracket_byte_13() {
    assert_bracket_byte("13");
}
#[test]
fn bracket_byte_14() {
    assert_bracket_byte("14");
}
#[test]
fn bracket_byte_15() {
    assert_bracket_byte("15");
}
#[test]
fn bracket_byte_16() {
    assert_bracket_byte("16");
}
#[test]
fn bracket_byte_17() {
    assert_bracket_byte("17");
}
#[test]
fn bracket_byte_18() {
    assert_bracket_byte("18");
}
#[test]
fn bracket_byte_19() {
    assert_bracket_byte("19");
}
#[test]
fn bracket_byte_1a() {
    assert_bracket_byte("1a");
}
#[test]
fn bracket_byte_1b() {
    assert_bracket_byte("1b");
}
#[test]
fn bracket_byte_1c() {
    assert_bracket_byte("1c");
}
#[test]
fn bracket_byte_1d() {
    assert_bracket_byte("1d");
}
#[test]
fn bracket_byte_1e() {
    assert_bracket_byte("1e");
}
#[test]
fn bracket_byte_1f() {
    assert_bracket_byte("1f");
}

#[test]
fn double_bracket_byte_11() {
    assert_double_bracket_byte("11");
}
#[test]
fn double_bracket_byte_13() {
    assert_double_bracket_byte("13");
}
#[test]
fn double_bracket_byte_14() {
    assert_double_bracket_byte("14");
}
#[test]
fn double_bracket_byte_16() {
    assert_double_bracket_byte("16");
}

/// The issue's seed: array `read -d ''` of an ANSI string, `[ = ]`.
#[test]
fn issue_seed_array_ansi_bracket() {
    let script = r#"export LC_ALL=C
out=$(printf "\033[0m\033[31;1m")
IFS=$'\n' read -d "" -r -a lines <<<"$out"
[ "${lines[0]}" = $'\033[0m\033[31;1m' ] && echo PASS || echo FAIL
"#;
    let (stdout, stderr, status) = rubash_file("seed", script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Array `read` of a single carrier byte, `[ = ]`.
#[test]
fn array_read_bracket_byte_1b() {
    let script = r#"export LC_ALL=C
printf "\033[0m\n" | { IFS=$'\n' read -r -a a; [ "${a[0]}" = $'\033[0m' ] && echo PASS || echo FAIL; }
"#;
    let (stdout, stderr, status) = rubash_file("array-1b", script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Scalar `read` from a regular file, `[ = ]`.
#[test]
fn scalar_read_file_bracket_byte_1b() {
    let script = r#"export LC_ALL=C
printf "\033[0m\n" > ./in.txt
read x < ./in.txt
[ "$x" = $'\033[0m' ] && echo PASS || echo FAIL
"#;
    let (stdout, stderr, status) = rubash_file("file-1b", script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Control: a plain-ASCII read value compares equal (green on the base).
#[test]
fn ascii_control_bracket() {
    let script = r#"export LC_ALL=C
printf "hello\n" | { read x; [ "$x" = hello ] && echo PASS || echo FAIL; }
"#;
    let (stdout, stderr, status) = rubash_file("ascii", script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Control: a `mapfile`-filled value compares equal (green on the base).
/// NOTE: `mapfile` must not sit in a pipeline — there it runs in a
/// subshell and the array does not survive (GNU fails that form too);
/// the file redirect keeps it in the main shell.
#[test]
fn mapfile_control_bracket() {
    let script = r#"export LC_ALL=C
printf "\033[0m\n" > ./in.txt
mapfile -t lines < ./in.txt
[ "${lines[0]}" = $'\033[0m' ] && echo PASS || echo FAIL
"#;
    let (stdout, stderr, status) = rubash_file("mapfile", script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Control: under a multibyte locale the byte domain never engages, so
/// the comparison passes even on the base.
#[test]
fn multibyte_locale_control_bracket() {
    let script = r#"export LC_ALL=C.UTF-8
out=$(printf "\033[0m\033[31;1m")
IFS=$'\n' read -d "" -r -a lines <<<"$out"
[ "${lines[0]}" = $'\033[0m\033[31;1m' ] && echo PASS || echo FAIL
"#;
    let (stdout, stderr, status) = rubash_file("utf8", script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}
