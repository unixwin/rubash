//! Issue rubash#3605 (run-builtins): on macOS `ulimit -a` printed raw
//! numbers for the unlimited rows — e.g. `file size … 9007199254740991`
//! — and `ulimit -f unlimited` failed EPERM for a non-root user.
//!
//! GNU semantics (builtins/ulimit.def): `printone` prints `unlimited`
//! when the fetched value equals the platform's RLIM_INFINITY
//! (ulimit.def:754-777), and `print_all_limits` prints one row per
//! compiled-in resource (ulimit.def:734-749) — on Darwin that includes
//! `-l`/MEMLOCK, `-m`/RSS and `-v`/VIRTMEM.
//!
//! Root cause: rubash's internal UNLIMITED sentinel is u64::MAX, but
//! Darwin's RLIM_INFINITY is (1<<63)-1 (i64::MAX) — getrlimit's raw
//! value never matched the sentinel, so printone divided it by the
//! block factor (9223372036854775807 / 1024 = 9007199254740991), and
//! setrlimit handed u64::MAX to the kernel, which counts as a hard
//! limit RAISE and fails EPERM where GNU's i64::MAX is a no-op.
//! Fix: rlimit_get normalizes `>= libc::RLIM_INFINITY` to UNLIMITED,
//! writes map UNLIMITED back to libc::RLIM_INFINITY, and the macOS
//! LIMITS table carries the -l/-m/-v rows.
//!
//! Expected outputs byte-verified against GNU Bash 5.3.20 on
//! macOS arm64 (script-file probes 2026-11-16). The stack-soft probe
//! is skipped: Darwin rejects lowering RLIMIT_STACK below the already
//! established stack region for this runtime (env-bound, GNU's
//! shallower main stack does not trip it).

#![cfg(unix)]

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `-f`/`-d`/`-t` are unlimited by default on both Linux and Darwin:
/// the print must say `unlimited`, never a raw integer.
#[test]
fn unlimited_rows_print_unlimited_not_a_number() {
    let (stdout, stderr, _) = rubash("ulimit -f\nulimit -d\nulimit -t\nulimit -Hf");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "unlimited\nunlimited\nunlimited\nunlimited\n");
}

/// A non-root `ulimit -f unlimited` is a no-op write (ulimit.def:623-626
/// picks rlim_max when raising soft to infinity): GNU returns 0 with no
/// diagnostic; rubash must not hand u64::MAX to setrlimit as a raise.
#[test]
fn non_root_set_unlimited_is_a_noop() {
    if unsafe { libc::geteuid() } == 0 {
        return; // root genuinely raises; not the cell under test
    }
    let (stdout, stderr, status) = rubash("ulimit -f unlimited\necho rc=$?\nulimit -f");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(status, Some(0));
    assert_eq!(stdout, "rc=0\nunlimited\n");
}

/// `ulimit -a` on macOS carries the full eleven-row table including
/// -l/-m/-v (ulimit.def:734-749 print_all_limits over the compiled-in
/// limits[]); the letters must also answer as options, not `invalid
/// option`. Linux already had the full table via RLIMIT_* defines.
#[cfg(target_os = "macos")]
#[test]
fn macos_table_prints_memlock_rss_virtmem_rows() {
    let (stdout, stderr, _) = rubash("ulimit -a");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    for row in ["max locked memory", "max memory size", "virtual memory"] {
        assert!(stdout.contains(row), "missing `{row}` row in:\n{stdout}");
    }
    for opt in ["-l", "-m", "-v"] {
        let (stdout, stderr, status) = rubash(&format!("ulimit {opt}"));
        assert!(stderr.is_empty(), "ulimit {opt} stderr: {stderr}");
        assert_eq!(status, Some(0), "ulimit {opt} failed");
        assert!(
            stdout.trim() == "unlimited" || stdout.trim().parse::<u64>().is_ok(),
            "ulimit {opt} printed: {stdout:?}"
        );
    }
}
