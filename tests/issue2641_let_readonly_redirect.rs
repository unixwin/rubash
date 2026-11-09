//! rubash#2641 — `let`'s readonly-variable diagnostic bypasses the
//! command's own stderr redirection (`2>&1` / `2>file` / `2>/dev/null`
//! all ineffective); it lands on the process's original stderr.
//!
//! GNU anchor: `let` assigns through expr.c:331-349
//! `expr_bind_variable` -> `bind_int_variable` (array elements via
//! expr.c:382-397 `expr_bind_array_element`), and the readonly refusal
//! is reported by error.c:455 `report_error("%s: readonly variable")`
//! (builtins/common.c:242 `builtin_error` carries the same text on the
//! builtin path). The builtin executes after the command's
//! redirections have been applied, so the diagnostic is written to the
//! command's current fd 2 and follows the redirection; the original
//! stderr stays empty, the exit status is 1, and the variable is
//! unchanged. Array-element refusals name the array (`A: readonly
//! variable`).
//!
//! rubash drains the arithmetic readonly marker through
//! executor/arithmetic/mod.rs:1464-1481
//! `report_arithmetic_readonly_error`, which prints with a raw
//! `eprintln!` to the process stderr (line 1477), bypassing the
//! executor's per-command output/redirection plumbing — so every
//! redirection shape leaks. Evaluation errors (division by zero)
//! instead travel as values to the command layer and are printed
//! through the redirection-aware channel, which is why they are a
//! passing control here.
//!
//! Expectations are from the GNU bash 5.3.20 oracle (probes in the
//! lane REPORT). The diagnostic line is asserted by its stable tail
//! `: <name>: readonly variable` (the program-name prefix varies with
//! the invocation name, per the repo's existing readonly pins); the
//! stream placement, the remaining stdout lines, the exit status and
//! the variable's value are asserted byte-for-byte.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`. `LC_ALL=C.UTF-8`
//! is set on the child explicitly so the pin is hermetic under any
//! ambient locale.

use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    run_at(script, None)
}

fn run_at(script: &str, cwd: Option<&PathBuf>) -> Outcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_string)
        .collect()
}

