//! Deep-audit lane A2 (executor runtime semantics) regression pins.
//!
//! Each test pins one confirmed differential finding against GNU
//! Bash 5.3.20 (the vendored oracle binary), reproduced twice during
//! the audit before being fixed. Expectations below are the oracle's
//! observed stdout/stderr/status for the same script text.
//!
//! UNVERIFIED-BY-CAPTAIN: run by the lane via the build slot; the
//! captain re-verifies centrally.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-audit-a2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn collect(child: std::process::Child) -> (String, String, Option<i32>) {
    let mut child = child;
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
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Run `script` as a `-c` string with cwd = `dir`.
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    collect(child)
}

/// A2-06: an empty PATH element means the current directory (GNU
/// findcmd.c get_next_path_element replaces an empty element with
/// "."). Leading, doubled, and trailing colons all search the cwd.
#[cfg(unix)]
#[test]
fn a2_06_empty_path_element_means_cwd() {
    use std::os::unix::fs::PermissionsExt;
    let dir = test_dir("path");
    let tool = dir.join("mytool");
    std::fs::write(&tool, "#!/bin/sh\necho tool-ran\n").expect("write mytool");
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    for path in [":/usr/bin:/bin", "/usr/bin::/bin", "/usr/bin:/bin:"] {
        let script = format!("PATH={path} mytool");
        let (stdout, _stderr, code) = run_script(&dir, &script);
        assert_eq!(stdout, "tool-ran\n", "PATH={path}");
        assert_eq!(code, Some(0), "PATH={path}");
    }
}

/// A2-11: a bounded, callback-free `mapfile -n` must consume exactly
/// the records it binds (plus any `-s` skips) and leave the rest of a
/// shared input stream for the next reader — GNU
/// builtins/mapfile.def read_array pulls one record at a time via
/// zgetline and stops after -n. The old slurp drained the whole fd.
#[test]
fn a2_11_mapfile_bounded_read_leaves_tail() {
    let dir = test_dir("mapfile");
    std::fs::write(dir.join("data.txt"), "l1\nl2\nl3\nl4\n").expect("write data");

    // Shared stdin redirected from a file.
    let (stdout, _stderr, code) = run_script(
        &dir,
        "exec 0<data.txt; mapfile -n 1 -t arr; echo \"got:${arr[0]}\"; cat",
    );
    assert_eq!(stdout, "got:l1\nl2\nl3\nl4\n");
    assert_eq!(code, Some(0));

    // NOT pinned: the pipeline brace-group face
    // (`printf ... | { mapfile -n 1 -t arr; cat; }`) still starves
    // cat — a stage's stdin there is an executor-staged snapshot
    // (command_input_scope), not a real handle, and the snapshot
    // take consumes the shared remainder wholesale. That face is
    // reported as A2-11's residual in the lane report; it belongs
    // to the stage-input staging layer, not to mapfile.

    // Bounded read on an explicit fd opened read-write.
    let (stdout, _stderr, code) =
        run_script(&dir, "exec 3<>data.txt; mapfile -n 2 -u 3 -t arr; cat <&3");
    assert_eq!(stdout, "l3\nl4\n");
    assert_eq!(code, Some(0));

    // A skip count is bounded too: skip 2, take 1, tail remains.
    let (stdout, _stderr, code) = run_script(
        &dir,
        "exec 0<data.txt; mapfile -s 2 -n 1 -t arr; echo \"got:${arr[0]}\"; cat",
    );
    assert_eq!(stdout, "got:l3\nl4\n");
    assert_eq!(code, Some(0));
}

/// A2-15: a hard read(2) error binds NOTHING (GNU builtins/read.def
/// read_builtin: the names are assigned from the obtained record;
/// on a read error there is no record) — a pre-existing value must
/// survive `read -u 3 v` on a directory fd. A plain EOF, by
/// contrast, binds the empty record in both shells.
#[cfg(unix)]
#[test]
fn a2_15_read_error_binds_nothing() {
    let dir = test_dir("readerr");
    let (stdout, stderr, _code) = run_script(
        &dir,
        "l=PRE; exec 3</tmp; read -u 3 l; echo \"dirfd: rc=$? l=[$l]\"",
    );
    assert_eq!(stdout, "dirfd: rc=1 l=[PRE]\n");
    assert!(
        stderr.contains("read error: Is a directory"),
        "stderr: {stderr}"
    );

    let (stdout, _stderr, _code) =
        run_script(&dir, "l=PRE; read l </dev/null; echo \"eof: rc=$? l=[$l]\"");
    assert_eq!(stdout, "eof: rc=1 l=[]\n");
}

/// A2-05: a subshell that sets its own EXIT trap must not consume
/// the parent's EXIT trap. GNU's forked child owns a private trap
/// table (execute_cmd.c execute_in_subshell); the in-place subshell
/// bracket therefore saves/restores the executor's exit_trap_ran
/// latch alongside the rest of the boundary state.
#[test]
fn a2_05_subshell_exit_trap_preserves_parent() {
    let dir = test_dir("exittrap");
    let (stdout, _stderr, code) = run_script(
        &dir,
        "trap 'echo PARENT' EXIT; (trap 'echo CHILD' EXIT; echo in); echo after",
    );
    assert_eq!(stdout, "in\nCHILD\nafter\nPARENT\n");
    assert_eq!(code, Some(0));

    // The reverse direction keeps working: a parent trap set AFTER
    // the subshell still fires (no latch poisoning either way).
    let (stdout, _stderr, code) = run_script(
        &dir,
        "(trap 'echo CHILD' EXIT; echo in); trap 'echo PARENT' EXIT; echo after",
    );
    assert_eq!(stdout, "in\nCHILD\nafter\nPARENT\n");
    assert_eq!(code, Some(0));
}
