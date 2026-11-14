//! rubash#2722 — $BASH_SUBSHELL over-counted when a `( )` was a
//! pipeline stage: `( echo $BASH_SUBSHELL ) | cat` printed 2.
//!
//! GNU forks ONE child per pipeline element (execute_cmd.c:2620
//! execute_pipeline); when the element IS a subshell, that fork is
//! the subshell — the child runs execute_in_subshell's single level
//! increment (execute_cmd.c:1588) as its element dispatch. The stage
//! clone here already pays the element level in
//! command_substitution_executor, and the subshell node bumped a
//! second time in execute_subshell_command_with_redirects. The fix
//! is the pipeline twin of the background path's paren absorb
//! (background_paren_absorbable): execute_compound_pipeline_stage
//! hands the paid level back for a `( )` element, so the node's own
//! bump lands on the stage level and a nested `( )` still counts
//! from there.
//!
//! BASHPID rides the same depth counter (pid + depth - origin), so
//! the matrix pins its invariants in the same shapes: distinct from
//! $$ and from sibling subshells' fabrications, never exact values
//! (GNU's are real child pids).
//!
//! Every expectation below was cross-checked against GNU bash
//! 5.3.20 (script-file probes).

use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2722_{}_{}", std::process::id(), seq));
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
    }
}

#[test]
fn subshell_levels_across_stage_shapes() {
    let run = run_script(
        "( echo \"A=$BASH_SUBSHELL\" ) | cat\ntrue | ( ( echo \"B=$BASH_SUBSHELL\" ) )\n( echo \"C=$BASH_SUBSHELL\" ) & wait\necho \"D=$BASH_SUBSHELL\" | cat\n{ echo \"E=$BASH_SUBSHELL\"; } | cat\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    // D is a simple builtin stage: GNU expands the element's words
    // in the parent before the fork (execute_simple_command), so
    // $BASH_SUBSHELL there reads the parent's 0 — probe-verified.
    for marker in ["A=1", "B=2", "C=1", "D=0", "E=1"] {
        assert!(
            run.stdout.contains(marker),
            "missing {marker}; stdout: {}",
            run.stdout
        );
    }
}

#[test]
fn nested_paren_inside_a_stage_still_counts_up() {
    // The absorb is for the element's own paren only: a paren nested
    // in the stage's body adds its level on top of the stage level.
    let run = run_script("( ( echo \"inner=$BASH_SUBSHELL\" ) ) | cat\n( echo \"plain=$BASH_SUBSHELL\"; ( echo \"nested=$BASH_SUBSHELL\" ) ) | cat\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    for marker in ["inner=2", "plain=1", "nested=2"] {
        assert!(
            run.stdout.contains(marker),
            "missing {marker}; stdout: {}",
            run.stdout
        );
    }
}

#[test]
fn bashpid_invariants_in_the_same_shapes() {
    let run = run_script(
        "echo \"main pid=$$ sub=$BASH_SUBSHELL\"\n( echo \"stage2 sub=$BASH_SUBSHELL bashpid=$BASHPID\" ) | cat\n( echo \"bg sub=$BASH_SUBSHELL bashpid=$BASHPID\" ) & wait\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    // Line shapes: "main pid=<P> sub=0", "stage2 sub=1 bashpid=<B1>",
    // "bg sub=1 bashpid=<B2>".
    let mut main_pid = 0i64;
    let mut stage_pid = 0i64;
    let mut bg_pid = 0i64;
    for line in run.stdout.lines() {
        if let Some(rest) = line.strip_prefix("main pid=") {
            main_pid = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
        if line.starts_with("stage2") {
            assert!(line.contains("sub=1"), "line: {line}");
            if let Some(pos) = line.find("bashpid=") {
                stage_pid = line[pos + 8..]
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
        }
        if line.starts_with("bg") {
            assert!(line.contains("sub=1"), "line: {line}");
            if let Some(pos) = line.find("bashpid=") {
                bg_pid = line[pos + 8..]
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
        }
    }
    assert!(
        main_pid > 0 && stage_pid > 0 && bg_pid > 0,
        "stdout: {}",
        run.stdout
    );
    assert_ne!(
        stage_pid, main_pid,
        "a stage subshell's BASHPID must differ from $$"
    );
    assert_ne!(
        bg_pid, main_pid,
        "a background subshell's BASHPID must differ from $$"
    );
    assert_ne!(
        stage_pid, bg_pid,
        "sibling subshells must not collide on BASHPID (pre-fix both fabricated main+2)"
    );
}
