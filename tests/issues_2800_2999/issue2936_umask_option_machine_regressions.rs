//! rubash#2936 — `umask` option-machine divergences (Stage-2 ④ Phase 2 E1).
//!
//! GNU anchors: `builtins/umask.def:82-83` (`internal_getopt (list, "Sp")`)
//! + `builtins/bashgetopt.c:61-192` — a bare `-` is an operand (a mode);
//! `--` ends the scan and the following words are operands (modes);
//! `--help` in option position prints help, rc 2.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (E1 differential,
//! 532 cells, master `854eed80`). Each probe runs in its own rubash
//! child process, so the `umask <mode>` sets below never leak into
//! sibling tests.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2936_{}_{}", std::process::id(), seq));
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

#[test]
fn bare_dash_is_a_mode_operand() {
    // GNU: `umask -` parses `-` as a symbolic mode (operator `-` with
    // an empty action list) — rc 0, mask unchanged.
    let run = run_script("umask 027\numask -\numask\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "0027\n", "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn bare_dash_after_s_is_a_mode_operand() {
    // GNU: `umask -S -` prints the symbolic form of the current mask.
    // Mask 027 allows u=rwx, g=rx (w denied), o= (all denied).
    let run = run_script("umask 027\numask -S -\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "u=rwx,g=rx,o=\n", "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn dashdash_help_is_a_mode() {
    // GNU: the word after `--` is the mode, so `--help` reaches the
    // symbolic parser: `h' is an invalid symbolic mode character, rc 1,
    // and the mask is unchanged.
    let run = run_script("umask 027\numask -- --help\necho \"rc=$?\"\numask\n");
    assert!(
        run.stderr.contains("`h': invalid symbolic mode character"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.stdout, "rc=1\n0027\n", "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn dashdash_capital_s_is_a_mode() {
    // GNU: `umask -- -S` → `` `S': invalid symbolic mode character ``,
    // rc 1, mask unchanged.
    let run = run_script("umask 027\numask -- -S\necho \"rc=$?\"\numask\n");
    assert!(
        run.stderr.contains("`S': invalid symbolic mode character"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.stdout, "rc=1\n0027\n", "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn help_first_word_prints_help() {
    // GNU: `umask --help` prints the long help to stdout, rc 2.
    let run = run_script("umask --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("umask: umask [-p] [-S] [mode]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn help_after_option_prints_help() {
    // GNU: `--help` in option position (after `-S`) is help, rc 2.
    let run = run_script("umask -S --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("umask: umask [-p] [-S] [mode]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn help_first_word_ignores_trailing_invalid_option() {
    // GNU: `umask --help -z` prints help, rc 2 — the `-z` after
    // `--help` is never scanned.
    let run = run_script("umask --help -z\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("umask: umask [-p] [-S] [mode]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}
