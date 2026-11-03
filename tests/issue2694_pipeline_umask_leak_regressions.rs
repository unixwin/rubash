//! rubash#2694 — a pipeline stage's `umask` leaked back into the
//! parent shell: stages run in-process on a cloned executor, and the
//! process umask is process-global state the clone shares, so
//! `umask 077 | cat` changed the PARENT's mask.
//!
//! GNU forks every pipeline element (execute_cmd.c:2620
//! execute_pipeline); the element's umask dies with the child. The
//! fix brackets execute_pipeline_stage with the same process-umask
//! snapshot/restore the comsub and `( )` fork boundaries already
//! carry (InteriorSnapshot::process_umask /
//! fork_saved_process_umask): snapshot at stage entry, restore when
//! the stage returns.
//!
//! Every expectation below was cross-checked byte-for-byte against
//! GNU bash 5.3.20 (script-file probes).

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
    let dir = std::env::temp_dir().join(format!("issue2694_{}_{}", std::process::id(), seq));
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
fn builtin_stage_umask_does_not_leak_to_parent() {
    let run = run_script("umask 022\numask 077 | cat\necho \"after1=$(umask)\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("after1=0022"),
        "the stage's umask must die with the stage; stdout: {}",
        run.stdout
    );
}

#[test]
fn brace_group_stage_umask_does_not_leak_to_parent() {
    let run = run_script("umask 022\n{ umask 077; } | cat\necho \"after2=$(umask)\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("after2=0022"), "stdout: {}", run.stdout);
}

#[test]
fn final_stage_and_function_stage_umask_do_not_leak() {
    let run = run_script(
        "umask 022\necho x | { umask 077; }\necho \"brace_last=$(umask)\"\nf() { umask 077; }\nf | cat\necho \"func=$(umask)\"\numask 077 | cat | cat\necho \"three=$(umask)\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    for marker in ["brace_last=0022", "func=0022", "three=0022"] {
        assert!(
            run.stdout.contains(marker),
            "missing {marker}; stdout: {}",
            run.stdout
        );
    }
}

#[test]
fn no_preset_stage_umask_does_not_leak() {
    // The leak also fired with no earlier in-script umask: the mask
    // the harness process started with must survive the stage. Read
    // it back through a sub-shell-free second line.
    let run = run_script("before=$(umask)\numask 077 | cat\necho \"same=$([ \"$(umask)\" = \"$before\" ] && echo yes || echo no)\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("same=yes"), "stdout: {}", run.stdout);
}

#[test]
fn plain_subshell_umask_still_isolated() {
    // Control: the `( )` boundary already carried the snapshot; it
    // must keep working.
    let run = run_script("umask 022\n( umask 077 )\necho \"after3=$(umask)\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("after3=0022"), "stdout: {}", run.stdout);
}
