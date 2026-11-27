//! Regression pins for rubash#1307 and rubash#1309 — errno fidelity in
//! diagnostics (the errno text and the prefix shape are the contract;
//! scripts branch on them).
//!
//! #1309: the in-process `cat` mapped EACCES to ENOENT and wrapped the
//! diagnostic in the shell's `line N:` prefix. GNU coreutils cat (and
//! GNU bash's own error shape for it):
//!   cat noperm        (chmod 000)  -> `cat: noperm: Permission denied`, rc 1
//!   old niu: `niu: line 1: cat: noperm: No such file or directory`
//! A missing file is `No such file or directory` on both — the mapping
//! must stay errno-true in both directions.
//!
//! #1307: `source` on an unreadable DIRECTORY took an is-a-directory
//! shortcut and added a `source:` prefix. GNU bash 5.3 (builtins/source
//! via find_shell_builtin / open path — the raw open() errno wins):
//!   source /data      (drwxrwx--x) -> `<file>: line 1: /data: Permission denied`
//!   old niu: `...: source: /data: is a directory`
//! EACCES on a regular file and ENOENT agreed on both shells already;
//! only the directory case diverged, in both the `source` and `.` forms.
//!
//! EACCES legs need a non-root euid (root bypasses permission checks),
//! so they self-skip under euid 0 — the same guard the #1404 pins use.
//! The directory `cat` leg (EISDIR) is root-safe and always runs.

use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
fn euid_is_root() -> bool {
    let uid = unsafe { libc::geteuid() };
    uid == 0
}

#[cfg(not(unix))]
fn euid_is_root() -> bool {
    // Off-unix there is no chmod/geteuid EACCES shape to observe;
    // treat as "cannot observe" so the EACCES legs skip there too.
    true
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-bfd-err-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_c(dir: &PathBuf, script: &str) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn run_script_file(dir: &PathBuf, name: &str, body: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join(name), body).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(name)
        .current_dir(dir)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[cfg(unix)]
fn set_mode(path: &PathBuf, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

#[test]
fn issue1309_cat_directory_reports_is_a_directory() {
    let dir = scratch("1309dir");
    std::fs::create_dir_all(dir.join("somedir")).expect("mkdir");
    let (_o, stderr, code) = run_c(&dir, "cat somedir");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("cat: somedir: Is a directory"),
        "cat on a directory reports EISDIR with the bare cat: prefix, got: {stderr:?}"
    );
    assert!(
        !stderr.contains("No such file or directory") && !stderr.contains("line 1"),
        "no errno remap, no line prefix: {stderr:?}"
    );
}

#[test]
fn issue1309_cat_eacces_reports_permission_denied() {
    if euid_is_root() {
        return; // root bypasses chmod 000; EACCES is not observable.
    }
    let dir = scratch("1309eacces");
    let file = dir.join("noperm");
    std::fs::write(&file, b"secret").expect("write noperm");
    #[cfg(unix)]
    set_mode(&file, 0o000);
    let (_o, stderr, code) = run_c(&dir, "cat noperm; echo rc=$?");
    #[cfg(unix)]
    set_mode(&file, 0o644);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        code,
        Some(0),
        "wrapper echo runs; cat's own rc is in the echo"
    );
    assert!(
        stderr.contains("cat: noperm: Permission denied"),
        "cat on EACCES file reports Permission denied, got: {stderr:?}"
    );
    assert!(
        !stderr.contains("No such file or directory") && !stderr.contains("line 1"),
        "EACCES must not be remapped to ENOENT or line-prefixed: {stderr:?}"
    );
}

#[test]
fn issue1309_cat_missing_file_reports_enoent_control() {
    let dir = scratch("1309missing");
    let (_o, stderr, code) = run_c(&dir, "cat missing_xyz");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("cat: missing_xyz: No such file or directory"),
        "missing file keeps the ENOENT wording, got: {stderr:?}"
    );
}

#[test]
fn issue1307_source_eacces_directory_reports_permission_denied() {
    if euid_is_root() {
        return; // root can open the directory; EACCES is not observable.
    }
    let dir = scratch("1307");
    let data = dir.join("data");
    std::fs::create_dir_all(&data).expect("mkdir data");
    #[cfg(unix)]
    set_mode(&data, 0o111); // execute-only: open() for read fails EACCES
    for (tag, form) in [("source", "source ./data x"), ("dot", ". ./data x")] {
        let (_o, stderr, code) =
            run_script_file(&dir, "repro.sh", &format!("{form}; echo rc=$?\n"));
        assert!(
            stderr.contains("./data: Permission denied"),
            "{tag} form: GNU reports the raw open errno with no builtin prefix, got: {stderr:?}"
        );
        assert!(
            !stderr.contains("is a directory")
                && !stderr.contains(": source:")
                && !stderr.contains(": .:"),
            "{tag} form: no is-a-directory shortcut, no builtin-name prefix: {stderr:?}"
        );
        let _ = code;
    }
    #[cfg(unix)]
    set_mode(&data, 0o755);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1307_source_missing_and_eacces_file_controls() {
    let dir = scratch("1307ctl");
    // ENOENT control: agreed on both shells (prefix-less, raw errno).
    let (_o, stderr, _c) = run_script_file(&dir, "repro.sh", "source ./nope.sh; echo rc=$?\n");
    assert!(
        stderr.contains("./nope.sh: No such file or directory"),
        "source of a missing file keeps ENOENT wording, got: {stderr:?}"
    );
    if euid_is_root() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    // EACCES regular-file control: agreed on both shells already.
    let file = dir.join("locked.sh");
    std::fs::write(&file, b"echo hi\n").expect("write locked.sh");
    #[cfg(unix)]
    set_mode(&file, 0o000);
    let (_o2, stderr2, _c2) = run_script_file(&dir, "repro.sh", "source ./locked.sh; echo rc=$?\n");
    #[cfg(unix)]
    set_mode(&file, 0o644);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        stderr2.contains("./locked.sh: Permission denied"),
        "source of an EACCES file keeps Permission denied, got: {stderr2:?}"
    );
}
