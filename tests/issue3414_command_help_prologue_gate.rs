//! rubash#3414 — the `command` re-dispatch's `--help` gate must
//! exit through the shared `apply_builtin_help`, so the target
//! builtin's pre-option-scan prologue runs there too.
//!
//! GNU rebuilds a bare simple command and calls `execute_command`
//! (builtins/command.def:141), so the target's own prologue runs
//! before its option scan:
//!
//! * `bind`: `bind.def:122-123` prints `bind: warning: line
//!   editing not enabled` (and `bind.def:130-131` runs
//!   `initialize_readline()`) before `CASE_HELPOPT` at
//!   `bind.def:191`.
//! * `hash`: `hash.def:86-90` refuses the whole builtin with
//!   `hash: hashing disabled` (status 1, no help) before the
//!   getopt loop, when hashing is disabled (`set +h`).
//!
//! The funnel gate and the `builtin` re-dispatch already share
//! `apply_builtin_help` (rubash#3340), which replays that
//! prologue. The `command` gate (command_no_alias.rs, added by
//! rubash#3339) printed the help inline instead, so on this face
//! the bind warning was swallowed whole and the hash refusal was
//! pre-empted by the help text. The hash shapes are a divergence
//! the #3339 gate itself introduced: on the pre-gate tree the
//! handler was reached directly and its refusal fired first.
//!
//! Fail-first baseline is the master after BOTH #3381 and #3385
//! merged: the nine divergent cells below are all red there (the
//! hash cells are green on any tree without the #3339 gate).
//! The two conservation cells (`command hash --help` with hashing
//! on, `command bind -l` off the help path) are green on both
//! sides and must stay green. `command builtin ...` shapes are
//! NOT this issue's face — they ride the `builtin` gate and are
//! covered by the issue3340 pin.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    #[allow(dead_code)]
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue3414_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

const BIND_SYNOPSIS: &str = "bind: bind [-lpsvPSVX]";
const BIND_WARNING: &str = "bind: warning: line editing not enabled";
const HASH_SYNOPSIS: &str = "hash: hash [-lr] [-p pathname] [-dt] [name ...]";
const HASH_DISABLED: &str = "hash: hashing disabled";

fn assert_bind_help(run: &Run) {
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with(BIND_SYNOPSIS),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.contains(BIND_WARNING), "stderr: {}", run.stderr);
}

fn assert_hash_refusal(run: &Run) {
    assert_eq!(
        run.status, 1,
        "stdout: {}\nstderr: {}",
        run.stdout, run.stderr
    );
    assert!(run.stderr.contains(HASH_DISABLED), "stderr: {}", run.stderr);
    assert!(
        !run.stdout.contains(HASH_SYNOPSIS),
        "no help may print when hashing is disabled: {}",
        run.stdout
    );
}

#[test]
fn command_bind_help_warns_before_help() {
    // GNU oracle (5.3.20, script file): stderr carries the
    // warning, stdout the long help, status 2.
    let run = run_script("command bind --help\n");
    assert_bind_help(&run);
}

#[test]
fn command_bind_help_after_option_warns_before_help() {
    let run = run_script("command bind -p --help\n");
    assert_bind_help(&run);
}

#[test]
fn command_bind_help_merged_warning_precedes_help() {
    // On a merged stream the prologue's stderr precedes the help
    // text (GNU flushes diagnostics before the builtin's buffered
    // output).
    let run = run_script("command bind --help 2>&1\n");
    assert_eq!(run.status, 2, "stdout: {}", run.stdout);
    let warning = run
        .stdout
        .find(BIND_WARNING)
        .expect("warning on merged stream");
    let synopsis = run
        .stdout
        .find(BIND_SYNOPSIS)
        .expect("help synopsis on merged stream");
    assert!(
        warning < synopsis,
        "warning must precede help on merged stream: {}",
        run.stdout
    );
}

#[test]
fn command_p_bind_help_warns_before_help() {
    let run = run_script("command -p bind --help\n");
    assert_bind_help(&run);
}

#[test]
fn command_command_bind_help_warns_before_help() {
    let run = run_script("command command bind --help\n");
    assert_bind_help(&run);
}

#[test]
fn command_bind_help_in_pipeline_warns() {
    // Pipeline element: the pipeline's status is the last
    // element's (0); the warning still lands on the shell's
    // stderr and the help text flows down the pipe.
    let run = run_script("command bind --help | cat\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with(BIND_SYNOPSIS),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.contains(BIND_WARNING), "stderr: {}", run.stderr);
}

#[test]
fn command_hash_help_when_hashing_disabled_refuses_without_help() {
    // GNU oracle (5.3.20): `set +h; command hash --help` prints
    // only `hash: hashing disabled` on stderr, status 1 — the
    // hash.def:86-90 refusal runs before the option scan, so no
    // help is printed at all.
    let run = run_script("set +h; command hash --help\n");
    assert_hash_refusal(&run);
}

#[test]
fn command_p_hash_help_when_hashing_disabled_refuses_without_help() {
    let run = run_script("set +h; command -p hash --help\n");
    assert_hash_refusal(&run);
}

#[test]
fn command_command_hash_help_when_hashing_disabled_refuses_without_help() {
    let run = run_script("set +h; command command hash --help\n");
    assert_hash_refusal(&run);
}

#[test]
fn command_hash_help_when_hashing_enabled() {
    // Conservation: with hashing on, `command hash --help` is a
    // plain help dispatch — status 2, long help on stdout, empty
    // stderr, on both the baseline and the fixed tree.
    let run = run_script("command hash --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with(HASH_SYNOPSIS),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
}

#[test]
fn command_bind_non_help_path_still_warns() {
    // Conservation: off the help path the handler's own prologue
    // already warned; the gate change must not alter it.
    let run = run_script("command bind -l\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.contains(BIND_WARNING), "stderr: {}", run.stderr);
}
