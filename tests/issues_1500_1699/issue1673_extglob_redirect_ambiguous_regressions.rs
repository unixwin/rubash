//! rubash#1673 — an extglob redirect target with multiple matches was
//! not an ambiguous redirect: the matches were space-joined into one
//! literal filename and written (rc=0).
//!
//! GNU ground truth: redir.c:298 redirection_expand returns NULL for a
//! multi-field operand and the caller reports AMBIGUOUS_REDIRECT
//! (redir.c:325-333, 911-912). Whether one raw word can become several
//! fields is decided by its unquoted expansion introducers
//! (pathexp.c unquoted_glob_pattern_p), whose class includes the
//! extglob operators `+`, `@`, `!` when followed by `(` and extglob is
//! enabled. The plain-glob path already gated on that class; the
//! extglob operators were missing from it. Expectations verified
//! against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1673-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn fixture(dir: &std::path::Path) {
    for f in ["a.txt", "b.txt", "c.txt", "keep.log"] {
        std::fs::write(dir.join(f), "").expect("fixture file");
    }
}

#[test]
fn extglob_redirect_multi_match_is_ambiguous() {
    let dir = tempdir("multi");
    fixture(&dir);
    let (stdout, stderr, _code) = run(
        &dir,
        "shopt -s extglob\n\
         printf \"HI\\n\" > @(a.txt|b.txt); echo \"rc1=$?\"\n\
         printf \"HI\\n\" > !(a.txt); echo \"rc2=$?\"\n",
    );
    assert_eq!(stdout, "rc1=1\nrc2=1\n");
    assert!(
        stderr.contains("@(a.txt|b.txt): ambiguous redirect"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("!(a.txt): ambiguous redirect"),
        "stderr: {stderr}"
    );
    // Nothing was created or written: no space-joined literal file, and
    // the pre-existing targets keep their empty content.
    assert!(!dir.join("a.txt b.txt").exists());
    for f in ["a.txt", "b.txt", "c.txt", "keep.log"] {
        assert_eq!(std::fs::read_to_string(dir.join(f)).unwrap(), "");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extglob_redirect_single_match_still_opens() {
    let dir = tempdir("single");
    fixture(&dir);
    let (stdout, _stderr, _code) = run(
        &dir,
        "shopt -s extglob\n\
         printf \"ONE\\n\" > @(a.txt); echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "ONE\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extglob_off_redirect_target_is_literal() {
    let dir = tempdir("off");
    fixture(&dir);
    let (stdout, _stderr, _code) = run(
        &dir,
        "printf \"HI\\n\" > '@(a.txt|b.txt)'; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("@(a.txt|b.txt)")).unwrap(),
        "HI\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
