//! rubash#1590 — `exec N>&-` / `exec N<&M-` only cleared ONE direction
//! of the descriptor in the fd table: an fd opened for reading survived
//! an output-spelled close (and vice versa), so `read -u N`, builtin
//! dups `<&N`, and spawned children all kept using a descriptor GNU
//! considers closed.
//!
//! GNU redir.c: r_close_this is close(redirector) and the move case of
//! do_redirection_internal ends with close(redir_fd) — a descriptor is
//! one object; closing it kills both directions.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1590-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
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

#[test]
fn issue1590_exec_output_close_kills_read_endpoint() {
    let dir = scratch("close-read");
    let script = "printf 'data\\n' > d; exec 3<d; exec 3>&-; cat <&3; echo \"rc=$?\"";
    let (stdout, stderr, _code) = run_c_in(&dir, script);
    assert_eq!(stdout, "rc=1\n", "the closed fd must not yield data");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr must report the closed fd, got: {stderr}"
    );
}

#[test]
fn issue1590_exec_close_makes_read_u_report_invalid_fd() {
    let dir = scratch("close-read-u");
    let script = "printf 'line\\nrest\\n' > d2; exec 3<d2; exec 3>&-; read -u 3 x; echo \"rc=$?\"";
    let (stdout, stderr, _code) = run_c_in(&dir, script);
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("invalid file descriptor"),
        "read -u on the closed fd must say invalid file descriptor, got: {stderr}"
    );
}

#[test]
fn issue1590_exec_move_closes_source_wholesale() {
    let dir = scratch("move-source");
    let script = "printf 'data\\n' > d3; exec 3<d3; exec 4>&3-; \
                  cat <&3; echo \"rc1=$?\"; cat <&4; echo \"rc2=$?\"";
    let (stdout, _stderr, _code) = run_c_in(&dir, script);
    assert!(
        stdout.contains("rc1=1\n"),
        "moved-from fd must fail: {stdout}"
    );
    assert!(
        stdout.contains("rc2=0\n"),
        "move target must work: {stdout}"
    );
    assert_eq!(
        stdout.matches("data").count(),
        1,
        "data must be read exactly once, through fd 4: {stdout}"
    );
}

#[test]
fn issue1590_exec_close_is_visible_to_spawned_children() {
    let dir = scratch("close-child");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let script = format!(
        "printf 'data\\n' > d; exec 3<d; exec 3>&-; \"{exe}\" -c 'cat <&3'; echo \"rc=$?\""
    );
    let (stdout, stderr, _code) = run_c_in(&dir, &script);
    assert_eq!(stdout, "rc=1\n", "child must not read the closed fd");
    assert!(
        stderr.contains("Bad file descriptor"),
        "child diagnostic expected, got: {stderr}"
    );
}
