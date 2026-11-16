//! Issue rubash#3300 regressions: a fork-shaped respawn child orphaned
//! to a live unrelated parent (a subreaper) must still adopt its
//! spawner provenance — the fd/pipe proof carries the verdict where
//! the ppid no longer can.
//!
//! The defect: `respawn_provenance_validated` (rubash#2136) only
//! opened the orphan branch when the child was reparented to the
//! namespace init (ppid 1). A child whose parent shell exited while
//! it started, and which a subreaper then adopted, kept a ppid that
//! was neither its spawner nor 1 — so the claim failed, `$$` stayed
//! the child's own pid, `$BASH_SUBSHELL` stayed 0, and the line seed
//! fell back to the body (the #3300 diagnostic reported the body
//! line instead of the definition line).
//!
//! The fix: every fork-shaped spawn site mints a fresh anonymous pipe
//! holding a fresh 16-byte nonce, closes the write end before spawn,
//! and hands the child the read end at a spawner-chosen number >= 100
//! plus two per-spawn keys (`__RUBASH_RESPAWN_PROOF_FD` /
//! `__RUBASH_RESPAWN_PROOF`). The child's entry validates the pipe
//! exactly once (FIFO + O_NONBLOCK + 16 bytes == nonce + EOF) and
//! the orphan branch opens when the proof is valid and the stamped
//! spawner is gone.
//!
//! Shape of these pins: the test process sets PR_SET_CHILD_SUBREAPER
//! on itself, so every orphaned grandchild is reparented to THIS
//! process (ppid == test pid, != spawner, != 1) — the live-unrelated-
//! parent shape, not the init shape. Each cell asserts the
//! reparenting shape itself (the ppid value), not just the verdict:
//! the diagnostic cell asserts the line the orphaned child reports,
//! the PPID cells assert PPID == harness pid, SUB == 1, and PID ==
//! spawner pid. The parent exits immediately (no wait for the async
//! child), and the child sleeps 0.3s before observing, so the parent
//! is long dead at observation time and the reparenting shape (PPID)
//! is deterministic. Note the adoption verdict itself is validated
//! at child entry, so on an UNFIXED binary it depends on whether the
//! parent was still alive then (usually dead — 3–4 of 5 cells red
//! per base run); on a fixed binary both orders adopt, so head is
//! deterministically green. GNU ground truth: the forked child is the
//! subshell (variables.c `$$`, execute_cmd.c:1576 execute_in_subshell,
//! subst.c:6362 process_substitute) regardless of who adopted it
//! after its parent died.

// PR_SET_CHILD_SUBREAPER exists only on Linux/Android; every cell
// in this file needs the subreaper shape, so the whole pin is gated
// to those targets (macOS/*BSD have no prctl subreaper).
#![cfg(any(target_os = "linux", target_os = "android"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(20);

fn bin() -> String {
    env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3300-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// The subreaper flag is process-global: set it once, and every
/// orphaned grandchild of this test binary is reparented to this
/// process instead of init. All cells in this file run in one test
/// binary, so one Once covers them.
fn ensure_subreaper() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let rc = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
        assert_eq!(rc, 0, "prctl PR_SET_CHILD_SUBREAPER failed");
    });
}

