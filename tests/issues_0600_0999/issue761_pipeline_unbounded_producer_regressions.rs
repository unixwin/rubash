//! Issue rubash#761 — an unbounded external producer feeding a **shell-side**
//! tail never returns: the shell drains the producer into memory instead of
//! closing the read end and letting it die of SIGPIPE.
//!
//! GNU bash: `yes | true` returns in ~0.2 s with status 0 (`true` exits, the
//! pipe closes, `yes` gets SIGPIPE on its next write).  rubash: no output, no
//! diagnostic, no exit — the shell's RSS grows ~250 MB/s, reaching ~1 GB in
//! seconds, because the producer is materialised rather than bounded.
//!
//! The trigger is *not* "how many externals": `yes | head -1` is fine (the tail
//! is the process that exits) and `yes | cat | true` hangs too.  The
//! discriminator is whether some stage bounds the producer.
//!
//! These four reproductions were `#[ignore]`d while the defect was live; the fix
//! has landed (rubash#761, PR #784) and they are now armed as ordinary tests.
//!
//! Every test here stays bounded on purpose.  A hang test that is not bounded turns
//! a red CI into a stuck CI, and a red run that does not kill the tree leaves a
//! `yes` behind to be inherited by the next test.  On expiry the whole process
//! tree is killed with `taskkill /T` and the test fails with the bound it hit.

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Generous relative to GNU's ~0.2 s, small enough that a hang is a hang.
const BOUND: Duration = Duration::from_secs(10);

fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Returns `(terminated, exit_code)`.  `terminated == false` means the bound was
/// hit and the tree has been killed.
fn run_bounded(script: &str) -> (bool, Option<i32>) {
    // stdout/stderr are discarded rather than piped: a child that fills a pipe
    // nobody reads would block for a reason that has nothing to do with this
    // issue, and none of these pipelines is expected to produce output.
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash -c");

    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => return (true, status.code()),
            None if start.elapsed() >= BOUND => {
                kill_tree(child.id());
                let _ = child.wait();
                return (false, None);
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

// ---------------------------------------------------------------------------
// Controls — these pass today: they prove `yes` resolves and that a bounded
// pipeline terminates, so a hanging reproduction cannot be blamed on the fixture
// ---------------------------------------------------------------------------

#[test]
fn control_bounded_pipeline_terminates() {
    let (terminated, code) = run_bounded("yes | head -1");
    assert!(
        terminated,
        "`yes | head -1` did not terminate within {BOUND:?}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn control_bounded_pipeline_with_shell_tail_terminates() {
    // Bounded by `head -1` even though the tail is shell-side.
    let (terminated, code) = run_bounded("yes | head -1 | true");
    assert!(
        terminated,
        "`yes | head -1 | true` did not terminate within {BOUND:?}"
    );
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------------
// Reproductions
// ---------------------------------------------------------------------------

#[test]
fn yes_piped_into_true_terminates() {
    let (terminated, code) = run_bounded("yes | true");
    assert!(
        terminated,
        "`yes | true` did not terminate within {BOUND:?} — GNU returns in ~0.2 s with status 0"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn yes_piped_into_colon_terminates() {
    let (terminated, code) = run_bounded("yes | :");
    assert!(terminated, "`yes | :` did not terminate within {BOUND:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn yes_piped_through_cat_into_true_terminates() {
    let (terminated, code) = run_bounded("yes | cat | true");
    assert!(
        terminated,
        "`yes | cat | true` did not terminate within {BOUND:?} — the trigger is the missing bound, not the external count"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn seq_into_true_terminates_promptly() {
    let (terminated, code) = run_bounded("seq 1 20000000 | true");
    assert!(
        terminated,
        "`seq 1 20000000 | true` did not terminate within {BOUND:?} — GNU exits as soon as `true` does"
    );
    assert_eq!(code, Some(0));
}
