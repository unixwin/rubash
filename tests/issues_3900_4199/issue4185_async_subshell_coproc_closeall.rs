//! Issue rubash#4185 — a background (async) child must not
//! inherit the parent's coproc endpoints either.
//!
//! GNU runs `coproc_closeall` at EVERY fork site
//! (execute_cmd.c:1714 execute_in_subshell, :4229 the
//! null-command fork, :4575 the simple-command fork): whatever
//! the payload shape — `( ) &`, a bare builtin `&`, a brace
//! group `&` — the forked child is born with the coproc
//! descriptors closed, live or dead, so a redirect naming one
//! inside the child fails EBADF and only the child reports it.
//! rubash#4181 severed the in-place `( )` region; the async
//! face is separate machinery — the background command respawns
//! a `rubash -c` wrapper whose fd table is seeded from the
//! parent's fd>=3 delivery loop, and that loop delivered the
//! coproc endpoints as plain File bindings, so the async child
//! silently wrote into the live coproc (and died of SIGPIPE on
//! a dead one's readerless pipe, job status 141, where GNU
//! reports EBADF with status 1).
//!
//! The delivery loop is the child's fd-table snapshot: it now
//! runs the same severance as the in-place entries
//! (FdTable::close_coproc_endpoints_for_subshell) over a clone
//! of the live table and delivers only the arms the snapshot
//! still carries. Nothing is retired and no shared handle is
//! closed, so the parent's own endpoints — and its round trip
//! after the async child — are untouched, and plain File
//! endpoints still ride the delivery as before.
//!
//! Verified against the cloud GNU Bash 5.3.20 oracle via script
//! files (statement status, stdout/stderr bytes, parent state);
//! every shape below is byte-identical to the oracle, including
//! the parent-side coproc retirement at the async `wait`.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4185-{tag}-{}-{seq}", std::process::id()));
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

#[test]
fn issue4185_async_subshell_write_to_live_coproc_fails_ebadf() {
    let script = "coproc cat\n( echo hi >&\"${COPROC[1]}\" ) 2>child_err.txt &\nbg=$!\nwait $bg\necho \"bg_rc=$?\"\necho \"child_err=[$(cat child_err.txt)]\"\nexec {COPROC[1]}>&-\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\n";
    let (stdout, stderr, code, signal) = run_script("s2live", script);
    assert_eq!(
        stdout,
        "bg_rc=1\nchild_err=[probe.sh: line 2: \"${COPROC[1]}\": Bad file descriptor]\nread_rc=1 got=\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue4185_async_subshell_write_to_dead_coproc_fails_ebadf_not_sigpipe() {
    let script = "coproc true\nwhile IFS= read -r -u \"${COPROC[0]}\" _line; do :; done\necho \"after-read\"\n( echo hi >&\"${COPROC[1]}\" ) 2>child_err.txt &\nbg=$!\nwait $bg\necho \"bg_rc=$?\"\necho \"child_err=[$(cat child_err.txt)]\"\necho \"vars: r=${COPROC[0]:-unset} w=${COPROC[1]:-unset} pid=${COPROC_PID:-unset}\"\necho \"alive\"\n";
    let (stdout, stderr, code, signal) = run_script("s2dead", script);
    assert_eq!(
        stdout,
        "after-read\nbg_rc=1\nchild_err=[probe.sh: line 4: \"${COPROC[1]}\": Bad file descriptor]\nvars: r=unset w=unset pid=unset\nalive\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue4185_async_simple_write_to_live_coproc_fails_ebadf() {
    let script = "coproc cat\necho hi 2>child_err.txt >&\"${COPROC[1]}\" &\nbg=$!\nwait $bg\necho \"bg_rc=$?\"\necho \"child_err=[$(cat child_err.txt)]\"\nexec {COPROC[1]}>&-\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\n";
    let (stdout, stderr, code, _) = run_script("s2simple", script);
    assert_eq!(
        stdout,
        "bg_rc=1\nchild_err=[probe.sh: line 2: \"${COPROC[1]}\": Bad file descriptor]\nread_rc=1 got=\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4185_async_group_write_to_live_coproc_fails_ebadf() {
    let script = "coproc cat\n{ echo hi >&\"${COPROC[1]}\"; } 2>child_err.txt &\nbg=$!\nwait $bg\necho \"bg_rc=$?\"\necho \"child_err=[$(cat child_err.txt)]\"\nexec {COPROC[1]}>&-\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\n";
    let (stdout, stderr, code, _) = run_script("s2group", script);
    assert_eq!(
        stdout,
        "bg_rc=1\nchild_err=[probe.sh: line 2: \"${COPROC[1]}\": Bad file descriptor]\nread_rc=1 got=\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4185_async_subshell_read_from_live_coproc_fails() {
    let script = "coproc cat\necho seed >&\"${COPROC[1]}\"\n( read -r -u \"${COPROC[0]}\" x; echo \"in_async_read_rc=$? x=$x\" ) 2>child_err.txt &\nbg=$!\nwait $bg\necho \"bg_rc=$?\"\necho \"child_err=[$(cat child_err.txt)]\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("s2read", script);
    assert!(
        stdout.contains("in_async_read_rc=1 x=\n"),
        "unexpected stdout: {stdout}"
    );
    assert!(stdout.contains("bg_rc=0\n"), "unexpected stdout: {stdout}");
    // The diagnostic names the inherited fd number, which is an
    // allocation detail; the failure kind is the pinned face.
    assert!(
        stdout.contains("invalid file descriptor: Bad file descriptor"),
        "unexpected stdout: {stdout}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Guard: the parent's own coproc endpoints are untouched by the
/// async spawn — no false EOF after the child runs.
#[test]
fn issue4185_parent_endpoints_survive_async_subshell() {
    let script = "coproc cat\n( true ) &\nbg=$!\nwait $bg\necho \"bg_rc=$?\"\necho hi >&\"${COPROC[1]}\"\necho \"write_rc=$?\"\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("s2guard", script);
    assert_eq!(stdout, "bg_rc=0\nwrite_rc=0\nread_rc=0 got=hi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Guard: the severance is coproc-only — a plain File endpoint
/// still rides the fd delivery into the async child.
#[test]
fn issue4185_async_subshell_still_inherits_plain_file_fds() {
    let script = "exec 3>async_out.txt\n( echo hi >&3 ) &\nbg=$!\nwait $bg\necho \"bg_rc=$?\"\nexec 3>&-\necho \"file=[$(cat async_out.txt)]\"\n";
    let (stdout, stderr, code, _) = run_script("s2fdfwd", script);
    assert_eq!(stdout, "bg_rc=0\nfile=[hi]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
