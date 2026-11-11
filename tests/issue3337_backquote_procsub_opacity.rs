//! rubash#3337: a process substitution inside a BACKQUOTE command
//! substitution is claimed by the OUTER word's raw procsub scan, so
//! the consumer is materialized at the outer word's level — where an
//! `exec 3<&1` inside the backquoted body has NOT rebound fd 3 to the
//! comsub capture pipe, and fd 3 is whatever the ambient context has
//! (here: the outer `exec 3>tap.log` TAP file). The `$(...)` form of
//! the same shape was settled by #3221/#3132: the parser's three raw
//! word scans (embedded extraction, has-unquoted admission, verbatim
//! span location in src/parser/process_substitution.rs) skip
//! `$`-introduced groups whole (dollar_group_end), because GNU
//! expand_word_internal consumes those units in their own switch
//! arms. The backquote group was not covered: GNU consumes it whole
//! in its own arm too (subst.c:11572 `case '`'`: string_extract pulls
//! the span, command_substitute (subst.c:7143) executes the body),
//! so a `<`/`>` inside one never reaches the `<`/`>` arms
//! (subst.c:11364-11378) at the outer word's level.
//!
//! Expectations are GNU bash 5.3.20 behavior, measured with the
//! probe matrix in the fix lane's probes (probe-bq-matrix.sh: GNU
//! 10/10). One test per face:
//!   01 — the issue shape: assignment RHS is a backquote group whose
//!        body rebinds fd 3 (`exec 3<&1`) and feeds `>(cat >&3)`
//!   02 — the group embedded in a larger word (prefix/suffix)
//!   03 — the group inside double quotes (backquote stays active)
//!   04 — the group in an argument word, not an assignment
//!   05 — an INPUT procsub `<(...)` inside the backquote body
//!   06 — (linux) the consumer's fd 3 IS the capture pipe (readlink),
//!        not the ambient log file — the mechanism face of 01
//!   07 — guard: an ESCAPED backquote is literal; a following `>(...)`
//!        is a real top-level procsub (green on base and head)
//!   08 — guard: a real top-level embedded procsub in a plain word
//!        still materializes (green on base and head)

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3337-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_single_process(dir: &Path, script: &str) -> std::process::Output {
    let path = dir.join("single.sh");
    fs::write(&path, script).expect("write single");
    let mut command = rubash_command();
    command.current_dir(dir).arg(shell_test_path(&path));
    command.output().expect("run single")
}

fn read_or_empty(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_default()
}

