//! niubash#993 §4 — `command -v` / `type -p` / `type -t` report the
//! GNU file answer for emulated commands.
//!
//! Reported defect (Git Bash drop-in): for the 17 emulated commands
//! (cat, cp, mv, rm, mkdir, rmdir, touch, chmod, sed, uname, arch,
//! basename, dirname, mkfifo, sleep, stty, tput) niubash answered
//! `type -t` -> `emulation` (a type word GNU has not — GNU
//! `describe_command`, builtins/type.def:211, prints `file` for a
//! resolved PATH hit), `command -v` -> the bare name (a
//! function/alias answer, never a file answer — type.def:336-359 the
//! CDESC_REUSABLE arm prints find_user_command's path), and
//! `type -p` -> empty (CDESC_PATH_ONLY prints the same path).
//!
//! GNU baseline (WSL bash 5.3.x, script file): `type -t cat` ->
//! `file`, `command -v cat` -> `/usr/bin/cat`, `type -p cat` ->
//! `/usr/bin/cat`, `type cat` -> `cat is /usr/bin/cat`,
//! `command -V cat` -> `cat is /usr/bin/cat`.
//!
//! The fake-applet PATH below mirrors the product face: an
//! `stty.exe`-style applet file on PATH is the body dispatch would
//! run, so the resolved-path answer is the honest one. niubash#993
//! S7 (same file, later tests): the displayed join drops the `.exe`
//! probe suffix Git Bash/MSYS never shows — `command -v cat` ->
//! `/usr/bin/cat`, while a user-typed `cat.exe` keeps its suffix and
//! a non-.exe body (`foo.bat`) keeps its real filename.

#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-993s4-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("fakebin")).expect("fakebin");
        Scratch(dir)
    }

    fn fakebin(&self) -> PathBuf {
        self.0.join("fakebin")
    }

    fn rubash(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            // POSIX-form PATH element keeps the describe display
            // spelling clean; the executable resolution walks it the
            // same either way.
            .env("PATH", "/c/tmp-never-exists-993")
            .env("HOME", &self.0)
            .stdin(Stdio::null())
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).replace('\r', ""),
            output.status.code(),
        )
    }

    fn rubash_with_path(&self, path: &str, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.0)
            .stdin(Stdio::null())
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).replace('\r', ""),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `type -t` reports `file`, never the retired `emulation` word, for
/// every emulated command the issue names.
#[test]
fn niubash993_s4_type_t_reports_file_for_emulated() {
    let scratch = Scratch::new("typet");
    for name in [
        "cat.exe", "cp.exe", "rm.exe", "sed.exe", "stty.exe", "tput.exe",
    ] {
        fs::write(scratch.fakebin().join(name), b"").expect("fake applet");
    }
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) =
        scratch.rubash_with_path(&fakebin, "type -t cat cp rm sed stty tput; echo rc=$?");
    // GNU type -t prints one bare `file` line per resolved name.
    assert_eq!(
        stdout.matches("file").count(),
        6,
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(!stdout.contains("emulation"), "stdout: {stdout}");
}

/// `command -v` prints the resolved applet path, not the bare name.
#[test]
fn niubash993_s4_command_v_prints_resolved_path() {
    let scratch = Scratch::new("commandv");
    fs::write(scratch.fakebin().join("cat.exe"), b"").expect("fake applet");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, code) = scratch.rubash_with_path(&fakebin, "command -v cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("/cat"), "stdout: {stdout}");
    assert!(!stdout.contains("cat.exe"), "stdout: {stdout}");
    assert!(stdout.contains("rc=0"), "stdout: {stdout}");
}

