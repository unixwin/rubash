//! rubash#1671 — globstar `**/<literal>/…` appended the bare names of
//! ALL cwd symlinks (valid, file-target, and dangling) after the correct
//! results.
//!
//! GNU ground truth (lib/glob/glob.c glob_filename, the all_starstar
//! symlink special case): a symlinked directory enumerated by `**` is
//! returned as a result only when NO pattern remains after the `**`;
//! when more pattern follows it is skipped entirely — the link name is
//! never a final match for `**/<more>` and `**` never descends into it.
//! Expectations verified against GNU bash 5.3.20 (script files).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1671-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(dir: &std::path::Path, script: &str) -> String {
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn fixture(dir: &std::path::Path) {
    let fix = dir.join("fix");
    std::fs::create_dir_all(fix.join("d1/sub/deep")).expect("fixture dirs");
    std::fs::write(fix.join("top.txt"), "").expect("fixture file");
    std::fs::write(fix.join("d1/sub/deep/f3.txt"), "").expect("fixture file");
    std::fs::write(fix.join("d1/sub/deep/g.log"), "").expect("fixture file");
    std::os::unix::fs::symlink("d1", fix.join("linkd")).expect("symlink fixture");
    std::os::unix::fs::symlink("top.txt", fix.join("linkf")).expect("symlink fixture");
    std::os::unix::fs::symlink("missing-target", fix.join("linkdead")).expect("symlink fixture");
}

#[test]
fn globstar_literal_tail_emits_no_bare_symlink_names() {
    let dir = tempdir("tail");
    fixture(&dir);
    let fix = dir.join("fix");
    let stdout = run(
        &fix,
        "shopt -s globstar\n\
         for x in **/deep/*; do printf \"<%s>\\n\" \"$x\"; done\n",
    );
    assert_eq!(stdout, "<d1/sub/deep/f3.txt>\n<d1/sub/deep/g.log>\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn globstar_literal_mid_then_trailing_starstar_emits_no_bare_symlink_names() {
    let dir = tempdir("mid");
    fixture(&dir);
    let fix = dir.join("fix");
    let stdout = run(
        &fix,
        "shopt -s globstar\n\
         for x in **/sub/**; do printf \"<%s>\\n\" \"$x\"; done\n",
    );
    assert_eq!(
        stdout,
        "<d1/sub>\n<d1/sub/deep>\n<d1/sub/deep/f3.txt>\n<d1/sub/deep/g.log>\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn globstar_literal_tail_nullglob_emits_no_bare_symlink_names() {
    let dir = tempdir("nullglob");
    fixture(&dir);
    let fix = dir.join("fix");
    let stdout = run(
        &fix,
        "shopt -s globstar nullglob\n\
         for x in **/deep/*; do printf \"<%s>\\n\" \"$x\"; done\n",
    );
    assert_eq!(stdout, "<d1/sub/deep/f3.txt>\n<d1/sub/deep/g.log>\n");
    let _ = std::fs::remove_dir_all(&dir);
}
