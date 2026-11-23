//! rubash#4149 — guard: the canonical console line discipline
//! (read_helpers::ConsoleLineDiscipline) arms ONLY on a Windows
//! console in line-input mode. Every other stdin source — pipes,
//! files, Unix ttys (whose kernel applies the discipline itself) —
//! must keep the raw byte stream: a piped 0x04 is data and a piped
//! CRLF stays CRLF, exactly as GNU cat.c's copy loop delivers them.
//!
//! The discipline's own verdicts (VEOF flush/EOF, CRLF→LF) are pinned
//! at the pure byte layer in read_helpers' unit tests
//! (console_line_discipline_tests): a synthetic console cannot be
//! driven from a script (0x04 arrives as text through every injection
//! route — the reporter's methodology note on #4149), so the
//! console-side behavior is unit-pinned and owed a real-host recheck.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_cat(script: &str, input: &[u8]) -> Vec<u8> {
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
fn piped_bytes_pass_through_cat_unchanged() {
    // 0x04 mid-stream and a CRLF pair: on a pipe both are plain data.
    let input = b"a\x04b\r\nsecond line\n";
    assert_eq!(run_cat("cat", input), input);
}

#[test]
fn piped_bytes_pass_through_cat_dash_unchanged() {
    let input = b"a\x04b\r\nsecond line\n";
    assert_eq!(run_cat("cat -", input), input);
}

#[test]
fn piped_bytes_pass_through_cat_redirect_unchanged() {
    let dir = std::env::temp_dir().join(format!("rubash4149-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let input = b"a\x04b\r\nsecond line\n";
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("cat > out.bin")
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input)
        .expect("write stdin");
    let status = child.wait().expect("wait rubash");
    assert!(status.success(), "rubash cat > out.bin failed");
    let written = std::fs::read(dir.join("out.bin")).expect("read out.bin");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(written, input);
}
