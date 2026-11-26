//! rubash#3633 — the coproc retirement path had two coupled leaks:
//!
//! 1. `exec >&-` inside a coproc child dropped the fd-table
//!    endpoint's transport handle but left the process's real fd1
//!    open as an orphaned c2p writer, so the parent's `read` blocked
//!    until the child exited instead of meeting instant EOF. GNU's
//!    r_close_this (redir.c) closes the redirector descriptor itself.
//! 2. A `read` on a pipe never ran the coproc retire step — GNU
//!    read.def:482 classifies input_is_pipe and the blocking read
//!    hands waitchld a synchronous reap window — so a dead coproc
//!    left `${COPROC[@]}` bound as the stale `63 60` after
//!    `read <&${COPROC[0]}` (coproc.tests xcase shape).
//!
//! Expectations cross-checked against the vendored GNU Bash 5.3.20
//! (`third_party/bash/bash`, script-file probes).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue3633_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

#[test]
fn dead_coproc_unbinds_after_pipe_read() {
    // The coproc.tests xcase shape verbatim: `xcase` fails at spawn
    // so the coproc child dies immediately; GNU's read on the pipe is
    // where the dead coproc's retirement lands, so ${COPROC[@]} is
    // empty at the very next command. The leak left it bound.
    let run = run_script(
        "coproc xcase -n -u\n\
         read <&${COPROC[0]}\n\
         echo \"COPROC:${COPROC[@]}\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "COPROC:\n", "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("xcase: command not found"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn coproc_child_exec_close_delivers_eof_while_alive() {
    // `exec >&-` inside the coproc child must close the real c2p
    // writer: GNU's read then meets EOF while the child still sleeps
    // (the registration stays bound — GNU keeps `63 60` and
    // COPROC_PID live), so the whole script finishes far inside the
    // child's 5s lifetime. With the orphaned fd1 the read blocked
    // until the child died of its own accord.
    //
    // Teardown notes (post-merge with master): the coproc child is a
    // `rubash -c` respawn wrapper whose `sleep 5` is a grandchild —
    // `kill $COPROC_PID` cannot reach it, and a TERM landing in the
    // child's early blocked-signal window leaves the wrapper waiting
    // out the sleep. The orphaned sleep would keep the parent's
    // stderr (inherited through the coproc) open and stall the test
    // harness's pipe drain, so the body detaches fd 2 and the kill
    // uses SIGKILL — cleanup determinism only; the fd-1 EOF contract
    // under test is unchanged (the kill still runs strictly after the
    // read returns, so it cannot mask an unfixed blocking read).
    let start = std::time::Instant::now();
    let run = run_script(
        "coproc { exec >&-; exec 2>/dev/null; sleep 5; }\n\
         read -r r <&${COPROC[0]}; echo \"eof:$?\"\n\
         echo \"bound:${COPROC[@]}\"\n\
         kill -9 $COPROC_PID 2>/dev/null; wait $COPROC_PID 2>/dev/null\n\
         echo done\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "eof:1\nbound:63 60\ndone\n",
        "stdout: {}",
        run.stdout
    );
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(4),
        "the read must not wait out the child's sleep: {:?}",
        start.elapsed()
    );
}
