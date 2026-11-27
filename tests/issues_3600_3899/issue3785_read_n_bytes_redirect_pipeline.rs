//! rubash#3785 — `read -n`/`-N` counts characters instead of bytes in a
//! single-byte locale on redirected / pipeline stdin (residual of #3334).
//!
//! #3477 (Refs #3334) fixed the inherited-stdin serve only: it taught
//! `StdinCharDecoder` that one byte is one unit when `!is_multi_byte()`.
//! Every other serve still counted characters in `LC_ALL=C`:
//! - the slurp serves (`< file`, `0<file`, `<<<`, heredocs) cut the record
//!   with `trim_read_input` → `truncate_read_post_escape`, whose
//!   marker-free walk counted `chars()`; the EOF verdict walk in
//!   `slurped_record_eof_no_delimiter` counted the same way, so a
//!   budget-satisfied read over a short input also reported EOF failure;
//! - the pipeline serve (`read_function_stdin`, the FUNCTION_STDIN mirror)
//!   ran its character scan whenever `byte_unit_scan_applies` declined,
//!   and that gate only fired for raw-byte markers in a multibyte locale.
//!
//! GNU builtins/read.def:851,874 only calls read_mbchar when
//! `mb_cur_max > 1`, so with MB_CUR_MAX == 1 every byte is one unit
//! (`nr++` per byte, read.def:880-886): `read -n 2` over `é` (c3 a9)
//! takes both bytes of the first `é` on every stdin shape. The fix routes
//! all three serves through the byte-domain unit model
//! (`decoded_unit_len` with the locale flag).
//!
//! GNU bash 5.3.20 oracle expectations, from the issue's regression seed
//! (verified: GNU 10/10; rubash 70e962b9 4/10 — the four passing cases are
//! the inherited-stdin, group-redirect, C.utf8 and ASCII controls below).
//! Input bytes are UTF-8 `é` = `c3 a9` throughout; expectations are
//! asserted byte-for-byte with `od`-style hex in the seed, here with raw
//! stdout bytes.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const INPUT: &[u8] = b"\xc3\xa9\xc3\xa9\n";
const INPUT_NO_NL: &[u8] = b"\xc3\xa9";

fn run_c_stdin(script: &str, locale: &str, stdin: &[u8]) -> (Vec<u8>, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", locale)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_script_file(
    name: &str,
    script: &str,
    files: &[(&str, &[u8])],
    locale: &str,
) -> (Vec<u8>, String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash-issue3785-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    for (file_name, bytes) in files {
        std::fs::write(dir.join(file_name), bytes).expect("write probe input");
    }
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env("LC_ALL", locale)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_bytes(stdout: &[u8], stderr: &str, code: Option<i32>, expected: &[u8]) {
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, expected);
}

// --- `< file` redirect on the read itself ---