/// Run `rubash case.sh` in a fresh scratch dir with the parent's
/// stderr captured to `err.txt`, wait for the parent to exit, and
/// return the parent pid, its exit code, and the dir. The async
/// child keeps running after the parent exits; callers poll the
/// output file with a deadline.
fn run_script_file(source: &str) -> (u32, Option<i32>, PathBuf) {
    ensure_subreaper();
    let dir = scratch("run");
    std::fs::write(dir.join("case.sh"), source).expect("write case.sh");
    let err = std::fs::File::create(dir.join("err.txt")).expect("create err.txt");
    let mut command = Command::new(bin());
    command
        .arg("case.sh")
        .current_dir(&dir)
        .env("HOME", dir.to_string_lossy().replace('\\', "/"))
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("__RUBASH_SHELL_PID")
        .env_remove("__RUBASH_COPROC_CHILD")
        .env_remove("__RUBASH_SPAWNER_PID")
        .env_remove("__RUBASH_SUBSHELL_DEPTH")
        .env_remove("__RUBASH_BG_DEPTH")
        .env_remove("__RUBASH_BG_BASHPID_BASE")
        .env_remove("__RUBASH_POSITIONAL_PARAMS")
        .env_remove("__RUBASH_RESPAWN_PROOF_FD")
        .env_remove("__RUBASH_RESPAWN_PROOF")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err));
    // Hermetic fd table: the local build slot holds its flock on fd 9
    // and children inherit it; an extra inherited fd flips the shell's
    // async spawn-path gate (direct spawn vs wrapper respawn), which
    // changes the pid topology these pins assert on. CI never runs
    // under the slot, but the pin must mean the same thing locally.
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            libc::close(9);
            Ok(())
        });
    }
    let mut child = command.spawn().expect("spawn rubash");
    let pid = child.id();
    let deadline = Instant::now() + WATCHDOG;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status.code().unwrap_or(-1)),
            None => {
                assert!(Instant::now() < deadline, "watchdog: parent {pid} hung");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    (pid, code, dir)
}

