//! rubash#817 regression: the alias same-read visibility guard compared the
//! definition's PHYSICAL line against `__RUBASH_CURRENT_LINE`, so a
//! definition recorded inside a nested read unit — a `source`d file
//! (renumbered from 1 per file) or an `eval` string (renumbered
//! `caller_line + i - 1`) — collided with the caller's own later physical
//! lines and was suppressed with `command not found` where GNU bash
//! expands (the rule is same-READ, not same-line; parse.y read-time
//! expansion, evalfile.c push_stream / run_unwind_frame). The fix stamps
//! each definition with a monotonic READ-UNIT GENERATION
//! (`__RUBASH_ALIAS_GEN_<name>`), pushed/popped at the source/eval seams,
//! and the guard suppresses only when generation AND line both match.
//!
//! Differential oracle: WSL GNU bash 5.3.0(1), stdin method
//! (`wsl bash --noprofile --norc < probe`) — every expectation below was
//! byte-verified against it. Rows 1-5 of the guard-rail table from the
//! niubash i368 audit (`docs/audit/findings/i368-read-unit-line-identity.md`)
//! are pinned here as suppressed/expand pairs; row 9 pins the grouped `-c`
//! driver (no marker guard); rows 6-8 exercise the `-i` host route and stay
//! on the interactive harness side (no marker guard either).
//!
//! Owner: src/executor/arithmetic_aliases.rs alias_defined_on_current_line,
//! src/executor/alias_set_builtins.rs record/clear, src/shell/state.rs
//! read_generation, src/builtins/source/execution.rs enter/exit_read_unit,
//! src/executor/trap_exec.rs execute_eval_source.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Outcome {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// Feed `script` to the shell's stdin (the run_stdin_script route, where
/// the executor-level alias guard lives), running in `cwd` so `source`
/// operands resolve.
fn run_stdin_script_in(cwd: &Path, script: &str) -> Outcome {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--noprofile", "--norc"])
        .current_dir(cwd)
        .env("HOME", cwd)
        .env("USERPROFILE", cwd)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut stdin = child.stdin.take().expect("stdin");
    let bytes = script.as_bytes().to_vec();
    std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&bytes);
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().expect("poll rubash").is_none() {
        assert!(Instant::now() < deadline, "rubash stdin probe timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("wait for rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// Same, with the script handed to `-c` (the grouped driver route, which
/// expands aliases at text level against the live table and never consults
/// the marker guard).
fn run_dash_c_in(cwd: &Path, script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--noprofile", "--norc", "-c", script])
        .current_dir(cwd)
        .env("HOME", cwd)
        .env("USERPROFILE", cwd)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash -c");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// Scratch directory with sourced-file payloads written, unique per test
/// (tests run in parallel threads of one process).
fn scratch_dir(tag: &str, files: &[(&str, &str)]) -> LoopGuardDir {
    let directory = std::env::temp_dir().join(format!(
        "rubash-817-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch dir");
    for (name, content) in files {
        std::fs::write(directory.join(name), content).expect("write sourced file");
    }
    LoopGuardDir(directory)
}

struct LoopGuardDir(std::path::PathBuf);

impl Drop for LoopGuardDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Repro 1 (ticket): a definition on a sourced file's line 5 collides with
/// the sourcer's own later physical line 5. GNU expands (the definition
/// happened in an earlier read); the old line-only guard suppressed with
/// `command not found` / rc=127.
#[test]
fn source_defined_alias_use_at_colliding_caller_line_expands() {
    let dir = scratch_dir("repro1", &[("def5.sh", ":\n:\n:\n:\nalias x='echo AX'\n")]);
    let driver = "shopt -s expand_aliases\nsource def5.sh\n:\n:\nx\necho rc=$?\n";
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "AX\nrc=0\n");
    assert!(outcome.stderr.is_empty(), "{outcome:?}");
    assert_eq!(outcome.code, Some(0));
}

/// Repro 2 (ticket): an eval-body line renumbers onto the caller's
/// physical line; a use there must expand like GNU.
#[test]
fn eval_defined_alias_use_at_colliding_caller_line_expands() {
    let dir = scratch_dir("repro2", &[]);
    let driver = concat!(
        "shopt -s expand_aliases\n",
        "body=$': \\n: \\n: \\nalias y=\\'echo AY\\''\n",
        "eval \"$body\"\n",
        ":\n:\ny\n",
        "echo rc=$?\n"
    );
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "AY\nrc=0\n");
    assert!(outcome.stderr.is_empty(), "{outcome:?}");
    assert_eq!(outcome.code, Some(0));
}

/// Control for repro 2 (audit): the identical script with the use moved to
/// a non-colliding line — pins that the fix kills the COLLISION, not the
/// eval/source definition route.
#[test]
fn eval_defined_alias_use_at_non_colliding_line_still_expands() {
    let dir = scratch_dir("repro2ctl", &[]);
    let driver = concat!(
        "shopt -s expand_aliases\n",
        "body=$': \\n: \\n: \\nalias y=\\'echo AY\\''\n",
        "eval \"$body\"\n",
        ":\n:\n:\ny\n",
        "echo rc=$?\n"
    );
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "AY\nrc=0\n");
    assert!(outcome.stderr.is_empty(), "{outcome:?}");
    assert_eq!(outcome.code, Some(0));
}

/// Sibling read units never share a generation: the second `source` must
/// see the alias the first one defined even at the same physical line
/// (GNU expands). A depth-style counter would collide here; the monotonic
/// ceiling is load-bearing for this shape.
#[test]
fn sibling_source_units_do_not_share_read_generation() {
    let dir = scratch_dir(
        "sibling",
        &[
            ("def5.sh", ":\n:\n:\n:\nalias x='echo AX'\n"),
            ("b5.sh", ":\n:\n:\n:\nx\n"),
        ],
    );
    let driver = "shopt -s expand_aliases\nsource def5.sh\nsource b5.sh\necho rc=$?\n";
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "AX\nrc=0\n");
    assert!(outcome.stderr.is_empty(), "{outcome:?}");
    assert_eq!(outcome.code, Some(0));
}

