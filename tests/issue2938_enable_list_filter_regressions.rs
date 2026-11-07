//! rubash#2938 — `enable` list-filter precedence + `--help` (Stage-2 ④
//! Phase 2 E1).
//!
//! GNU anchor: `builtins/enable.def:172-180` — the list branch fires when
//! `list == 0 || (flags & PFLAG)`, and the filter is
//! `AFLAG ? ENABLED|DISABLED : NFLAG ? DISABLED : ENABLED` (with `SFLAG`
//! adding `SPECIAL`). `-a` takes precedence over `-n`: `enable -an`
//! lists every builtin.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (E1 differential,
//! 532 cells, master `854eed80`). The listing also differs from GNU by
//! the single deliberate `enable env` row (rubash has an `env` builtin;
//! user-ruled deliberate deviation) — these pins assert the listing is
//! present and complete modulo that one row, never byte-equality.

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
    let dir = std::env::temp_dir().join(format!("issue2938_{}_{}", std::process::id(), seq));
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

/// The listing, minus the one deliberate `enable env` row: every other
/// line must be `enable <name>` for an enabled builtin or
/// `enable -n <name>` for a disabled one.
fn listing_lines(run: &Run) -> Vec<&str> {
    run.stdout
        .lines()
        .filter(|line| *line != "enable env")
        .collect()
}

#[test]
fn an_lists_every_builtin() {
    // GNU: `-a` wins over `-n` — the full listing, not an empty one.
    let run = run_script("enable -an\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let lines = listing_lines(&run);
    assert_eq!(lines.len(), 61, "stdout: {}", run.stdout);
    assert!(lines.contains(&"enable echo"), "stdout: {}", run.stdout);
    assert!(lines.contains(&"enable enable"), "stdout: {}", run.stdout);
    assert!(lines.contains(&"enable dirs"), "stdout: {}", run.stdout);
    assert!(
        lines.iter().all(|l| l.starts_with("enable ")),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn na_lists_every_builtin() {
    // Same rule, options in the other order.
    let run = run_script("enable -na\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let lines = listing_lines(&run);
    assert_eq!(lines.len(), 61, "stdout: {}", run.stdout);
    assert!(lines.contains(&"enable echo"), "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn anp_lists_every_builtin() {
    let run = run_script("enable -anp\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let lines = listing_lines(&run);
    assert_eq!(lines.len(), 61, "stdout: {}", run.stdout);
    assert!(lines.contains(&"enable echo"), "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn adn_lists_every_builtin() {
    let run = run_script("enable -adn\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let lines = listing_lines(&run);
    assert_eq!(lines.len(), 61, "stdout: {}", run.stdout);
    assert!(lines.contains(&"enable echo"), "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn an_shows_disabled_builtins_with_n() {
    // GNU: with `-a` in the filter, a disabled builtin prints as
    // `enable -n <name>` while enabled ones print plain.
    let run = run_script("enable -n echo\nenable -an\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let lines = listing_lines(&run);
    assert!(lines.contains(&"enable -n echo"), "stdout: {}", run.stdout);
    assert!(lines.contains(&"enable cd"), "stdout: {}", run.stdout);
    assert!(!lines.contains(&"enable echo"), "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn n_alone_lists_only_disabled_builtins() {
    // GNU: `-n` with no names lists the DISABLED filter only.
    let run = run_script("enable -n echo\nenable -n\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "enable -n echo\n", "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn help_first_word_prints_help() {
    let run = run_script("enable --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .starts_with("enable: enable [-a] [-dnps] [-f filename] [name ...]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn help_after_option_prints_help() {
    let run = run_script("enable -n --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .starts_with("enable: enable [-a] [-dnps] [-f filename] [name ...]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn help_first_word_ignores_trailing_invalid_option() {
    let run = run_script("enable --help -z\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .starts_with("enable: enable [-a] [-dnps] [-f filename] [name ...]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn special_list_filter_uses_the_same_a_before_n_rule() {
    // GNU enable.def:172-180: the A-before-N filter applies to the `-s`
    // listing too (SFLAG is OR'd into the filter). The old special
    // branch ignored both the filter and the disabled state.
    // The disabled special builtin here is `.`, not `:` — `:` cannot be
    // the subject: the disabled-state carrier is colon-separated, so
    // `enable -n :` does not persist at all (rubash#2942, filed
    // separately; the carrier encoding is not this filter's mechanism).
    let run = run_script("enable -n .\nenable -as\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("enable -n .\n"),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stdout.contains("enable :\n"), "stdout: {}", run.stdout);
    assert!(!run.stdout.contains("enable .\n"), "stdout: {}", run.stdout);
    let _ = run.dir;

    let run = run_script("enable -n .\nenable -ns\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "enable -n .\n", "stdout: {}", run.stdout);
    let _ = run.dir;

    let run = run_script("enable -n .\nenable -s\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("enable :\n"), "stdout: {}", run.stdout);
    assert!(!run.stdout.contains("enable .\n"), "stdout: {}", run.stdout);
    let _ = run.dir;
}
