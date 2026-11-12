//! rubash#3340 — the shared `--help` gate (issue #2753) must not
//! swallow a builtin's leading diagnostics.
//!
//! GNU runs each builtin's prologue BEFORE its option scan, so a
//! `--help` dispatch still emits the prologue first:
//!
//! * `bind`: `bind.def:122-131` prints `bind: warning: line
//!   editing not enabled` and runs `initialize_readline()` before
//!   `CASE_HELPOPT` at `bind.def:191`.
//! * `hash`: `hash.def:86-90` refuses the whole builtin with
//!   `hash: hashing disabled` (status 1, no help) before the
//!   getopt loop, when hashing is disabled (`set +h`).
//!
//! The gate (command_dispatch.rs `builtin_help_gate`) used to
//! return before the handler was ever entered, so both prologues
//! were skipped on the `--help` path: `bind --help` lost its
//! warning, and `set +h; hash --help` wrongly printed the long
//! help with status 2.
//!
//! Survey (this issue): every other builtin's `.def` prologue was
//! read — `cd`, `kill`, `trap`, `read`, `fc`, `history`,
//! `mapfile`, `pwd`, `complete`/`compgen`, `declare`,
//! `shopt`, `set`, `jobs`, `hash`, `alias`, `command`, `exec`,
//! `source`, `suspend`, `type`, `umask`, `ulimit`, `wait`,
//! `printf`, `echo`, `test`, `times`, `eval`, `let`, `shift`,
//! `return`, `break`, `exit`, `caller`, `fg_bg`, `pushd`, `help`,
//! `getopts` — and only `bind` and `hash` have observable code
//! before the help check. (`getopts`/`times`/`eval`/`caller`/
//! `fg_bg`/`builtin` reach help through `no_options`, which IS
//! the help check; `type`'s pre-scan only rewrites `-type`/
//! `-path`/`-all` words and never touches `--help`.)
//!
//! The gate has three sites, all three were checked: the funnel
//! gate (command_dispatch.rs `builtin_help_gate`), the `builtin`
//! re-dispatch (builtin_direct_command.rs, which never re-enters
//! the funnel — both share `apply_builtin_help` now), and the
//! trap-only pipeline-stage site (pipeline_exec.rs), which is
//! trap-only and trap has no prologue, so it keeps its own inline
//! decision.

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
    let dir = std::env::temp_dir().join(format!("issue3340_{}_{}", std::process::id(), seq));
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

#[test]
fn bind_help_warns_before_help() {
    // GNU oracle (5.3.20, script file): stderr carries the
    // warning, stdout the long help, status 2.
    let run = run_script("bind --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with(BIND_SYNOPSIS),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.contains(BIND_WARNING), "stderr: {}", run.stderr);
}

#[test]
fn bind_help_after_option_warns_before_help() {
    // The warning precedes the getopt scan in GNU, so it also
    // precedes a `--help` that arrives after a no-arg option.
    let run = run_script("bind -p --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with(BIND_SYNOPSIS),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.contains(BIND_WARNING), "stderr: {}", run.stderr);
}

#[test]
fn bind_help_merged_warning_precedes_help() {
    // GNU emits the warning (stderr, unbuffered) before the help
    // text (stdout, flushed at builtin exit): on a merged stream
    // the warning line comes first.
    let run = run_script("bind --help 2>&1\n");
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
fn bind_non_help_path_still_warns() {
    // Control: the warning belongs to the handler prologue, not
    // to the gate — the plain path must keep warning exactly as
    // before.
    let run = run_script("bind -l\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.contains(BIND_WARNING), "stderr: {}", run.stderr);
}

#[test]
fn hash_help_when_hashing_enabled() {
    // Control: with hashing on, `hash --help` is a plain help
    // dispatch — status 2, long help on stdout, empty stderr.
    let run = run_script("hash --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with(HASH_SYNOPSIS),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
}

#[test]
fn hash_help_when_hashing_disabled_refuses_without_help() {
    // GNU oracle (5.3.20): `set +h; hash --help` prints only
    // `hash: hashing disabled` on stderr, status 1 — the
    // hash.def:86-90 refusal runs before the option scan, so no
    // help is printed at all.
    let run = run_script("set +h; hash --help\n");
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
fn hash_non_help_when_hashing_disabled_refuses() {
    // Control: the refusal on the non-help path already worked;
    // the gate must not change it.
    let run = run_script("set +h; hash -l\n");
    assert_eq!(
        run.status, 1,
        "stdout: {}\nstderr: {}",
        run.stdout, run.stderr
    );
    assert!(run.stderr.contains(HASH_DISABLED), "stderr: {}", run.stderr);
}

#[test]
fn builtin_bind_help_warns_before_help() {
    // The `builtin` re-dispatch never re-enters the funnel, so it
    // has its own gate copy — it must replay the prologue too
    // (shared `apply_builtin_help`): GNU `builtin bind --help`
    // warns on stderr, prints help on stdout, status 2.
    let run = run_script("builtin bind --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with(BIND_SYNOPSIS),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.contains(BIND_WARNING), "stderr: {}", run.stderr);
}

#[test]
fn builtin_bind_help_merged_warning_precedes_help() {
    let run = run_script("builtin bind --help 2>&1\n");
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
fn builtin_hash_help_when_hashing_disabled_refuses_without_help() {
    // GNU oracle (5.3.20): `set +h; builtin hash --help` refuses
    // with `hash: hashing disabled`, status 1, no help.
    let run = run_script("set +h; builtin hash --help\n");
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
