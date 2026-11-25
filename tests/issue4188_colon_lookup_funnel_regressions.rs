//! rubash#4188 — a directory created through the named-mkdir chain
//! under a colon parent (`mkdir -p x:y/z`, on disk `x<U+F03A>y/z`)
//! was unreachable by lookup: `[ -d x:y/z ]` missed, `touch
//! x:y/z/f` and `ls x:y/z` failed, `cd x:y/z` failed, and the
//! absolute spelling failed the same way.
//!
//! Mechanism (captain adjudication on the issue, host-proven): the
//! build side encodes `:` per component (the #4150 chain walk
//! resolves each component as a standalone final component), and
//! the MSYS2/cygwin reference layer encodes per component on BOTH
//! sides — its on-disk name for `x:y/z` is the same `x<F03A>y/z`
//! and its lookups round-trip. The single shell-name -> NT-name
//! funnel (`shell_path_to_windows` in executor/path.rs) encoded
//! only the FINAL component, so every lookup consumer (test, cd,
//! stat, glob walks, redirect targets) kept the non-final colon
//! literal and named a path that cannot exist on NTFS.
//!
//! The fix is in the funnel only: `map_ntfs_colon_components`
//! encodes every component, exempting just the path's own drive
//! designator in absolute form. The display direction
//! (`shell_path_display_from_windows`) already decodes with a
//! plain replace, so shell-visible names show the POSIX spelling
//! at the same granularity. No consumer was patched individually.
//!
//! Windows only: the shapes are NTFS colon semantics. On unix `:`
//! is an ordinary filename byte and none of this mapping exists.
//! Execution on a real Windows host is part of the lane record;
//! CI carries the stem compile on the Windows MSVC check leg.

#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-4188-{case}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

/// Names of the entries directly inside the scratch cwd, sorted.
fn entries(dir: &ScratchDir) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(&dir.0)
        .expect("read scratch")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn run_rubash(dir: &ScratchDir, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&dir.0)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

#[test]
fn issue4188_colon_parent_lookup_roundtrip() {
    // The report script: build the tree, then reach it by its
    // literal spelling through every lookup consumer.
    let dir = scratch("roundtrip");
    let (stdout, stderr, code) = run_rubash(
        &dir,
        "mkdir -p x:y/z; echo mk=$?; [ -d x:y/z ] && echo d1; [ -d x:y ] && echo d2; \
         touch x:y/z/f; echo t=$?; ls x:y/z; cd x:y/z; pwd; cd ../..; \
         echo hi > x:y/z/g.txt; cat x:y/z/g.txt\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "mk=0", "stdout:\n{stdout}");
    assert_eq!(lines[1], "d1", "stdout:\n{stdout}");
    assert_eq!(lines[2], "d2", "stdout:\n{stdout}");
    assert_eq!(lines[3], "t=0", "stdout:\n{stdout}");
    assert_eq!(lines[4], "f", "stdout:\n{stdout}");
    // pwd shows the shell spelling (display decode is per-component
    // too): the literal colon form, never the U+F03A carrier.
    assert!(
        lines[5].ends_with("x:y/z"),
        "pwd must show the literal spelling, got: {}",
        lines[5]
    );
    assert!(
        !stdout.contains('\u{F03A}'),
        "no U+F03A may leak into shell-visible output:\n{stdout}"
    );
    assert_eq!(lines[6], "hi", "stdout:\n{stdout}");
    // The tree on disk is the encoded form only.
    assert_eq!(entries(&dir), vec!["x\u{F03A}y".to_string()]);
    assert!(dir.0.join("x\u{F03A}y").join("z").join("f").is_file());
    assert!(dir.0.join("x\u{F03A}y").join("z").join("g.txt").is_file());
}

#[test]
fn issue4188_colon_parent_absolute_spelling() {
    // The absolute spelling of the same tree resolves identically
    // (the report's `ls /c/…/x:y/z` shape, on the scratch drive).
    let dir = scratch("absolute");
    let base = dir.0.to_string_lossy().replace('\\', "/");
    let script = format!(
        "mkdir -p x:y/z; touch x:y/z/f; ls '{base}/x:y/z'; [ -d '{base}/x:y' ] && echo absd\n"
    );
    let (stdout, stderr, code) = run_rubash(&dir, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(stdout, "f\nabsd\n", "stdout:\n{stdout}");
}

#[test]
fn issue4188_multi_colon_chain_and_final_form_guards() {
    // Several colon components in one chain round-trip, and the
    // pre-existing final-component forms keep working unchanged.
    let dir = scratch("chain");
    let (stdout, stderr, code) = run_rubash(
        &dir,
        "mkdir -p a:b/c:d/e; [ -d a:b/c:d/e ] && echo chain; \
         touch a:b/c:d/e/f; ls a:b/c:d; \
         mkdir -p e:f; [ -d e:f ] && echo final; \
         touch e:f/g; ls e:f\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(stdout, "chain\ne\nfinal\ng\n", "stdout:\n{stdout}");
    assert_eq!(
        entries(&dir),
        vec!["a\u{F03A}b".to_string(), "e\u{F03A}f".to_string()]
    );
}
