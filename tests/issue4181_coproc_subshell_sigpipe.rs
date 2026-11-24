//! Issue rubash#4181 — a subshell must not inherit the parent's
//! coproc endpoints.
//!
//! GNU runs `coproc_closeall` in every forked child
//! (execute_cmd.c:1713-1715, execute_in_subshell; also :4229 and
//! :4575 for the other fork paths): the descriptors the parent
//! holds for a coproc are closed in the child, live or dead, so a
//! redirect naming one inside `( )` fails EBADF and only the
//! subshell dies. The parent's own endpoints are untouched, and
//! the parent's wait for the subshell child is itself a coproc
//! disposal point (jobs.c:3494, wait_for's notify_and_cleanup ->
//! coproc_reap): a coproc death recorded before the `( )` is
//! retired before the next command expands.
//!
//! Rubash executes `( )` in place. Before this fix the region kept
//! the parent's live fd table, so `( echo hi >&${COPROC[1]} )`
//! after read-EOF wrote into a readerless pipe and SIGPIPE killed
//! the whole shell (141, no diagnostic); with a live coproc the
//! subshell write wrongly succeeded. The region's fd table now
//! drops the coproc arms at the boundary (the parent's saved_table
//! snapshot restores them at the close), and the region's close
//! records the emulated foreground wait GNU's parent performs.
//!
//! The two no-boundary faces are GNU-identical and must not move:
//! an immediate direct write after read-EOF races ahead of GNU's
//! death notice and dies 141 on both shells, and a write after an
//! external-command boundary already met the retired fd (EBADF).
//! Command substitution is not a coproc_closeall path in GNU — a
//! comsub write to a live coproc succeeds on both shells.
//!
//! Verified against the cloud GNU Bash 5.3.20 oracle via script
//! files (statement status, stdout/stderr bytes, parent state).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4181-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` as probe.sh in a fresh tempdir; return
/// (stdout, stderr, exit code, terminating signal).
fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>, Option<i32>) {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code();
    let signal = output.status.signal();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, code, signal)
}

const READ_TO_EOF: &str =
    "coproc true\nwhile IFS= read -r -u \"${COPROC[0]}\" _line; do :; done\necho \"after-read\"\n";

