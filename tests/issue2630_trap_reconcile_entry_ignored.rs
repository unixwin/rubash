//! Trap disposition reconcile fast path under entry-ignored signals
//! (rubash#2630; mechanism established by the issue's investigation,
//! GNU semantics from trap.c `initialize_traps`).
//!
//! GNU snapshots the entry dispositions once at startup
//! (trap.c:154 `initialize_traps`, `original_signals[]`, SIG_HARD_IGNORE
//! at trap.c:63): an entry-ignored signal is a recorded BASELINE, and
//! no per-command work scales with it. Rubash's
//! `seed_startup_traps` (src/builtins/trap.rs:874) materializes each
//! entry-ignored signal as an empty trap-table entry in every
//! Session, and the reconcile fast-path gate
//! `at_default_state` (src/builtins/kill.rs:2650) demands the packed
//! disposition words be ALL Default — with any entry ignore present
//! they never are, so every command boundary pays the full slow-path
//! reconcile (process lock + managed-key table scan). Measured on the
//! base tree (release, issue comment): 31.8µs vs 16.1µs per iteration
//! (≈2×); on this lane's debug calibration build the taxed/control
//! ratio was 1.9–4.1× (see lane REPORT).
//!
//! The defect is a pure constant-factor tax — outputs are identical —
//! so, following the perf-pin conventions of
//! issue2062_perf_scaling_regressions.rs (function + shape + a
//! magnitude bound, never absolute milliseconds), the timing cell
//! compares the SAME workload spawned with entry-ignored signals
//! against the same workload spawned with default dispositions on
//! the same machine, taking the best of two runs per mode. The
//! functional cells anchor that the ignore is genuinely in force
//! (hard-ignore listing, no-op set, signal does not kill) — they are
//! regression locks and pass on the base tree.

#![cfg(unix)]

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

fn write_script(tag: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("issue2630-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    path
}

/// Spawn rubash on `path` with SIGINT/SIGQUIT/SIGXFSZ/SIGPIPE forced
/// to SIG_IGN (the supervisor/nohup/background launch shape) or to
/// SIG_DFL (control). The pre-exec hook runs after fork, before
/// exec, so the dispositions are the child's ENTRY dispositions —
/// exactly the input `seed_startup_traps` consumes.
fn run_with_entry_dispositions(
    path: &PathBuf,
    ignored: bool,
) -> (String, Option<i32>, Option<i32>, Duration) {
    let mut cmd = Command::new(rubash_bin());
    cmd.arg(path).env("LC_ALL", "C.UTF-8");
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            let disposition = if ignored {
                libc::SIG_IGN
            } else {
                libc::SIG_DFL
            };
            for sig in [libc::SIGINT, libc::SIGQUIT, libc::SIGXFSZ, libc::SIGPIPE] {
                libc::signal(sig, disposition);
            }
            Ok(())
        });
    }
    let t0 = Instant::now();
    let output = cmd.output().expect("run rubash");
    let elapsed = t0.elapsed();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
        output.status.signal(),
        elapsed,
    )
}

#[test]
fn issue2630_entry_ignored_signals_list_as_hard_ignores() {
    // Regression lock: the seeded state the timing cell depends on —
    // entry ignores list as `trap -- '' SIG…` and survive a set
    // attempt as a silent no-op.
    let path = write_script(
        "listing",
        "trap -p XFSZ INT QUIT\necho \"list_rc=$?\"\ntrap 'echo nope' XFSZ\necho \"set_rc=$?\"\ntrap -p XFSZ\n",
    );
    let (stdout, code, _sig, _t) = run_with_entry_dispositions(&path, true);
    assert_eq!(
        stdout,
        "trap -- '' SIGXFSZ\ntrap -- '' SIGINT\ntrap -- '' SIGQUIT\nlist_rc=0\nset_rc=0\ntrap -- '' SIGXFSZ\n"
    );
    assert_eq!(code, Some(0));

    // Control: with default entry dispositions nothing is listed,
    // and the set attempt lands as a real trap.
    let (stdout, code, _sig, _t) = run_with_entry_dispositions(&path, false);
    assert_eq!(stdout, "list_rc=0\nset_rc=0\ntrap -- 'echo nope' SIGXFSZ\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2630_entry_ignored_signal_does_not_kill() {
    // Regression lock: an entry-ignored SIGXFSZ is genuinely ignored;
    // the default-disposition control dies by the signal.
    let path = write_script("xfsz", "kill -XFSZ $$\necho survived\n");
    let (stdout, code, signal, _t) = run_with_entry_dispositions(&path, true);
    assert_eq!(stdout, "survived\n");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);

    let (stdout, code, signal, _t) = run_with_entry_dispositions(&path, false);
    assert_eq!(stdout, "");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGXFSZ));
}

#[test]
fn issue2630_entry_ignored_loop_pays_no_per_command_tax() {
    // Defect cell: a fixed arithmetic workload must cost (nearly)
    // the same whether or not the shell was entered with ignored
    // signals. Base tree: the taxed run is ~2× the control (issue
    // measurement) because the reconcile fast path is unreachable.
    const N: usize = 60_000;
    let expected: i64 = (0..N as i64).map(|k| k % 7).sum();
    let body = format!("s=0\nfor ((k=0;k<{N};k++)); do s=$((s + k % 7)); done\necho \"s=$s\"\n");
    let path = write_script("loop", &body);

    let mut best_taxed = Duration::MAX;
    let mut best_control = Duration::MAX;
    for _ in 0..2 {
        let (stdout, code, _sig, t) = run_with_entry_dispositions(&path, true);
        assert_eq!(stdout, format!("s={expected}\n"));
        assert_eq!(code, Some(0));
        best_taxed = best_taxed.min(t);
        let (stdout, code, _sig, t) = run_with_entry_dispositions(&path, false);
        assert_eq!(stdout, format!("s={expected}\n"));
        assert_eq!(code, Some(0));
        best_control = best_control.min(t);
    }
    assert!(
        best_taxed < best_control * 3 / 2 + Duration::from_secs(2),
        "entry-ignored tax: taxed {best_taxed:?} vs control {best_control:?} — \
         the reconcile fast path must be reachable at the entry-ignored baseline \
         (base tree measures ~2x)"
    );
    assert!(
        best_control < Duration::from_secs(600),
        "magnitude sentinel: control run took {best_control:?}"
    );
}
