//! rubash#2680 (residual face owned by this wave) — when a later
//! segment of the SAME command line dies at parse time on an
//! unclosed extglob group, the `;`-separated segments before it on
//! that line must NOT execute: GNU parses the whole command list
//! before executing any of it (parse.y read_token / parse_matched_
//! pair, parse.y:5466-5489 + 3901-3912), so the line's earlier
//! segments are discarded with the line.
//!
//! Face division: the exact diagnostic WORDING for an unclosed
//! extglob group (GNU: `unexpected EOF while looking for matching
//! `)'`) is rubash#3229's face, not this wave's. These pins therefore
//! pin the execution-suppression facts exactly (stdout, exit code,
//! diagnostic line number, exactly one diagnostic) and require a
//! line-2 parse diagnostic without pinning its wording.
//!
//! Boundary (also pinned): a parse error on a LATER line does not
//! retract earlier lines — GNU executes complete command lists as
//! they are read, so `echo first` on its own line still runs.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-10),
//! script-file mode.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2680-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_suppressed(tag: &str, run: &Run, diag_line: usize) {
    assert_eq!(
        run.stdout, "",
        "{tag}: stdout — no segment of the bad line may execute"
    );
    assert_eq!(run.code, Some(2), "{tag}: rc");
    let diag_lines: Vec<&str> = run
        .stderr
        .lines()
        .filter(|line| line.contains("syntax error") || line.contains("unexpected EOF"))
        .collect();
    assert_eq!(
        diag_lines.len(),
        1,
        "{tag}: exactly one parse diagnostic: {:?}",
        run.stderr
    );
    assert!(
        diag_lines[0].contains(&format!("line {diag_line}:")),
        "{tag}: diagnostic must be attributed to line {diag_line}: {:?}",
        run.stderr
    );
}

#[test]
fn issue2680_unclosed_group_suppresses_earlier_segment() {
    let run = run_file(
        "plain",
        "shopt -s extglob\n\
         echo before; echo @(ab\n\
         echo next\n",
    );
    assert_suppressed("plain", &run, 2);
}

#[test]
fn issue2680_unclosed_group_with_bracket_suppresses_earlier_segment() {
    let run = run_file(
        "bracket",
        "shopt -s extglob\n\
         echo before; echo @(a[b\n\
         echo next\n",
    );
    assert_suppressed("bracket", &run, 2);
}

#[test]
fn issue2680_error_on_later_line_keeps_earlier_lines() {
    // Boundary: the failing command list is its own line; the
    // complete list on the line before it has already executed.
    let run = run_file(
        "later-line",
        "shopt -s extglob\n\
         echo first\n\
         echo @(ab\n\
         echo next\n",
    );
    assert_eq!(run.stdout, "first\n", "later-line: stdout");
    assert_eq!(run.code, Some(2), "later-line: rc");
    assert!(
        run.stderr.contains("line 3:"),
        "later-line: diagnostic on the opener line: {:?}",
        run.stderr
    );
}
