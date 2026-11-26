//! Issue rubash#1769: the redirect ambiguity gate false-positived on a
//! single glob match whose filename contains a space.
//!
//! GNU redir.c:298 redirection_expand returns NULL exactly when the
//! expanded word list has zero or more than one field, and subst.c:12602
//! glob_expand_word_list makes one list entry per match without
//! word-splitting the matches — so one match is one field even when its
//! name contains whitespace (`echo X > only*.txt` with one file
//! `only one.txt` opens it, rc 0). The old gate judged the joined
//! carrier String with a space heuristic and could not tell one
//! space-containing field from two fields.
//!
//! Assertions follow the GNU bash 5.3.20 behavior measured with the same
//! scripts (matrix in the cluster REPORT). The multi-field faces stay
//! ambiguous: multiple matches, an unquoted `$var` that word-splits, and
//! brace alternatives.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1769-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn issue1769_plain_single_match_with_space() {
    let dir = tempdir("plain-space");
    std::fs::write(dir.join("only one.txt"), "").expect("seed");
    let run = rubash_c("echo X > only*.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert!(
        !run.stderr.contains("ambiguous"),
        "unexpected ambiguity diagnostic: {}",
        run.stderr
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("only one.txt")).expect("read target"),
        "X\n"
    );
}

#[test]
fn issue1769_extglob_single_match_with_space() {
    let dir = tempdir("extglob-space");
    std::fs::write(dir.join("ext one.txt"), "").expect("seed");
    let run = rubash_c("shopt -s extglob; echo X > ext*.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.join("ext one.txt")).expect("read target"),
        "X\n"
    );
}

#[test]
fn issue1769_append_single_match_with_space() {
    let dir = tempdir("append-space");
    std::fs::write(dir.join("app one.txt"), "old\n").expect("seed");
    let run = rubash_c("echo Y >> app*.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.join("app one.txt")).expect("read target"),
        "old\nY\n"
    );
}

#[test]
fn issue1769_input_single_match_with_space() {
    let dir = tempdir("input-space");
    std::fs::write(dir.join("in one.txt"), "hello\n").expect("seed");
    let run = rubash_c("cat < in*.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "hello\n");
}

#[test]
fn issue1769_plain_single_match_without_space() {
    let dir = tempdir("plain-nospace");
    std::fs::write(dir.join("single.txt"), "").expect("seed");
    let run = rubash_c("echo X > sing*.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.join("single.txt")).expect("read target"),
        "X\n"
    );
}

#[test]
fn issue1769_plain_multi_match_stays_ambiguous() {
    let dir = tempdir("multi");
    std::fs::write(dir.join("a1.txt"), "").expect("seed a1");
    std::fs::write(dir.join("a2.txt"), "").expect("seed a2");
    let run = rubash_c("echo X > a*.txt", &dir);
    assert_eq!(
        run.code,
        Some(1),
        "stdout: {} stderr: {}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stderr.contains("ambiguous redirect"),
        "expected ambiguity diagnostic, got: {}",
        run.stderr
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("a1.txt")).expect("read a1"),
        "",
        "ambiguous redirect must not write"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("a2.txt")).expect("read a2"),
        "",
        "ambiguous redirect must not write"
    );
}

#[test]
fn issue1769_multi_match_script_continues() {
    let dir = tempdir("multi-cont");
    std::fs::write(dir.join("a1.txt"), "").expect("seed a1");
    std::fs::write(dir.join("a2.txt"), "").expect("seed a2");
    let run = rubash_c("echo X > a*.txt; echo after", &dir);
    assert_eq!(run.stdout, "after\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("ambiguous redirect"),
        "expected ambiguity diagnostic, got: {}",
        run.stderr
    );
}

#[test]
fn issue1769_zero_match_literal_unchanged() {
    let dir = tempdir("zero");
    let run = rubash_c("echo X > zz*.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.join("zz*.txt")).expect("read literal"),
        "X\n"
    );
}

#[test]
fn issue1769_unquoted_var_split_stays_ambiguous() {
    let dir = tempdir("varsplit");
    std::fs::write(dir.join("sp ace.txt"), "").expect("seed");
    let run = rubash_c("f=\"sp ace.txt\"; echo X > $f", &dir);
    assert_eq!(
        run.code,
        Some(1),
        "stdout: {} stderr: {}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stderr.contains("ambiguous redirect"),
        "expected ambiguity diagnostic, got: {}",
        run.stderr
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("sp ace.txt")).expect("read target"),
        "",
        "ambiguous redirect must not write"
    );
}

#[test]
fn issue1769_brace_multi_stays_ambiguous() {
    let dir = tempdir("brace");
    let run = rubash_c("echo X > b{1,2}.txt", &dir);
    assert_eq!(
        run.code,
        Some(1),
        "stdout: {} stderr: {}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stderr.contains("ambiguous redirect"),
        "expected ambiguity diagnostic, got: {}",
        run.stderr
    );
    assert!(!dir.join("b1.txt").exists());
    assert!(!dir.join("b2.txt").exists());
}

#[test]
fn issue1769_quoted_single_with_space() {
    let dir = tempdir("quoted");
    std::fs::write(dir.join("q one.txt"), "").expect("seed");
    let run = rubash_c("echo X > \"q one.txt\"", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.join("q one.txt")).expect("read target"),
        "X\n"
    );
}
