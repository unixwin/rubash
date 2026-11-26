//! rubash#3008: `printf %S` must fail in a single-byte locale.
//!
//! GNU (bash 5.3.20, builtins/printf.def) routes `%S` through the wide
//! conversion only when `locale_mb_cur_max > 1` (printf.def:532); in a
//! single-byte locale it falls to the PF macro, which hands the `%S`
//! format and the byte-string argument to the C library printf — and
//! glibc's `%S` reads the argument as `wchar_t *`, so the conversion
//! fails with EILSEQ. The PF macro then reports
//! `printf: Invalid or incomplete multibyte or wide character` via
//! builtin_error and PRETURNs with EXECUTION_FAILURE: no output from
//! the directive, status 1, and the rest of the format is never
//! processed (text already emitted before the directive stays).
//! In a multibyte locale the wide path (getwidestr/printwidestr,
//! printf.def:1546-1578,960-1060) prints the string normally.
//!
//! rubash's `%S` path (builtins/printf/value.rs `format_value`) never
//! asked the locale question in the failing direction: with the
//! active locale single-byte it fell into byte mode and printed the
//! argument as-is. The fix fails the conversion exactly when the
//! specifier is `S` and the active locale is not multibyte
//! (`locale::is_multi_byte_active`, the same question #2979's
//! byte_mode asks), emitting the EILSEQ diagnostic as a fatal
//! diagnostic and stopping the builtin — the PF/PRETURN shape.
//!
//! Scope pins: `%ls` and `%C` do NOT fail in C locale — GNU rewrites
//! `%ls` to `%s` before the PF call (printf.def:500-510, the modstart
//! rewrite) and `%C` takes the narrow getchr path — so the failure is
//! keyed on the `S` specifier alone, not on the wide flag.

#![cfg(unix)]

use std::process::{Command, Stdio};

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Run `script` (as a file) under rubash with a cleared environment
/// plus `envs`, returning (exit status, stdout bytes, stderr bytes).
fn run_case(script: &str, envs: &[(&str, &str)]) -> (i32, Vec<u8>, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut command = Command::new(rubash_bin());
    command
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir)
        .current_dir(&dir)
        .stdin(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code().unwrap_or(-1),
        output.stdout,
        output.stderr,
    )
}

const EILSEQ_MSG: &str = "printf: Invalid or incomplete multibyte or wide character";

fn assert_fails(script: &str, envs: &[(&str, &str)], expected_stdout: &[u8]) {
    let (status, stdout, stderr) = run_case(script, envs);
    assert_eq!(
        status,
        1,
        "status must be 1, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    assert!(
        String::from_utf8_lossy(&stderr).contains(EILSEQ_MSG),
        "stderr must name the EILSEQ failure, got: {}",
        String::from_utf8_lossy(&stderr)
    );
}

fn assert_prints(script: &str, envs: &[(&str, &str)], expected_stdout: &[u8]) {
    let (status, stdout, stderr) = run_case(script, envs);
    assert_eq!(
        status,
        0,
        "status must be 0, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    assert!(
        stderr.is_empty(),
        "stderr must be empty, got: {}",
        String::from_utf8_lossy(&stderr)
    );
}

const C: &str = "C";
const UTF8: &str = "C.UTF-8";

#[test]
fn issue3008_percent_s_c_locale_ascii_fails() {
    // The minimal repro from the issue: GNU errors, no stdout, rc 1.
    assert_fails("LC_ALL=C; printf \"%S\\n\" hello\n", &[], b"");
}

#[test]
fn issue3008_percent_s_c_locale_non_ascii_fails() {
    // Non-ASCII argument fails the same way in C locale.
    assert_fails("LC_ALL=C; printf \"%S\\n\" café\n", &[], b"");
}

#[test]
fn issue3008_percent_s_c_locale_from_process_env_fails() {
    // Same failure when C arrives via the process environment.
    assert_fails("printf \"%S\\n\" hello\n", &[("LC_ALL", C)], b"");
}

#[test]
fn issue3008_percent_s_utf8_control_prints() {
    // Control grid: C.UTF-8 prints, rc 0, both ASCII and non-ASCII.
    assert_prints("LC_ALL=C.UTF-8; printf \"%S\\n\" hello\n", &[], b"hello\n");
    assert_prints(
        "LC_ALL=C.UTF-8; printf \"%S\\n\" café\n",
        &[],
        b"caf\xc3\xa9\n",
    );
}

#[test]
fn issue3008_percent_s_c_locale_missing_arg_fails() {
    // A missing argument still takes the failing conversion (GNU rc 1).
    assert_fails("LC_ALL=C; printf \"%S\\n\"\n", &[], b"");
}

#[test]
fn issue3008_percent_s_c_locale_prefix_kept_suffix_dropped() {
    // PRETURN aborts at the failing directive: text before it stays,
    // the directive emits nothing, and nothing after it is processed.
    assert_fails("LC_ALL=C; printf \"pre %S post\\n\" hello\n", &[], b"pre ");
}

#[test]
fn issue3008_percent_s_c_locale_width_padding_not_emitted() {
    // No padding is emitted for the failed directive: the PF path
    // fails inside libc printf, so the width never materializes.
    // The literal `[` before the directive stays (PRETURN flushes
    // what was already emitted) — captain probe against GNU bash
    // 5.3.20: stdout is exactly "[", rc 1. The `]` and the newline
    // after the directive are never processed.
    assert_fails("LC_ALL=C; printf \"[%5S]\\n\" hello\n", &[], b"[");
}

#[test]
fn issue3008_percent_s_c_locale_v_store_fails() {
    // -v: the store keeps what was rendered before the failure (here
    // nothing) and the builtin still fails with rc 1.
    let (status, stdout, stderr) = run_case(
        "LC_ALL=C; printf -v v \"%S\" hello; rc=$?; printf 'rc=%s v=<%s>\\n' \"$rc\" \"$v\"\n",
        &[],
    );
    assert_eq!(
        status,
        0,
        "outer script must succeed, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"rc=1 v=<>\n".to_vec(), "stdout mismatch");
    assert!(
        String::from_utf8_lossy(&stderr).contains(EILSEQ_MSG),
        "stderr must name the EILSEQ failure, got: {}",
        String::from_utf8_lossy(&stderr)
    );
}

#[test]
fn issue3008_percent_s_follows_printf_v_locale_flip() {
    // The failure follows the variable-table locale (#2979's guard),
    // not the process environment: flipping down via `printf -v`
    // makes %S fail, flipping up makes it print.
    assert_fails(
        "printf -v LC_ALL C; printf \"%S\\n\" hello\n",
        &[("LC_ALL", UTF8)],
        b"",
    );
    assert_prints(
        "printf -v LC_ALL C.UTF-8; printf \"%S\\n\" hello\n",
        &[("LC_ALL", C)],
        b"hello\n",
    );
}

#[test]
fn issue3008_percent_ls_and_percent_c_c_locale_still_print() {
    // Scope guard: only the `S` specifier fails in C locale. GNU
    // rewrites `%ls` to `%s` (printf.def:500-510) and `%C` takes the
    // narrow path, so both still print.
    assert_prints("LC_ALL=C; printf \"%ls\\n\" hello\n", &[], b"hello\n");
    assert_prints("LC_ALL=C; printf \"%C\\n\" hello\n", &[], b"h\n");
}
