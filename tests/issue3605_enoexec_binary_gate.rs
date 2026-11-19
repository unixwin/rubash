//! rubash#3605 (execscript cluster) — the ENOEXEC classification's
//! binary gate: a file the kernel refuses with ENOEXEC must be
//! classified by its FIRST BYTES before any fallback, exactly as GNU
//! shell_execve does (execute_cmd.c:6139-6233; check_binary_file is
//! general.c:718-741). On Darwin the local kernel loads Mach-O, not
//! ELF — treating `\x7fELF` as a native image skipped the
//! classification entirely, so a corrupt ELF operand fell into libc's
//! silent /bin/sh spawn substitution and its bytes ran as a script
//! (`./x: line 1: ELFgarbage: command not found`, rc 127) where GNU
//! refuses it: `bash: line 1: ./x: cannot execute binary file: Exec
//! format error`, EX_BINARY_FILE = 126 (internal_error's
//! `%s: %s: %s`, strerror(ENOENTEXEC) appended).
//!
//! Pinned against GNU bash 5.3.20 on macOS arm64:
//!   bash -c './x'          -> bash: line 1: ./x: cannot execute binary file: Exec format error (126)
//!   bash -c 'l2\n./x'      -> bash: line 2: ./x: cannot execute binary file: Exec format error (126)
//!   bash -c './nul'        -> bash: line 1: ./nul: cannot execute binary file: Exec format error (126)
//!   bash -c './mz'         -> ./mz: line 1: $'MZgarbage\r': command not found (127; MZ has no
//!                             NUL in the first line and is not ELF, so GNU runs it as a script)
//!   bash -c './t.sh'       -> ran (non-#! text keeps the ENOEXEC script fallback)

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3605execbin-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn place(dir: &std::path::Path, name: &str, bytes: &[u8]) {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("write operand");
    let mut perms = fs::metadata(&path).expect("stat operand").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod operand");
}

fn run_c(dir: &std::path::Path, script: &str) -> (String, String, i32) {
    let output = Command::new(RUBASH)
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn elf_operand_is_refused_not_run_as_script() {
    let dir = scratch("elf");
    place(&dir, "x", b"\x7fELFgarbage");
    let (_stdout, stderr, code) = run_c(&dir, "./x");
    assert_eq!(code, 126, "stderr was: {stderr}");
    // The prolog is $0 verbatim (GNU error.c:90-118
    // get_name_for_error): the invocation path itself.
    assert_eq!(
        stderr.trim_end_matches(['\r', '\n']),
        format!("{RUBASH}: line 1: ./x: cannot execute binary file: Exec format error")
    );
    let (_stdout, stderr, code) = run_c(&dir, "echo l2\n./x");
    assert_eq!(code, 126, "stderr was: {stderr}");
    assert!(
        stderr.contains("line 2: ./x: cannot execute binary file: Exec format error"),
        "the command's own line is reported, got: {stderr:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nul_in_first_line_is_a_binary_refusal() {
    let dir = scratch("nul");
    place(&dir, "nul", b"ab\x00cd");
    let (_stdout, stderr, code) = run_c(&dir, "./nul");
    assert_eq!(code, 126, "stderr was: {stderr}");
    assert!(
        stderr.contains("cannot execute binary file: Exec format error"),
        "NUL in the first line refuses like ELF magic, got: {stderr:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn non_binary_bytes_keep_the_script_fallback() {
    let dir = scratch("text");
    // MZ bytes are not ELF magic and carry no NUL: GNU runs the file
    // as a script (the decode turns the CR into $'\r' carrier text).
    place(&dir, "mz", b"MZgarbage\r\n");
    let (_stdout, stderr, code) = run_c(&dir, "./mz");
    assert_eq!(code, 127, "stderr was: {stderr}");
    assert!(
        stderr.contains("command not found"),
        "non-binary ENOEXEC bytes run as a script, got: {stderr:?}"
    );
    // Plain text keeps the fallback outright.
    place(&dir, "t.sh", b"echo ran\n");
    let (stdout, _stderr, code) = run_c(&dir, "./t.sh");
    assert_eq!(code, 0, "stdout was: {stdout}");
    assert_eq!(stdout.trim_end_matches(['\r', '\n']), "ran");
    let _ = fs::remove_dir_all(&dir);
}
