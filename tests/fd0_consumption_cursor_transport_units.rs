//! fd 0 consumption cursor unit pinning (unixwin/niubash#383 follow-up).
//!
//! Between in-process stages the shell carries a payload as *transport text*:
//! a raw control byte is stored as the carrier pair `U+E000 + U+E0xx`
//! (`locale.rs:352`), and a lone invalid high byte likewise, so a payload's
//! transport length differs from its raw-byte length — x6 for a control byte,
//! x2 for a high byte (`shell_text_to_raw_bytes` recovers the raw length).
//!
//! The shared fd 0 cursor (`FUNCTION_STDIN_OFFSET`) is a byte index into that
//! *transport* buffer: `read_function_stdin` slices `&input[offset..]`,
//! `function_stdin_remaining` slices `input[offset..]`, `select` slices
//! `&stdin_content[*stdin_offset..]`. The pipeline drivers therefore advance
//! the cursor by the TRANSPORT length they handed the external child
//! (`pipeline_stages.rs`'s `input.len()`; `pipeline_exec.rs`'s
//! `stage0_input.len()`), NOT by `shell_text_to_raw_bytes(..).len()` — a raw
//! delta under-advances by the carrier overhead and re-exposes bytes the child
//! already consumed. (This is the unit-twin of #695: there a byte count handed
//! to the *user* had to be raw; here an *index into transport text* must be
//! transport.)
//!
//! Each test below leaves a carrier-bearing shared buffer fully consumed by an
//! external pipeline and asserts the follow-up reader sees EOF. Computing the
//! cursor delta in raw units makes the three drain tests fail with the already
//! consumed tail replayed (verified against a raw-unit build).

use std::process::{Command, Stdio};

/// Run `script` through `rubash -c`, returning (stdout, stderr, exit code).
/// Every script binds fd 0 with a here-string, so no external stdin is fed.
fn run(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    // The here-string supplies fd 0; close the inherited pipe so a stray read
    // sees EOF instead of blocking.
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// An external pipeline (all-external concurrent on Windows, sequential on
/// Unix) consumes the whole shared fd 0 buffer, which carries a control byte
/// (0x1B => transport x6). The cursor must land on the buffer's transport end
/// so the next `read` sees EOF; a raw-length delta (`base + 1 + 1 + 1` here)
/// would land inside the buffer and replay `A`.
#[test]
fn external_pipeline_drains_shared_stdin_control_byte() {
    let (stdout, stderr, rc) = run(
        r#"{ od -An -tx1 | od -An -tx1 >/dev/null; read -r x; printf 'after=[%s] rc=%s\n' "$x" "$?"; } <<< $'\033A'"#,
    );
    assert_eq!(stdout, "after=[] rc=1\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Same shape with a lone invalid high byte (0xCD => transport x2): the raw
/// length and transport length differ, so this pins the same unit from the
/// other divergence axis.
#[test]
fn external_pipeline_drains_shared_stdin_high_byte() {
    let (stdout, stderr, rc) = run(
        r#"{ od -An -tx1 | od -An -tx1 >/dev/null; read -r x; printf 'after=[%s] rc=%s\n' "$x" "$?"; } <<< $'ab\315X'"#,
    );
    assert_eq!(stdout, "after=[] rc=1\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// A nonzero `base`: `read` first advances the cursor past `line\n`, then the
/// external pipeline consumes the carrier-bearing remainder. The fold
/// (`base + stage0_input.len()`) must add a transport delta so the cursor
/// reaches end-of-buffer; a raw delta stops short by the carrier overhead and
/// leaves `B` / `TAIL` readable.
#[test]
fn external_pipeline_drains_shared_stdin_after_partial_read() {
    let (stdout, stderr, rc) = run(
        r#"{ read -r x; od -An -tx1 | od -An -tx1 >/dev/null; read -r y; printf 'y=[%s] rc=%s\n' "$y" "$?"; } <<< $'line\n\033B\nTAIL'"#,
    );
    assert_eq!(stdout, "y=[] rc=1\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// The remainder path: after `read` consumes one transport record, an external
/// consumer must receive exactly the raw bytes that remain. This pins the
/// cursor unit in the shared-buffer slice (`function_stdin_remaining`) that the
/// pipeline drivers fold into.
#[test]
fn partial_read_then_external_consumer_sees_raw_remainder() {
    let (stdout, stderr, rc) =
        run(r#"{ read -r x; printf 'x=[%s]\n' "$x"; od -An -tx1; } <<< $'line\n\033B'"#);
    assert_eq!(stdout, "x=[line]\n 1b 42 0a\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}
