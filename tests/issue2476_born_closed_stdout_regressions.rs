//! Issue unixwin/rubash#2476: a rubash process BORN with fd 1
//! already closed (a kernel-exec'd shebang script whose parent
//! launched it `>&-`) silently exited 0 on builtin output — GNU
//! bash and dash exit 1 with `echo: write error: Bad file
//! descriptor`.
//!
//! GNU semantics (bash 5.3 sources): builtins buffer their stdout
//! and return sh_chkwrite's verdict — sh_chkwrite
//! (builtins/common.c:319-334) fflushes stdout and, on error,
//! sh_wrerror (common.c:307) prints `<name>: write error:
//! <strerror>` and the builtin returns EXECUTION_FAILURE. The C
//! runtime never touches the descriptor set, so a descriptor
//! closed at exec stays closed and the flush fails with EBADF.
//!
//! Root cause of the rubash gap, in two layers:
//!  1. The Rust runtime sanitizes standard descriptors before
//!     main (std::rt::init opens /dev/null over a closed one), so
//!     the fd table seeded fd 1 as a live Stdout endpoint and every
//!     write "succeeded" into /dev/null — no diagnostic, no
//!     failure. The fd layer now captures the at-exec descriptor
//!     state in a loader constructor (before the sanitization) and
//!     FdTable::new seeds a born-closed fd 1 as closed. A fd 1
//!     genuinely redirected from /dev/null was OPEN at exec and
//!     is unaffected.
//!  2. The listing builtins (declare, alias, set, export -p,
//!     umask, ...) printed the write-error diagnostic from the
//!     buffered-write machinery but their dispatches never folded
//!     its failure flag into their status. The fold now happens
//!     at execute_materialized_command, the one completion point
//!     every simple command passes.
//!
//! Expectations are the GNU Bash 5.3.20 outputs of the same
//! shapes (oracle binary run side by side in the cluster
//! workdir). The born-closed pins spawn the script directly
//! (kernel shebang exec) with fd 1 closed in pre_exec — a foreign
//! parent, no rubash spawn funnel involved.
#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2476-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// Write an executable script whose interpreter is the rubash
/// binary under test; returns its absolute path.
fn write_child(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let script = format!("#!{}\n{body}", env!("CARGO_BIN_EXE_rubash"));
    std::fs::write(&path, script).expect("write child script");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod child script");
    path
}

/// Spawn the script with fd 1 CLOSED at exec (pre_exec runs after
/// stdio setup, so the close is the state the kernel hands the
/// new image). Returns (stderr, exit code).
fn run_born_closed_stdout(script: &Path) -> (String, Option<i32>) {
    let mut cmd = Command::new(script);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            if libc::close(1) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = cmd.output().expect("run born-closed child");
    (
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's headline shape: `echo hi` in a child born with
/// fd 1 closed reports the GNU write error and exits 1 (the
/// script's echo is on line 2, after the shebang line).
#[test]
fn issue2476_born_closed_stdout_echo_write_error_status_1() {
    let dir = test_dir("echo");
    let script = write_child(&dir, "child.sh", "echo hi\n");
    let (err, code) = run_born_closed_stdout(&script);
    assert_eq!(
        err,
        format!(
            "{}: line 2: echo: write error: Bad file descriptor\n",
            script.display()
        )
    );
    assert_eq!(code, Some(1));
}

/// printf shares the sh_chkwrite contract with echo.
#[test]
fn issue2476_born_closed_stdout_printf_write_error_status_1() {
    let dir = test_dir("printf");
    let script = write_child(&dir, "child.sh", "printf 'x\\n'\n");
    let (err, code) = run_born_closed_stdout(&script);
    assert_eq!(
        err,
        format!(
            "{}: line 2: printf: write error: Bad file descriptor\n",
            script.display()
        )
    );
    assert_eq!(code, Some(1));
}

/// The failure folds into `$?` inside the child, and a later
/// successful command decides the process exit — GNU prints the
/// diagnostic, reports inner status 1, and exits 0.
#[test]
fn issue2476_born_closed_stdout_status_propagates_inside_child() {
    let dir = test_dir("inner");
    let script = write_child(&dir, "child.sh", "echo hi\necho \"inner=$?\" >&2\n");
    let (err, code) = run_born_closed_stdout(&script);
    assert_eq!(
        err,
        format!(
            "{}: line 2: echo: write error: Bad file descriptor\ninner=1\n",
            script.display()
        )
    );
    assert_eq!(code, Some(0));
}

/// A listing builtin as the child's last command: the diagnostic
/// was already printed pre-fix, but the status stayed 0 — the
/// completion-point fold closes that half of the gap too.
#[test]
fn issue2476_born_closed_stdout_declare_status_1() {
    let dir = test_dir("declare");
    let script = write_child(&dir, "child.sh", "x=1\ndeclare -p x\n");
    let (err, code) = run_born_closed_stdout(&script);
    assert_eq!(
        err,
        format!(
            "{}: line 3: declare: write error: Bad file descriptor\n",
            script.display()
        )
    );
    assert_eq!(code, Some(1));
}

/// Control: fd 2 is unaffected by fd 1's birth state — a child
/// born with fd 1 closed still writes stderr normally, status 0.
#[test]
fn issue2476_born_closed_stdout_stderr_unaffected() {
    let dir = test_dir("stderr-ok");
    let script = write_child(&dir, "child.sh", "echo hi >&2\n");
    let (err, code) = run_born_closed_stdout(&script);
    assert_eq!(err, "hi\n");
    assert_eq!(code, Some(0));
}

/// Control: a fd 1 genuinely redirected from /dev/null was OPEN
/// at exec — output is silently discarded and the status is 0,
/// exactly GNU's `script >/dev/null`. This guards the at-exec
/// capture against misreading the runtime's sanitized descriptor
/// as "born closed".
#[test]
fn issue2476_devnull_stdout_stays_silent_success() {
    let dir = test_dir("devnull");
    let script = write_child(&dir, "child.sh", "echo hi\n");
    let output = Command::new(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run /dev/null child");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert_eq!(output.status.code(), Some(0));
}

/// The parent-session face from the issue: a no-shebang helper
/// script run by `rubash -c` with its stdout closed reports
/// status 1 to the parent, with the diagnostic naming the
/// resolved script path (line 1 — no shebang line).
#[test]
fn issue2476_parent_session_reports_child_status_1() {
    let dir = test_dir("parent");
    let helper = dir.join("emitout");
    std::fs::write(&helper, "echo hi\n").expect("write helper");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755))
        .expect("chmod helper");
    let path_env = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .env("PATH", path_env)
        .args(["-c", "emitout >&-; printf 'status:%s\\n' \"$?\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn parent rubash");
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait parent rubash");
    assert_eq!(out_thread.join().unwrap_or_default(), "status:1\n");
    assert_eq!(
        err_thread.join().unwrap_or_default(),
        format!(
            "{}: line 1: echo: write error: Bad file descriptor\n",
            helper.display()
        )
    );
    assert_eq!(status.code(), Some(0));
}
