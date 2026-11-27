//! rubash#1544 — the feeder's `set -o/+o posix` flip must land at
//! complete_command granularity, not line granularity.
//!
//! GNU's reader parses ONE complete_command in full before executing
//! any of it (parse.y read_command; evalstring.c parse_and_execute
//! cadence), and the posix `time` dash-word verdict is a parse-time
//! decision (parse.y:3179 CHECK_FOR_RESERVED_WORD -> parse.y:3286
//! time_command_acceptable; see the issue1520 pin file). A `set`
//! inside a still-open unit — a brace group, an if/while/case body, a
//! function definition, a subshell, a list continued by a trailing
//! connector, a unit awaiting heredoc bodies — therefore cannot
//! change how the rest of THAT unit is read; the flip applies from the
//! next unit on.
//!
//! The batch feeder used to apply its per-line `line_posix_mode_change`
//! flip the moment a `set` line was lexed, so lines of the same unit
//! gathered afterwards were lexed under the flipped mode and a later
//! `time -p` was frozen as a plain word (`time: command not found`,
//! rc 127 where GNU reports the timing and rc 0). The flip is now
//! parked while a unit is open and lands at the commit that closes it.
//!
//! Every script below starts with `PATH=/nonexistent` in its own first
//! unit, so a `time` frozen as a plain word fails the lookup (127, no
//! timing report) while a keyword `time -p` runs and reports (rc 0).
//! Expectations are the GNU bash 5.3.20 oracle's for the same files.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-1544-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    /// Script-file form: file in a fresh cwd, PATH/HOME pinned.
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

    /// `--posix` script-file form (the mirror flip direction).
    fn rubash_file_posix(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("--posix")
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

/// `-c` form under a cleared environment.
fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// stdin form: the script is piped to a bare `rubash`.
fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script to stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The unit's `time -p` stayed the keyword: the trailing marker reports
/// the timed command's status 0 and no lookup failure was diagnosed.
fn assert_after_ok(stdout: &str, stderr: &str) {
    assert_eq!(stdout, "after=0\n", "stderr: {stderr}");
    assert!(
        !stderr.contains("command not found"),
        "unexpected lookup failure: {stderr}"
    );
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

// ---------------------------------------------------------------------
// The filed form: a multi-line brace group, through every entry point.
// ---------------------------------------------------------------------

const GROUP_SCRIPT: &str = "PATH=/nonexistent\n{\nset -o posix\ntime -p true\n}\necho after=$?\n";

#[test]
fn issue1544_brace_group_script_file() {
    let scratch = Scratch::new("group-file");
    let (stdout, stderr, code) = scratch.rubash_file(GROUP_SCRIPT);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

#[test]
fn issue1544_brace_group_command_string() {
    let (stdout, stderr, code) = rubash_c(GROUP_SCRIPT);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

#[test]
fn issue1544_brace_group_stdin() {
    let (stdout, stderr, code) = rubash_stdin(GROUP_SCRIPT);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

#[test]
fn issue1544_brace_group_eval() {
    let (stdout, stderr, code) =
        rubash_c("PATH=/nonexistent; eval '{\nset -o posix\ntime -p true\n}'; echo after=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

// ---------------------------------------------------------------------
// Compound bodies of every keyword family.
// ---------------------------------------------------------------------

#[test]
fn issue1544_if_body() {
    let scratch = Scratch::new("if-body");
    let (stdout, stderr, code) = scratch.rubash_file(
        "PATH=/nonexistent\nif true; then\nset -o posix\ntime -p true\nfi\necho after=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

#[test]
fn issue1544_while_body() {
    let scratch = Scratch::new("while-body");
    let (stdout, stderr, code) = scratch.rubash_file(
        "PATH=/nonexistent\nn=0\nwhile [ $n -lt 1 ]; do\nn=1\nset -o posix\ntime -p true\ndone\necho after=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

#[test]
fn issue1544_case_arm_set_on_own_line() {
    let scratch = Scratch::new("case-arm");
    let (stdout, stderr, code) = scratch.rubash_file(
        "PATH=/nonexistent\ncase x in\nx)\nset -o posix\ntime -p true\n;;\nesac\necho after=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

#[test]
fn issue1544_subshell_body_and_no_leak_after() {
    let scratch = Scratch::new("subshell");
    let (stdout, stderr, code) = scratch.rubash_file(
        "PATH=/nonexistent\n(\nset -o posix\ntime -p true\n)\necho after=$?\ntime -p true\necho outer=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // The subshell's own `time` is judged at the unit's start mode, and
    // the subshell's `set` never reaches the parent reader: the outer
    // `time` on the next line is still the keyword.
    assert_eq!(stdout, "after=0\nouter=0\n", "stderr: {stderr}");
    assert!(
        !stderr.contains("command not found"),
        "unexpected lookup failure: {stderr}"
    );
}

#[test]
fn issue1544_function_definition_then_call() {
    let scratch = Scratch::new("func");
    let (stdout, stderr, code) = scratch
        .rubash_file("PATH=/nonexistent\nf() {\nset -o posix\ntime -p true\n}\nf\necho after=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

#[test]
fn issue1544_nested_groups() {
    let scratch = Scratch::new("nested");
    let (stdout, stderr, code) = scratch
        .rubash_file("PATH=/nonexistent\n{\n{\nset -o posix\n}\ntime -p true\n}\necho after=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

// ---------------------------------------------------------------------
// Unit-boundary precision: the flip lands exactly when the unit ends.
// ---------------------------------------------------------------------

/// The closing line still belongs to the unit: `time` after `};` on the
/// same line is judged with the whole unit, at its start mode.
#[test]
fn issue1544_time_on_the_closing_line() {
    let scratch = Scratch::new("closing-line");
    let (stdout, stderr, code) =
        scratch.rubash_file("PATH=/nonexistent\n{\nset -o posix\n}; time -p true\necho after=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

/// A list continued by a trailing `&&` is one unit across the newline.
#[test]
fn issue1544_connector_continued_list() {
    let scratch = Scratch::new("connector");
    let (stdout, stderr, code) =
        scratch.rubash_file("PATH=/nonexistent\nset -o posix &&\ntime -p true\necho after=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_after_ok(&stdout, &stderr);
}

/// After the unit closes (and executes), the NEXT unit does see the
/// flip: exactly one lookup failure, from the line after the group.
#[test]
fn issue1544_flip_lands_for_the_next_unit() {
    let scratch = Scratch::new("next-unit");
    let (stdout, stderr, code) = scratch.rubash_file(
        "PATH=/nonexistent\n{\nset -o posix\ntime -p true\n}\ntime -p true\necho after=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after=127\n", "stderr: {stderr}");
    assert_eq!(
        stderr.matches("time: command not found").count(),
        1,
        "exactly the post-group line must fail: {stderr}"
    );
}

/// Mirror direction: the unit starts in posix mode (--posix) and the
/// in-unit `set +o posix` must not rescue the unit's own `time`.
#[test]
fn issue1544_mirror_group_under_posix_start() {
    let scratch = Scratch::new("mirror");
    let (stdout, stderr, code) = scratch
        .rubash_file_posix("PATH=/nonexistent\n{\nset +o posix\ntime -p true\n}\necho after=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
}

/// A unit awaiting heredoc bodies is still one unit: the `set` after
/// the heredoc command and the `time` after the body both belong to
/// the group that opened before them.
#[test]
fn issue1544_heredoc_inside_group() {
    let scratch = Scratch::new("heredoc-group");
    let (stdout, stderr, code) = scratch.rubash_file(
        "PATH=/nonexistent\n{\ncat <<EOF\nbody\nEOF\nset -o posix\ntime -p true\n}\necho after=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after=0\n", "stderr: {stderr}");
    // GNU: `cat` does not resolve under the broken PATH (its failure
    // does not change the group's last status — `time` runs last).
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
}
