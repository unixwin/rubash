//! unixwin/niubash#519: `echo >/dev/tty` reported `Invalid argument` and
//! status 1, breaking niugit's askpass prompt script (whose trailing
//! `echo >/dev/tty` failed the whole prompt even after a successful read).
//!
//! /dev/tty maps to the CON device (executor/path.rs), which rejects
//! FILE_APPEND_DATA at CreateFileW with ERROR_INVALID_PARAMETER. The
//! redirect's own open succeeded (`File::create`), but every downstream
//! append-mode reopen of the target — OutputTarget::Path's deferred
//! writes, `>>` probes, and the fd-table's `exec N>>` bind
//! (fd::open_file_append) — failed the same way. DOS device targets now
//! open in plain write mode: GNU redir.c:725 redir_open's
//! open(O_WRONLY|O_CREAT|O_APPEND) succeeds on a POSIX tty because
//! O_APPEND is a no-op on a character device, and a stream device has no
//! offset for Windows append semantics to apply to.
//!
//! These legs need a real console (conhost); headless runs cannot open
//! CON at all, so each test skips — the same console gating as
//! issue282_stty_console_regressions' ConPTY legs.

use std::fs::OpenOptions;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Whether this process can open a console device at all (headless CI
/// has none; a write-mode open is exactly what the fix relies on).
fn console_available() -> bool {
    cfg!(windows) && OpenOptions::new().write(true).open("CON").is_ok()
}

/// The issue's repro: the buffered-builtin write used to fail the
/// append-mode reopen of CON.
#[test]
fn dev_tty_write_redirect_succeeds() {
    if !console_available() {
        return;
    }
    let (stdout, stderr, code) = rubash("echo >/dev/tty; echo \"s=$?\"");
    assert_eq!(stdout, "s=0\n", "stderr={stderr:?} code={code:?}");
    assert!(stderr.is_empty(), "stderr={stderr:?}");
}

/// `>>` failed earlier, at the redirect's own append-mode open probe.
#[test]
fn dev_tty_append_redirect_succeeds() {
    if !console_available() {
        return;
    }
    let (stdout, stderr, code) = rubash("echo >>/dev/tty; echo \"s=$?\"");
    assert_eq!(stdout, "s=0\n", "stderr={stderr:?} code={code:?}");
    assert!(stderr.is_empty(), "stderr={stderr:?}");
}

/// The issue's working control (`3>`) plus the append bind (`3>>`) that
/// shared the FILE_APPEND_DATA failure through fd::open_file_append.
#[test]
fn dev_tty_exec_bind_write_and_append_succeed() {
    if !console_available() {
        return;
    }
    let (stdout, stderr, code) = rubash(
        "exec 3>/dev/tty; echo >&3; echo \"a=$?\"; exec 3>&-; \
         exec 4>>/dev/tty; echo >&4; echo \"b=$?\"; exec 4>&-",
    );
    assert_eq!(stdout, "a=0\nb=0\n", "stderr={stderr:?} code={code:?}");
    assert!(stderr.is_empty(), "stderr={stderr:?}");
}

/// `<>` needs a read+write descriptor: CON rejects GENERIC_READ|
/// GENERIC_WRITE outright, so the bind splits it into CONIN$ (read) +
/// CONOUT$ (write) via FileFd::open_readwrite_pair. `exec N<>`,
/// `{var}<>`, and the command-scoped form all used to fail the open.
#[test]
fn dev_tty_readwrite_redirect_succeeds() {
    if !console_available() {
        return;
    }
    let (stdout, stderr, code) = rubash(
        "exec 5<>/dev/tty; echo \"a=$?\"; exec 5>&- 5<&-; \
         {v}<>/dev/tty; echo \"b=$?\"; \
         echo x <>/dev/tty; echo \"c=$?\"",
    );
    assert_eq!(
        stdout, "a=0\nb=0\nx\nc=0\n",
        "stderr={stderr:?} code={code:?}"
    );
    assert!(stderr.is_empty(), "stderr={stderr:?}");
}

/// `2>`, `&>`, `&>>` route through the same append-reopen machinery.
#[test]
fn dev_tty_stderr_and_combined_redirects_succeed() {
    if !console_available() {
        return;
    }
    let (stdout, stderr, code) =
        rubash("echo 2>/dev/tty; echo \"a=$?\"; echo &>>/dev/tty; echo \"b=$?\"");
    // echo's own newline still lands on un-redirected fd 1.
    assert_eq!(stdout, "\na=0\nb=0\n", "stderr={stderr:?} code={code:?}");
    assert!(stderr.is_empty(), "stderr={stderr:?}");
}
