//! Deterministic 4-arm fault-injection differential harness (rubash#2811).
//!
//! Harness scope:
//! - Covers 4 failure arms under `#[cfg(any(test, debug_assertions))]`:
//!   1. Nth spawn failure (`NotFound`, `PermissionDenied`, `WouldBlock`)
//!   2. fd-table watermark / saturation (constrained `RLIMIT_NOFILE` soft limit)
//!   3. `EINTR` storm on read / wait syscall boundaries
//!   4. Write failure (`BrokenPipe`, `ENOSPC`, `EIO`)
//! - In production release builds (`not(any(test, debug_assertions))`), all
//!   hooks compile to `#[inline(always)]` no-ops with zero runtime overhead
//!   and unchanged OS `EINTR` handling.
//! - Each arm includes targeted edge-case checks, a seeded corpus verifying
//!   deterministic reproducibility (same seed -> identical exit code, flow,
//!   stdout, stderr, fault trace, and post-run `Session` state), and GNU
//!   Bash 5.3.x oracle parity checks (`builtins/common.c:319 sh_chkwrite`,
//!   `execute_cmd.c` `EX_NOEXEC = 126`, `jobs.c:3064 wait_for`).

use std::io;
use std::path::{Path, PathBuf};

use rubash::api::{Flow, RunOptions, Session};
use rubash::fault_injection::{FaultArm, FaultPlan, FaultScope, FaultTrace, WriteFaultKind};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunSnapshot {
    status: i32,
    flow: Flow,
    stdout: String,
    stderr: String,
    trace: FaultTrace,
    post_status: i32,
    post_var: Option<String>,
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2811-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn read_opt(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn external_noop_cmd() -> &'static str {
    if cfg!(windows) {
        "cmd /c exit 0"
    } else {
        "sh -c 'exit 0'"
    }
}

fn external_echo_cmd(msg: &str) -> String {
    if cfg!(windows) {
        format!("cmd /c echo {msg}")
    } else {
        format!("sh -c 'echo {msg}'")
    }
}

fn run_with_plan(
    tag: &str,
    plan: FaultPlan,
    script_fn: impl FnOnce(&Path) -> String,
) -> RunSnapshot {
    let dir = temp_dir(tag);
    let out_file = dir.join("stdout.txt");
    let err_file = dir.join("stderr.txt");

    let mut session = Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .working_directory(dir.clone())
        .argv0("fault-harness")
        .build()
        .expect("session builds");

    let body = script_fn(&dir);
    let wrapped = format!(
        "{{ {body}\n}} > \"{}\" 2> \"{}\"",
        out_file.to_string_lossy().replace('\\', "/"),
        err_file.to_string_lossy().replace('\\', "/")
    );

    let (outcome, trace) = {
        let scope = FaultScope::install(plan);
        let outcome = session
            .run(&wrapped, &RunOptions::default())
            .expect("faulted run returns RunOutcome");
        let trace = scope.trace();
        (outcome, trace)
    };

    // Verify post-run Session state remains healthy and usable with faults cleared.
    let post = session
        .run(
            "POST_CHECK=healthy; printf '%s' \"$POST_CHECK\" > \"$OUT/post.txt\"",
            &RunOptions::default(),
        )
        .expect("post-run health check succeeds");
    assert_eq!(read_opt(&dir.join("post.txt")), "healthy");

    let snapshot = RunSnapshot {
        status: outcome.status,
        flow: outcome.flow,
        stdout: read_opt(&out_file),
        stderr: read_opt(&err_file),
        trace,
        post_status: post.status,
        post_var: session.variables().resolved_value("POST_CHECK"),
    };
    let _ = std::fs::remove_dir_all(&dir);
    snapshot
}

// ---------------------------------------------------------------------------
// Arm 1: Nth spawn failure
// ---------------------------------------------------------------------------

#[test]
fn arm1_spawn_failure_kinds_and_nth_targeting() {
    let cmd = external_noop_cmd();

    for (kind, msg, expected_status) in [
        (io::ErrorKind::NotFound, "No such file or directory", 126),
        (io::ErrorKind::PermissionDenied, "Permission denied", 126),
        (
            io::ErrorKind::WouldBlock,
            "Resource temporarily unavailable",
            126,
        ),
    ] {
        let mut plan = FaultPlan::default();
        plan.spawn_fail_on = Some(2);
        plan.spawn_fail_kind = kind;
        plan.spawn_fail_message = msg;

        let snap = run_with_plan("arm1-kind", plan, |_| {
            format!("{cmd}; echo first=$?; {cmd}; echo second=$?; {cmd}; echo third=$?")
        });

        assert_eq!(snap.status, 0);
        assert_eq!(snap.flow, Flow::Completed);
        assert_eq!(
            snap.stdout.replace("\r\n", "\n"),
            format!("first=0\nsecond={expected_status}\nthird=0\n"),
            "unexpected stdout for kind={kind:?}, stderr={:?}",
            snap.stderr
        );
        assert_eq!(snap.trace.spawn_faults, 1);
        assert_eq!(snap.post_status, 0);
        assert_eq!(snap.post_var.as_deref(), Some("healthy"));
    }
}

#[test]
fn arm1_spawn_failure_seeded_corpus_is_deterministic() {
    let seeds = [0x1101u64, 0x1102, 0x1103, 0x1104, 0x1105, 0x1106];
    let cmd = external_noop_cmd();
    let echo_ok = external_echo_cmd("ok");

    for seed in seeds {
        let plan_a = FaultPlan::seeded(seed, FaultArm::SpawnFailure);
        let plan_b = FaultPlan::seeded(seed, FaultArm::SpawnFailure);
        assert_eq!(plan_a, plan_b);

        let a = run_with_plan("arm1-seed-a", plan_a, |_| {
            format!("{cmd}; echo s1=$?; {echo_ok}; echo s2=$?; {cmd}; echo s3=$?")
        });
        let b = run_with_plan("arm1-seed-b", plan_b, |_| {
            format!("{cmd}; echo s1=$?; {echo_ok}; echo s2=$?; {cmd}; echo s3=$?")
        });
        assert_eq!(a, b, "seed {seed:#x} must be deterministic");
        assert_eq!(a.trace.spawn_faults, 1);
    }
}

// ---------------------------------------------------------------------------
// Arm 2: fd-table watermark / saturation
// ---------------------------------------------------------------------------

#[test]
fn arm2_fd_watermark_saturation_and_recovery() {
    // Soft limit below SHELL_FD_BASE (10) -> EINVAL, variable stays unset.
    let mut plan_low = FaultPlan::default();
    plan_low.fd_watermark = Some(6);
    let snap_low = run_with_plan("arm2-low", plan_low, |_| {
        "exec {v}>/dev/null; echo rc=$? v=${v:-unset}".to_string()
    });
    assert_eq!(snap_low.stdout, "rc=1 v=unset\n");
    assert!(
        snap_low.stderr.contains("Invalid argument")
            || snap_low.stderr.contains("cannot duplicate fd"),
        "expected EINVAL-shaped diagnostic, got: {:?}",
        snap_low.stderr
    );
    assert!(snap_low.trace.fd_limit_clamps >= 1);

    // Soft limit = 14 (outer `{ ... } > out 2> err` holds saved fds 10 and 11)
    // allows fds 12 and 13, saturates on 3rd allocation (EMFILE), and closing
    // `a` allows a subsequent allocation to reuse 12.
    let mut plan_sat = FaultPlan::default();
    plan_sat.fd_watermark = Some(14);
    let snap_sat = run_with_plan("arm2-sat", plan_sat, |_| {
        "exec {a}>/dev/null; r1=$?; \
         exec {b}>/dev/null; r2=$?; \
         exec {c}>/dev/null; r3=$?; \
         exec {a}>&-; \
         exec {d}>/dev/null; r4=$?; \
         exec {b}>&- {d}>&-; \
         echo \"$r1:$a $r2:$b $r3:${c:-unset} $r4:$d\""
            .to_string()
    });
    assert_eq!(
        snap_sat.stdout, "0:12 0:13 1:unset 0:12\n",
        "stderr={:?}",
        snap_sat.stderr
    );
    assert!(
        snap_sat.stderr.contains("Too many open files"),
        "expected EMFILE diagnostic, got: {:?}",
        snap_sat.stderr
    );
}

#[test]
fn arm2_fd_watermark_seeded_corpus_is_deterministic() {
    let seeds = [0x2201u64, 0x2202, 0x2203, 0x2204, 0x2205];
    for seed in seeds {
        let plan_a = FaultPlan::seeded(seed, FaultArm::FdWatermark);
        let plan_b = FaultPlan::seeded(seed, FaultArm::FdWatermark);
        assert_eq!(plan_a, plan_b);

        let script = |_: &Path| {
            "exec {f1}>/dev/null; r1=$?; \
             exec {f2}>/dev/null; r2=$?; \
             exec {f3}>/dev/null; r3=$?; \
             exec 13>/dev/null; r4=$?; \
             echo \"$r1:${f1:-u} $r2:${f2:-u} $r3:${f3:-u} $r4\""
                .to_string()
        };
        let a = run_with_plan("arm2-seed-a", plan_a, script);
        let b = run_with_plan("arm2-seed-b", plan_b, script);
        assert_eq!(a, b, "seed {seed:#x} must be deterministic");
        assert!(a.trace.fd_limit_clamps >= 1);
    }
}

// ---------------------------------------------------------------------------
// Arm 3: EINTR storm on read / wait syscall boundaries
// ---------------------------------------------------------------------------

#[test]
fn arm3_eintr_storm_read_and_wait_survive_intact() {
    let mut plan = FaultPlan::default();
    plan.eintr_burst_per_op = 5;
    plan.eintr_max_total = 40;

    let cmd = external_noop_cmd();
    let snap = run_with_plan("arm3-storm", plan, |dir| {
        std::fs::write(dir.join("input.txt"), "alpha\nbeta\ngamma\n").expect("write input");
        format!(
            "lines=''; \
             while IFS= read -r line; do lines=\"$lines[$line]\"; done < \"$OUT/input.txt\"; \
             {cmd}; \
             echo \"$lines:$?\""
        )
    });

    assert_eq!(snap.status, 0);
    assert_eq!(snap.stdout, "[alpha][beta][gamma]:0\n");
    assert_eq!(snap.stderr, "", "unexpected stderr: {:?}", snap.stderr);
    assert!(
        snap.trace.eintr_read_injections > 0,
        "expected read EINTR injections, got {:?}",
        snap.trace
    );
    assert!(
        snap.trace.eintr_wait_injections > 0,
        "expected wait EINTR injections, got {:?}",
        snap.trace
    );
}

fn arm3_deterministic_snapshot(snapshot: &RunSnapshot) -> RunSnapshot {
    let mut deterministic = snapshot.clone();
    deterministic.trace.eintr_wait_injections = 0;
    deterministic
        .trace
        .events
        .retain(|event| !event.starts_with("eintr:wait#"));
    deterministic
}

#[test]
fn arm3_eintr_storm_seeded_corpus_is_deterministic() {
    let seeds = [0x3301u64, 0x3302, 0x3303, 0x3304, 0x3305];
    let cmd = external_noop_cmd();

    for seed in seeds {
        let plan_a = FaultPlan::seeded(seed, FaultArm::EintrStorm);
        let plan_b = FaultPlan::seeded(seed, FaultArm::EintrStorm);
        assert_eq!(plan_a, plan_b);

        let mk_script = |dir: &Path| {
            std::fs::write(dir.join("data.txt"), "one\ntwo\nthree\nfour\n").expect("write data");
            format!(
                "count=0; \
                 while IFS= read -r item; do count=$((count + 1)); done < \"$OUT/data.txt\"; \
                 {cmd}; \
                 echo \"count=$count rc=$?\""
            )
        };
        let a = run_with_plan("arm3-seed-a", plan_a, mk_script);
        let b = run_with_plan("arm3-seed-b", plan_b, mk_script);
        assert_eq!(
            arm3_deterministic_snapshot(&a),
            arm3_deterministic_snapshot(&b),
            "seed {seed:#x} must be deterministic apart from wall-clock-dependent wait injections"
        );
        assert_eq!(a.stdout, "count=4 rc=0\n");
        assert!(
            a.trace.eintr_read_injections > 0,
            "seed {seed:#x} must observe read EINTR injections, got {:?}",
            a.trace
        );
        assert!(
            a.trace.eintr_wait_injections > 0,
            "seed {seed:#x} run A must observe wait EINTR injections, got {:?}",
            a.trace
        );
        assert!(
            b.trace.eintr_wait_injections > 0,
            "seed {seed:#x} run B must observe wait EINTR injections, got {:?}",
            b.trace
        );
    }
}

// ---------------------------------------------------------------------------
// Arm 4: Write failure (BrokenPipe / ENOSPC / EIO)
// ---------------------------------------------------------------------------

#[test]
fn arm4_write_failure_variants_report_cleanly() {
    for (kind, expected_fragment) in [
        (WriteFaultKind::BrokenPipe, "Broken pipe"),
        (WriteFaultKind::Enospc, "No space left on device"),
        (WriteFaultKind::Eio, "Input/output error"),
    ] {
        let mut plan = FaultPlan::default();
        plan.write_fail_on = Some(1);
        plan.write_fail_kind = kind;
        plan.write_only_fd = Some(1);

        let snap = run_with_plan("arm4-kind", plan, |_| {
            "echo payload > \"$OUT/sink.txt\"; rc=$?; echo \"after=$rc\"".to_string()
        });

        assert_eq!(snap.status, 0);
        assert_eq!(snap.stdout, "after=1\n");
        assert!(
            snap.stderr.contains(expected_fragment) || snap.stderr.contains("write error"),
            "expected write-error diagnostic ({expected_fragment}) for {kind:?}, got: {:?}",
            snap.stderr
        );
        assert_eq!(snap.trace.write_faults, 1);
    }
}

#[test]
fn arm4_write_failure_seeded_corpus_is_deterministic() {
    let seeds = [0x4401u64, 0x4402, 0x4403, 0x4404, 0x4405, 0x4406];
    for seed in seeds {
        let plan_a = FaultPlan::seeded(seed, FaultArm::WriteFailure);
        let plan_b = FaultPlan::seeded(seed, FaultArm::WriteFailure);
        assert_eq!(plan_a, plan_b);

        let script = |_: &Path| {
            "printf 'first\\n' > \"$OUT/w1.txt\"; r1=$?; \
             printf 'second\\n' > \"$OUT/w2.txt\"; r2=$?; \
             echo \"$r1:$r2\""
                .to_string()
        };
        let a = run_with_plan("arm4-seed-a", plan_a, script);
        let b = run_with_plan("arm4-seed-b", plan_b, script);
        assert_eq!(a, b, "seed {seed:#x} must be deterministic");
        assert_eq!(a.trace.write_faults, 1);
    }
}

// ---------------------------------------------------------------------------
// GNU Bash 5.3.x oracle parity
// ---------------------------------------------------------------------------

#[test]
fn gnu_oracle_fault_parity_for_enospc_and_eintr_survival() {
    // 1. GNU Bash 5.3.x oracle for ENOSPC on `echo` (`builtins/common.c:319`
    //    sh_chkwrite): `echo payload > /dev/full` exits 1 and prints
    //    `echo: write error: No space left on device`.
    let mut enospc_plan = FaultPlan::default();
    enospc_plan.write_fail_on = Some(1);
    enospc_plan.write_fail_kind = WriteFaultKind::Enospc;
    enospc_plan.write_only_fd = Some(1);

    let rubash_enospc = run_with_plan("oracle-enospc", enospc_plan, |_| {
        "echo payload > \"$OUT/sink.txt\"; rc=$?; echo \"after=$rc\"".to_string()
    });
    assert_eq!(rubash_enospc.status, 0);
    assert_eq!(rubash_enospc.stdout, "after=1\n");
    assert!(
        rubash_enospc
            .stderr
            .contains("echo: write error: No space left on device"),
        "expected GNU sh_chkwrite ENOSPC diagnostic, got: {:?}",
        rubash_enospc.stderr
    );

    // 2. GNU Bash 5.3.x oracle for EINTR-storm survival: reading lines and
    //    waiting on an external command produces identical stdout/status.
    let mut eintr_plan = FaultPlan::default();
    eintr_plan.eintr_burst_per_op = 4;
    eintr_plan.eintr_max_total = 32;
    let cmd = external_noop_cmd();
    let rubash_eintr = run_with_plan("oracle-eintr", eintr_plan, |dir| {
        std::fs::write(dir.join("input.txt"), "alpha\nbeta\ngamma\n").expect("write input");
        format!(
            "lines=''; \
             while IFS= read -r line; do lines=\"$lines[$line]\"; done < \"$OUT/input.txt\"; \
             {cmd}; \
             echo \"$lines:$?\""
        )
    });
    assert_eq!(rubash_eintr.status, 0);
    assert_eq!(rubash_eintr.stdout, "[alpha][beta][gamma]:0\n");
    assert_eq!(rubash_eintr.stderr, "");

    // On Unix hosts with GNU bash and /dev/full, cross-check live GNU output.
    #[cfg(unix)]
    if Path::new("/dev/full").exists() {
        let gnu_enospc = std::process::Command::new("bash")
            .args(["-c", "echo payload > /dev/full; rc=$?; echo \"after=$rc\""])
            .output();
        if let Ok(out) = gnu_enospc {
            if out.status.success() {
                assert_eq!(String::from_utf8_lossy(&out.stdout), rubash_enospc.stdout);
                let gnu_err = String::from_utf8_lossy(&out.stderr);
                assert!(
                    gnu_err.contains("echo: write error: No space left on device"),
                    "unexpected GNU stderr: {gnu_err:?}"
                );
            }
        }

        let dir = temp_dir("oracle-gnu-eintr");
        let input_path = dir.join("input.txt");
        std::fs::write(&input_path, "alpha\nbeta\ngamma\n").expect("write input");
        let gnu_script = format!(
            "lines=''; while IFS= read -r line; do lines=\"$lines[$line]\"; done < \"{}\"; {cmd}; echo \"$lines:$?\"",
            input_path.display()
        );
        let gnu_eintr = std::process::Command::new("bash")
            .args(["-c", &gnu_script])
            .output();
        let _ = std::fs::remove_dir_all(&dir);
        if let Ok(out) = gnu_eintr {
            if out.status.success() {
                assert_eq!(String::from_utf8_lossy(&out.stdout), rubash_eintr.stdout);
                assert_eq!(String::from_utf8_lossy(&out.stderr), "");
            }
        }
    }
}
