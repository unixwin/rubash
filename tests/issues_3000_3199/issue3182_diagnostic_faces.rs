//! Regression pins for rubash#3182 — diagnostic faces carrying the
//! PUA marker characters (the C0/PUA carrier protocol, markers.rs):
//! every diagnostic that renders command text passes it through the
//! one statutory decoder (`locale::decode_to_visible_text`), so a
//! marker-encoded name surfaces in diagnostics as exactly the bytes
//! GNU prints, and the sibling exec classification audit found next
//! to it: a slash command whose parent component is a regular file
//! is ENOTDIR — GNU file_error (execute_cmd.c:6126-6159) reports
//! "Not a directory" at EX_NOEXEC (126), not ENOENT/127.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8, measured against the oracle
//! binary (probe battery in the lane dossier, shapes 3182-A1..A4).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3182-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn write_file(root: &Path, name: &str, mode: u32) {
    let path = root.join(name);
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("fixture mode");
}

/// A1: a quoted marker-encoded word used as a command name renders
/// in the not-found diagnostic as the marker character itself,
/// byte-identical to GNU (the decoder's visible-text contract).
#[test]
fn marker_name_quoted_command_not_found() {
    let root = scratch("a1");
    let out = run(&root, "v=$'\\uE314'\n\"$v\"\n");
    assert_eq!(out.rc, 127);
    assert_eq!(out.stderr, "case.sh: line 2: \u{E314}: command not found\n");
}

/// A2: the unquoted form takes the same decode path.
#[test]
fn marker_name_unquoted_command_not_found() {
    let root = scratch("a2");
    let out = run(&root, "v=$'\\uE314'\n$v\n");
    assert_eq!(out.rc, 127);
    assert_eq!(out.stderr, "case.sh: line 2: \u{E314}: command not found\n");
}

/// A3: a marker-encoded name with a slash whose parent component
/// is a regular file — the decoded name renders in the ENOTDIR
/// classification, status 126. (The battery's original cell relied
/// on the probe directory's leftover marker file; the fixture is
/// created explicitly here so the cell is self-contained.)
#[test]
fn marker_slash_name_not_a_directory() {
    let root = scratch("a3");
    std::fs::write(root.join("\u{E314}"), "x").expect("marker parent fixture");
    let out = run(&root, "v=$'\\uE314/x'\n\"$v\"\n");
    assert_eq!(out.rc, 126);
    assert_eq!(out.stderr, "case.sh: line 2: \u{E314}/x: Not a directory\n");
}

/// The file_error face prints the name RAW in every locale: in a
/// single-byte locale the not-found face's printable quoting
/// ($'\ooo') must not leak into the shell_execve classification
/// (GNU sys_error uses a plain %s, execute_cmd.c:6126-6159).
#[test]
fn slash_error_name_is_raw_in_single_byte_locale() {
    let root = scratch("a3-posix");
    std::fs::write(root.join("\u{E314}"), "x").expect("marker parent fixture");
    // The value embeds the character itself (in the C locale the
    // $'\u' escape stays literal in GNU too, so it cannot stage
    // this face); both shells must print its raw bytes.
    std::fs::write(root.join("case.sh"), "v='\u{E314}/x'\n\"$v\"\n").expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(&root)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(out.status.code().unwrap_or(-1), 126);
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).into_owned(),
        "case.sh: line 2: \u{E314}/x: Not a directory\n"
    );
}

/// A4: a marker-encoded redirect target is a plain filename — the
/// file is created under the decoded name, no diagnostic.
#[test]
fn marker_redirect_target_creates_file() {
    let root = scratch("a4");
    let out = run(&root, "v=$'\\uE314'\necho hi > \"$v\"\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stderr, "");
    let created = root.join("\u{E314}");
    assert_eq!(
        std::fs::read_to_string(created).expect("marker file"),
        "hi\n"
    );
}

/// A slash command through a regular file: ENOTDIR at 126
/// (execute_cmd.c:6126-6159 file_error), where a missing parent is
/// ENOENT at 127 — the classification pair.
#[test]
fn slash_command_through_regular_file_is_enotdir() {
    let root = scratch("enotdir");
    write_file(&root, "parentfile", 0o755);
    let out = run(&root, "parentfile/x\n");
    assert_eq!(out.rc, 126);
    assert_eq!(
        out.stderr,
        "case.sh: line 1: parentfile/x: Not a directory\n"
    );

    let out = run(&root, "notthere/x\n");
    assert_eq!(out.rc, 127);
    assert_eq!(
        out.stderr,
        "case.sh: line 1: notthere/x: No such file or directory\n"
    );
}

/// The `exec` builtin's slash face carries the same ENOTDIR
/// classification (builtins/exec.def via shell_execve), reported
/// under the full pathname.
#[test]
fn exec_builtin_through_regular_file_is_enotdir() {
    let root = scratch("exec-enotdir");
    write_file(&root, "parentfile", 0o755);
    let out = run(&root, "exec parentfile/x\n");
    assert_eq!(out.rc, 126);
    let full = root.join("parentfile/x");
    assert_eq!(
        out.stderr,
        format!(
            "case.sh: line 1: {}: Not a directory\n",
            full.to_string_lossy()
        )
    );
}

/// Contrast guard (unchanged at base): a non-executable slash path
/// is Permission denied at 126, a missing one ENOENT at 127.
#[test]
fn slash_command_permission_denied_contrast() {
    let root = scratch("eacces");
    write_file(&root, "plain", 0o644);
    let out = run(&root, "./plain\n");
    assert_eq!(out.rc, 126);
    assert_eq!(out.stderr, "case.sh: line 1: ./plain: Permission denied\n");
}
