//! rubash#2895 — a coproc in the last pipeline position was never
//! set up: COPROC stayed unset and reading it reported "ambiguous
//! redirect". GNU's execute_pipeline (execute_cmd.c:2620) sends
//! every element through execute_command_internal, whose cm_coproc
//! case (execute_cmd.c:667-669) runs execute_coproc in the parent
//! shell — the rightmost element's registration and COPROC binds
//! land in the current shell (elements in other positions fork,
//! and their registrations die with them, exactly as in GNU).
//! The sequential stage loop now runs a coproc tail on the current
//! executor, and the streaming-sandwich fast path declines a
//! coproc shell stage (its in-process stage runner models a stream
//! transformer, not a parent-state setup).
//!
//! Expectations cross-checked against GNU bash 5.3.20 (the issue's
//! regression seed verbatim, plus external-head variants).

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
    let dir = std::env::temp_dir().join(format!("issue2895_{}_{}", std::process::id(), seq));
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

fn assert_seed_shape(run: &Run) {
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "set:yes\nr:alive rc:0\nw:0\n",
        "stdout: {}",
        run.stdout
    );
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
}

#[test]
fn coproc_pipeline_tail_is_set_up_builtin_head() {
    // The issue's regression seed, verbatim.
    let run = run_script(
        "printf 'piped\\n' | coproc { sleep 0.1; echo alive; }; echo \"set:$([ -n \"${COPROC[0]}\" ] && echo yes || echo no)\"; read -t 0.5 -r r <&${COPROC[0]}; echo \"r:$r rc:$?\"; wait $COPROC_PID; echo \"w:$?\"\n",
    );
    assert_seed_shape(&run);
    let _ = &run.dir;
}

#[test]
fn coproc_pipeline_tail_is_set_up_external_head() {
    let run = run_script(
        "/bin/printf 'piped\\n' | coproc { sleep 0.1; echo alive; }; echo \"set:$([ -n \"${COPROC[0]}\" ] && echo yes || echo no)\"; read -t 0.5 -r r <&${COPROC[0]}; echo \"r:$r rc:$?\"; wait $COPROC_PID; echo \"w:$?\"\n",
    );
    assert_seed_shape(&run);
}

#[test]
fn coproc_pipeline_tail_is_set_up_external_chain() {
    let run = run_script(
        "/bin/printf 'piped\\n' | /bin/cat | coproc { sleep 0.1; echo alive; }; echo \"set:$([ -n \"${COPROC[0]}\" ] && echo yes || echo no)\"; read -t 0.5 -r r <&${COPROC[0]}; echo \"r:$r rc:$?\"; wait $COPROC_PID; echo \"w:$?\"\n",
    );
    assert_seed_shape(&run);
}
