//! Live-ConPTY execution-level regressions for the `stty` builtin's dump
//! and window-size faces (niubash#282 family).
//!
//! The non-terminal pins in `issue282_stty_console_regressions.rs` prove
//! the diagnostics; these legs prove the console faces the only way a
//! console face can be pinned — through a real ConPTY
//! ([`tests/interactive/support/conpty_stty_probe.py`]): `stty -a`
//! printing the owner-approved WinuxCmd dump shape with real CONOUT$
//! geometry, geometry agreement between `-a` and `size`, the GNU
//! `rows N cols M` window-size setting (stty.c:1216 set_window_size)
//! reflected by later queries, mixed-order settings, the apply-pass
//! `invalid integer argument` wording (stty.c:1220), and the honest
//! console-mode `-g` hex field.
//!
//! The driver skips itself (exit 2) wherever pywinpty is missing — the
//! console face needs a real console host and cannot run on a pipe.

use std::process::{Command, Stdio};

const DRIVER: &str = "tests/interactive/support/conpty_stty_probe.py";

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
        "ConPTY stty legs failed (see driver output above)"
    );
    for leg in legs {
        assert!(
            stdout.contains(&format!("{leg}=PASS")),
            "leg {leg} did not report PASS (driver output above)"
        );
    }
}

/// `stty -a` prints the owner-approved dump shape and its geometry agrees
/// with `stty size` (one CONOUT$ source).
#[test]
fn conpty_stty_all_dump_and_geometry() {
    run_conpty_legs(&["stty-a-dump", "geometry-matches"]);
}

/// The GNU window-size setting: `stty rows R cols M` resizes the console
/// so later `size` and the bare `rows`/`cols` queries reflect it, and
/// settings mix with `sane` in any order while an output style still
/// refuses setting operands on a live console.
#[test]
fn conpty_stty_window_size_setting() {
    run_conpty_legs(&["rows-cols-set", "bare-dimension-queries", "mixed-order"]);
}

/// Apply-pass integer parsing (GNU integer_arg wording, stty.c:1220) and
/// the honest console-mode `-g` field with its style refusal.
#[test]
fn conpty_stty_integer_arg_and_gfmt() {
    run_conpty_legs(&["invalid-integer", "gfmt-hex"]);
}
