//! rubash#3354: the embedded process-substitution span matcher is
//! blind to backquote groups — an unmatched `(` inside a backquote
//! group inflates the paren depth, the true closer never balances,
//! and the whole span goes unclaimed; the expander then treats the
//! body text as word-level text and expands away even the backquote
//! group inside it.
//!
//! GNU anchor: subst.c:1311 `extract_process_subst` hands the span
//! to `xparse_dolparen` (parse.y:4686), which re-parses the body
//! with the real parser, and parse.y:3877 `parse_matched_pair`
//! consumes nested substitution units — including backquote groups,
//! via the shellquote arm — whole. A `(` inside a backquote group
//! is data for the inner command text, never a nesting level of
//! the outer span.
//!
//! Rust owners: `embedded_procsub_close`
//! (src/parser/process_substitution.rs) and its byte twin
//! `skip_procsub_paren` (src/parser/token_actions.rs) both counted
//! parens through backquote spans; the fix skips a backquote group
//! whole via the existing `backquote_group_end` walker (PR #3351).
//!
//! Expectations are GNU bash 5.3.20 behavior, measured with the
//! fix lane's probes (probe-3354-repro.sh, probe-3354-matrix.sh,
//! probe-3354-quote.sh). One test per face:
//!   01 — the issue shape: assignment RHS, unmatched `(` inside a
//!        backquote group in the procsub body
//!   02 — control: the word-initial form of the same body, which
//!        GNU and the base already agree on (green on base and head)
//!   03 — the compound-assignment face (`skip_procsub_paren`):
//!        the same body as an array element
//!   04 — guard: a backquote group with a BALANCED paren inside
//!        still works (green on base and head)
//!   05 — guard: an escaped backquote is literal; a following
//!        `>(...)` is a real top-level procsub (green on base
//!        and head)
//!   06 — the quote-state face: an unmatched `"` inside the
//!        backquote group corrupts the matcher's quote state and
//!        swallows the true closer the same way

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
    let dir = std::env::temp_dir().join(format!("rubash-issue3354-{tag}-{}", std::process::id()));
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

