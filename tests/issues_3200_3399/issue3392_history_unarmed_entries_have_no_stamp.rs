//! rubash#3392 — entries created before HISTTIMEFORMAT is set carry no stamp.
//! GNU stamps an entry at creation from `history_comment_char`
//! (lib/readline/history.c hist_inittime: the stamp's first char IS the
//! comment char, `'\0'` while unarmed), so an unarmed entry renders `??`
//! and `history -a`/`-w` write no `#<epoch>` line for it. Arming is the
//! sticky latch (variables.c sv_histtimefmt arms history_comment_char on
//! the first HISTTIMEFORMAT bind and never disarms it); the write gate is
//! the separate current-existence check.
//!
//! Expectations measured with GNU bash 5.2.21 (script file, same shapes).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3392-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_rubash(tag: &str, script: &str) -> (String, String, PathBuf) {
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    )
}

fn read_hist(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn is_stamp_line(line: &str) -> bool {
    line.starts_with('#') && line.len() > 1 && line[1..].chars().all(|c| c.is_ascii_digit())
}

#[test]
fn unarmed_entry_displays_question_marks() {
    let (stdout, stderr, dir) = run_rubash(
        "display",
        "history -s 'echo new'\nHISTTIMEFORMAT='%F '\nhistory\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "    1  ??echo new\n", "stderr: {stderr}");
}

#[test]
fn append_writes_no_stamp_line_for_unarmed_entry() {
    let (_stdout, stderr, dir) = run_rubash(
        "append",
        "HISTFILE=h\nhistory -s 'echo a'\nHISTTIMEFORMAT='%F '\nhistory -a\n",
    );
    let content = read_hist(&dir, "h");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(content, "echo a\n", "stderr: {stderr}");
}

#[test]
fn write_writes_no_stamp_line_for_unarmed_entry() {
    let (_stdout, stderr, dir) = run_rubash(
        "write",
        "HISTFILE=h\nhistory -s 'echo b'\nHISTTIMEFORMAT='%F '\nhistory -w\n",
    );
    let content = read_hist(&dir, "h");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(content, "echo b\n", "stderr: {stderr}");
}

#[test]
fn mixed_entries_only_armed_one_displays_a_date() {
    let (stdout, stderr, dir) = run_rubash(
        "mixed",
        "history -s 'echo first'\nHISTTIMEFORMAT='%F '\nhistory -s 'echo second'\nhistory\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout} stderr: {stderr}");
    assert_eq!(lines[0], "    1  ??echo first");
    assert!(
        lines[1].starts_with("    2  ")
            && lines[1].ends_with(" echo second")
            && !lines[1].contains('?'),
        "{stdout}"
    );
}

#[test]
fn mixed_entries_append_writes_stamp_only_for_armed_one() {
    let (_stdout, stderr, dir) = run_rubash(
        "mixedappend",
        "HISTFILE=h\nhistory -s 'echo first'\nHISTTIMEFORMAT='%F '\nhistory -s 'echo second'\nhistory -a\n",
    );
    let content = read_hist(&dir, "h");
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3, "file: {content} stderr: {stderr}");
    assert_eq!(lines[0], "echo first");
    assert!(is_stamp_line(lines[1]), "file: {content}");
    assert_eq!(lines[2], "echo second");
}

#[test]
fn format_set_before_add_stamps_entry() {
    let (_stdout, stderr, dir) = run_rubash(
        "armed",
        "HISTFILE=h\nHISTTIMEFORMAT='%F '\nhistory -s 'echo c'\nhistory -a\n",
    );
    let content = read_hist(&dir, "h");
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "file: {content} stderr: {stderr}");
    assert!(is_stamp_line(lines[0]), "file: {content}");
    assert_eq!(lines[1], "echo c");
}

#[test]
fn unset_after_armed_keeps_creation_stamp() {
    let (stdout, stderr, dir) = run_rubash(
        "sticky",
        "HISTTIMEFORMAT='%F '\nhistory -s 'echo armed'\nunset HISTTIMEFORMAT\nhistory -s 'echo later'\nHISTTIMEFORMAT='%F '\nhistory\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout} stderr: {stderr}");
    assert!(
        lines[0].ends_with(" echo armed") && !lines[0].contains('?'),
        "{stdout}"
    );
    assert!(
        lines[1].ends_with(" echo later") && !lines[1].contains('?'),
        "{stdout}"
    );
}

#[test]
fn append_after_unset_writes_no_stamp_lines() {
    let (_stdout, stderr, dir) = run_rubash(
        "unsetappend",
        "HISTFILE=h\nHISTTIMEFORMAT='%F '\nhistory -s 'echo armed'\nunset HISTTIMEFORMAT\nhistory -a\n",
    );
    let content = read_hist(&dir, "h");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(content, "echo armed\n", "stderr: {stderr}");
}

#[test]
fn empty_format_bind_arms_creation_stamp() {
    let (stdout, stderr, dir) = run_rubash(
        "emptybind",
        "HISTTIMEFORMAT=''\nhistory -s 'echo e'\nHISTTIMEFORMAT='%F '\nhistory\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "stdout: {stdout} stderr: {stderr}");
    assert!(
        lines[0].ends_with(" echo e") && !lines[0].contains('?'),
        "{stdout}"
    );
}
