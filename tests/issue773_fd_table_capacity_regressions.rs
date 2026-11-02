//! rubash#773: the {var} fd table capped at 1014 allocations with a
//! misleading errno.
//!
//! GNU anchors (WSL GNU bash 5.3.0, probed 2026-10-19, script files):
//! - redir.c:940-946 (do_redirection_internal REDIR_VARASSIGN): a {var}
//!   redirect allocates with `fcntl(fd, F_DUPFD, SHELL_FD_BASE)` — the
//!   lowest free fd >= 10 below the RLIMIT_NOFILE *soft* limit. There is no
//!   table cap of GNU's own: under the oracle's default soft 10240 the
//!   issue repro `i=0; while [ $i -lt 1200 ]; do exec {fd}>>f || break;
//!   i=$((i+1)); done` prints `allocated=1200`.
//! - Exhaustion below the soft limit fails with EMFILE:
//!   `redirection error: cannot duplicate fd: Too many open files`
//!   (probe: ulimit -Sn 12 → two allocations, then the third fails with
//!   "Too many open files"); the base-beyond-limit case fails with EINVAL:
//!   "Invalid argument" (probe: ulimit -Sn 6).
//! - A raised soft limit lifts the ceiling (probe: ulimit -Sn 5000 →
//!   thousands of allocations succeed).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str], cwd: &Path) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue773-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The exact issue repro: 1200 concurrent `exec {fd}>>f` allocations must
/// succeed (the old table died at exactly 1014 with "Invalid argument").
#[test]
fn issue_repro_allocates_1200_dynamic_fds() {
    let dir = scratch_dir("repro");
    let (stdout, stderr, rc) = run(
        &["-c", "i=0; while [ $i -lt 1200 ]; do exec {fd}>>f || break; i=$((i+1)); done; echo \"allocated=$i\""],
        &dir,
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "allocated=1200\n", "stdout: {stdout:?}");
    assert!(
        !stderr.contains("cannot duplicate fd"),
        "unexpected stderr: {stderr:?}"
    );
}

/// The default ceiling follows the emulated soft limit, which mirrors the
/// WSL oracle (10240): the allocator must serve well past the old 1014 cap
/// without any ulimit call.
#[test]
fn default_capacity_exceeds_old_1014_cap() {
    let dir = scratch_dir("default");
    let (stdout, _, rc) = run(
        &["-c", "i=0; while [ $i -lt 2000 ]; do exec {fd}>>f || break; i=$((i+1)); done; echo \"allocated=$i\""],
        &dir,
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "allocated=2000\n", "stdout: {stdout:?}");
}

/// Raising the soft limit lifts the ceiling (GNU: F_DUPFD is bounded by the
/// soft limit — probe ulimit -Sn 5000 → thousands of allocations).
#[test]
fn raised_ulimit_n_lifts_the_ceiling() {
    let dir = scratch_dir("raised");
    let (stdout, stderr, rc) = run(
        &["-c", "ulimit -n 4096; i=0; while [ $i -lt 3000 ]; do exec {fd}>>f || break; i=$((i+1)); done; echo \"allocated=$i\""],
        &dir,
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "allocated=3000\n", "stdout: {stdout:?}");
    assert!(
        !stderr.contains("cannot duplicate fd"),
        "unexpected stderr: {stderr:?}"
    );
}

/// Exhaustion below a soft limit > SHELL_FD_BASE reports GNU's EMFILE text,
/// not the old hardcoded "Invalid argument" (probe: ulimit -Sn 12 → fds 10
/// and 11 allocate, the third fails with "Too many open files").
#[test]
fn exhaustion_below_limit_reports_too_many_open_files() {
    let dir = scratch_dir("emfile");
    let (stdout, stderr, _) = run(
        &[
            "-c",
            "ulimit -n 12; exec {a}>>f; exec {b}>>f; exec {c}>>f 2>/dev/null; \
             echo \"a=${a:+set} b=${b:+set} c=${c:+set}\"",
        ],
        &dir,
    );
    assert_eq!(stdout, "a=set b=set c=\n", "stdout: {stdout:?}");
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Too many open files"),
        "stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("Invalid argument"),
        "stderr still carries EINVAL: {stderr:?}"
    );
}

