//! unixwin/niubash#177 — virtual system-root FILE operands fail without `.exe`.
//!
//! After the wt60 root-map translation, directory operands worked
//! (`ls /usr/bin | wc -l`) but FILE operands failed while their `.exe`-spelled
//! forms worked: `head -1 /usr/bin/seq` rc=1, `wc -l < /usr/bin/seq` rc=1,
//! `cat /usr/bin/cat | wc -c` → 0 — while `test -f /usr/bin/seq` answered YES
//! (the internal lookup already resolves the executable extension). The
//! translation layer now hands out the MSYS-consistent spelling: a translated
//! operand that does not exist as-spelled but does exist as `<name>.exe` is
//! handed to children/readers in the `.exe` spelling (existence-checked,
//! never blind-appended; existing files and directories keep the plain
//! translated form; a name missing under both spellings keeps the original).
//!
//! Hermetic consumers only: `read`/`cat` exercise the in-process reader
//! funnels, `cmd /c echo` exercises the argv funnel (cmd.exe exists on every
//! Windows host), so the test does not need a WinuxCmd installation.

#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn fixture_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rubash-i177-cli-{tag}"));
    let _ = fs::remove_dir_all(&root);
    let usr_bin = root.join("usr").join("bin");
    fs::create_dir_all(&usr_bin).unwrap();
    fs::create_dir_all(root.join("etc")).unwrap();
    // The operand exists ONLY under its .exe spelling, mirroring the
    // WinuxCmd tree layout the bug report describes.
    fs::write(usr_bin.join("data.exe"), b"hello\n").unwrap();
    fs::write(root.join("etc").join("i177.conf"), b"one\ntwo\n").unwrap();
    root
}

fn run_rubash(root: &PathBuf, script: &str) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("WINUXSH_ROOT", root.to_string_lossy().into_owned())
        .env_remove("__RUBASH_ARGV_DIALECT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn input_redirect_resolves_virtual_root_file_operand() {
    // `wc -l < /usr/bin/seq` used to fail with "No such file or directory"
    // (rc=1): the input-redirect funnel mapped the name but never resolved
    // the executable extension.
    let root = fixture_root("redirect");
    let (stdout, stderr, ok) = run_rubash(&root, "read -r line < /usr/bin/data; echo \"$line\"");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "hello\n", "stderr: {stderr}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn comsub_cat_resolves_virtual_root_file_operand() {
    // `cat /usr/bin/cat | wc -c` used to yield 0: the in-process cat
    // fast paths read the plain mapped spelling. One shell view, one
    // namespace: what `test -f` sees, the readers must open.
    let root = fixture_root("cat");
    let (stdout, stderr, ok) = run_rubash(&root, "x=$(cat /usr/bin/data); echo \"$x\"");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "hello\n", "stderr: {stderr}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn argv_funnel_resolves_operand_but_keeps_dirs_and_misses() {
    let root = fixture_root("argv");

    // File operand missing as-spelled, present as .exe: the child receives
    // the resolved spelling.
    let (stdout, stderr, ok) = run_rubash(&root, "cmd /c echo /usr/bin/data");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        root.join("usr")
            .join("bin")
            .join("data.exe")
            .to_string_lossy()
            .into_owned(),
        "argv operand must resolve to the existing .exe spelling"
    );

    // Directory operand: plain translated form, NO suffix appended.
    let (stdout, stderr, ok) = run_rubash(&root, "cmd /c echo /usr/bin");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        root.join("usr").join("bin").to_string_lossy().into_owned(),
        "directory operand must keep the plain translated form"
    );

    // Missing under both spellings: the ORIGINAL spelling stands (never a
    // blind append), so the child errors exactly as before the resolution.
    let (stdout, stderr, ok) = run_rubash(&root, "cmd /c echo /usr/bin/nope");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        root.join("usr")
            .join("bin")
            .join("nope")
            .to_string_lossy()
            .into_owned(),
        "missing operand must keep the original translated spelling"
    );

    // Existing as-spelled file operand: verbatim.
    let (stdout, stderr, ok) = run_rubash(&root, "cmd /c echo /etc/i177.conf");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        root.join("etc")
            .join("i177.conf")
            .to_string_lossy()
            .into_owned(),
        "existing file operand must keep the plain translated form"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_builtin_and_operand_view_stay_consistent() {
    // The consistency contract of the fix: `test -f` (which already
    // resolved the extension) and the operand consumers agree on what
    // exists under the virtual root.
    let root = fixture_root("consistency");
    let (stdout, stderr, ok) = run_rubash(
        &root,
        "test -f /usr/bin/data && read -r line < /usr/bin/data && echo \"$line\"",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "hello\n", "stderr: {stderr}");
    let _ = fs::remove_dir_all(&root);
}
