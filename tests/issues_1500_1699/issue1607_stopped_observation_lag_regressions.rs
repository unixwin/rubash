//! Issue rubash#1607 (residual of #1494): a stop the shell's own
//! `kill` sent becomes visible to `jobs` at a NOTIFICATION POINT
//! (GNU learns it asynchronously via jobs.c sigchld_handler ->
//! waitchld, and waitchld also runs when the shell blocks in
//! wait_for) — not on a fixed reader-command count. With only a fast
//! builtin intervening, GNU still shows the pre-signal state and the
//! old current/previous marks; once a real wait point has passed
//! (here: a foreground `sleep`), the stopped state appears and the
//! stopped job is promoted to current. Rubash's deferred marks were
//! applied purely by command count (seq + 1 < now), publishing one
//! command too early in the fast-builtin case.

use std::process::Command;

fn rubash(script: &str) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn job_line(stdout: &str, id: u32) -> String {
    let prefix = format!("[{id}]");
    stdout
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no [{id}] line in jobs output: {stdout:?}"))
        .to_string()
}

/// The issue seed: one fast builtin between kill -STOP and jobs —
/// GNU shows both jobs Running with the ORIGINAL marks ([1]- [2]+).
///
/// De-flake adjudication (tests-only; the product's 5ms term is
/// deliberate semantics and is NOT changed here): a deferred mark
/// publishes once it is one reader command old AND at least 5ms of
/// wall time have passed since the signal was sent (see
/// `JobTable::apply_deferred_state_marks`). A single run of this
/// script therefore cannot tell "published because the mark aged
/// while the shell process was descheduled inside the kill -> jobs
/// window" (legitimate — at load ~11 that window exceeds 5ms in
/// roughly 1/8 of runs) from "published because a count rule fired
/// early" (the #1607 bug). The bug, however, is wall-clock
/// INDEPENDENT: the old `seq + 1 < now` counting rule published at
/// the second reader command after the mark on EVERY run, as would
/// any regression that drops the aging term. So this test samples
/// fresh processes: a run showing the fully published shape
/// ([1]+ Stopped / [2]- Running — exactly what the mirror test
/// below expects after a real wait point) is an "aged" sample and
/// is retried; the first un-published sample passes; if all 25
/// samples age, the test fails (probability <= (1/8)^25 at the
/// worst measured load for a correct product). Any third shape
/// fails immediately — it is neither the pinned behavior nor
/// legitimate aging.
#[test]
fn stopped_state_not_published_after_one_fast_builtin() {
    let script = "\
set -m
sleep 30 &
sleep 30 &
kill -STOP %1
echo intervening-builtin
jobs
kill -9 %1 %2 2>/dev/null; wait 2>/dev/null
";
    const MAX_ATTEMPTS: usize = 25;
    let mut last_aged: Option<(String, String)> = None;
    for _ in 0..MAX_ATTEMPTS {
        let (stdout, stderr) = rubash(script);
        let one = job_line(&stdout, 1);
        let two = job_line(&stdout, 2);
        if one.starts_with("[1]-")
            && one.contains("Running")
            && two.starts_with("[2]+")
            && two.contains("Running")
        {
            // Un-published sample: the pinned #1607 behavior.
            return;
        }
        if one.starts_with("[1]+")
            && one.contains("Stopped")
            && two.starts_with("[2]-")
            && two.contains("Running")
        {
            // Aged sample: the 5ms term legitimately fired under
            // scheduler preemption. Retry with a fresh process.
            last_aged = Some((one, two));
            continue;
        }
        panic!(
            "unexpected jobs shape after one fast builtin: {one:?} / {two:?} (stderr={stderr:?})"
        );
    }
    let (one, two) = last_aged.expect("at least one attempt ran");
    panic!(
        "stop published after one fast builtin in all {MAX_ATTEMPTS} samples \
         (last: {one:?} / {two:?}) — unless this machine is pathologically \
         loaded, this is the #1607 early-publication regression"
    );
}

/// The mirror: after a foreground wait point the stop IS published
/// and the stopped job becomes current (GNU wait_for -> waitchld).
#[test]
fn stopped_state_published_after_foreground_wait() {
    let script = "\
set -m
sleep 30 &
sleep 30 &
kill -STOP %1
sleep 0.3
jobs
kill -9 %1 %2 2>/dev/null; wait 2>/dev/null
";
    let (stdout, stderr) = rubash(script);
    let one = job_line(&stdout, 1);
    let two = job_line(&stdout, 2);
    assert!(
        one.starts_with("[1]+") && one.contains("Stopped"),
        "job 1 stop never published: {one:?} (stderr={stderr:?})"
    );
    assert!(
        two.starts_with("[2]-") && two.contains("Running"),
        "job 2 mark not demoted: {two:?} (stderr={stderr:?})"
    );
}

/// Same-line form: `kill -STOP %1; jobs` never observes the stop.
#[test]
fn stopped_state_not_published_on_same_line() {
    let script = "\
set -m
sleep 30 &
kill -STOP %1; jobs
kill -9 %1 2>/dev/null; wait 2>/dev/null
";
    let (stdout, stderr) = rubash(script);
    let one = job_line(&stdout, 1);
    assert!(
        one.contains("Running") && !one.contains("Stopped"),
        "same-line jobs saw the stop: {one:?} (stderr={stderr:?})"
    );
}
