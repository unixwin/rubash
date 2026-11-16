//! rubash#2883 — interactive `read -p` prompt renders raw-byte marker
//! pairs instead of the control bytes.
//!
//! The expanded prompt word carries control bytes as raw-byte marker
//! pairs (U+E000 + payload, the transport encoding of
//! `src/executor/markers.rs`). The PS0/PS1/PS2/PS4 channels decode the
//! pairs after expansion (rubash#2878, `decode_prompt_transport`); the
//! `read -p` prompt travels a different display path (read builtin ->
//! stderr print / line editor) and reached the terminal undecoded, so
//! an ESC in the prompt displayed as the pair glyphs
//! (EE 80 80 EE 80 9C) instead of the ESC byte.
//!
//! GNU anchors: builtins/read.def:316 (prompt is the word-expanded
//! argument), read.def:670-674 (non-edit path prints the expanded
//! bytes verbatim to stderr), read.def:697 (edit path hands the same
//! bytes to the line editor). No prompt-escape decoding applies to
//! `read -p` (`\[` stays literal text).
//!
//! Harness: util-linux `script(1)` PTY, the repository's #1255 form.
//! Assertions are byte-level over the typescript: the typed command
//! lines are echoed with their literal backslash spellings, so a real
//! control byte in the typescript can only come from the shell's own
//! prompt rendering. Seed: the #2879 sweep's PTY cell s4
//! (`~/workspace/pua-audit/probes/pty_prompts.py`).

#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(unix)]
use std::time::Duration;

mod common;
#[cfg(unix)]
use common::wait_bounded;

#[cfg(unix)]
static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// U+E000 in UTF-8 — the raw-byte marker escape. Its presence in the
/// typescript means a transport pair leaked to the terminal.
#[cfg(unix)]
const E000_UTF8: &[u8] = "\u{E000}".as_bytes();

