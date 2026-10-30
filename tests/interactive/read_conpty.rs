//! Live-ConPTY execution-level regressions for the `read` builtin on an
//! active console (rubash#806).
//!
//! The ticket's two symptoms are interactive-console faces: `read -n1`
//! hanging on a live ConPTY console, and a plain console read needing an
//! extra Enter for already-queued input. Both are pinned here the only way
//! a console face can be pinned — keystroke-level, through a real ConPTY
//! ([`scripts`-style pywinpty driver]
//! (tests/interactive/support/conpty_read_probe.py)), with marker-payload
//! assertions so the echoed command line can never satisfy a check.
//!
//! Engine faces covered (each leg's GNU anchor lives in the driver):
//! `-n1` with the key queued before the read starts / while it waits /
//! Enter as the key (shtty.c:115 tt_setonechar ICRNL); `read -t` with the
//! key arriving inside the deadline window while the submitting Enter's
//! key-release record sits at the queue head (wait_readable must look at
//! the whole peeked batch, not the head record); deadline line reads stay
//! line-atomic (no cooked-ahead orphan into the next reader); queued input
//! consumed with ONE Enter, from the raw burst and from the cooked window
//! of a prior command; the read.def:553 partial-assign-on-timeout and the
//! niubash#280 deadline-fires contract.
//!
//! The driver skips itself (exit 2) wherever pywinpty is missing — the
//! console face needs a real console host and cannot run on a pipe.

use std::process::{Command, Stdio};

const DRIVER: &str = "tests/interactive/support/conpty_read_probe.py";

fn python_interpreter() -> Option<String> {
    for candidate in ["python", "python3", "py"] {
        let ok = Command::new(candidate)
            .arg("-c")
            .arg("import winpty")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Run the named legs through the ConPTY driver and require every one to
/// report PASS. The driver prints `leg=PASS` / `leg=FAIL detail` lines and
/// exits 2 (with a `SKIP` line) when pywinpty is unavailable.
fn run_conpty_legs(legs: &[&str]) {
    let Some(python) = python_interpreter() else {
        eprintln!("skip: pywinpty not available; the live-console face needs a real console host");
        return;
    };
    let mut cmd = Command::new(&python);
    cmd.arg(env!("CARGO_MANIFEST_DIR").to_string() + "/" + DRIVER)
        .arg(env!("CARGO_BIN_EXE_rubash"))
        .args(legs)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd
        .output()
        .expect("failed to spawn the ConPTY driver (is python on PATH?)");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    eprintln!("driver output:\n{stdout}");
    if !stderr.trim().is_empty() {
        eprintln!("driver stderr:\n{stderr}");
    }
    if stdout.trim_start().starts_with("SKIP") {
        eprintln!("skip: ConPTY driver reports unavailable dependencies");
        return;
    }
    assert!(
        output.status.success(),
        "ConPTY read legs failed (see driver output above)"
    );
    for leg in legs {
        assert!(
            stdout.contains(&format!("{leg}=PASS")),
            "leg {leg} did not report PASS (driver output above)"
        );
    }
}

/// #806 symptom 1: `read -n1` completes on the live console — with the key
/// queued ahead of the read, with the key arriving while the read is
/// blocked, with Enter as the key (ICRNL), and with the key arriving inside
/// a `-t` window even while the submitting Enter's key-release record sits
/// at the queue head (the wait_readable head-of-line starvation this ticket
/// fixed).
#[test]
fn conpty_read_n1_completes_on_live_console() {
    run_conpty_legs(&["n1-typed-ahead", "n1-while-waiting", "n1-enter-icrnl"]);
}

/// #806 symptom 1, deadline face: `read -t 2 -n1` takes the key that
/// arrives inside the window (GNU select parity; used to burn the whole
/// deadline and return 142 with the variable empty).
#[test]
fn conpty_read_t_takes_key_inside_window() {
    run_conpty_legs(&["t-key-in-window"]);
}

/// #806 symptom 2: queued input is consumed with ONE Enter — the raw burst
/// shape and the shape queued while a prior command still runs — and a
/// deadline line read stays line-atomic: no cooked-ahead byte-split and no
/// orphan leaking into the next reader.
#[test]
fn conpty_plain_read_consumes_queued_line_with_one_enter() {
    run_conpty_legs(&["plain-queued", "plain-queued-cooked"]);
}

#[test]
fn conpty_deadline_read_stays_line_atomic() {
    run_conpty_legs(&["t-line-complete", "line-atomic"]);
}

/// read.def:553 (partial record assigned on timeout, rc 128+SIGALRM) and
/// the niubash#280 protection (the deadline still fires with no input).
#[test]
fn conpty_deadline_contract_faces() {
    run_conpty_legs(&["t-partial-assign", "t-no-input"]);
}

/// niubash#684: Ctrl-D at a plain canonical console `read` is the EOF
/// (rc 1, empty assignment, shell survives) and mid-line Ctrl-D is the
/// VEOF flush (record continues, marker is not data). These legs are
/// the ConPTY half of the #684 pair; the POSIX-pty half lives in
/// read_pty.rs.
#[test]
fn conpty_ctrld_read_eof_niubash684() {
    run_conpty_legs(&["ctrld-read-eof", "ctrld-midline-flush"]);
}

/// The whole suite in one pass — same legs, one ConPTY session per leg, the
/// shape the ticket acceptance calls for (`--test-threads=1` comes from the
/// repo's .cargo/config.toml). Each leg bounds its own wall clock with an
/// internal marker timeout, so no outer per-test guard is needed.
#[test]
fn conpty_read_faces_full_pass() {
    run_conpty_legs(&[
        "n1-typed-ahead",
        "n1-while-waiting",
        "n1-enter-icrnl",
        "t-key-in-window",
        "t-line-complete",
        "line-atomic",
        "plain-queued",
        "plain-queued-cooked",
        "t-partial-assign",
        "t-no-input",
        "ctrld-read-eof",
        "ctrld-midline-flush",
    ]);
}
