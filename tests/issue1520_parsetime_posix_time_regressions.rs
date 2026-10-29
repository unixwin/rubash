//! rubash#1520 — parse timing of `set -o posix` inside one reader unit.
//!
//! GNU parses a whole `complete_command` (the full `;`-list up to the
//! newline / EOF) before executing any of it, and the posix dash-word
//! rule for `time` is a PARSE-time decision: `parse.y:3286
//! time_command_acceptable()` is consulted by `CHECK_FOR_RESERVED_WORD`
//! (parse.y:3179) when the `time` token is read — if POSIX mode is in
//! effect AT THAT MOMENT and the next token in the current input line
//! begins with `-`, `time` is not returned as the TIME reserved word.
//! A `set -o posix` earlier in the SAME unit therefore cannot change
//! how a later `time` in that unit was parsed; it only takes effect
//! from the next reader unit (the next line) on.
//!
//! rubash#1218 implemented the rule at execution time against the LIVE
//! mode, so a same-line `set` leaked into the decision. Every
//! expectation below is the GNU bash 5.3.20 oracle's behaviour for the
//! same form. Timing reports go to stderr (`real ...` lines).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// `-c` form under a cleared environment.
fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    run_rubash(&[], script)
}

/// `-c` form with leading shell flags (e.g. `--posix`).
fn rubash_c_with_flags(flags: &[&str], script: &str) -> (String, String, Option<i32>) {
    run_rubash(flags, script)
}

fn run_rubash(flags: &[&str], script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(flags)
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

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-1520-{tag}-{}", std::process::id()));
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
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_timed_ok(stdout: &str, stderr: &str, code: Option<i32>) {
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("real"),
        "expected a time report on stderr, got: {stderr}"
    );
    assert_eq!(stdout, "");
}

