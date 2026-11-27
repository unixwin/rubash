//! unixwin/niubash#659 — `pwd -P` (and every PWD producer) must print the
//! POSIX getcwd form; the Windows extended-length `\\?\` / `//?/` prefix is
//! an artifact of `std::fs::canonicalize` and must never reach the display
//! face.
//!
//! GNU ground truth: builtins/common.c:592 `get_working_directory` and
//! builtins/cd.def (bindpwd / pwd) report a getcwd string after a real
//! chdir — never a verbatim form. The rubash-side root cause (G-02,
//! 501a6d82) replaced cd/pushd/popd's real chdir+getcwd with a
//! canonicalize probe and was fixed at the mechanism exits by 790a29f7
//! (PR #1995): `canonicalize_native` strips the prefix for the
//! session-cwd probes and `shell_pwd_display_path` strips `//?/` /
//! `//?/UNC/` at the one display exit every PWD producer shares. The four
//! in-lib pins (`issue659_*`) cover those units; this file pins the whole
//! binary-level face the issue reported — `cd <temp> && pwd -P` — so a
//! future producer that bypasses the display exit fails here too.
//!
//! Deterministic: the operand is the host temp directory (stable), the
//! expectation is derived from the same directory via canonicalize, and
//! the primary contract is the negative pin: no `//?/` prefix.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// `canonicalize` returned to the getcwd form — the same normalization
/// `executor::path::strip_verbatim_prefix` performs (mirrored here because
/// that helper is pub(crate)).
fn canonical_native(path: &std::path::Path) -> PathBuf {
    let resolved = path.canonicalize().expect("canonicalize temp dir");
    let value = resolved.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        resolved
    }
}

/// The POSIX `/<drive>/…` display form of a native absolute path — the
/// same normalization `shell_pwd_display_path` performs (rubash#537).
fn posix_display_form(path: &std::path::Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    let value = if let Some(rest) = value.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else if let Some(rest) = value.strip_prefix("//?/") {
        rest.to_string()
    } else {
        value
    };
    let bytes = value.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        format!("/{}/{}", drive, &value[3..])
    } else {
        value
    }
}

/// Spawn rubash non-interactively and require a zero exit.
fn run_rubash(script: &str) -> (String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("--norc")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .env_remove("WINUXSH_SHELL_PATH_STYLE")
        .env_remove("NIU_SHELL_PATH_STYLE")
        .env_remove("__RUBASH_PATH_STYLE");
    let output = command.output().expect("spawn rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "rubash exited {:?}: {stderr}",
        output.status.code()
    );
    (String::from_utf8_lossy(&output.stdout).into_owned(), stderr)
}

fn single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The issue's repro face: `cd <temp> && pwd -P`. The physical form must
/// be the POSIX getcwd spelling, never `//?/…`.
#[test]
fn issue659_pwdp_of_temp_path_is_posix_getcwd_form() {
    let temp = canonical_native(&std::env::temp_dir());
    let operand = temp.to_string_lossy().replace('\\', "/");
    let expected = posix_display_form(&temp);

    let (stdout, stderr) = run_rubash(&format!("cd {} && pwd -P", single_quoted(&operand)));
    let line = stdout.trim_end_matches(['\r', '\n']);
    assert!(
        !line.starts_with("//?/"),
        "pwd -P leaked the extended-length prefix: {line}"
    );
    assert_eq!(line, expected, "stderr: {stderr}");
}

/// `cd -P` binds the physical form into $PWD (bindpwd) — the same-source
/// pollution the issue notes must stay clean too.
#[test]
fn issue659_cd_dash_p_binds_posix_physical_pwd() {
    let temp = canonical_native(&std::env::temp_dir());
    let operand = temp.to_string_lossy().replace('\\', "/");
    let expected = posix_display_form(&temp);

    let (stdout, stderr) = run_rubash(&format!(
        "cd -P {} && pwd -P && printf '%s\\n' \"$PWD\"",
        single_quoted(&operand)
    ));
    for (idx, line) in stdout.lines().enumerate() {
        assert!(
            !line.starts_with("//?/"),
            "line {idx} leaked the extended-length prefix: {line}"
        );
        assert_eq!(line, expected, "line {idx}; stderr: {stderr}");
    }
}

/// The issue's impact statement: `//?/` inside a command substitution made
/// string comparison against the expected path permanently false.
#[test]
fn issue659_cmdsub_string_equality_on_physical_pwd() {
    let temp = canonical_native(&std::env::temp_dir());
    let operand = temp.to_string_lossy().replace('\\', "/");
    let expected = posix_display_form(&temp);

    let (stdout, stderr) = run_rubash(&format!(
        "[ \"$(cd {} && pwd -P)\" = {} ] && echo EQUAL || echo MISMATCH",
        single_quoted(&operand),
        single_quoted(&expected)
    ));
    assert_eq!(
        stdout.trim_end_matches(['\r', '\n']),
        "EQUAL",
        "stderr: {stderr}"
    );
}
