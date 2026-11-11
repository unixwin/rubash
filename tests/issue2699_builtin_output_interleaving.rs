//! rubash#2699 (deep-audit A3-R58A) — a multi-operand builtin that emits
//! both stdout lines and stderr diagnostics must interleave them per
//! operand in a merged stream; the shared buffered-output path collected
//! the two streams into separate Vec buffers and emitted all of stdout
//! before any of stderr, so the diagnostic for operand N landed after
//! the output for operand N+1.
//!
//! GNU anchor: builtins/type.def `describe_command` runs once per name
//! and writes through the shell's descriptors as it goes (diagnostics
//! via builtin_error, which is unbuffered), so in a merged stream the
//! per-operand order is observable: `type echo nosuchcmd123 ls 2>&1`
//! prints the echo line, the not-found diagnostic, then the ls line
//! (GNU bash 5.3.20 oracle, probed 2026-11-08). Separate streams and
//! exit statuses were already correct; only the merged order diverged.
//!
//! The `declare -p HOME nosuchvar123 PATH 2>&1` shape from the issue
//! shares the mechanism but its operand loop is reached through
//! `execute_declare` (src/executor/declare_local.rs), which PR #2968
//! (stage-2 ④ Phase 4) is currently rewriting; that conversion is
//! tracked separately and is NOT pinned here.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`; the merge is done
//! by the shell itself (`2>&1` inside the script, or a shared file),
//! so the byte order of the single captured stream is the assertion.
//! The diagnostic line carries the binary's own argv[0] prefix, so the
//! expected text is built from the same path the harness spawns.
//! `LC_ALL=C.UTF-8` is set on the child so the pin is hermetic.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn diag(builtin: &str, name: &str) -> String {
    format!(
        "{}: line 1: {builtin}: {name}: not found\n",
        env!("CARGO_BIN_EXE_rubash")
    )
}

/// Merged stream: the diagnostic for the middle operand sits between
/// the two describe lines, exactly where GNU emits it.
#[test]
fn type_merged_stream_interleaves_per_operand() {
    let out = run("type echo nosuchcmd123 cd 2>&1");
    let expected = format!(
        "echo is a shell builtin\n{}cd is a shell builtin\n",
        diag("type", "nosuchcmd123")
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        expected,
        "merged stdout must interleave per operand"
    );
    assert_eq!(out.code, 1, "type with a missing operand fails");
    assert!(
        out.stderr.is_empty(),
        "2>&1 leaves the real stderr empty; got {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The missing operand first: its diagnostic precedes any output.
#[test]
fn type_merged_stream_missing_operand_first() {
    let out = run("type nosuchcmd123 echo 2>&1");
    let expected = format!("{}echo is a shell builtin\n", diag("type", "nosuchcmd123"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(out.code, 1);
}

/// Two missing operands: each diagnostic lands at its own position.
#[test]
fn type_merged_stream_two_missing_operands() {
    let out = run("type echo nosuchcmd123 cd nosuchcmd456 2>&1");
    let expected = format!(
        "echo is a shell builtin\n{}cd is a shell builtin\n{}",
        diag("type", "nosuchcmd123"),
        diag("type", "nosuchcmd456")
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(out.code, 1);
}

/// `command -V` shares the describe loop: same interleave, and the
/// ANY-found status rule still yields rc 0 (GNU oracle).
#[test]
fn command_v_merged_stream_interleaves_per_operand() {
    let out = run("command -V echo nosuchcmd123 cd 2>&1");
    let expected = format!(
        "echo is a shell builtin\n{}cd is a shell builtin\n",
        diag("command", "nosuchcmd123")
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(out.code, 0, "command -V succeeds when any operand resolves");
}

/// Merged into one file (`> f 2>&1`): the file's byte order is the
/// per-operand order (the shared-description pairing must not regroup
/// the streams either).
#[test]
fn type_merged_file_interleaves_per_operand() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2699-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let file = dir.join("merged.out");
    let script = format!(
        "type echo nosuchcmd123 cd > {} 2>&1",
        file.to_string_lossy()
    );
    let out = run(&script);
    assert_eq!(out.code, 1);
    let bytes = std::fs::read(&file).expect("read merged file");
    let expected = format!(
        "echo is a shell builtin\n{}cd is a shell builtin\n",
        diag("type", "nosuchcmd123")
    );
    assert_eq!(String::from_utf8_lossy(&bytes), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Control: with separate streams nothing changes — stdout carries the
/// two describe lines, stderr the diagnostic, status 1. This cell
/// passes before and after the fix; it guards the unmerged semantics.
#[test]
fn type_separate_streams_unchanged() {
    let out = run("type echo nosuchcmd123 cd");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "echo is a shell builtin\ncd is a shell builtin\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        diag("type", "nosuchcmd123")
    );
    assert_eq!(out.code, 1);
}

/// Control: command substitution keeps capturing stdout only
/// (`$(type -t echo)`), unaffected by the ordered channel.
#[test]
fn type_comsub_capture_unchanged() {
    let out = run("x=$(type -t echo); echo \"[$x]\"");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[builtin]\n");
    assert_eq!(out.code, 0);
}