// 01 — the issue shape (see the file header). GNU 5.3.20:
// `[pre/dev/fd/63post] cap=[hello]` (plus the inner comsub's
// syntax-error diagnostic on stderr, which is NOT a divergence
// point — both shells emit it). On the base the span is never
// claimed: `[pre>(echo ; printf 'hello\n' > ./cap.txt)post]
// cap=[]` and the body never runs.
#[test]
fn issue3354_01_assignment_procsub_backquote_unbalanced_paren() {
    let dir = test_dir("01");
    let output = run_single_process(
        &dir,
        "v=pre>(echo `echo (`; printf 'hello\\n' > ./cap.txt)post\nsleep 1\necho \"V:[${v}] CAP:[$(cat cap.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // The body's failed backquote leaves an empty `echo` line on the
    // procsub pipe, which reaches stdout as a blank line on GNU too,
    // so the V line is picked out by prefix.
    let line = stdout
        .lines()
        .find(|l| l.starts_with("V:["))
        .unwrap_or_default();
    assert!(
        line.starts_with("V:[pre/dev/fd/") && line.ends_with("post] CAP:[hello]"),
        "stdout: {stdout}"
    );
    let cap = read_or_empty(&dir, "cap.txt");
    assert_eq!(cap, "hello\n", "cap.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 02 — control: the word-initial form of the same body. GNU and
// the base already agree (cap=[hello]); the defect is specific
// to the embedded span matcher. Green on base and head.
#[test]
fn issue3354_02_word_initial_form_control() {
    let dir = test_dir("02");
    let output = run_single_process(
        &dir,
        "echo >(echo `echo (`; printf 'hello\\n' > ./cap.txt) > /dev/null\nsleep 1\necho \"CAP:[$(cat cap.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // The body's failed backquote leaves an empty `echo` line on the
    // procsub pipe, which reaches stdout as a blank line on GNU too,
    // so the CAP line is picked out by prefix.
    let line = stdout
        .lines()
        .find(|l| l.starts_with("CAP:["))
        .unwrap_or_default();
    assert_eq!(line, "CAP:[hello]", "stdout: {stdout}");
    let cap = read_or_empty(&dir, "cap.txt");
    assert_eq!(cap, "hello\n", "cap.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 03 — the compound-assignment face: the same body as an array
// element, where the span is skipped by `skip_procsub_paren`
// (src/parser/token_actions.rs). GNU 5.3.20:
// `a0=[x/dev/fd/63y] cap2=[hello]`. On the base the element
// boundary is misread (`a0=[(x/dev/fd/62`).
#[test]
fn issue3354_03_compound_assignment_procsub_backquote_paren() {
    let dir = test_dir("03");
    let output = run_single_process(
        &dir,
        "a=(x>(echo `echo (`; printf 'hello\\n' > ./cap2.txt)y)\nsleep 1\necho \"A0:[${a[0]}] CAP2:[$(cat cap2.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let line = stdout
        .lines()
        .find(|l| l.starts_with("A0:["))
        .unwrap_or_default();
    assert!(
        line.starts_with("A0:[x/dev/fd/") && line.ends_with("y] CAP2:[hello]"),
        "stdout: {stdout}"
    );
    let cap = read_or_empty(&dir, "cap2.txt");
    assert_eq!(cap, "hello\n", "cap.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 04 — guard: a backquote group with a balanced paren inside still
// works; the skip must not over-reach past genuine spans. Green
// on base and head. (The body's `echo hi` output reaches stdout
// through the procsub pipe, so the V2 line is picked out by
// prefix rather than asserting the whole stream.)
#[test]
fn issue3354_04_backquote_group_with_balanced_paren() {
    let dir = test_dir("04");
    let output = run_single_process(
        &dir,
        "v2=pre>(echo `echo hi`; printf 'yo\\n' > ./cap3.txt)post\nsleep 1\necho \"V2:[${v2}] CAP3:[$(cat cap3.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let line = stdout
        .lines()
        .find(|l| l.starts_with("V2:["))
        .unwrap_or_default();
    assert!(
        line.starts_with("V2:[pre/dev/fd/") && line.ends_with("post] CAP3:[yo]"),
        "stdout: {stdout}"
    );
    let cap = read_or_empty(&dir, "cap3.txt");
    assert_eq!(cap, "yo\n", "cap3.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 05 — guard: an escaped backquote is a literal character, NOT a
// group opener; the `>(...)` after it is a real top-level process
// substitution of this word and must still materialize. Green on
// base and head: the skip must not treat `\`` as a span start.
#[test]
fn issue3354_05_escaped_backquote_does_not_open_a_group() {
    let dir = test_dir("05");
    let output = run_single_process(
        &dir,
        "v3=\\`>(printf 'z\\n' > ./cap3.txt)\nsleep 1\necho \"V3:[${v3}] CAP3:[$(cat cap3.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let line = stdout.lines().next().unwrap_or_default();
    assert!(
        line.starts_with("V3:[`/dev/fd/") && line.ends_with("] CAP3:[z]"),
        "stdout: {stdout}"
    );
    let cap = read_or_empty(&dir, "cap3.txt");
    assert_eq!(cap, "z\n", "cap3.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 06 — the quote-state face: an unmatched `"` inside the backquote
// group corrupts the matcher's quote state (the `"` toggles
// double-quote mode, so the true closer `)` is read as quoted
// data and the span is never claimed). GNU 5.3.20:
// `[pre/dev/fd/63post] cap=[hello]` with the same inner
// "unexpected EOF while looking for matching `\"'" diagnostic
// both shells emit. On the base: span unclaimed, cap empty.
#[test]
fn issue3354_06_backquote_group_with_unmatched_dquote() {
    let dir = test_dir("06");
    let output = run_single_process(
        &dir,
        "v=pre>(echo `echo \"x`; printf 'hello\\n' > ./cap.txt)post\nsleep 1\necho \"V:[${v}] CAP:[$(cat cap.txt)]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // Same leading blank line as test 01 (the failed backquote's
    // empty `echo` on the procsub pipe); pick the V line by prefix.
    let line = stdout
        .lines()
        .find(|l| l.starts_with("V:["))
        .unwrap_or_default();
    assert!(
        line.starts_with("V:[pre/dev/fd/") && line.ends_with("post] CAP:[hello]"),
        "stdout: {stdout}"
    );
    let cap = read_or_empty(&dir, "cap.txt");
    assert_eq!(cap, "hello\n", "cap.txt: {cap:?}");
    let _ = fs::remove_dir_all(&dir);
}
