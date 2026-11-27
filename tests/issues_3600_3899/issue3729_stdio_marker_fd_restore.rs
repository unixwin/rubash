//! Regression pins for rubash#3729 — the stdio-marker reinstate in
//! `close_persistent_output_fd` (src/executor/trap_exec.rs) restores a
//! retired entry (same hazard as #3706).
//!
//! Mechanism (GNU anchors: redir.c:965/1082/1144 — the undo save is a
//! kernel dup `fcntl (fd, F_DUPFD, SHELL_FD_BASE)`; execute_cmd.c:854 —
//! compound groups run `do_redirections (..., RX_ACTIVE|RX_UNDOABLE)`).
//! After `close_output(fd)` the face took a bare `Rc` clone of the
//! fd's entry (`entries.get(&fd).cloned()`); for fd 1/2 it then
//! reinstated the stdio marker via `open_output(fd, Stdout/Stderr
//! marker)`, which retires the displaced descriptor
//! (`retire_read_arm`/`retire_write_arm`, rubash#3213); the restore
//! then re-inserted the retired entry, and the next spawn's pin pass
//! dup2'd whatever recycled that kernel number onto the fd. The fix
//! snapshots via a kernel duplicate (`FdTable::saved_entry` /
//! `FdEntry::dup_file_endpoints`, the #3723/#3739 pattern), never a
//! bare clone.
//!
//! REACHABILITY (measured, not assumed). The face needs a File-backed
//! read arm AND the output-procsub registration to coexist on fd 1/2
//! at close/flush time. They cannot: every input bind
//! (`set_fd_input_file`/`set_fd_input_bytes`/`set_fd_input_directory`)
//! calls `clear_output_fd_mirrors` (rubash#3655) and drops the
//! registration, and every output bind (`open_output`) retires any
//! read arm. A full trigger battery against GNU bash 5.3.20 and the
//! base binary (f56716b5) confirms it — `exec 2> >(...)` +
//! `exec 2<input.txt` + close/rebind/dup-to-closed/failed-rebind
//! (b2–b5, b1), group variants (g1–g4), and the `{var}`-close form
//! (h1–h2) all agree with GNU on the read result. So there is NO
//! shell-level fail-first cell for the face itself: these pins are
//! GUARDS on the face's code path (the stdio-marker reinstate and
//! the fd state after close/rebind), written so that a future change
//! which makes the face reachable (e.g. keeping the registration
//! across an input bind) cannot silently reintroduce the hazard —
//! the b5/b4 cells would then read the recycled file instead of
//! INPUT1. All cells pass on the base and on the fixed head.
//!
//! RESIDUAL (same area, different face, NOT fixed here). After an
//! input rebind the deferred procsub body never runs
//! (`exec 2> >(cat > body2.txt); exec 2<input.txt; exec 2>&-` —
//! GNU creates body2.txt, rubash does not; same in the group form
//! g2 and the dup form g4). That is the #3655 over-broad clearing
//! dropping the pending body along with the output ledger, and it
//! needs its own issue + design decision (ledger vs. body
//! registration split). It is deliberately NOT pinned here: the
//! cell would fail on the fixed head too.
//!
//! GNU ground truth (bash 5.3.20, measured): b2/b3 read rc=1 l=[];
//! b4/b5/b1 read rc=0 l=[INPUT1]; the canonical procsub body
//! delivers "hello-body" at close on both fd 1 and fd 2.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-bfd-3729-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = scratch(tag);
    std::fs::write(dir.join("driver.sh"), driver).expect("write driver");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let code = out.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, code)
}

/// Run with a hard deadline; a hang is a test failure, never a frozen
/// suite.
fn rubash_deadline(
    dir: &PathBuf,
    script: &str,
    deadline: Duration,
) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let t0 = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if t0.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash did not finish within {deadline:?} — HANG regression for script: {script}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_end(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        let _ = se.read_to_end(&mut err);
    }
    let status = child.wait().expect("wait");
    (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
        status.code(),
    )
}

