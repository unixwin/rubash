//! Marker leak golden — PTY sibling (rubash#2885 E6).
//!
//! The interactive-only channels of the #2879 sweep's §2.1 matrix —
//! prompt rendering, history storage, `read -p` — cannot be driven
//! through pipes, so they run through a Python PTY driver
//! (`tests/marker_leak_golden_pty.py`) that mirrors the sweep battery's
//! harness: a GNU bash parent as job-control session leader, rubash as
//! its foreground job. The non-interactive channels live in
//! `tests/marker_leak_golden.rs`; the per-marker dimension is the
//! registry-driven test in `src/executor/markers.rs`.
//!
//! Hosts without python3 or a PTY skip cleanly, following the
//! `tests/interactive` precedent.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_driver(legs: &[&str]) -> Option<String> {
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/marker_leak_golden_pty.py");
    let out = Command::new("python3")
        .arg(&driver)
        .arg(env!("CARGO_BIN_EXE_rubash"))
        .args(legs)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // Exit 2 (or a SKIP line) means the host cannot run the harness.
    if out.status.code() == Some(2) || text.contains("SKIP") {
        eprintln!("marker PTY golden skipped: {}", text.trim());
        return None;
    }
    Some(text)
}

fn assert_legs(legs: &[&str]) {
    let Some(text) = run_driver(legs) else {
        return;
    };
    for leg in legs {
        assert!(
            text.contains(&format!("LEG {leg} PASS")),
            "PTY leg {leg} must pass; driver output:\n{text}"
        );
    }
}

#[test]
fn pty_prompt_and_history_channels_carry_no_markers() {
    assert_legs(&[
        "ps1_comsub_esc",
        "ps1_stored_esc",
        "ps2_comsub_esc",
        "ps1_typed_pua",
        "ps1_tab_value",
        "ps4_tab_value",
        "hist_typed_line",
    ]);
}

/// Sweep s4 / F4 (rubash#2883): expansion results are prompt-decoded
/// since #3001 landed the read -p decode, so this leg is live.
#[test]
fn pty_read_p_prompt_decodes_expansion_results() {
    assert_legs(&["read_p_esc"]);
}