#[test]
fn read_n2_redirect_file_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_script_file(
        "n2-file",
        "read -n 2 x < in.bin; printf %s \"$x\"",
        &[("in.bin", INPUT)],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_upper_n2_redirect_file_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_script_file(
        "N2-file",
        "read -N 2 x < in.bin; printf %s \"$x\"",
        &[("in.bin", INPUT)],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n1_redirect_file_takes_first_byte_in_c_locale() {
    let (stdout, stderr, code) = run_script_file(
        "n1-file",
        "read -n 1 x < in.bin; printf %s \"$x\"",
        &[("in.bin", INPUT)],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3");
}

#[test]
fn read_n2_redirect_0file_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_script_file(
        "n2-0file",
        "read -n 2 x 0<in.bin; printf %s \"$x\"",
        &[("in.bin", INPUT)],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n2_redirect_file_succeeds_at_budget_without_newline() {
    // EOF verdict: a budget-satisfied read over a 2-byte input with no
    // delimiter is a limit stop, not EOF (GNU rc 0). The char-counting
    // verdict walk saw one unit and reported EOF failure (rc 1).
    let (stdout, stderr, code) = run_script_file(
        "n2-file-rc",
        "read -n 2 x < in2.bin; rc=$?; printf %s \"$x\"; echo \" rc=$rc\"",
        &[("in2.bin", INPUT_NO_NL)],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9 rc=0\n");
}

// --- here-string ---

#[test]
fn read_n2_here_string_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_script_file(
        "n2-herestring",
        "read -n 2 x <<< $'\\xc3\\xa9\\xc3\\xa9'; printf %s \"$x\"",
        &[],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_upper_n2_here_string_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_script_file(
        "N2-herestring",
        "read -N 2 x <<< $'\\xc3\\xa9\\xc3\\xa9'; printf %s \"$x\"",
        &[],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

// --- pipeline elements (FUNCTION_STDIN mirror) ---

#[test]
fn read_n2_pipeline_brace_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin(
        "printf '\\xc3\\xa9\\xc3\\xa9\\n' | { read -n 2 x; printf %s \"$x\"; }",
        "C",
        b"",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n2_pipeline_parens_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin(
        "printf '\\xc3\\xa9\\xc3\\xa9\\n' | ( read -n 2 x; printf %s \"$x\" )",
        "C",
        b"",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_upper_n2_pipeline_brace_counts_bytes_in_c_locale() {
    // -N over the same FUNCTION_STDIN mirror: the byte-domain scan is
    // shared with -n (char_limit + exact flag differ only in delimiter
    // handling), but the issue's DIFF list names only -n pipeline faces,
    // so pin -N here as well.
    let (stdout, stderr, code) = run_c_stdin(
        "printf '\\xc3\\xa9\\xc3\\xa9\\n' | { read -N 2 x; printf %s \"$x\"; }",
        "C",
        b"",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n2_pipeline_function_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin(
        "f() { read -n 2 x; printf %s \"$x\"; }; printf '\\xc3\\xa9\\xc3\\xa9\\n' | f",
        "C",
        b"",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n1_pipeline_sequential_reads_consume_byte_by_byte() {
    // The first cut lands mid-character (c3 of `é`), so the mirror
    // rewrite path must hand the continuation byte (a9) to the next read.
    let (stdout, stderr, code) = run_c_stdin(
        "printf '\\xc3\\xa9\\xc3\\xa9\\n' | { read -n 1 x; read -n 1 y; printf %s \"$x$y\"; }",
        "C",
        b"",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n2_pipeline_succeeds_at_budget_without_newline() {
    let (stdout, stderr, code) = run_c_stdin(
        "printf '\\xc3\\xa9' | { read -n 2 x; echo \"rc=$?\"; }",
        "C",
        b"",
    );
    assert_bytes(&stdout, &stderr, code, b"rc=0\n");
}

// --- `< /dev/stdin` redirect on the read itself ---

#[test]
fn read_n2_dev_stdin_counts_bytes_in_c_locale() {
    let (stdout, stderr, code) =
        run_c_stdin("read -n 2 x < /dev/stdin; printf %s \"$x\"", "C", INPUT);
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

// --- controls: the faces #3477 already fixed, and the locales that
// must keep counting characters ---

#[test]
fn read_n2_inherited_stdin_still_counts_bytes_in_c_locale() {
    // The #3334 form itself: must stay green.
    let (stdout, stderr, code) = run_c_stdin("read -n 2 x; printf %s \"$x\"", "C", INPUT);
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n2_group_redirect_still_counts_bytes_in_c_locale() {
    // Redirect on the enclosing group, read itself bare (fd-table File
    // endpoint serve — was already byte-correct).
    let (stdout, stderr, code) = run_script_file(
        "n2-group-redir",
        "{ read -n 2 x; printf %s \"$x\"; } < in.bin",
        &[("in.bin", INPUT)],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9");
}

#[test]
fn read_n2_pipeline_counts_characters_in_utf8_locale() {
    let (stdout, stderr, code) = run_c_stdin(
        "printf '\\xc3\\xa9\\xc3\\xa9\\n' | { read -n 2 x; printf %s \"$x\"; }",
        "C.utf8",
        b"",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9\xc3\xa9");
}

#[test]
fn read_n2_redirect_file_counts_characters_in_utf8_locale() {
    let (stdout, stderr, code) = run_script_file(
        "n2-file-utf8",
        "read -n 2 x < in.bin; printf %s \"$x\"",
        &[("in.bin", INPUT)],
        "C.utf8",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9\xc3\xa9");
}

#[test]
fn read_n2_redirect_file_ascii_control() {
    let (stdout, stderr, code) = run_script_file(
        "n2-file-ascii",
        "read -n 2 x < in.bin; printf %s \"$x\"",
        &[("in.bin", b"ab\n")],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"ab");
}

#[test]
fn plain_read_redirect_file_takes_whole_line() {
    // No -n/-N budget: the whole line is the record, untouched by the fix.
    let (stdout, stderr, code) = run_script_file(
        "plain-file",
        "read x < in.bin; printf %s \"$x\"",
        &[("in.bin", INPUT)],
        "C",
    );
    assert_bytes(&stdout, &stderr, code, b"\xc3\xa9\xc3\xa9");
}
