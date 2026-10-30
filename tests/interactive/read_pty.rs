//! POSIX-pty execution-level regressions for the niubash#684 Ctrl-D
//! read faces (the interop face: interactive engine, canonical pty
//! stdin, byte-level driving — the local analog of the WSL-interop
//! harness in niubash pkg9 A6).
//!
//! GNU anchor (bash 5.3.20 on the same pty script): `read` at a
//! Ctrl-D EOF fails with rc 1 and the shell SURVIVES — the EOF ends
//! only the current input; mid-line Ctrl-D is a flush, not an EOF;
//! prompt Ctrl-D exits 0. Pre-fix, the read acquisition's first
//! transport saw the EOF but the cascade asked a second blocking
//! reader on the same descriptor, which swallowed the EOF and consumed
//! the next typed line as the read's record.
//!
//! The driver skips itself (exit 2) where python3 or the pty module is
//! unavailable (Windows hosts run the ConPTY sibling instead).

use std::process::{Command, Stdio};

const DRIVER: &str = "tests/interactive/support/pty_read_probe.py";

fn python_interpreter() -> Option<String> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("-c")
            .arg("import pty")
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

/// Run the named legs through the pty driver and require every one to
/// report PASS. The driver prints `leg=PASS` / `leg=FAIL detail` lines
/// and exits 2 (with a `SKIP` line) when no POSIX pty is available.
fn run_pty_legs(legs: &[&str]) {
    let Some(python) = python_interpreter() else {
        eprintln!("skip: python3 with pty module not available");
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
        .expect("failed to spawn the pty driver (is python3 on PATH?)");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    eprintln!("driver output:\n{stdout}");
    if !stderr.trim().is_empty() {
        eprintln!("driver stderr:\n{stderr}");
    }
    if stdout.trim_start().starts_with("SKIP") {
        eprintln!("skip: pty driver reports unavailable dependencies");
        return;
    }
    assert!(
        output.status.success(),
        "pty read legs failed (see driver output above)"
    );
    for leg in legs {
        assert!(
            stdout.contains(&format!("{leg}=PASS")),
            "leg {leg} did not report PASS (driver output above)"
        );
    }
}

/// niubash#684: Ctrl-D at a plain canonical `read` is the EOF — the
/// builtin fails (rc 1, empty assignment), the shell survives and runs
/// the next command, and a later prompt Ctrl-D still exits 0.
#[test]
fn pty_ctrld_read_eof_ends_read_not_shell_niubash684() {
    run_pty_legs(&["ctrld-read-eof"]);
}

/// niubash#684 controls: mid-line Ctrl-D is the VEOF flush (the record
/// continues past it) and prompt Ctrl-D exits the shell with status 0.
#[test]
fn pty_ctrld_flush_and_prompt_eof_niubash684() {
    run_pty_legs(&["ctrld-read-flush", "ctrld-prompt-eof"]);
}

/// The whole 684 pty set in one pass (same legs, one session per leg).
#[test]
fn pty_ctrld_faces_full_pass_niubash684() {
    run_pty_legs(&["ctrld-read-eof", "ctrld-read-flush", "ctrld-prompt-eof"]);
}