/// The redirected shapes: the diagnostic line must be the FIRST stdout
/// line (matched by its stable tail), followed byte-for-byte by
/// `rest`; the process stderr must be completely empty; rc 0 (the
/// probe scripts end in `echo`).
fn expect_redirected(script: &str, name: &str, rest: &[&str], ctx: &str) {
    let out = run(script);
    let stdout_lines = lines(&out.stdout);
    let needle = format!(": {name}: readonly variable");
    assert!(
        stdout_lines.first().is_some_and(|l| l.contains(&needle)),
        "{ctx}: first stdout line must be the readonly diagnostic ending {needle:?}; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let tail: Vec<&str> = stdout_lines[1..].iter().map(String::as_str).collect();
    assert_eq!(
        tail,
        rest,
        "{ctx}: stdout lines after the diagnostic must match exactly; full stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty (the diagnostic belongs to the redirected stream); got {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "{ctx}: rc must be 0");
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue2641-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------
// Defect cells: the diagnostic must follow the command's redirection.
// ---------------------------------------------------------------------

#[test]
fn seed_redirect_2to1() {
    // The issue's regression seed.
    expect_redirected(
        "readonly R=1; let \"R=2\" 2>&1; echo rc=$?; echo R=$R",
        "R",
        &["rc=1", "R=1"],
        "seed: let R=2 2>&1",
    );
}

#[test]
fn redirect_to_file() {
    let td = TempDir::new("file");
    let out = run_at(
        "readonly R=1; let \"R=2\" 2>err.txt; echo rc=$?",
        Some(&td.0),
    );
    assert_eq!(
        out.stdout,
        b"rc=1\n",
        "2>file: stdout must be just the rc line; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "2>file: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "2>file: rc must be 0");
    let content = std::fs::read_to_string(td.0.join("err.txt")).expect("err.txt must exist");
    let content_lines = lines(content.as_bytes());
    assert_eq!(
        content_lines.len(),
        1,
        "2>file: err.txt must hold exactly the one diagnostic line; got {content:?}"
    );
    assert!(
        content_lines[0].contains(": R: readonly variable"),
        "2>file: err.txt line must be the readonly diagnostic; got {content:?}"
    );
}

#[test]
fn redirect_devnull_silences() {
    let out = run("readonly R=1; let \"R=2\" 2>/dev/null; echo rc=$?");
    assert_eq!(
        out.stdout,
        b"rc=1\n",
        "2>/dev/null: stdout must be just the rc line; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "2>/dev/null: stderr must be empty (fully silenced); got {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "2>/dev/null: rc must be 0");
}

#[test]
fn pipeline_redirect() {
    expect_redirected(
        "readonly R=1; let \"R=2\" 2>&1 | cat; echo done",
        "R",
        &["done"],
        "pipeline: let R=2 2>&1 | cat",
    );
}

#[test]
fn subshell_redirect() {
    expect_redirected(
        "readonly R=1; ( let \"R=2\" 2>&1; echo rc=$? )",
        "R",
        &["rc=1"],
        "subshell: ( let R=2 2>&1; ... )",
    );
}

#[test]
fn function_redirect() {
    expect_redirected(
        "readonly R=1; f(){ let \"R=2\" 2>&1; echo rc=$?; }; f",
        "R",
        &["rc=1"],
        "function: let R=2 2>&1 inside f",
    );
}

#[test]
fn array_element_redirect() {
    // GNU names the array, not the subscripted element.
    expect_redirected(
        "readonly -a A=(1 2); let \"A[0]=9\" 2>&1; echo rc=$?; echo A0=${A[0]}",
        "A",
        &["rc=1", "A0=1"],
        "array element: let A[0]=9 2>&1 on a readonly array",
    );
}

#[test]
fn chain_file_then_stdout() {
    // Chain: the first refusal is captured to a file (asserted both
    // through the later `cat` in the same script and by reading the
    // file afterwards), the second is redirected to stdout, and the
    // variable never changes.
    let td = TempDir::new("chain");
    let out = run_at(
        "readonly R=1; let \"R=2\" 2>err.txt; echo \"after1 rc=$?\"; let \"R=3\" 2>&1; echo \"after2 rc=$?\"; echo \"file:[$(cat err.txt)]\"; echo R=$R",
        Some(&td.0),
    );
    let stdout_lines = lines(&out.stdout);
    assert_eq!(
        stdout_lines.len(),
        5,
        "chain: stdout must have 5 lines; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(stdout_lines[0], "after1 rc=1", "chain: first let fails");
    assert!(
        stdout_lines[1].contains(": R: readonly variable"),
        "chain: second diagnostic must be on stdout; got {:?}",
        stdout_lines[1]
    );
    assert_eq!(stdout_lines[2], "after2 rc=1", "chain: second let fails");
    assert!(
        stdout_lines[3].starts_with("file:[")
            && stdout_lines[3].ends_with(']')
            && stdout_lines[3].contains(": R: readonly variable"),
        "chain: the file captured the first diagnostic; got {:?}",
        stdout_lines[3]
    );
    assert_eq!(stdout_lines[4], "R=1", "chain: R never changes");
    assert!(
        out.stderr.is_empty(),
        "chain: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "chain: rc must be 0");
    let content = std::fs::read_to_string(td.0.join("err.txt")).expect("err.txt must exist");
    assert_eq!(
        lines(content.as_bytes()).len(),
        1,
        "chain: err.txt holds exactly the first diagnostic; got {content:?}"
    );
}

// ---------------------------------------------------------------------
// Regression locks / positive controls: shapes that already agree
// with GNU on the base tree and must not move.
// ---------------------------------------------------------------------

#[test]
fn no_redirect_lock() {
    // Without a redirection the diagnostic belongs on stderr.
    let out = run("readonly R=1; let \"R=2\"; echo rc=$?");
    assert_eq!(
        out.stdout,
        b"rc=1\n",
        "no redirect: stdout must be just the rc line; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr_lines = lines(&out.stderr);
    assert_eq!(
        stderr_lines.len(),
        1,
        "no redirect: exactly one diagnostic line on stderr; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stderr_lines[0].contains(": R: readonly variable"),
        "no redirect: stderr line must be the readonly diagnostic; got {}",
        stderr_lines[0]
    );
    assert_eq!(out.code, 0, "no redirect: rc must be 0");
}

#[test]
fn plain_assignment_fatal_lock() {
    // A bare assignment to a readonly variable is fatal to a
    // non-interactive shell: the diagnostic is on stderr, the
    // following command never runs, and the shell exits 1.
    let out = run("readonly R=1; R=2 2>&1; echo rc=$?");
    assert!(
        out.stdout.is_empty(),
        "plain assignment: stdout must be empty (echo never runs); got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.code, 1, "plain assignment: shell must exit 1");
    let stderr_lines = lines(&out.stderr);
    assert!(
        stderr_lines
            .iter()
            .any(|l| l.contains(": R: readonly variable")),
        "plain assignment: stderr must carry the readonly diagnostic; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn eval_control_lock() {
    // The same refusal reached through `eval` already follows the
    // redirection on the base tree (different emission path).
    expect_redirected(
        "readonly R=1; eval \"R=2\" 2>&1; echo rc=$?; echo R=$R",
        "R",
        &["rc=1", "R=1"],
        "control: eval R=2 2>&1",
    );
}

#[test]
fn divzero_control_lock() {
    // `let`'s own evaluation errors already follow the redirection on
    // the base tree — only the readonly bind report leaks.
    let out = run("let \"q=1/0\" 2>&1; echo rc=$?");
    let stdout_lines = lines(&out.stdout);
    assert!(
        stdout_lines
            .first()
            .is_some_and(|l| { l.contains("let: q=1/0: division by 0 (error token is \"0\")") }),
        "div-zero control: diagnostic must be on stdout; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        stdout_lines.get(1).map(String::as_str),
        Some("rc=1"),
        "div-zero control: rc line must follow; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "div-zero control: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "div-zero control: rc must be 0");
}
