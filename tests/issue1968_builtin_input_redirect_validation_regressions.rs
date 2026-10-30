//! rubash#1968 — the buffered builtin route (echo/printf) never
//! validated failed INPUT redirects: `echo hi < /dev/fd/63` (bad fd)
//! and the `<>` form executed silently with status 0 and produced
//! output, where GNU suppresses execution with status 1. The fd
//! shapes were settled by the #1963 reject-gate fix; the NET shapes
//! remained: the gate's input probes exempt every registry class,
//! so `echo hi < /dev/tcp/<refused>` still executed silently and
//! `echo hi < /dev/tcp/<open>` never connected at all. The buffered
//! route's flush funnel now settles the registry classes the gate
//! exempts (net/synthetic) before writing anything.
//!
//! GNU model: do_redirections runs before the builtin body
//! (execute_cmd.c:5606 execute_builtin_or_function); a failed open is
//! a redirection_error (redir.c:149-176) — the command does not run.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::io::Read;
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// A loopback listener reporting (connection count) once the test's
/// shell exits and a short settle window passes.
struct Listener {
    port: u16,
    rx: mpsc::Receiver<usize>,
}

fn start_listener() -> Listener {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let mut count = 0usize;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    count += 1;
                    let mut sink = [0u8; 64];
                    let _ = stream.read(&mut sink);
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
            if tx.send(count).is_err() {
                break;
            }
        }
    });
    Listener { port, rx }
}

impl Listener {
    fn connections(&self) -> usize {
        std::thread::sleep(Duration::from_millis(400));
        let mut last = 0;
        while let Ok(n) = self.rx.try_recv() {
            last = n;
        }
        last
    }
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

#[test]
fn issue1968_echo_input_from_closed_fd_is_suppressed() {
    let (stdout, stderr, _code) = run_c("echo hi < /dev/fd/63; echo rc=$?");
    assert_eq!(stdout, "rc=1\n", "echo must not produce output");
    assert!(
        stderr.contains("/dev/fd/63: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_echo_readwrite_from_closed_fd_is_suppressed() {
    let (stdout, stderr, _code) = run_c("echo hi <> /dev/fd/63; echo rc=$?");
    assert_eq!(stdout, "rc=1\n", "echo must not produce output");
    assert!(
        stderr.contains("/dev/fd/63: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_printf_input_from_closed_fd_is_suppressed() {
    let (stdout, stderr, _code) = run_c("printf 'hi\\n' < /dev/fd/63; echo rc=$?");
    assert_eq!(stdout, "rc=1\n", "printf must not produce output");
    assert!(
        stderr.contains("/dev/fd/63: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_echo_input_from_missing_file_is_suppressed() {
    // Control: the plain-file face already failed correctly before
    // this change (the reject gate's Input probe) and must keep
    // failing identically.
    let (stdout, stderr, _code) = run_c("echo hi < /nonexistent-x-1968; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent-x-1968: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_echo_input_from_refused_tcp_is_suppressed() {
    let port = closed_port();
    let (stdout, stderr, _code) =
        run_c(&format!("echo hi < /dev/tcp/127.0.0.1/{port}; echo rc=$?"));
    assert_eq!(stdout, "rc=1\n", "echo must not produce output");
    assert!(
        stderr.contains(&format!("/dev/tcp/127.0.0.1/{port}")),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_echo_readwrite_from_refused_tcp_is_suppressed() {
    let port = closed_port();
    let (stdout, stderr, _code) =
        run_c(&format!("echo hi <> /dev/tcp/127.0.0.1/{port}; echo rc=$?"));
    assert_eq!(stdout, "rc=1\n", "echo must not produce output");
    assert!(
        stderr.contains(&format!("/dev/tcp/127.0.0.1/{port}")),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_echo_input_from_open_tcp_still_runs_and_connects_once() {
    let listener = start_listener();
    let (stdout, _stderr, code) = run_c(&format!("echo hi < /dev/tcp/127.0.0.1/{}", listener.port));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\n");
    assert_eq!(
        listener.connections(),
        1,
        "GNU performs the input netopen exactly once"
    );
}

#[test]
fn issue1968_echo_with_open_input_redirects_still_runs() {
    // Controls: input redirects that open fine must not suppress.
    let (stdout, _stderr, code) = run_c("echo hi < /dev/null");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\n");
    let (stdout, _stderr, code) = run_c("echo hi <> /dev/null");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\n");
}

// rubash#1968 residual coverage: the issue's follow-up names the
// representative builtins "echo/read 等" — the buffered echo/printf
// route landed with #2051; these pins lock the remaining representative
// builtins on the same bad-fd/synthetic faces (GNU: do_redirections
// precedes every command, execute_cmd.c:5606 — the suppression is not
// echo-specific).

#[test]
fn issue1968_read_input_from_closed_fd_is_suppressed() {
    let (stdout, stderr, _code) = run_c("read x < /dev/fd/63; echo rc=$?");
    assert_eq!(stdout, "rc=1\n", "read must not run with a failed redirect");
    assert!(
        stderr.contains("/dev/fd/63: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_read_readwrite_from_closed_fd_is_suppressed() {
    let (stdout, stderr, _code) = run_c("read x <> /dev/fd/63; echo rc=$?");
    assert_eq!(stdout, "rc=1\n", "read must not run with a failed redirect");
    assert!(
        stderr.contains("/dev/fd/63: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_read_input_from_missing_file_is_suppressed() {
    let (stdout, stderr, _code) = run_c("read x < /nonexistent-x-1968; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent-x-1968: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_colon_input_from_closed_fd_is_suppressed() {
    let (stdout, stderr, _code) = run_c(": < /dev/fd/63; echo rc=$?");
    assert_eq!(stdout, "rc=1\n", ": must not succeed through a failed open");
    assert!(
        stderr.contains("/dev/fd/63: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_external_command_input_from_closed_fd_is_suppressed() {
    let (stdout, stderr, _code) = run_c("true < /dev/fd/63; echo rc=$?");
    assert_eq!(stdout, "rc=1\n", "the external face must not run either");
    assert!(
        stderr.contains("/dev/fd/63: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1968_read_with_open_input_redirect_still_runs() {
    // Control: read consumes the bound input when the redirect opens.
    let (stdout, _stderr, code) = run_c("echo hi | { read x; echo \"got=$x\"; }");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got=hi\n");
}