/// The substitution body is a real child process; its file write can
/// land a moment after the shell exits. Poll briefly for the content.
fn wait_file(dir: &PathBuf, name: &str, expected: &str) -> String {
    let path = dir.join(name);
    let t0 = Instant::now();
    loop {
        let got = std::fs::read_to_string(&path).unwrap_or_default();
        if got == expected || t0.elapsed() > Duration::from_secs(5) {
            return got;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

const INPUT_SETUP: &str =
    "printf 'INPUT1\\nINPUT2\\n' > input.txt\nprintf 'RECYCLED\\n' > recycled.txt\n";

#[test]
fn issue3729_procsub_body_delivers_on_fd2_close() {
    // Canonical #1336 marker path on fd 2: the deferred body runs at
    // close and its output lands in the body file. Guards the
    // stdio-marker reinstate the face sits in.
    let dir = scratch("body-fd2");
    let (stdout, stderr, code) = rubash_deadline(
        &dir,
        "exec 2> >(cat > body2.txt); echo hello-body >&2; exec 2>&-; echo after-close",
        Duration::from_secs(30),
    );
    let log = wait_file(&dir, "body2.txt", "hello-body\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after-close\n");
    assert_eq!(stderr, "");
    assert_eq!(
        log, "hello-body\n",
        "fd 2 procsub body must deliver at close"
    );
}

#[test]
fn issue3729_procsub_body_delivers_on_fd1_close() {
    // Same marker path on fd 1.
    let dir = scratch("body-fd1");
    let (stdout, stderr, code) = rubash_deadline(
        &dir,
        "exec 1> >(cat > body1.txt); echo hello-body; exec 1>&-; echo after-close >&2",
        Duration::from_secs(30),
    );
    let log = wait_file(&dir, "body1.txt", "hello-body\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "after-close\n");
    assert_eq!(stdout, "");
    assert_eq!(
        log, "hello-body\n",
        "fd 1 procsub body must deliver at close"
    );
}

#[test]
fn issue3729_failed_rebind_leaves_fd2_read_arm_intact() {
    // b5: procsub + input + failed rebind. The failed rebind leaves
    // fd 2 unchanged on GNU (rc=0, reads INPUT1). If the marker
    // reinstate ever retired the read arm and restored it dead, the
    // read would fail or return RECYCLED after the recycle below.
    let driver = format!(
        "{INPUT_SETUP}exec 2> >(cat > body2.txt)\nexec 2<input.txt\nexec 2>/nonexistent-dir/out.txt\nexec 4<recycled.txt\nIFS= read -r l <&2; echo \"rc=$? l=[$l]\"\n"
    );
    let (stdout, _stderr, code) = run_driver("b5", &driver);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout, "rc=0 l=[INPUT1]\n",
        "fd 2 must still read the input file after a failed rebind"
    );
}

#[test]
fn issue3729_dup_to_closed_fd_leaves_fd2_read_arm_intact() {
    // b4: procsub + input + dup to a closed fd. GNU reads INPUT1.
    let driver = format!(
        "{INPUT_SETUP}exec 2> >(cat > body2.txt)\nexec 2<input.txt\nexec 2>&3\nexec 4<recycled.txt\nIFS= read -r l <&2; echo \"rc=$? l=[$l]\"\n"
    );
    let (stdout, _stderr, code) = run_driver("b4", &driver);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout, "rc=0 l=[INPUT1]\n",
        "fd 2 must still read the input file after dup to a closed fd"
    );
}

#[test]
fn issue3729_full_close_drops_fd2_read_arm() {
    // b2 (control): procsub + input + full close. GNU: fd 2 is
    // closed, the read fails.
    let driver = format!(
        "{INPUT_SETUP}exec 2> >(cat > body2.txt)\nexec 2<input.txt\nexec 2>&-\nexec 3<recycled.txt\nIFS= read -r l <&2; echo \"rc=$? l=[$l]\"\n"
    );
    let (stdout, stderr, code) = run_driver("b2", &driver);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 l=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3729_rebind_to_file_makes_fd2_write_only() {
    // b3 (control): procsub + input + rebind to a file. GNU: fd 2
    // is write-only after the rebind, the read fails.
    let driver = format!(
        "{INPUT_SETUP}exec 2> >(cat > body2.txt)\nexec 2<input.txt\nexec 2>out2.txt\nexec 4<recycled.txt\nIFS= read -r l <&2; echo \"rc=$? l=[$l]\"\n"
    );
    let (stdout, stderr, code) = run_driver("b3", &driver);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 l=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3729_failed_rebind_fd1_reads_input() {
    // b1: the fd-1 twin of b5. The verdict goes to stderr (fd 2 is
    // the only intact channel once fd 1 is the read-only input).
    let driver = format!(
        "{INPUT_SETUP}exec 1> >(cat > body1.txt)\nexec 1<input.txt\nexec 1>/nonexistent-dir/out.txt\nexec 4<recycled.txt\nIFS= read -r l <&1; echo \"rc=$? l=[$l]\" >&2\n"
    );
    let (stdout, stderr, code) = run_driver("b1", &driver);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("rc=0 l=[INPUT1]\n"),
        "fd 1 must still read the input file after a failed rebind; stderr was: {stderr:?}"
    );
}