// ---------------------------------------------------------------- -----
// Guard-rail rows that MUST NOT flip: same-read suppression shapes the
// guard exists for (all byte-verified against WSL GNU bash 5.3.0).
// ---------------------------------------------------------------------

/// Row 1: `alias ll=…; ll` on one physical line of a stdin script —
/// suppressed (127) in both shells.
#[test]
fn same_line_use_still_suppressed_on_stdin_script() {
    let dir = scratch_dir("row1", &[]);
    let driver = "shopt -s expand_aliases\nalias ll='echo LL'; ll\necho rc=$?\n";
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "rc=127\n");
    assert!(
        outcome.stderr.contains("line 2: ll: command not found"),
        "{outcome:?}"
    );
    assert_eq!(outcome.code, Some(0));
}

/// Row 2: same-line suppression INSIDE a sourced file; the diagnostic
/// carries the sourced file's own line, byte-identical with GNU
/// (`def2.sh: line 2: x2: command not found`).
#[test]
fn same_line_use_still_suppressed_inside_sourced_file() {
    let dir = scratch_dir("row2", &[("def2.sh", ":\nalias x2='echo XX'; x2\n")]);
    let driver = "shopt -s expand_aliases\nsource def2.sh\necho rc=$?\n";
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "rc=127\n");
    assert!(
        outcome
            .stderr
            .contains("def2.sh: line 2: x2: command not found"),
        "{outcome:?}"
    );
    assert_eq!(outcome.code, Some(0));
}

/// Row 3: same-line suppression inside an `eval` string — suppressed
/// (127) in both shells.
#[test]
fn same_line_use_still_suppressed_inside_eval_string() {
    let dir = scratch_dir("row3", &[]);
    let driver = concat!(
        "shopt -s expand_aliases\n",
        "eval $'alias z=\\'echo ZZ\\'; z'\n",
        "echo rc=$?\n"
    );
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "rc=127\n");
    assert!(
        outcome.stderr.contains("line 2: z: command not found"),
        "{outcome:?}"
    );
    assert_eq!(outcome.code, Some(0));
}

/// Row 4: definition line N, use at line N+1 of a plain stdin script —
/// expands in both shells.
#[test]
fn plain_script_next_line_use_still_expands() {
    let dir = scratch_dir("row4", &[]);
    let driver = "shopt -s expand_aliases\nalias w='echo WW'\nw\necho rc=$?\n";
    let outcome = run_stdin_script_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "WW\nrc=0\n");
    assert!(outcome.stderr.is_empty(), "{outcome:?}");
    assert_eq!(outcome.code, Some(0));
}

/// Row 9: the `-c` collision layout — GNU expands (the grouped driver
/// expands at text level against the live table, no marker guard).
#[test]
fn dash_c_collision_layout_still_expands() {
    let dir = scratch_dir("row9", &[("def5.sh", ":\n:\n:\n:\nalias x='echo AX'\n")]);
    let driver = "shopt -s expand_aliases\n. ./def5.sh\n:\n:\nx\necho rc=$?\n";
    let outcome = run_dash_c_in(&dir.0, driver);
    assert_eq!(outcome.stdout, "AX\nrc=0\n");
    assert!(outcome.stderr.is_empty(), "{outcome:?}");
    assert_eq!(outcome.code, Some(0));
}