/// `type -p` prints the resolved path (silent-but-found only for
/// builtins/aliases — a file answer must name the file).
#[test]
fn niubash993_s4_type_p_prints_resolved_path() {
    let scratch = Scratch::new("typep");
    fs::write(scratch.fakebin().join("cat.exe"), b"").expect("fake applet");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) = scratch.rubash_with_path(&fakebin, "type -p cat; echo rc=$?");
    assert!(
        stdout.contains("/cat") && !stdout.contains("cat.exe"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(stdout.contains("rc=0"), "stdout: {stdout}");
}

/// `type` / `command -V` print the GNU `cat is <path>` sentence.
#[test]
fn niubash993_s4_verbose_describe_names_the_file() {
    let scratch = Scratch::new("verbose");
    fs::write(scratch.fakebin().join("cat.exe"), b"").expect("fake applet");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) = scratch.rubash_with_path(&fakebin, "type cat; command -V cat");
    assert!(
        stdout.contains("cat is ") && stdout.contains("/cat") && !stdout.contains("cat.exe"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(!stdout.contains("shell emulation"), "stdout: {stdout}");
}

/// `type -a` enumerates the real file hits — no `/bin/cat` bridge
/// spelling beside the real hit, and `emulation` appears nowhere.
#[test]
fn niubash993_s4_type_a_lists_only_real_hits() {
    let scratch = Scratch::new("typea");
    fs::write(scratch.fakebin().join("cat.exe"), b"").expect("fake applet");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) = scratch.rubash_with_path(&fakebin, "type -a cat");
    assert!(stdout.contains("/cat"), "stdout: {stdout} stderr: {stderr}");
    assert!(!stdout.contains("/bin/cat"), "stdout: {stdout}");
    assert!(!stdout.contains("emulation"), "stdout: {stdout}");
}

/// An emulated name that resolves nowhere still reports not found —
/// the file answer is honest, not a fixed `file` claim.
#[test]
fn niubash993_s4_unresolvable_name_reports_not_found() {
    let scratch = Scratch::new("notfound");
    let (stdout, stderr, code) = scratch.rubash("type -t definitely-not-a-command-993; echo rc=$?");
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(stdout.contains("rc=1"), "stdout: {stdout}");
    assert!(!stdout.contains("emulation"), "stdout: {stdout}");
    let _ = stderr;
}

/// A shell builtin keeps its builtin answer — the retirement touched
/// only the emulation face, not the builtin arm above it.
#[test]
fn niubash993_s4_builtins_still_report_builtin() {
    let scratch = Scratch::new("builtin");
    let (stdout, stderr, _) =
        scratch.rubash("type -t echo; command -v echo; type -p echo; echo rc=$?");
    assert!(
        stdout.contains("builtin"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("echo\n") || stdout.contains("echo\r\n"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("rc=0"), "stdout: {stdout}");
}

/// niubash#993 S7: a name typed WITH the `.exe` suffix keeps it —
/// Git Bash `command -v cat.exe` -> `/usr/bin/cat.exe`.
#[test]
fn niubash993_s7_typed_exe_suffix_is_preserved() {
    let scratch = Scratch::new("typedexe");
    fs::write(scratch.fakebin().join("cat.exe"), b"").expect("fake applet");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) = scratch.rubash_with_path(&fakebin, "command -v cat.exe; echo rc=$?");
    assert!(
        stdout.contains("cat.exe"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(stdout.contains("rc=0"), "stdout: {stdout}");
}

/// niubash#993 S7: a non-.exe body keeps its real filename — the
/// strip is the `.exe` probe suffix only, never an arbitrary
/// extension (`foo.bat` is not a suffix Git Bash hides either).
#[test]
fn niubash993_s7_non_exe_body_keeps_filename() {
    let scratch = Scratch::new("batbody");
    fs::write(scratch.fakebin().join("foo.bat"), b"@echo off\n").expect("fake bat");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) = scratch.rubash_with_path(&fakebin, "command -v foo; echo rc=$?");
    assert!(
        stdout.contains("foo.bat"),
        "stdout: {stdout} stderr: {stderr}"
    );
}

/// niubash#993 S7: a bare `name` file beside `name.exe` in one PATH
/// element collapses to the single as-written join — `type -a` must
/// not print the same spelling twice.
#[test]
fn niubash993_s7_bare_and_exe_pair_enumerate_once() {
    let scratch = Scratch::new("pair");
    fs::write(scratch.fakebin().join("cat"), b"").expect("bare body");
    fs::write(scratch.fakebin().join("cat.exe"), b"").expect("exe body");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) = scratch.rubash_with_path(&fakebin, "type -a cat");
    assert_eq!(
        stdout.matches("/cat").count(),
        1,
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(!stdout.contains("cat.exe"), "stdout: {stdout}");
}

/// Dispatch routing is untouched: the emulated body still runs
/// in-process, only its describe answer changed. `cat` reads the file
/// the emulation owns — the fake applet's empty body could never
/// produce this output.
#[test]
fn niubash993_s4_dispatch_still_runs_emulation() {
    let scratch = Scratch::new("dispatch");
    fs::write(scratch.fakebin().join("cat.exe"), b"").expect("fake applet");
    fs::write(scratch.0.join("f.txt"), "emulated-body-output\n").expect("fixture");
    let fakebin = scratch.fakebin().to_string_lossy().replace('\\', "/");
    let (stdout, stderr, _) = scratch.rubash_with_path(&fakebin, "cat f.txt; echo rc=$?");
    assert!(
        stdout.contains("emulated-body-output"),
        "stdout: {stdout} stderr: {stderr}"
    );
}