// 01 — the issue shape (see the file header). GNU 5.3.20: OUT
// carries hello twice, tap.log empty. On the base the outer scan
// claims `>(cat >&3)`, the consumer runs at the outer level with
// fd 3 = tap.log, so one hello lands in the log instead.
#[test]
fn issue3337_01_backquote_assignment_procsub_consumer_reaches_capture_pipe() {
    let dir = test_dir("01");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nout=`exec 3<&1; printf 'hello\\n' | tee >(cat >&3)`\necho \"OUT:[${out}] TAP:[$(cat tap.log | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[hello\nhello] TAP:[]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = read_or_empty(&dir, "tap.log");
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 02 — the backquote group embedded in a larger assignment word
// (literal prefix and suffix around it). Same claim face as 01:
// the embedded extraction scan walks the whole raw word.
#[test]
fn issue3337_02_backquote_group_embedded_in_word() {
    let dir = test_dir("02");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nout=pre`exec 3<&1; printf 'hello\\n' | tee >(cat >&3)`post\necho \"OUT:[${out}] TAP:[$(cat tap.log | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[prehello\nhellopost] TAP:[]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = read_or_empty(&dir, "tap.log");
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 03 — the backquote group inside double quotes. GNU still runs the
// command substitution there (only the `<`/`>` arms are suppressed
// by Q_DOUBLE_QUOTES, subst.c:11364-11378 — the backquote arm at
// subst.c:11572 is not), so the group's interior is still the inner
// command text's own business.
#[test]
fn issue3337_03_backquote_group_inside_double_quotes() {
    let dir = test_dir("03");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nout=\"`exec 3<&1; printf 'hello\\n' | tee >(cat >&3)`\"\necho \"OUT:[${out}] TAP:[$(cat tap.log | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[hello\nhello] TAP:[]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = read_or_empty(&dir, "tap.log");
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 04 — the backquote group in an argument word (word-splitting
// position), not an assignment RHS: the command-words metadata scan
// is the same raw scan family.
#[test]
fn issue3337_04_backquote_group_in_argument_word() {
    let dir = test_dir("04");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nprintf '%s\\n' `exec 3<&1; printf 'hello\\n' | tee >(cat >&3)` > a.out\necho \"AOUT:[$(cat a.out | tr '\\n' '|')] TAP:[$(cat tap.log | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "AOUT:[hello|hello|] TAP:[]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = read_or_empty(&dir, "tap.log");
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 05 — an INPUT process substitution inside the backquote body: the
// read loop's `< <(printf ...)` belongs to the inner command text.
#[test]
fn issue3337_05_input_procsub_inside_backquote_body() {
    let dir = test_dir("05");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nout=`exec 3<&1; while IFS= read -r line; do printf '%s\\n' \"$line\" >&3; done < <(printf 'hello\\n')`\necho \"OUT:[${out}] TAP:[$(cat tap.log | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[hello] TAP:[]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = read_or_empty(&dir, "tap.log");
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 06 — the mechanism face of 01, directly: the procsub consumer's
// fd 3 must be the comsub capture PIPE (what `exec 3<&1` bound it
// to inside the body), not the ambient tap.log regular file it is
// on the base. Linux-only: reads /proc/self/fd.
#[cfg(target_os = "linux")]
#[test]
fn issue3337_06_consumer_fd3_is_capture_pipe_not_ambient_file() {
    let dir = test_dir("06");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nout=`exec 3<&1; printf 'hello\\n' | tee >(readlink /proc/self/fd/3 > f.txt; cat >&3)`\necho \"OUT:[${out}]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[hello\nhello]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let desc = read_or_empty(&dir, "f.txt");
    assert!(
        desc.starts_with("pipe:"),
        "consumer fd 3 should be the capture pipe, got: {desc:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 07 — guard: an escaped backquote is a literal character, NOT a
// group opener; the `>(...)` after it is a real top-level process
// substitution of this word and must still materialize. Green on
// base and head: the skip must not treat `\`` as a span start.
#[test]
fn issue3337_07_escaped_backquote_does_not_open_a_group() {
    let dir = test_dir("07");
    let output = run_single_process(
        &dir,
        "v=\\`>(printf 'hello\\n' > cap.txt)\necho \"V:[${v}] CAP:[$(cat cap.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.starts_with("V:[`/dev/fd/") && stdout.ends_with("] CAP:[hello]\n"),
        "stdout: {stdout}"
    );
    let cap = read_or_empty(&dir, "cap.txt");
    assert_eq!(cap, "hello\n", "cap.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 08 — guard: a real top-level embedded process substitution in a
// plain word (no group anywhere) still materializes and splices:
// the opacity skip must not over-reach past genuine spans. Green on
// base and head. Assignment route on purpose: the value is the
// synchronous readback of the splice (an echo-argument form would
// hit the separate builtin-argument splice face recorded in the
// lane report — not this pin's subject). The sleep settles the
// async body before the cap readback.
#[test]
fn issue3337_08_top_level_embedded_procsub_still_materializes() {
    let dir = test_dir("08");
    let output = run_single_process(
        &dir,
        "v=pre>(printf 'hello\\n' > cap.txt)post\necho \"V:[${v}]\"\nsleep 1\necho \"CAP:[$(cat cap.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or_default();
    assert!(
        first.starts_with("V:[pre/dev/fd/") && first.ends_with("post]"),
        "first line: {first}"
    );
    assert_eq!(lines.next(), Some("CAP:[hello]"), "stdout: {stdout}");
    let cap = read_or_empty(&dir, "cap.txt");
    assert_eq!(cap, "hello\n", "cap.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}
