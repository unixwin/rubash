//! rubash#2389 (test debt for rubash#2339) — `read -e` landed with
//! the bind/read cluster (PR #2364) without a named integration
//! regression file. This file ports the PTY probe shapes from
//! `~/workspace/fix-r4-bind-read/probes/read_e_pty.py` (the hunt r35
//! PTY case shapes rs01–rs29) into the repo suite with the GNU
//! expectations baked into the probe.
//!
//! Harness mirrors the probe: the shell is spawned on a fresh PTY
//! (stdin/stdout/stderr all on the slave), the harness waits ~0.35s
//! for the shell to install its termios, writes the key bytes, then
//! reads until the process exits (6s deadline) and checks the
//! expected suffix appears in the captured output. Verdicts recorded
//! 2026-11-01: GNU bash 5.3.20 passes 17/17; rubash master passed
//! 16/17 — rs24 (C-d on an empty buffer must end the read with EOF)
//! was the one divergence, and its pin kept the GNU expectation
//! under an `#[ignore]` marker instead of weakening it.
//! rubash#2416 closed the divergence (the editor surfaced EOF
//! correctly; the read builtin dropped the verdict and fell into a
//! second blocking read), so the rs24 pin now runs un-ignored.
//!
//! Unix-only: the harness needs openpty(3).

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

#[cfg(unix)]
fn run_case(script: &str, keys: &[u8]) -> String {
    unsafe {
        let mut master = -1;
        let mut slave = -1;
        let rc = libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, 0, "openpty failed");
        let stdin_f = File::from_raw_fd(libc::dup(slave));
        let stdout_f = File::from_raw_fd(libc::dup(slave));
        let stderr_f = File::from_raw_fd(slave);
        let mut child = Command::new(RUBASH)
            .args(["-c", script])
            .stdin(Stdio::from(stdin_f))
            .stdout(Stdio::from(stdout_f))
            .stderr(Stdio::from(stderr_f))
            .spawn()
            .expect("spawn rubash on pty");
        // Give the shell a moment to install its termios before
        // typing (same settle as the python probe).
        std::thread::sleep(Duration::from_millis(350));
        let mut master_f = File::from_raw_fd(master);
        master_f.write_all(keys).expect("write key bytes");
        let flags = libc::fcntl(master_f.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(
            master_f.as_raw_fd(),
            libc::F_SETFL,
            flags | libc::O_NONBLOCK,
        );
        let mut out: Vec<u8> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut buf = [0u8; 4096];
        loop {
            match master_f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => {}
            }
            if let Ok(Some(_)) = child.try_wait() {
                // Drain whatever is left, then stop.
                loop {
                    match master_f.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => out.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.wait();
        String::from_utf8_lossy(&out).into_owned()
    }
}

#[cfg(unix)]
macro_rules! read_e_case {
    ($name:ident, $script:expr, $keys:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let out = run_case($script, $keys);
            assert!(
                out.contains($expected),
                "expected suffix {:?} in captured output {:?}",
                $expected,
                out
            );
        }
    };
}

#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs01_plain,
    "read -e v; echo \"<$v>\"",
    b"hello\r",
    "<hello>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs03_c_a,
    "read -e v; echo \"<$v>\"",
    b"hello world\x01X\r",
    "<Xhello world>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs04_c_e,
    "read -e v; echo \"<$v>\"",
    b"abc\x01\x05XYZ\r",
    "<abcXYZ>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs05_c_k,
    "read -e v; echo \"<$v>\"",
    b"abc\x01\x0b\r",
    "<>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs07_c_u_c_y,
    "read -e v; echo \"<$v>\"",
    b"junk\x15\x19\r",
    "<junk>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs08_c_b,
    "read -e v; echo \"<$v>\"",
    b"ab\x02X\r",
    "<aXb>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs09_c_f,
    "read -e v; echo \"<$v>\"",
    b"ab\x01\x06X\r",
    "<aXb>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs10_meta_b,
    "read -e v; echo \"<$v>\"",
    b"foo bar\x1bbX\r",
    "<foo Xbar>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs11_meta_d,
    "read -e v; echo \"<$v>\"",
    b"foo bar\x01\x1bd\r",
    // Editor buffer after ESC-d is " bar"; the assignment strips
    // the leading IFS whitespace, so the variable is "bar".
    "<bar>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs12_c_t,
    "read -e v; echo \"<$v>\"",
    b"ab\x14\r",
    "<ba>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs13_init,
    "read -e -i INIT v; echo \"<$v>\"",
    b"\r",
    "<INIT>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs14_init_del,
    "read -e -i INIT v; echo \"<$v>\"",
    b"\x7f\x7fOK\r",
    "<INOK>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs17_s_e_c_a,
    "read -e -s v; echo \"<$v>\"",
    b"hello world\x01X\r",
    "<Xhello world>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs19_n3,
    "read -e -n 3 v; echo \"<$v>\"",
    b"abcdef\r",
    "<abc>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs20_delim,
    "read -e -d : v; echo \"<$v>\"",
    b"ab:cd\r",
    "<ab>"
);
#[cfg(unix)]
read_e_case!(
    issue2339_read_e_rs29_c_h,
    "read -e v; echo \"<$v>\"",
    b"ab\x08\r",
    "<a>"
);

/// GNU bash 5.3.20: C-d on an EMPTY buffer is EOF — `read` returns
/// 1 with the variable empty (`<> rc=1`). rubash#2416: the editor
/// always surfaced this EOF (EditEnd::Eof); the read builtin's
/// scalar consumption chain did not recognize the verdict and fell
/// through to a second, canonical-mode blocking read on the same
/// descriptor, so only repaint residue was emitted and the read
/// never terminated. The chain now ends the read on the editor's
/// EOF verdict (read.def:705-710, :949), and this pin runs live.
#[cfg(unix)]
#[test]
fn issue2339_read_e_rs24_c_d_empty() {
    let out = run_case("read -e v; echo \"<$v> rc=$?\"", b"\x04");
    assert!(
        out.contains("<> rc=1"),
        "expected suffix \"<> rc=1\" in captured output {out:?}"
    );
}
