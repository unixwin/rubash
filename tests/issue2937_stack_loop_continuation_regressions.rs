//! rubash#2937 — `dirs`/`pushd`/`popd` option-loop divergences (Stage-2 ④
//! Phase 2 E1).
//!
//! GNU anchor: `builtins/pushd.def` — the three builtins run a faithful
//! `ISOPTION` loop that CONTINUES after a `+N`/`-N` word (last `+N` wins),
//! `-n` accumulates, a post-`+N` invalid word errors before any action,
//! and `CHECK_HELPOPT` answers a first-word `--help`. The pushd rotate
//! block (pushd.def:262-286) runs `num` times and, with `-n`, the
//! rotated-to entry is DISCARDED. The popd loop's `else break` arm for
//! an empty word also clobbers `direction` to `'\0'`, so `popd ""`
//! removes the BOTTOM entry without chdir.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (E1 differential,
//! 532 cells, master `854eed80`). Every probe builds its own stack
//! first (display `[d2, d1, d0, <cell>]`, cwd `d2`).

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
    let dir = std::env::temp_dir().join(format!("issue2937_{}_{}", std::process::id(), seq));
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

/// The stack setup: display `[d2, d1, d0, <cell>]`, cwd `d2`.
/// `$PWD` is captured BEFORE any pushd (`B=$PWD`).
const SETUP: &str = "mkdir -p d0 d1 d2\nB=$PWD\npushd \"$B/d0\" >/dev/null\npushd \"$B/d1\" >/dev/null\npushd \"$B/d2\" >/dev/null\n";

fn stack_run(probe: &str) -> Run {
    run_script(&format!("{SETUP}{probe}"))
}

