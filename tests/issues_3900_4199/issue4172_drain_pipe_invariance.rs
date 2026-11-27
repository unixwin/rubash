//! rubash#4172 — guard: the drain-style inherited-stdin consumer
//! (read_inherited_process_stdin_to_string, serving the emulated
//! sed and mapfile fallbacks) applies the canonical console line
//! discipline ONLY when the console gate is armed (a Windows console
//! in line-input mode). Every other stdin source — pipes, files,
//! Unix ttys (whose kernel applies the discipline itself) — must
//! keep the raw stream: a piped 0x04 is the consumer's data and a
//! piped CRLF reaches the consumer unnormalized, exactly as GNU's
//! consumers see a pipe's bytes.
//!
//! The discipline's verdicts over drained text are pinned at the
//! pure text layer in read_helpers' unit tests
//! (disciplined_text_byte_table): a synthetic console cannot be
//! driven from a script (0x04 arrives as text through every
//! injection route — the reporter's methodology note on #4149), so
//! the console-side behavior is unit-pinned and owed a real-host
//! recheck.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(script: &str, input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash");
    assert!(output.status.success(), "rubash -c {script:?} failed");
    output.stdout
}

#[test]
fn piped_bytes_pass_through_mapfile_drain_unchanged() {
    // 0x04 mid-stream and CRLF pairs: on a pipe both are plain data
    // and mapfile must receive them byte-for-byte (the -t strip
    // removes only the LF delimiter; the CR stays in the element).
    let input = b"a\x04b\r\nsecond\r\n";
    let out = run(
        "__RUBASH_INHERIT_PROCESS_STDIN=1; mapfile -t arr; printf '[%s]' \"${arr[@]}\"",
        input,
    );
    assert_eq!(out, b"[a\x04b\r][second\r]");
}

#[test]
fn piped_bytes_pass_through_sed_drain_unchanged() {
    // The emulated sed's own line handling is the only transform a
    // pipe may see (it has always split on LF and dropped the CR of
    // a CRLF pair); the 0x04 is data and passes through verbatim,
    // and no discipline verdict truncates or rewrites the stream.
    let input = b"a\x04b\r\nsecond\r\n";
    let out = run("__RUBASH_INHERIT_PROCESS_STDIN=1; sed 's/Q/Z/'", input);
    assert_eq!(out, b"a\x04b\nsecond\n");
}