fn assert_time_not_found(stderr: &str, code: Option<i32>) {
    assert_eq!(code, Some(127), "stderr: {stderr}");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------
// The filed trigger: single-line -c, posix off -> on inside the unit.
// ---------------------------------------------------------------------

#[test]
fn issue1520_single_line_c_set_posix_time_dash_p_stays_keyword() {
    let (stdout, stderr, code) = rubash_c("set -o posix; PATH=/nonexistent; time -p true");
    assert_timed_ok(&stdout, &stderr, code);
}

#[test]
fn issue1520_single_line_script_file_set_posix_time_dash_p_stays_keyword() {
    let scratch = Scratch::new("one-line-file");
    let (stdout, stderr, code) =
        scratch.rubash_file("set -o posix; PATH=/nonexistent; time -p true\n");
    assert_timed_ok(&stdout, &stderr, code);
}

#[test]
fn issue1520_set_after_time_on_same_line_is_irrelevant() {
    let (stdout, stderr, code) = rubash_c("PATH=/nonexistent; set -o posix; time -p true");
    assert_timed_ok(&stdout, &stderr, code);
}

// ---------------------------------------------------------------------
// Both flip directions inside one unit.
// ---------------------------------------------------------------------

#[test]
fn issue1520_flip_on_then_off_same_line_stays_keyword() {
    let (stdout, stderr, code) =
        rubash_c("set -o posix; set +o posix; PATH=/nonexistent; time -p true");
    assert_timed_ok(&stdout, &stderr, code);
}

/// Reverse direction: the unit STARTS in posix mode (`--posix`), so the
/// whole line is parsed with the dash-word rule active; a same-line
/// `set +o posix` executes too late to rescue the keyword.
#[test]
fn issue1520_posix_from_start_set_plus_o_same_line_still_not_keyword() {
    let (_stdout, stderr, code) = rubash_c_with_flags(&["--posix"], "set +o posix; time -p true");
    assert_time_not_found(&stderr, code);
}

// ---------------------------------------------------------------------
// Controls: the NEXT reader unit does see the new mode (the #1110 pin
// family) — these must keep returning 127.
// ---------------------------------------------------------------------

#[test]
fn issue1520_multiline_c_next_line_sees_posix() {
    let (stdout, stderr, code) = rubash_c("set -o posix\nPATH=/nonexistent\ntime -p true\n");
    assert_eq!(stdout, "");
    assert_time_not_found(&stderr, code);
}

#[test]
fn issue1520_multiline_script_file_next_line_sees_posix() {
    let scratch = Scratch::new("multi-line-file");
    let (stdout, stderr, code) =
        scratch.rubash_file("set -o posix\nPATH=/nonexistent\ntime -p true\n");
    assert_eq!(stdout, "");
    assert_time_not_found(&stderr, code);
}

// ---------------------------------------------------------------------
// The unit is the whole list: groups, && lists and function bodies
// defined in the unit are parsed with the unit-start mode.
// ---------------------------------------------------------------------

#[test]
fn issue1520_group_after_semicolon_parsed_with_whole_line() {
    let (stdout, stderr, code) = rubash_c("set -o posix; { PATH=/nonexistent; time -p true; }");
    assert_timed_ok(&stdout, &stderr, code);
}

#[test]
fn issue1520_set_inside_group_same_unit() {
    let (stdout, stderr, code) = rubash_c("{ set -o posix; PATH=/nonexistent; time -p true; }");
    assert_timed_ok(&stdout, &stderr, code);
}

#[test]
fn issue1520_and_or_list_parsed_upfront() {
    let (stdout, stderr, code) = rubash_c("set -o posix && PATH=/nonexistent; time -p true");
    assert_timed_ok(&stdout, &stderr, code);
}

/// GNU parses a multi-line group as ONE complete_command, so the
/// `set` on line 2 is invisible to line 3's parse and `time` stays
/// the keyword (rc 0). rubash's feeder used to flip its parse mode at
/// LINE granularity while gathering the group, so line 3 was lexed
/// under posix and `time` was frozen as a plain word (rc 127). Fixed
/// by rubash#1544: the feeder parks the flip while a unit is open and
/// lands it at the commit that closes the unit — this pin is active
/// again (see the issue1544 pin file for the full surface).
#[test]
fn issue1520_group_spanning_lines_parsed_before_execution() {
    let (stdout, stderr, code) = rubash_c("{\nset -o posix\ntime -p true\n}");
    assert_timed_ok(&stdout, &stderr, code);
}

#[test]
fn issue1520_function_body_frozen_at_definition_unit() {
    let (stdout, stderr, code) =
        rubash_c("f() { time -p true; }; set -o posix; PATH=/nonexistent; f");
    assert_timed_ok(&stdout, &stderr, code);
}

// ---------------------------------------------------------------------
// The `time` keyword itself: bare form, other dash words, and a dash
// that only appears after expansion (never a parse-time dash word).
// ---------------------------------------------------------------------

#[test]
fn issue1520_bare_time_after_set_posix_stays_keyword() {
    let (stdout, stderr, code) = rubash_c("set -o posix; time true");
    assert_timed_ok(&stdout, &stderr, code);
}

#[test]
fn issue1520_time_dash_dash_after_set_posix_stays_keyword() {
    let (stdout, stderr, code) = rubash_c("set -o posix; PATH=/nonexistent; time -- true");
    assert_timed_ok(&stdout, &stderr, code);
}

/// Parse-time peek sees the TOKEN `$opt`, not its expansion, so `time`
/// is the keyword even in posix mode (multi-line form; GNU prints the
/// timing report, then the expanded `-p` is not a command: rc 127 with
/// the report still emitted — asserted loosely as "report present").
#[test]
fn issue1520_expanded_dash_is_not_a_parse_time_dash_word() {
    let (_stdout, stderr, code) = rubash_c("set -o posix\nopt=-p\ntime $opt true\n");
    assert_eq!(code, Some(127), "stderr: {stderr}");
    assert!(
        stderr.contains("real"),
        "expected a time report on stderr, got: {stderr}"
    );
    assert!(stderr.contains("-p: command not found"), "stderr: {stderr}");
}

// ---------------------------------------------------------------------
// Reparse-at-execution contexts keep using the mode live at their own
// parse moment: eval strings and command-substitution bodies.
// ---------------------------------------------------------------------

#[test]
fn issue1520_eval_reparse_uses_live_posix() {
    let (stdout, stderr, code) = rubash_c("set -o posix; eval \"time -p true\"");
    assert_eq!(stdout, "");
    assert_time_not_found(&stderr, code);
}

#[test]
fn issue1520_comsub_body_reparse_uses_live_posix() {
    let (stdout, stderr, code) =
        rubash_c("set -o posix; PATH=/nonexistent; x=$(time -p true 2>&1); echo \"rc=$?\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n");
}

// ---------------------------------------------------------------------
// Same invariant, neighbouring head form: a QUOTED `time` was never
// the reserved word (parse.y CHECK_FOR_RESERVED_WORD only fires for an
// unquoted token), in either mode.
// ---------------------------------------------------------------------

#[test]
fn issue1520_quoted_time_is_not_the_keyword() {
    let (stdout, stderr, code) = rubash_c("'time' -p true");
    assert_eq!(stdout, "");
    assert_time_not_found(&stderr, code);
    // No dash word involved: a quoted head is an ordinary command name
    // whatever follows it.
    let (stdout, stderr, code) = rubash_c("'time' true");
    assert_eq!(stdout, "");
    assert_time_not_found(&stderr, code);
}