fn p(dir: &PathBuf, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

// ---------------------------------------------------------------- dirs ---

#[test]
fn dirs_help_first_word_prints_help() {
    let run = stack_run("dirs --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("dirs: dirs [-clpv] [+N] [-N]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn dirs_help_first_word_ignores_trailing_invalid_option() {
    let run = stack_run("dirs --help -z\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("dirs: dirs [-clpv] [+N] [-N]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn dirs_index_then_verbose_formats_the_entry() {
    // GNU: the loop continues after `+1`, so `-v` formats the indexed
    // entry as `%2d  %s`.
    let run = stack_run("dirs +1 -v\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(" 1  {}\n", p(&run.dir, "d1")),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn dirs_index_then_clear_clears() {
    // GNU: CLEARSTAK is checked before the index print, so `dirs +1 -c`
    // clears the stack and prints nothing; the following `dirs` shows
    // only the cwd.
    let run = stack_run("dirs +1 -c\ndirs\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{}\n", p(&run.dir, "d2")),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn dirs_last_index_word_wins() {
    let run = stack_run("dirs +1 +2\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{}\n", p(&run.dir, "d0")),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;

    let run = stack_run("dirs +2 +1\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{}\n", p(&run.dir, "d1")),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn dirs_invalid_word_after_index_is_an_error() {
    let run = stack_run("dirs +1 foo\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("dirs: foo: invalid option"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("dirs: usage:"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn dirs_help_after_index_is_an_invalid_number() {
    // GNU: `--help` after `+N` is a number word, not help.
    let run = stack_run("dirs +1 --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("dirs: --help: invalid number"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn dirs_clear_after_long_form_clears() {
    let run = stack_run("dirs -l -c\ndirs\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{}\n", p(&run.dir, "d2")),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

// --------------------------------------------------------------- pushd ---

#[test]
fn pushd_help_first_word_prints_help() {
    let run = stack_run("pushd --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .starts_with("pushd: pushd [-n] [+N | -N | dir]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn pushd_help_first_word_ignores_trailing_invalid_option() {
    let run = stack_run("pushd --help -z\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .starts_with("pushd: pushd [-n] [+N | -N | dir]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn pushd_help_after_index_is_an_invalid_number() {
    let run = stack_run("pushd +1 --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("pushd: --help: invalid number"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn pushd_index_then_n_rotates_without_chdir_and_discards() {
    // GNU: one rotation with `-n` — no chdir, no print from pushd
    // itself; the rotated-to entry (d1) is DISCARDED, so the display
    // is [d2, d0, <cell>, d2] and pwd stays d2.
    let d = run_script(&format!("{SETUP}pushd +1 -n\npwd\ndirs\n"));
    let cell = d.dir.to_string_lossy().into_owned();
    let d2 = p(&d.dir, "d2");
    let d0 = p(&d.dir, "d0");
    assert_eq!(d.status, 0, "stderr: {}", d.stderr);
    assert_eq!(
        d.stdout,
        format!("{d2}\n{d2} {d0} {cell} {d2}\n"),
        "stdout: {}",
        d.stdout
    );
    let _ = d.dir;
}

#[test]
fn pushd_invalid_word_after_index_aborts_before_rotation() {
    let run = stack_run("pushd +1 -z\npwd\ndirs\n");
    assert!(
        run.stderr.contains("pushd: -z: invalid number"),
        "stderr: {}",
        run.stderr
    );
    // No rotation happened: pwd stays d2 and the stack is unchanged.
    let cell = run.dir.to_string_lossy().into_owned();
    let d2 = p(&run.dir, "d2");
    let d1 = p(&run.dir, "d1");
    let d0 = p(&run.dir, "d0");
    assert_eq!(
        run.stdout,
        format!("{d2}\n{d2} {d1} {d0} {cell}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn pushd_last_index_word_wins() {
    // `pushd +1 +2` rotates twice and chdirs to d0; pushd prints the
    // stack itself, then pwd and dirs follow.
    let run = stack_run("pushd +1 +2\npwd\ndirs\n");
    let cell = run.dir.to_string_lossy().into_owned();
    let d0 = p(&run.dir, "d0");
    let d1 = p(&run.dir, "d1");
    let d2 = p(&run.dir, "d2");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{d0} {cell} {d2} {d1}\n{d0}\n{d0} {cell} {d2} {d1}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;

    // `pushd +2 +1` rotates once and chdirs to d1.
    let run = stack_run("pushd +2 +1\npwd\ndirs\n");
    let cell = run.dir.to_string_lossy().into_owned();
    let d0 = p(&run.dir, "d0");
    let d1 = p(&run.dir, "d1");
    let d2 = p(&run.dir, "d2");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{d1} {d0} {cell} {d2}\n{d1}\n{d1} {d0} {cell} {d2}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn pushd_n_before_and_after_index_rotates_without_chdir() {
    // `-n` accumulates on both sides of `+1`; the rotation applies
    // with no chdir and no print.
    let run = stack_run("pushd -n +1 -n\npwd\ndirs\n");
    let cell = run.dir.to_string_lossy().into_owned();
    let d2 = p(&run.dir, "d2");
    let d0 = p(&run.dir, "d0");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{d2}\n{d2} {d0} {cell} {d2}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn pushd_zero_index_with_n_is_a_silent_success() {
    // GNU: num==0 with `-n` returns success silently — no dirs print.
    let run = stack_run("pushd +0 -n\npwd\ndirs\n");
    let cell = run.dir.to_string_lossy().into_owned();
    let d2 = p(&run.dir, "d2");
    let d1 = p(&run.dir, "d1");
    let d0 = p(&run.dir, "d0");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{d2}\n{d2} {d1} {d0} {cell}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

// ---------------------------------------------------------------- popd ---

#[test]
fn popd_help_first_word_prints_help() {
    let run = stack_run("popd --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("popd: popd [-n] [+N | -N]\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn popd_help_after_index_is_an_invalid_number() {
    let run = stack_run("popd +1 --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("popd: --help: invalid number"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn popd_last_index_word_wins() {
    // `popd +1 +2` removes the entry at offset−2 (d0); popd prints
    // the stack itself, then pwd and dirs follow.
    let run = stack_run("popd +1 +2\npwd\ndirs\n");
    let cell = run.dir.to_string_lossy().into_owned();
    let d2 = p(&run.dir, "d2");
    let d1 = p(&run.dir, "d1");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{d2} {d1} {cell}\n{d2}\n{d2} {d1} {cell}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn popd_invalid_word_after_index_aborts_before_pop() {
    let run = stack_run("popd +1 foo\npwd\ndirs\n");
    assert!(
        run.stderr.contains("popd: foo: invalid argument"),
        "stderr: {}",
        run.stderr
    );
    // No pop happened: the stack is unchanged.
    let cell = run.dir.to_string_lossy().into_owned();
    let d2 = p(&run.dir, "d2");
    let d1 = p(&run.dir, "d1");
    let d0 = p(&run.dir, "d0");
    assert_eq!(
        run.stdout,
        format!("{d2}\n{d2} {d1} {d0} {cell}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn popd_invalid_number_after_index_aborts_before_pop() {
    let run = stack_run("popd +1 -z\npwd\ndirs\n");
    assert!(
        run.stderr.contains("popd: -z: invalid number"),
        "stderr: {}",
        run.stderr
    );
    let cell = run.dir.to_string_lossy().into_owned();
    let d2 = p(&run.dir, "d2");
    let d1 = p(&run.dir, "d1");
    let d0 = p(&run.dir, "d0");
    assert_eq!(
        run.stdout,
        format!("{d2}\n{d2} {d1} {d0} {cell}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn popd_empty_word_removes_the_bottom_entry_without_chdir() {
    // GNU quirk (pushd.def popd loop): the empty word breaks the loop
    // AND clobbers `direction` to `'\0'`, so the post-loop else-branch
    // removes `list[0]` — the BOTTOM entry — with no chdir. popd prints
    // the stack itself, then pwd and dirs follow.
    let run = stack_run("popd \"\"\npwd\ndirs\n");
    let d2 = p(&run.dir, "d2");
    let d1 = p(&run.dir, "d1");
    let d0 = p(&run.dir, "d0");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{d2} {d1} {d0}\n{d2}\n{d2} {d1} {d0}\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}
