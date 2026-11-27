//! Issue unixwin/rubash#607 (loop-machinery slice): the arith-for loop
//! re-walks its body by pointer and gates the per-evaluation DEBUG consult
//! behind a cheap presence probe instead of paying the reader-level wrapper
//! and unconditional trap-table fires per iteration.
//!
//! GNU contract pinned here (execute_cmd.c:3219 execute_arith_for_command,
//! :3163 eval_arith_for_expr, :3236 line restore, :3267 REAP):
//! - the DEBUG trap fires for init once and test/update once per
//!   iteration, all with $LINENO at the for line — including the trailing
//!   update fire after the final body;
//! - a trap installed by the body starts firing at the very next
//!   evaluation (the update), and a removed trap stops immediately — the
//!   probe is per-evaluation, never hoisted;
//! - `set -x` traces each header expression with the for line;
//! - break/continue (including `break 2`) and `wait` over background jobs
//!   behave exactly as before (loop_depth bookkeeping, reap preserved).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn arith_for_debug_fires_init_test_update_at_for_line() {
    let (stdout, _stderr, code) =
        rubash("trap 'echo \"L$LINENO-i$i\"' DEBUG\nfor ((i=0;i<2;i++)); do\necho body\ndone");
    assert_eq!(
        stdout,
        "L2-i\nL2-i0\nL3-i0\nbody\nL2-i0\nL2-i1\nL3-i1\nbody\nL2-i1\nL2-i2\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn arith_for_debug_midloop_install_fires_next_evaluation() {
    let (stdout, _stderr, _code) = rubash(
        "for ((i=0;i<3;i++)); do if ((i==1)); then trap 'echo FIRED' DEBUG; fi; echo \"i=$i\"; done",
    );
    assert_eq!(
        stdout,
        "i=0\nFIRED\ni=1\nFIRED\nFIRED\nFIRED\nFIRED\ni=2\nFIRED\nFIRED\n"
    );
}

#[test]
fn arith_for_debug_midloop_remove_stops_immediately() {
    let (stdout, _stderr, _code) = rubash(
        "trap 'echo FIRED' DEBUG; for ((i=0;i<3;i++)); do if ((i==1)); then trap - DEBUG; fi; echo \"i=$i\"; done",
    );
    assert_eq!(
        stdout,
        "FIRED\nFIRED\nFIRED\nFIRED\ni=0\nFIRED\nFIRED\nFIRED\nFIRED\ni=1\ni=2\n"
    );
}

#[test]
fn arith_for_xtrace_reports_for_line() {
    let (_stdout, stderr, _code) =
        rubash("PS4='$LINENO> '\nset -x\nfor ((i=0;i<2;i++)); do\n:\ndone");
    assert_eq!(
        stderr,
        "3> (( i=0 ))\n3> (( i<2 ))\n4> :\n3> (( i++ ))\n3> (( i<2 ))\n4> :\n3> (( i++ ))\n3> (( i<2 ))\n"
    );
}

#[test]
fn arith_for_break_continue_nesting_preserved() {
    let (stdout, _stderr, code) = rubash(
        "s=0; for ((i=0;i<5;i++)); do for ((j=0;j<5;j++)); do ((j==2)) && break 2; s=$((s+1)); done; done; echo \"s=$s\"; \
         for ((i=0;;i++)); do ((i>=3)) && break; done; echo \"i=$i\"; \
         for ((i=0;i<3;i++)); do :; done; echo \"st=$?\"",
    );
    assert_eq!(stdout, "s=2\ni=3\nst=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn arith_for_reap_preserves_background_wait() {
    let (stdout, _stderr, code) = rubash(
        "sleep 0.05 & pid=$!; for ((i=0;i<500;i++)); do :; done; wait $pid; echo \"w=$?\"; jobs",
    );
    assert_eq!(stdout, "w=0\n");
    assert_eq!(code, Some(0));
}