#[test]
fn issue4181_subshell_write_to_dead_coproc_fails_ebadf_shell_survives() {
    let script = format!(
        "{READ_TO_EOF}( echo hi >&\"${{COPROC[1]}}\" )\necho \"subshell_rc=$?\"\necho \"alive\"\n"
    );
    let (stdout, stderr, code, signal) = run_script("p2", &script);
    assert_eq!(stdout, "after-read\nsubshell_rc=1\nalive\n");
    assert_eq!(
        stderr,
        "probe.sh: line 4: \"${COPROC[1]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue4181_subshell_wait_retires_dead_coproc_vars() {
    let script = format!(
        "{READ_TO_EOF}( echo hi >&\"${{COPROC[1]}}\" )\necho \"subshell_rc=$?\"\necho \"vars: r=${{COPROC[0]:-unset}} w=${{COPROC[1]:-unset}} pid=${{COPROC_PID:-unset}}\"\necho \"alive\"\n"
    );
    let (stdout, stderr, code, _) = run_script("q2", &script);
    assert_eq!(
        stdout,
        "after-read\nsubshell_rc=1\nvars: r=unset w=unset pid=unset\nalive\n"
    );
    assert_eq!(
        stderr,
        "probe.sh: line 4: \"${COPROC[1]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4181_subshell_write_to_live_coproc_fails_ebadf() {
    let script = "coproc cat\n( echo hi >&\"${COPROC[1]}\" )\necho \"sub_rc=$?\"\nexec {COPROC[1]}>&-\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\n";
    let (stdout, stderr, code, _) = run_script("q1b", script);
    assert_eq!(stdout, "sub_rc=1\nread_rc=1 got=\n");
    assert_eq!(
        stderr,
        "probe.sh: line 2: \"${COPROC[1]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4181_subshell_read_from_live_coproc_fails() {
    let script = "coproc cat\necho seed >&\"${COPROC[1]}\"\n( read -r -u \"${COPROC[0]}\" x; echo \"in_sub_read_rc=$? x=$x\" )\necho \"outer_rc=$?\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("n2", script);
    assert_eq!(stdout, "in_sub_read_rc=1 x=\nouter_rc=0\n");
    // The diagnostic names the inherited fd number, which is an
    // allocation detail; the failure kind is the pinned face.
    assert!(
        stderr.contains("invalid file descriptor: Bad file descriptor"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4181_nested_subshell_write_to_live_coproc_fails_ebadf() {
    let script = "coproc cat\n( ( echo hi >&\"${COPROC[1]}\" ); echo \"inner_rc=$?\" )\necho \"outer_rc=$?\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("n1", script);
    assert_eq!(stdout, "inner_rc=1\nouter_rc=0\n");
    assert_eq!(
        stderr,
        "probe.sh: line 2: \"${COPROC[1]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4181_subshell_own_redirect_from_coproc_fails_ebadf() {
    let script = "coproc cat\necho seed >&\"${COPROC[1]}\"\n( read -r line; echo \"sub_got=$line\" ) <&\"${COPROC[0]}\"\necho \"sub_rc=$?\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("s3", script);
    assert_eq!(stdout, "sub_rc=1\n");
    assert_eq!(
        stderr,
        "probe.sh: line 3: \"${COPROC[0]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4181_pipeline_stage_subshell_write_to_live_coproc_fails_ebadf() {
    let script = "coproc cat\n( echo hi >&\"${COPROC[1]}\" ) | wc -c\necho \"pipe_rc=$?\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("s1", script);
    assert_eq!(stdout, "0\npipe_rc=0\n");
    assert_eq!(
        stderr,
        "probe.sh: line 2: \"${COPROC[1]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4181_parent_endpoints_survive_subshell() {
    let script = "coproc cat\n( true )\necho hi >&\"${COPROC[1]}\"\necho \"write_rc=$?\"\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("n3", script);
    assert_eq!(stdout, "write_rc=0\nread_rc=0 got=hi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Invariance guard (GNU-identical before and after): with no
/// intervening boundary, the direct write races ahead of the death
/// notice and SIGPIPE kills the shell on both implementations.
#[test]
fn issue4181_direct_write_after_read_eof_still_dies_sigpipe() {
    let script = format!(
        "{READ_TO_EOF}echo hi >&\"${{COPROC[1]}}\"\necho \"write_rc=$?\"\necho \"alive\"\n"
    );
    let (stdout, stderr, code, signal) = run_script("p1", &script);
    assert_eq!(stdout, "after-read\n");
    assert_eq!(stderr, "");
    assert_eq!(code, None);
    assert_eq!(signal, Some(13));
}

/// Invariance guard: the external-command boundary already retired
/// the fd before this fix; its shape must not move.
#[test]
fn issue4181_external_boundary_write_after_read_eof_still_ebadf() {
    let script = format!("{READ_TO_EOF}/bin/true\necho hi >&\"${{COPROC[1]}}\"\necho \"write_rc=$?\"\necho \"alive\"\n");
    let (stdout, stderr, code, _) = run_script("p3", &script);
    assert_eq!(stdout, "after-read\nwrite_rc=1\nalive\n");
    assert_eq!(
        stderr,
        "probe.sh: line 5: \"${COPROC[1]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}

/// Invariance guard: command substitution is not a coproc_closeall
/// path in GNU — the comsub child keeps the endpoints.
#[test]
fn issue4181_comsub_write_to_live_coproc_still_works() {
    let script = "coproc cat\nx=$(echo hi >&\"${COPROC[1]}\"; echo \"in_comsub_rc=$?\")\necho \"comsub_out=$x\"\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("q3", script);
    assert_eq!(stdout, "comsub_out=in_comsub_rc=0\nread_rc=0 got=hi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Invariance guard: the plain parent-side round trip is untouched.
#[test]
fn issue4181_direct_write_to_live_coproc_still_works() {
    let script = "coproc cat\necho hi >&\"${COPROC[1]}\"\necho \"write_rc=$?\"\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("q4", script);
    assert_eq!(stdout, "write_rc=0\nread_rc=0 got=hi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
