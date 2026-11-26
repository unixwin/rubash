//! rubash#1089 — `compgen -A command` deduplicated builtin+file hits
//! and `compgen -A file` sorted its results. GNU keeps duplicates (a
//! name that is both a builtin and a PATH file appears once per
//! source) and returns `-A file` matches in directory-read order.
//! Closed in the zero-queue campaign on w1-cluster evidence ("both
//! shells print the same"); these pins keep both halves in CI.
//!
//! GNU source: builtins/common.def compgen via bashline.c completion
//! generators — matches are emitted per source without dedup, and the
//! file generator walks the directory with readdir().

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-bfa-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn issue1089_compgen_command_keeps_builtin_and_file_hits() {
    let scratch = Scratch::new("1089-cmd");
    let bin = scratch.0.join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let echo_file = bin.join("echo");
    fs::write(&echo_file, "#!/bin/sh\necho EXT\n").expect("write fake echo");
    fs::set_permissions(&echo_file, fs::Permissions::from_mode(0o755)).expect("chmod fake echo");

    // PATH contains only the scratch bin dir: `echo` is a builtin and
    // a file hit, so GNU lists it twice.
    let script = format!("PATH={}\ncompgen -A command echo\n", bin.display());
    let (stdout, stderr, code) = scratch.rubash_file(&script);
    assert_eq!(stdout, "echo\necho\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1089_compgen_command_builtin_only_without_file_hit() {
    let scratch = Scratch::new("1089-cmd0");
    let bin = scratch.0.join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");

    let script = format!("PATH={}\ncompgen -A command echo\n", bin.display());
    let (stdout, stderr, code) = scratch.rubash_file(&script);
    assert_eq!(stdout, "echo\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1089_compgen_file_follows_directory_order() {
    let scratch = Scratch::new("1089-file");
    // Created in scrambled order on purpose: GNU emits readdir()
    // order, which is neither creation order nor sorted order in
    // general. The expectation is this test's own read_dir() walk of
    // the same directory, which is the order source GNU uses.
    for name in ["z9", "z3", "z7", "z1"] {
        fs::write(scratch.0.join(name), "").expect("create candidate");
    }
    let mut expected: Vec<String> = fs::read_dir(&scratch.0)
        .expect("read scratch dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('z'))
        .collect();
    assert_eq!(expected.len(), 4, "scratch candidates: {expected:?}");
    expected.iter_mut().for_each(|line| line.push('\n'));

    let (stdout, stderr, code) = scratch.rubash_file("compgen -A file -- z\n");
    assert_eq!(stdout, expected.concat());
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
