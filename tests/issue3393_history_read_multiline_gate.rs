//! rubash#3393 — `history -r` regrouped multi-line entries unconditionally.
//! GNU regroups (appends an unstamped line to the previous entry and keeps
//! blank lines) only once `history_multiline_entries` is armed, and a read
//! arms it only when the file starts with a timestamp line AND
//! HISTTIMEFORMAT exists at read time.
//!
//! GNU model (third_party/bash, Bash-5.3 patch 20):
//! - `lib/readline/histfile.c:384` read_history_range:
//!   `history_multiline_entries += has_timestamps && history_write_timestamps;`
//! - `histfile.c:387` `default_skipblanks = history_multiline_entries == 0;`
//! - `histfile.c:436-437` the append-to-previous branch needs
//!   `history_multiline_entries`.
//! - `variables.c:6177` sv_histtimefmt: `history_write_timestamps = (v != 0)`
//!   — set-but-empty HISTTIMEFORMAT still arms.
//! - `history_multiline_entries` is never reset (histfile.c:126 is its only
//!   other mention), so once armed it survives `history -c` and unsetting
//!   HISTTIMEFORMAT.
//!
//! Expectations measured with WSL GNU bash 5.3.20 (script file).

use std::path::PathBuf;
use std::process::{Command, Stdio};

const HIST: &str = "printf '#86400\\necho a\\nb\\n#1234567890\\necho c\\n' > h1\n";

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3393-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_rubash(tag: &str, script: &str) -> (String, String) {
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, format!("{HIST}{script}")).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn read_without_format_splits_physical_lines() {
    let (stdout, stderr) = run_rubash("noformat", "history -r h1; history\n");
    assert_eq!(
        stdout, "    1  echo a\n    2  b\n    3  echo c\n",
        "stderr: {stderr}"
    );
}

#[test]
fn format_set_after_read_does_not_regroup() {
    let (stdout, stderr) = run_rubash(
        "after",
        "HISTFILE=h1; history -r; HISTTIMEFORMAT=; history\n",
    );
    assert_eq!(
        stdout, "    1  echo a\n    2  b\n    3  echo c\n",
        "stderr: {stderr}"
    );
}

#[test]
fn format_set_after_read_shows_file_stamps_per_split_entry() {
    let (stdout, stderr) = run_rubash(
        "afteryear",
        "history -r h1; HISTTIMEFORMAT='%Y '; history\n",
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "stdout: {stdout} stderr: {stderr}");
    assert_eq!(lines[0], "    1  1970 echo a");
    assert!(
        lines[1].starts_with("    2  ") && lines[1].ends_with(" b"),
        "{stdout}"
    );
    assert_eq!(lines[2], "    3  2009 echo c");
}

#[test]
fn backslash_line_still_splits_without_format() {
    let (stdout, stderr) = run_rubash(
        "backslash",
        "printf '#86400\\necho a\\\\\\nb\\n#1234567890\\necho c\\n' > h2\nhistory -r h2; HISTTIMEFORMAT=; history\n",
    );
    assert_eq!(
        stdout, "    1  echo a\\\n    2  b\n    3  echo c\n",
        "stderr: {stderr}"
    );
}

#[test]
fn two_orphan_lines_are_two_entries_without_format() {
    let (stdout, stderr) = run_rubash(
        "orphans",
        "printf '#86400\\necho a\\nb\\nc\\n#1234567890\\necho d\\n' > h3\nhistory -r h3; history\n",
    );
    assert_eq!(
        stdout, "    1  echo a\n    2  b\n    3  c\n    4  echo d\n",
        "stderr: {stderr}"
    );
}

#[test]
fn blank_line_is_skipped_without_format() {
    let (stdout, stderr) = run_rubash(
        "blank",
        "printf '#86400\\necho a\\n\\nb\\n#1234567890\\necho c\\n' > h4\nhistory -r h4; history\n",
    );
    assert_eq!(
        stdout, "    1  echo a\n    2  b\n    3  echo c\n",
        "stderr: {stderr}"
    );
}

#[test]
fn format_set_before_read_regroups() {
    let (stdout, stderr) = run_rubash("before", "HISTTIMEFORMAT=; history -r h1; history\n");
    assert_eq!(
        stdout, "    1  echo a\nb\n    2  echo c\n",
        "stderr: {stderr}"
    );
}

#[test]
fn armed_regrouping_is_sticky_after_unset_and_clear() {
    let (stdout, stderr) = run_rubash(
        "sticky",
        "HISTTIMEFORMAT=; history -r h1; history -c; unset HISTTIMEFORMAT\nhistory -r h1; history\n",
    );
    assert_eq!(
        stdout, "    1  echo a\nb\n    2  echo c\n",
        "stderr: {stderr}"
    );
}