#[cfg(unix)]
fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash2883-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `rubash -i` under a `script(1)` PTY, feed `input`, and return
/// the raw typescript bytes.
#[cfg(unix)]
fn run_pty(input: &[u8]) -> Vec<u8> {
    let dir = scratch("pty");
    let mut child = Command::new("script")
        .args([
            "-qec",
            &format!("{} -i", env!("CARGO_BIN_EXE_rubash")),
            "/dev/null",
        ])
        .current_dir(&dir)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn script(1) PTY harness");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input)
        .expect("write stdin");
    drop(child.stdin.take());
    let outcome = wait_bounded(&mut child, Duration::from_secs(30));
    assert!(
        !outcome.timed_out,
        "child {} did not exit within the bound; partial stderr: {}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    outcome.stdout
}

/// Run `rubash -i` under a `script(1)` PTY, feeding each chunk with
/// a settle delay after it, and return the raw typescript bytes.
/// The `-e` editor path needs paced input: with an all-at-once write
/// the REPL editor's read-ahead competes with the builtin's own
/// editor for the answer line (captain-verified harness artifact —
/// paced input completes the read).
#[cfg(unix)]
fn run_pty_staged(steps: &[(&[u8], u64)]) -> Vec<u8> {
    let dir = scratch("pty");
    let mut child = Command::new("script")
        .args([
            "-qec",
            &format!("{} -i", env!("CARGO_BIN_EXE_rubash")),
            "/dev/null",
        ])
        .current_dir(&dir)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn script(1) PTY harness");
    {
        let stdin = child.stdin.as_mut().expect("stdin pipe");
        // Let the REPL print its first prompt before typing.
        std::thread::sleep(Duration::from_millis(800));
        for (chunk, settle_ms) in steps {
            stdin.write_all(chunk).expect("write stdin");
            std::thread::sleep(Duration::from_millis(*settle_ms));
        }
    }
    drop(child.stdin.take());
    let outcome = wait_bounded(&mut child, Duration::from_secs(30));
    assert!(
        !outcome.timed_out,
        "child {} did not exit within the bound; partial stderr: {}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    outcome.stdout
}

#[cfg(unix)]
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(unix)]
#[test]
fn read_p_prompt_renders_esc_from_ansi_c_quoting() {
    // Sweep cell s4 verbatim: the prompt argument's ESC arrives via
    // $'...' in the source word.
    let typescript = run_pty(b"read -p $'R\\033S: ' x\nhello\necho \"got=$x\"\nexit\n");
    assert!(
        contains(&typescript, b"R\x1bS"),
        "prompt carries the real ESC byte; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        !contains(&typescript, E000_UTF8),
        "no raw-byte marker pair leaks; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        contains(&typescript, b"got=hello"),
        "the read itself still works; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
}

#[cfg(unix)]
#[test]
fn read_p_prompt_renders_esc_from_variable_value() {
    // The ESC is introduced by expansion (a stored variable value) —
    // the #2878 family shape on the read channel.
    let typescript =
        run_pty(b"v=$(printf 'A\\033B')\nread -p \"<$v> \" x\nhello\necho \"got=$x\"\nexit\n");
    assert!(
        contains(&typescript, b"<A\x1bB> "),
        "prompt carries the stored ESC byte; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        !contains(&typescript, E000_UTF8),
        "no raw-byte marker pair leaks; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        contains(&typescript, b"got=hello"),
        "the read itself still works; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
}

#[cfg(unix)]
#[test]
fn read_p_prompt_renders_esc_from_command_substitution() {
    let typescript = run_pty(b"read -p \"$(printf 'C\\033D') \" x\nhello\necho \"got=$x\"\nexit\n");
    assert!(
        contains(&typescript, b"C\x1bD "),
        "prompt carries the substitution's ESC byte; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        !contains(&typescript, E000_UTF8),
        "no raw-byte marker pair leaks; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        contains(&typescript, b"got=hello"),
        "the read itself still works; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
}

#[cfg(unix)]
#[test]
fn read_e_p_prompt_renders_esc_in_line_editor() {
    // The -e path hands the prompt to the line editor, whose repaint
    // frames must carry the real byte too. Paced input (see
    // run_pty_staged): the editor read must not race the REPL's
    // read-ahead for the answer line.
    let typescript = run_pty_staged(&[
        (b"read -e -p $'E\\033F: ' x\n".as_slice(), 2000),
        (b"hello\n".as_slice(), 1000),
        (b"echo \"got=$x\"\nexit\n".as_slice(), 500),
    ]);
    assert!(
        contains(&typescript, b"E\x1bF"),
        "editor prompt carries the real ESC byte; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        !contains(&typescript, E000_UTF8),
        "no raw-byte marker pair leaks; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        contains(&typescript, b"got=hello"),
        "the read itself still works; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
}

#[cfg(unix)]
#[test]
fn read_p_prompt_keeps_decoded_sx01_data_byte() {
    // A real 0x01 byte that is prompt DATA (from $'\001') is emitted
    // verbatim by GNU's fprintf (read.def:670-674); decoding the
    // transport pairs must not eat it as a prompt-ignore marker.
    let typescript = run_pty(b"read -p $'P\\001Q: ' x\nhello\necho \"got=$x\"\nexit\n");
    assert!(
        contains(&typescript, b"P\x01Q"),
        "prompt carries the real 0x01 data byte; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        !contains(&typescript, E000_UTF8),
        "no raw-byte marker pair leaks; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        contains(&typescript, b"got=hello"),
        "the read itself still works; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
}

#[cfg(unix)]
#[test]
fn read_p_prompt_keeps_pair_encoded_sx01_from_comsub() {
    // The same 0x01 data byte arriving pair-encoded (command
    // substitution output is transport-encoded) must also decode
    // back to the real byte — not be eaten as a prompt-ignore
    // marker (there is no template stage on this channel).
    let typescript = run_pty(b"read -p \"$(printf 'P\\001Q') \" x\nhello\necho \"got=$x\"\nexit\n");
    assert!(
        contains(&typescript, b"P\x01Q"),
        "prompt carries the decoded 0x01 data byte; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        !contains(&typescript, E000_UTF8),
        "no raw-byte marker pair leaks; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        contains(&typescript, b"got=hello"),
        "the read itself still works; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
}

#[cfg(unix)]
#[test]
fn read_p_prompt_keeps_literal_backslash_brackets() {
    // Control: `read -p` undergoes NO prompt-escape decoding in GNU —
    // a literal \[ in the prompt argument prints as backslash+bracket.
    // (The prompt value is built with doubled backslashes so the tty
    // echo of the typed line cannot satisfy the assertion by itself.)
    let typescript =
        run_pty(b"p=$(printf 'A\\\\[B\\\\] ')\nread -p \"$p\" x\nhello\necho \"got=$x\"\nexit\n");
    assert!(
        contains(&typescript, b"A\\[B\\] "),
        "literal backslash-brackets survive; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
    assert!(
        contains(&typescript, b"got=hello"),
        "the read itself still works; typescript: {}",
        String::from_utf8_lossy(&typescript)
    );
}
