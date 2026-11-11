//! rubash#2769 — job death notices at command boundaries carry
//! the just-finished command's line stamp; GNU stamps the line
//! its reader has reached.
//!
//! GNU's branch-1 notice prefix is `"%s: line %d: "` with the
//! global `line_number` (jobs.c:4641, notify_of_job_status) —
//! the shell's READ position at the moment the notice prints:
//! - A BACKGROUND job's notice is deferred to the reader
//!   boundary (parse.y:2533 notify_and_cleanup, fired when
//!   shell_getc pulls the next list) — by then the parser has
//!   READ the next command, so the stamp is the next command's
//!   line, not the line of the command that just finished. At
//!   EOF the reader has advanced one past the last line.
//! - A FOREGROUND job's notice prints inside its own wait
//!   (wait_for reaps and reports before the reader advances),
//!   so the stamp is the dying command's own line — or, for a
//!   member of a multi-line compound, the compound's end line,
//!   because the reader consumed the whole compound before
//!   executing any of it.
//!
//! rubash formats the notice with `diagnostic_prefix()`, which
//! reads `__RUBASH_CURRENT_LINE` — the line of the command
//! currently/last executing (an execution position), so at the
//! boundary it still holds the just-finished command's line.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! 2026-11-09). Only the line stamp is asserted here; the notice
//! text is asserted by the rubash#2697 pins.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2769-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr)
}

/// The line stamp of the (single) death notice in stderr.
fn notice_line(stderr: &str, description: &str) -> usize {
    let line = stderr
        .lines()
        .find(|line| line.contains(description))
        .unwrap_or_else(|| panic!("no notice with {description:?} in stderr: {stderr}"));
    let after = line
        .split_once(": line ")
        .unwrap_or_else(|| panic!("notice line lacks a line stamp: {line}"))
        .1;
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("unparsable line stamp in: {line}"))
}

/// The issue's shape (the rubash#2698 script): the death is
/// noticed at the boundary before `jobs` on line 4; GNU stamps
/// line 4, the upcoming command's line.
#[test]
fn boundary_notice_stamps_the_next_commands_line() {
    let (stdout, stderr) = run(
        "boundary",
        "sleep 5 &\nkill -KILL %1\nsleep 0.3\njobs\necho done\n",
    );
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(notice_line(&stderr, "Killed"), 4, "stderr: {stderr}");
}

/// With another command interposed, the stamp is the first line
/// the reader reaches after the death — line 4 (`true`), not the
/// later `jobs` line and not the just-finished line 3.
#[test]
fn boundary_notice_stamps_the_first_line_read_after_the_death() {
    let (stdout, stderr) = run(
        "interposed",
        "sleep 5 &\nkill -KILL %1\nsleep 0.3\ntrue\njobs\necho done\n",
    );
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(notice_line(&stderr, "Killed"), 4, "stderr: {stderr}");
}

/// The script ends right after the death settles: the notice
/// fires at EOF cleanup, and GNU's reader has advanced one past
/// the last line (3), stamping line 4.
#[test]
fn eof_notice_stamps_one_past_the_last_line() {
    let (stdout, stderr) = run("eof", "sleep 5 &\nkill -KILL %1\nsleep 0.3\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(notice_line(&stderr, "Killed"), 4, "stderr: {stderr}");
}

/// A member death inside a multi-line subshell: the subshell's
/// line_number is its read position — the end of the compound
/// (line 6, the closing paren) — not the member's own line (4).
/// Only the stamp is asserted; the notice text in this shape is
/// rubash#2697's face.
#[test]
fn subshell_member_notice_stamps_the_compound_end_line() {
    let (stdout, stderr) = run(
        "compound",
        "echo start\n\
         (\n\
         ulimit -f 1\n\
         dd if=/dev/zero of=$PWD/big bs=1024 count=4 2>/dev/null\n\
         echo inner\n\
         )\n\
         echo \"rc=$?\"\n\
         rm -f $PWD/big\n",
    );
    assert_eq!(stdout, "start\ninner\nrc=0\n", "stderr: {stderr}");
    assert_eq!(
        notice_line(&stderr, "File size limit exceeded"),
        6,
        "stderr: {stderr}"
    );
}

/// A foreground job's notice prints inside its own wait,
/// before the reader advances: the stamp is the dying
/// command's own line (2), not the next line (3). This cell
/// guards the fix against over-applying the boundary rule to
/// foreground notices.
#[test]
fn fg_notice_stamps_the_dying_commands_own_line() {
    let (stdout, stderr) = run(
        "fgown",
        "ulimit -f 1\n\
         dd if=/dev/zero of=big bs=1024 count=4 2>/dev/null\n\
         echo \"rc=$?\"\n\
         rm -f big\n",
    );
    assert_eq!(stdout, "rc=153\n", "stderr: {stderr}");
    assert_eq!(
        notice_line(&stderr, "File size limit exceeded"),
        2,
        "stderr: {stderr}"
    );
}
