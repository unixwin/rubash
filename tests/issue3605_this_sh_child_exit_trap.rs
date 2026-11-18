//! Issue rubash#3605 (class H, run-trap): an in-process `${THIS_SH} script`
//! child's end-of-script run_exit_trap latched the shared executor's
//! `exit_trap_ran`, so an EXIT trap the PARENT armed afterwards was silently
//! dropped (trap.tests tail: `trap 'echo exiting' 0; exit` printed nothing).
//!
//! GNU ownership: a `bash script` child is a separate process
//! (execute_cmd.c:6139 execute_disk_command forks and execs); run_exit_trap
//! (trap.c:1027) keys on the child's own sigmodes[EXIT_TRAP] — SIG_TRAPPED /
//! SIG_INPROGRESS are per-process and never leak into the parent.
//! execute_direct_shell_script (external_finish.rs) emulates that boundary
//! in-process and now parks/restores the parent's latch for the child run.
//!
//! Pin shape: a CLI `${THIS_SH} script` child is a REAL spawned process
//! wherever `real_shell_children_apply()` holds (external_finish.rs
//! :181-183 — owns_process_image && the binary's own opt-in), so a
//! Command::new(CARGO_BIN_EXE_rubash) probe never reaches the emulation
//! and is vacuous on both base and head. The discriminating surface is
//! the library Session: `enable_real_shell_children` runs only in
//! rubash's own main (builtins/exec.rs), so a Session executor keeps the
//! in-process emulation and the child's EXIT trap fires on the shared
//! latch exactly like the pre-fix path. The finish guard
//! (`Session::begin_finish` -> `Finish::run_exit_trap`) is the session's
//! exit funnel — the same run_exit_trap_action the CLI exit takes.
use rubash::api::{RunOptions, Session};
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn posix_path(path: &PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A Session rooted at `dir` with the child script and output paths
/// exported, so the probe text never embeds a platform path spelling.
fn session_in(dir: &PathBuf, child: &PathBuf) -> Session {
    Session::builder()
        .working_directory(dir.clone())
        .env("I3605_CHILD", posix_path(child))
        .env("I3605_DIR", posix_path(dir))
        .build()
        .expect("session builds")
}

/// GNU: the child's EXIT trap runs inside the child only; the parent's
/// re-armed EXIT trap still fires at the parent's real exit.
#[test]
fn issue3605_this_sh_child_exit_trap_does_not_latch_parent() {
    let dir = scratch("latch");
    let child = dir.join("child.sub");
    // The child arms an EXIT trap whose end-of-script run is what used
    // to leak the parent's exit_trap_ran latch.
    std::fs::write(&child, "trap ':' EXIT\n").expect("write child");
    let mut session = session_in(&dir, &child);
    session
        .run("\"$THIS_SH\" \"$I3605_CHILD\"", &RunOptions::new())
        .expect("child run");
    session
        .run(
            "trap 'echo exiting > \"$I3605_DIR/parent-exit.txt\"' 0",
            &RunOptions::new(),
        )
        .expect("arm parent trap");
    let mut finish = session.begin_finish(0);
    finish.run_exit_trap();
    drop(finish);
    drop(session);
    assert_eq!(
        std::fs::read_to_string(dir.join("parent-exit.txt"))
            .expect("parent EXIT trap wrote its file"),
        "exiting\n"
    );
}

/// Same latch, second shape: several in-process children each fire their
/// own EXIT trap, then the parent's still fires.
#[test]
fn issue3605_parent_exit_trap_after_multiple_children() {
    let dir = scratch("multi");
    let child = dir.join("child.sub");
    std::fs::write(&child, "trap ':' EXIT\n").expect("write child");
    let mut session = session_in(&dir, &child);
    for _ in 0..2 {
        session
            .run("\"$THIS_SH\" \"$I3605_CHILD\"", &RunOptions::new())
            .expect("child run");
    }
    session
        .run(
            "trap 'echo exiting > \"$I3605_DIR/parent-exit.txt\"' 0",
            &RunOptions::new(),
        )
        .expect("arm parent trap");
    session
        .run(
            "echo before-exit > \"$I3605_DIR/before.txt\"",
            &RunOptions::new(),
        )
        .expect("marker run");
    let mut finish = session.begin_finish(0);
    finish.run_exit_trap();
    drop(finish);
    drop(session);
    assert_eq!(
        std::fs::read_to_string(dir.join("parent-exit.txt"))
            .expect("parent EXIT trap wrote its file"),
        "exiting\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("before.txt")).expect("marker file"),
        "before-exit\n"
    );
}