/// Base-beyond-limit stays EINVAL (probe: ulimit -Sn 6 then
/// `exec {v}</dev/null` → "redirection error: cannot duplicate fd:
/// Invalid argument", v unset — the vredir6.sub anchor).
#[test]
fn base_beyond_limit_reports_invalid_argument() {
    let dir = scratch_dir("einval");
    let (stdout, stderr, _) = run(
        &[
            "-c",
            "ulimit -n 6; exec {v}</dev/null; echo \"v=${v:+set}\"",
        ],
        &dir,
    );
    assert_eq!(stdout, "v=\n", "stdout: {stdout:?}");
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Invalid argument"),
        "stderr: {stderr:?}"
    );
}

/// A dup (N>&M) onto the raised table works — the #482 shared-offset dup
/// surface must not regress through the new allocator (GNU dup path shares
/// the open file description).
#[test]
fn dup_semantics_survive_the_raised_table() {
    let dir = scratch_dir("dup");
    std::fs::write(dir.join("payload"), b"abcdefghij").expect("write payload");
    let (stdout, _, rc) = run(
        &[
            "-c",
            "ulimit -n 4096; i=0; while [ $i -lt 1500 ]; do exec {fd}>>f || break; i=$((i+1)); done; \
             exec 9<payload; exec 1500<&9; \
             read -r -N 4 head <&1500; exec 1500<&- 9<&-; echo \"$head\"",
        ],
        &dir,
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "abcd\n", "stdout: {stdout:?}");
}

/// The emulated `ulimit -n` defaults mirror the process rlimit,
/// exactly as GNU's do: GNU `ulimit` reports RLIMIT_NOFILE,
/// whatever the ambient values are, so the expectation is read
/// from this test process's own limits at test start.
/// READJUDICATION (adj-fd §issue773): the original pin hardcoded
/// 10240/1048576 — the ambient limits of the WSL machine the
/// probe was recorded on, not GNU constants. In this battery's
/// VM (ambient 4096/4096) oracle and rubash both print
/// `4096\n4096\nunl_rc=1`; under a lowered ambient (soft 2048)
/// both print 2048/4096. `ulimit -Sn unlimited` over a finite
/// hard limit fails EINVAL (unl_rc=1); over an unlimited hard
/// limit it succeeds (unl_rc=0).
#[test]
fn emulated_ulimit_defaults_match_oracle() {
    let (soft, hard, unl_rc) = ambient_nofile_expectation();
    let dir = scratch_dir("ulimit");
    let (stdout, _, rc) = run(
        &[
            "-c",
            "ulimit -Sn; ulimit -Hn; ulimit -Sn unlimited 2>/dev/null; echo \"unl_rc=$?\"",
        ],
        &dir,
    );
    assert_eq!(rc, Some(0));
    assert_eq!(
        stdout,
        format!("{soft}\n{hard}\nunl_rc={unl_rc}\n"),
        "stdout: {stdout:?}"
    );
}

/// The ambient RLIMIT_NOFILE of this test process (inherited by
/// the spawned shell), formatted the way GNU `ulimit` prints it,
/// plus the expected status of `ulimit -Sn unlimited`.
#[cfg(unix)]
fn ambient_nofile_expectation() -> (String, String, i32) {
    let mut rl: libc::rlimit = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) };
    assert_eq!(rc, 0, "getrlimit(RLIMIT_NOFILE) failed");
    fn fmt(v: libc::rlim_t) -> String {
        if v == libc::RLIM_INFINITY {
            "unlimited".to_string()
        } else {
            v.to_string()
        }
    }
    let unl_rc = if rl.rlim_max == libc::RLIM_INFINITY {
        0
    } else {
        1
    };
    (fmt(rl.rlim_cur), fmt(rl.rlim_max), unl_rc)
}

/// The non-unix persona has no OS rlimit to mirror; the shell
/// emulates the defaults recorded from the WSL oracle probe.
#[cfg(not(unix))]
fn ambient_nofile_expectation() -> (String, String, i32) {
    ("10240".to_string(), "1048576".to_string(), 1)
}