/// Poll a file until it is non-empty (or the deadline passes) and
/// return its contents. The async child writes after the parent has
/// exited, so the file starts empty.
fn wait_for_file_content(path: &Path, deadline: Instant) -> Option<String> {
    loop {
        match std::fs::read_to_string(path) {
            Ok(content) if !content.trim().is_empty() => return Some(content),
            _ => {
                if Instant::now() >= deadline {
                    return std::fs::read_to_string(path).ok();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn parse_ppid_line(line: &str) -> (u32, u32, u32) {
    let mut ppid = None;
    let mut sub = None;
    let mut pid = None;
    for cell in line.split_whitespace() {
        let (key, value) = cell.split_once('=').expect("KEY=VALUE cell");
        let value: u32 = value.parse().expect("numeric cell");
        match key {
            "PPID" => ppid = Some(value),
            "SUB" => sub = Some(value),
            "PID" => pid = Some(value),
            _ => panic!("unexpected cell {key}"),
        }
    }
    (
        ppid.expect("PPID cell"),
        sub.expect("SUB cell"),
        pid.expect("PID cell"),
    )
}

#[test]
fn subreaper_orphan_background_reports_definition_line() {
    // rubash#3300: the async child runs the body `y foooo` whose
    // definition sits on line 1 of case.sh; the orphaned child must
    // report the definition line, not the body line.
    let (_spawner, code, dir) = run_script_file("f() { >x\\\ny foooo & }\nf\n");
    assert_eq!(code, Some(0));
    let deadline = Instant::now() + WATCHDOG;
    let err = wait_for_file_content(&dir.join("err.txt"), deadline).expect("err.txt readable");
    assert_eq!(err, "case.sh: line 1: foooo: command not found\n");
}

#[test]
fn subreaper_orphan_async_child_adopts_provenance() {
    // The parent exits immediately; the child sleeps 0.3s, then
    // observes its parentage. It must have been reparented to THIS
    // process (the subreaper), not to init and not to the spawner —
    // and with the proof it must still adopt the spawner's identity:
    // SUB == 1 and PID == spawner pid.
    let (spawner, code, dir) = run_script_file(
        "( sleep 0.3; echo \"PPID=$PPID SUB=$BASH_SUBSHELL PID=$$\" > ppid.txt ) & exit\n",
    );
    assert_eq!(code, Some(0));
    let deadline = Instant::now() + WATCHDOG;
    let content =
        wait_for_file_content(&dir.join("ppid.txt"), deadline).expect("ppid.txt readable");
    let (ppid, sub, pid) = parse_ppid_line(content.trim());
    let harness = std::process::id();
    assert_eq!(ppid, harness, "orphan must be reparented to the subreaper");
    assert_ne!(ppid, spawner, "spawner must be dead at observation time");
    assert_ne!(ppid, 1, "this is the subreaper shape, not the init shape");
    assert_eq!(sub, 1, "orphan must adopt the subshell depth");
    assert_eq!(pid, spawner, "$$ must adopt the spawner pid");
}

#[test]
fn subreaper_orphan_coproc_child_adopts_provenance() {
    // Same shape through the coproc respawn: the parent exits right
    // after the coproc command, the child sleeps 0.3s, then observes.
    let (spawner, code, dir) = run_script_file(
        "coproc { sleep 0.3; echo \"PPID=$PPID SUB=$BASH_SUBSHELL PID=$$\" > ppid.txt; }\n",
    );
    assert_eq!(code, Some(0));
    let deadline = Instant::now() + WATCHDOG;
    let content =
        wait_for_file_content(&dir.join("ppid.txt"), deadline).expect("ppid.txt readable");
    let (ppid, sub, pid) = parse_ppid_line(content.trim());
    let harness = std::process::id();
    assert_eq!(ppid, harness, "orphan must be reparented to the subreaper");
    assert_ne!(ppid, spawner, "spawner must be dead at observation time");
    assert_ne!(ppid, 1, "this is the subreaper shape, not the init shape");
    assert_eq!(sub, 1, "orphan must adopt the subshell depth");
    assert_eq!(pid, spawner, "$$ must adopt the spawner pid");
}

#[test]
fn subreaper_orphan_input_procsub_child_adopts_provenance() {
    // Same shape through the `<(...)` producer respawn: the parent
    // exits right after the `:` builtin, the producer sleeps 0.3s,
    // then observes.
    let (spawner, code, dir) = run_script_file(
        ": <(sleep 0.3; echo \"PPID=$PPID SUB=$BASH_SUBSHELL PID=$$\" > ppid.txt)\n",
    );
    assert_eq!(code, Some(0));
    let deadline = Instant::now() + WATCHDOG;
    let content =
        wait_for_file_content(&dir.join("ppid.txt"), deadline).expect("ppid.txt readable");
    let (ppid, sub, pid) = parse_ppid_line(content.trim());
    let harness = std::process::id();
    assert_eq!(ppid, harness, "orphan must be reparented to the subreaper");
    assert_ne!(ppid, spawner, "spawner must be dead at observation time");
    assert_ne!(ppid, 1, "this is the subreaper shape, not the init shape");
    assert_eq!(sub, 1, "orphan must adopt the subshell depth");
    assert_eq!(pid, spawner, "$$ must adopt the spawner pid");
}

#[test]
fn subreaper_orphan_output_procsub_child_adopts_provenance() {
    // Same shape through the `>(...)` consumer respawn: the parent
    // exits right after the `echo`, the consumer sleeps 0.3s, then
    // observes.
    let (spawner, code, dir) = run_script_file(
        "echo hi > >(sleep 0.3; echo \"PPID=$PPID SUB=$BASH_SUBSHELL PID=$$\" > ppid.txt)\n",
    );
    assert_eq!(code, Some(0));
    let deadline = Instant::now() + WATCHDOG;
    let content =
        wait_for_file_content(&dir.join("ppid.txt"), deadline).expect("ppid.txt readable");
    let (ppid, sub, pid) = parse_ppid_line(content.trim());
    let harness = std::process::id();
    assert_eq!(ppid, harness, "orphan must be reparented to the subreaper");
    assert_ne!(ppid, spawner, "spawner must be dead at observation time");
    assert_ne!(ppid, 1, "this is the subreaper shape, not the init shape");
    assert_eq!(sub, 1, "orphan must adopt the subshell depth");
    assert_eq!(pid, spawner, "$$ must adopt the spawner pid");
}
