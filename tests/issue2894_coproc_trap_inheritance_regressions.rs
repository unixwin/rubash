//! rubash#2894 — a coproc body did not see the parent shell's
//! traps at all: `trap -p` inside the body printed nothing. GNU
//! runs the body through execute_in_subshell with SUBSHELL_COPROC
//! (execute_cmd.c:2477, :2546): the subshell resets the trap
//! HANDLERS (reset_signal_handlers) but the trap strings remain
//! listed by `trap -p` — the body sees the parent's trap table in
//! its disarmed, listed form. Rubash's respawn-child environment
//! now carries the reset trap table (the same
//! reset_for_subshell_child product the async-subshell path uses),
//! not just SIG_IGN dispositions.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes in the r75 hunt).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2894_{}_{}", std::process::id(), seq));
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
fn coproc_body_lists_parent_traps_in_reset_form() {
    // The body prints the inherited (reset) trap table, then blocks
    // on its stdin; closing the parent's write end ends it with the
    // read's EOF status, so no kill/wait timing is involved.
    let run = run_script(
        "trap 'echo parent-exit' EXIT\n\
         trap 'echo parent-usr1' USR1\n\
         coproc C { trap -p EXIT; trap -p USR1; echo ready; read x; }\n\
         read -t 2 l <&${C[0]}; echo \"first:$l\"\n\
         read -t 2 l <&${C[0]}; echo \"second:$l\"\n\
         read -t 2 l <&${C[0]}; echo \"third:$l\"\n\
         exec {C[1]}>&-\n\
         wait ${C_PID}; echo \"w:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "first:trap -- 'echo parent-exit' EXIT\n\
         second:trap -- 'echo parent-usr1' SIGUSR1\n\
         third:ready\n\
         w:1\n\
         parent-exit\n",
        "stdout: {}",
        run.stdout
    );
    // The child's EXIT trap is listed but RESET: "parent-exit" may
    // appear only in the listing line and at the parent's own exit,
    // never as a firing from inside the child.
    assert_eq!(
        run.stdout.matches("parent-exit").count(),
        2,
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}
