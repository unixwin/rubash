//! Issue rubash#328: rubash unconditionally created `$TMPDIR/var/tmp` at
//! startup — even for `rubash -c ':'` — so any script listing its TMPDIR
//! (or the cwd, when TMPDIR defaults to `<cwd>/target`) saw a phantom
//! `var` entry (pbb sections 028/081/082 printed a stray `var` line; GNU
//! creates nothing for a no-op command).
//!
//! GNU contract: bash touches $TMPDIR only when it actually needs a
//! scratch file (here-doc to command substitution, FIFO process
//! substitutions, etc.) and uses $TMPDIR DIRECTLY — never a var/tmp
//! subtree (vendored third_party/bash: subst.c process_substitute uses
//! get_string_value("TMPDIR") straight into the mkfifo/mktemp path).
//!
//! Root cause: Executor init ran ensure_var_tmp_dir on every startup to
//! back the /var/tmp → <temp>/var/tmp Windows mapping eagerly.
//!
//! Fix: the startup call is gone; the /var/tmp mapping in
//! executor/path.rs materializes the backing directory lazily on the
//! FIRST /var/tmp path resolution (a create_dir_all no-op afterwards).
//! Temp-file writers already create their parent directories
//! (external_setup.rs create_dir_all), so process substitutions keep
//! working with a default TMPDIR.
//!
//! Expected behavior verified against WSL GNU Bash 5.3.0 (2026-09-28:
//! `TMPDIR=/tmp/g328x bash -c ':'` leaves no directory behind).

use std::path::PathBuf;
use std::process::Command;

fn tmp_probe_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash328-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rubash_in(dir: &PathBuf, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("TMPDIR", dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn dir_entries(dir: &PathBuf) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The issue's reproducer: `TMPDIR=$PWD/t rubash -c ':'` must leave `t`
/// empty (GNU creates nothing).
#[test]
fn no_op_command_leaves_tmpdir_untouched() {
    let dir = tmp_probe_dir("noop");
    let (stdout, stderr, code) = rubash_in(&dir, ":");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(dir_entries(&dir), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Ordinary commands and even temp-file users write into TMPDIR directly
/// — no `var` subtree appears (GNU shape).
#[test]
fn working_commands_do_not_create_var_subtree() {
    let dir = tmp_probe_dir("work");
    let (stdout, stderr, code) = rubash_in(&dir, "echo hi > f.txt; cat f.txt; rm -f f.txt");
    assert_eq!(stdout, "hi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(dir_entries(&dir), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The /var/tmp mapping still works — materialized lazily on first use
/// (GNU suites like vredir.tests default TMPDIR:=/var/tmp and rely on
/// writes succeeding).
#[test]
fn var_tmp_backing_created_lazily_on_use() {
    let dir = tmp_probe_dir("lazy");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        ": > /var/tmp/p328; echo wrote; cat /var/tmp/p328; echo rc=$?",
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "wrote\nrc=0\n");
    // The backing subtree exists only after the /var/tmp use.
    let entries = dir_entries(&dir);
    assert_eq!(entries, vec!["var".to_string()]);
    assert!(dir.join("var").join("tmp").is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Process substitutions keep working with a TMPDIR that starts empty
/// (temp-file writers create their parents).
#[test]
fn process_substitution_works_with_empty_tmpdir() {
    let dir = tmp_probe_dir("procsub");
    let (stdout, stderr, code) = rubash_in(&dir, "cat <(echo procsub-ok)");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "procsub-ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}
