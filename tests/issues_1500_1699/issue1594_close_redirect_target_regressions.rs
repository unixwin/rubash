//! rubash#1594 — `cat 1>&-` created a file literally named `&-` and
//! exited 0: the emulated cat opened the close marker as a filename.
//!
//! GNU: redir.c r_close_this closes fd 1 in the child before cat runs,
//! so cat's first write fails EBADF — `cat: standard output: Bad file
//! descriptor`, status 1, and no file is created. The parse side was
//! already correct (kind=CloseOutput, target=`&-`); the bug was the
//! emulated-cat output path treating the mirror target as a path.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1594-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(dir.join("in"), "hello\n").expect("write input file");
    dir
}

fn run_c_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_c_stdin(dir: &std::path::Path, script: &str, stdin_bytes: &[u8]) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin_bytes)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue1594_cat_stdout_close_creates_no_amp_dash_file() {
    let dir = scratch("cat-close-out");
    let (stdout, stderr, _code) = run_c_in(&dir, "cat 1>&- < in; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr must report the closed fd, got: {stderr}"
    );
    assert!(
        !dir.join("&-").exists(),
        "no file named `&-` may be created"
    );
}

#[test]
fn issue1594_cat_stdin_close_reports_bad_fd() {
    let dir = scratch("cat-close-in");
    let (stdout, stderr) = run_c_stdin(&dir, "cat <&-; echo \"rc=$?\"", b"hello\n");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr must report the closed fd, got: {stderr}"
    );
    assert!(!dir.join("&-").exists());
}

#[test]
fn issue1594_builtin_stdout_close_fails_the_write() {
    let dir = scratch("printf-close");
    let (stdout, _stderr, _code) = run_c_in(&dir, "printf x >&-; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(!dir.join("&-").exists());
}
