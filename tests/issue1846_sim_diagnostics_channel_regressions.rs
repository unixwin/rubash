//! Regression pins for rubash#1846: the rm/touch emulations in
//! `src/executor/external_file_builtins.rs` printed part of their
//! diagnostics with a bare `eprintln!`, so the text reached the
//! terminal even when the command itself carried a `2>` redirect —
//! the opposite failure of rubash#1835 (there a stage diagnostic was
//! swallowed; here it bypassed the channel). Every coreutils
//! diagnostic of the retained surface now goes through the command's
//! own redirections via `write_buffered_builtin_output`, with GNU's
//! emission shape: a getopt failure is terminal (the FIRST bad
//! option's line plus the `Try ... --help` line, nothing touched),
//! an unrecognized long option is reported as the whole word
//! (`--bogus=3`, not `--bogus`), and a failed `-r` reference is the
//! only diagnostic when the date is also bad (coreutils evaluates
//! the reference before the date grammar).
//!
//! Expected texts are the local oracle's (GNU coreutils 9.4, C
//! locale), probed directly. The pins are unix-only: they drive the
//! built shell binary in a scratch directory like the #1636 pins.

#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(unix)]
fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

#[cfg(unix)]
struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

#[cfg(unix)]
fn run_in_dir(dir: &Path, script: &str, path_env: &str) -> Output {
    let mut child = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let _ = child.stdin.take();
    let output = child.wait_with_output().expect("wait for rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

#[cfg(unix)]
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-sim-diag-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[cfg(unix)]
const SYSTEM_PATH: &str = "/usr/bin:/bin";

// --------------------------------------------------------------------
// rm: every diagnostic rides the command's own `2>`.
// --------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn issue1846_rm_unrecognized_option_rides_the_commands_stderr() {
    let dir = scratch("rm-unrecognized");
    std::fs::write(dir.join("f"), b"x").expect("fixture f");
    let out = run_in_dir(
        &dir,
        "rm --bogus f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let kept = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: unrecognized option '--bogus'\nTry 'rm --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "", "nothing may bypass the 2> redirect");
    assert!(kept, "a usage error must not remove the operand");
}

#[cfg(unix)]
#[test]
fn issue1846_rm_invalid_short_option_rides_the_commands_stderr() {
    let dir = scratch("rm-invalid-short");
    std::fs::write(dir.join("f"), b"x").expect("fixture f");
    let out = run_in_dir(
        &dir,
        "rm -Z f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let kept = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout, "rc=1\nrm: invalid option -- 'Z'\nTry 'rm --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert!(kept);
}

#[cfg(unix)]
#[test]
fn issue1846_rm_unrecognized_option_prints_the_whole_word() {
    let dir = scratch("rm-unrecognized-eq");
    std::fs::write(dir.join("f"), b"x").expect("fixture f");
    // glibc getopt_long reports the word as given, `=value` included;
    // the emulation used to print only the part before the `=`.
    let out = run_in_dir(
        &dir,
        "rm --bogus=3 f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: unrecognized option '--bogus=3'\nTry 'rm --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1846_rm_reports_only_the_first_bad_option() {
    let dir = scratch("rm-first-error");
    std::fs::write(dir.join("f"), b"x").expect("fixture f");
    // GNU's getopt failure is terminal: the first bad option ends the
    // invocation, so `--alsono` and `-Y` are never reported.
    let out = run_in_dir(
        &dir,
        "rm --bogus --alsono f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    assert_eq!(
        out.stdout,
        "rc=1\nrm: unrecognized option '--bogus'\nTry 'rm --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    let out = run_in_dir(&dir, "rm -Z -Y f 2>err2.txt; cat err2.txt", SYSTEM_PATH);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout, "rm: invalid option -- 'Z'\nTry 'rm --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1846_rm_missing_operand_rides_the_commands_stderr() {
    let dir = scratch("rm-missing-operand");
    let out = run_in_dir(&dir, "rm 2>err.txt; echo rc=$?; cat err.txt", SYSTEM_PATH);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout, "rc=1\nrm: missing operand\nTry 'rm --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1846_rm_diagnostic_is_swallowed_by_a_dev_null_redirect() {
    let dir = scratch("rm-dev-null");
    std::fs::write(dir.join("f"), b"x").expect("fixture f");
    // The sim-decline report's seed shape: `rm --bogus f 2>/dev/null`
    // printed to the terminal anyway before the fix.
    let out = run_in_dir(&dir, "rm --bogus f 2>/dev/null; echo rc=$?", SYSTEM_PATH);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.stdout, "rc=1\n", "stderr={:?}", out.stderr);
    assert_eq!(out.stderr, "");
}

// --------------------------------------------------------------------
// touch: every diagnostic rides the command's own `2>`.
// --------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn issue1846_touch_unrecognized_option_rides_the_commands_stderr() {
    let dir = scratch("touch-unrecognized");
    let out = run_in_dir(
        &dir,
        "touch --bogus f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let made = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout,
        "rc=1\ntouch: unrecognized option '--bogus'\nTry 'touch --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert!(!made, "a usage error must not touch the operand");
}

#[cfg(unix)]
#[test]
fn issue1846_touch_invalid_option_rides_the_commands_stderr() {
    let dir = scratch("touch-invalid-short");
    let out = run_in_dir(
        &dir,
        "touch -x f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let made = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout,
        "rc=1\ntouch: invalid option -- 'x'\nTry 'touch --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert!(!made);
}

#[cfg(unix)]
#[test]
fn issue1846_touch_reports_only_the_first_bad_option() {
    let dir = scratch("touch-first-error");
    let out = run_in_dir(
        &dir,
        "touch -x -y f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let made = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout,
        "rc=1\ntouch: invalid option -- 'x'\nTry 'touch --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert!(!made);
}

#[cfg(unix)]
#[test]
fn issue1846_touch_reference_failure_rides_the_commands_stderr() {
    let dir = scratch("touch-ref-failure");
    let out = run_in_dir(
        &dir,
        "touch -r /nonexistent-ref-1846 f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let made = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout,
        "rc=1\ntouch: failed to get attributes of '/nonexistent-ref-1846': No such file or directory\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert!(!made, "a failed -r must leave the operands untouched");
}

#[cfg(unix)]
#[test]
fn issue1846_touch_cannot_touch_rides_the_commands_stderr() {
    let dir = scratch("touch-cannot-touch");
    let out = run_in_dir(
        &dir,
        "touch nodir1846/f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout, "rc=1\ntouch: cannot touch 'nodir1846/f': No such file or directory\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1846_touch_invalid_date_reports_through_the_redirect() {
    let dir = scratch("touch-invalid-date");
    // `-d` text outside the evaluated grammar delegates to the real
    // binary when one resolves; either way the diagnostic belongs to
    // the command's own stderr and nowhere else.
    let out = run_in_dir(
        &dir,
        "touch -d '###' f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let made = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.stdout.contains("rc=1"), "stdout={:?}", out.stdout);
    assert!(
        out.stdout.contains("invalid date format") && out.stdout.contains("###"),
        "the date diagnostic must land in err.txt, stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        !out.stderr.contains("invalid date format"),
        "nothing may bypass the 2> redirect, stderr={:?}",
        out.stderr
    );
    assert!(!made);
}

#[cfg(unix)]
#[test]
fn issue1846_touch_reference_failure_precedes_a_bad_date() {
    let dir = scratch("touch-ref-before-date");
    // GNU evaluates the reference file before the date grammar, so
    // the reference failure is the ONLY diagnostic even though the
    // -d text is also bad. (With a real touch on PATH the invocation
    // delegates and the real binary gives the same single line.)
    let out = run_in_dir(
        &dir,
        "touch -r /nonexistent-ref-1846 -d '###' f 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let made = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.stdout.contains("rc=1"), "stdout={:?}", out.stdout);
    assert!(
        out.stdout
            .contains("failed to get attributes of '/nonexistent-ref-1846'"),
        "stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        !out.stdout.contains("invalid date format") && !out.stderr.contains("invalid date"),
        "the date diagnostic must not appear, stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(!made);
}

// --------------------------------------------------------------------
// Control: the kept forms still answer in-process and stay quiet.
// --------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn issue1846_rm_and_touch_kept_forms_are_unchanged() {
    let dir = scratch("kept-forms");
    std::fs::write(dir.join("victim"), b"x").expect("fixture victim");
    let out = run_in_dir(&dir, "rm -v victim; touch made; echo done", SYSTEM_PATH);
    let gone = !dir.join("victim").exists();
    let made = dir.join("made").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status, 0, "stderr={:?}", out.stderr);
    assert_eq!(
        out.stdout, "removed 'victim'\ndone\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert!(gone && made);
}
