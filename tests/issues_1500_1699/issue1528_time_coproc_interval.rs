//! Issue rubash#1528: `time coproc` measured the SPAWN, not the job.
//!
//! GNU model (measured on the 5.3.20 oracle, script files): `time` on
//! a coproc times the job's LIFETIME — spawn to completion — and the
//! report is emitted when the completion is noticed (at `wait`, while
//! a later foreground command waits, or at shell-exit disposal):
//! - wait form, `sleep 2` body: real 0m2.017s, printed at the wait.
//! - wait form, instant body: real 0m0.000s, exactly one report.
//! - plain form, `sleep 2` body, immediate marker: no report (the job
//!   is still running at exit; presence in the plain form is racy in
//!   GNU and is NOT asserted here).
//! - plain form, instant body: one report by exit at the latest.
//! Pre-fix rubash printed real 0m0.003s immediately after the spawn
//! for the sleep-2 wait form. The fix defers the report of a timed
//! coproc that is still running into the job-completion funnel
//! (flush_timed_coproc_report), so the metrics span the lifetime.
//!
//! The assertions below pin the MODEL (lifetime-scale real values,
//! exactly one report), never an exact millisecond figure: the spawn
//! cost of a debug binary legitimately exceeds GNU's fork cost.
#![cfg(unix)]

use std::process::Command;

/// Run a script file under the rubash binary; return (stdout, stderr).
fn run(script: &str) -> (String, String) {
    let script_path = std::env::temp_dir().join(format!(
        "rubash-issue1528-probe-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&script_path);
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Parse every `real` value (seconds) out of a default-format time
/// report stream ("real\t0m1.003s" / "real 1.003s" shapes).
fn real_values(stderr: &str) -> Vec<f64> {
    stderr
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("real")?.trim_start();
            let (mins, secs) = rest.split_once('m')?;
            let secs: f64 = secs.trim_end_matches('s').parse().ok()?;
            let mins: f64 = mins.parse().ok()?;
            Some(mins * 60.0 + secs)
        })
        .collect()
}

#[test]
fn issue1528_time_coproc_wait_form_measures_job_lifetime() {
    let (stdout, stderr) = run("time coproc C { sleep 1; echo hi; }\nwait\necho DONE\n");
    assert!(stdout.contains("DONE"));
    let reals = real_values(&stderr);
    assert_eq!(reals.len(), 1, "exactly one report: {stderr}");
    assert!(
        reals[0] >= 0.9,
        "real must span the sleep-1 lifetime, got {}",
        reals[0]
    );
}

#[test]
fn issue1528_time_coproc_wait_form_instant_body_one_small_report() {
    let (stdout, stderr) = run("time coproc C { echo hi; }\nwait\necho DONE\n");
    assert!(stdout.contains("DONE"));
    let reals = real_values(&stderr);
    assert_eq!(reals.len(), 1, "exactly one report: {stderr}");
    assert!(
        reals[0] < 0.5,
        "instant body must stay sub-second, got {}",
        reals[0]
    );
}

#[test]
fn issue1528_time_coproc_plain_form_reports_lifetime_at_notice() {
    // The completion is noticed while the shell waits on the later
    // foreground sleep (a GNU observation point): one report,
    // lifetime-scale. The body (0.5s) is much shorter than the
    // foreground sleep (2s) so the completion is comfortably visible
    // at the next command boundary — equal-length sleeps would race
    // the boundary poll.
    let (stdout, stderr) = run("time coproc C { sleep 0.5; echo hi; }\nsleep 2\necho MARKER\n");
    assert!(stdout.contains("MARKER"));
    let reals = real_values(&stderr);
    assert_eq!(reals.len(), 1, "exactly one report: {stderr}");
    // The interval must span the body (>= 0.4s, not the ~0.03s spawn
    // scale of the bug). Its upper end is the notice point: rubash
    // polls completions at command boundaries (the waitchld-lag
    // model), so with a 2s foreground sleep the reap is observed at
    // that sleep's end and real can reach ~2s; GNU's SIGCHLD-driven
    // waitchld stamps the death itself (~0.5s here). The wait form
    // (the issue's deterministic criterion) is exact — see the test
    // above.
    assert!(
        (0.4..2.5).contains(&reals[0]),
        "real must span the sleep-0.5 lifetime, got {}",
        reals[0]
    );
}

#[test]
fn issue1528_time_sync_and_background_forms_unchanged() {
    let (_, stderr) = run("time sleep 0.3\n");
    let reals = real_values(&stderr);
    assert_eq!(reals.len(), 1);
    assert!((0.25..1.5).contains(&reals[0]), "sync real {}", reals[0]);

    let (stdout, stderr) = run("time sleep 0.3 &\nwait\necho DONE\n");
    assert!(stdout.contains("DONE"));
    let reals = real_values(&stderr);
    assert_eq!(reals.len(), 1, "exactly one report: {stderr}");
    assert!(
        reals[0] >= 0.25,
        "background form must span its lifetime, got {}",
        reals[0]
    );
}
