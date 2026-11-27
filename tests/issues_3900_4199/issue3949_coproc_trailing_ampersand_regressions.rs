//! rubash#3949 — `coproc cmd &` (trailing ampersand) does not establish
//! COPROC/COPROC_PID. GNU's `coproc` is already asynchronous, so a
//! trailing `&` is a redundant terminator: execute_connection '&'
//! (execute_cmd.c:2822-2845) runs the first command with
//! asynchronous=1, and execute_command_internal dispatches cm_coproc
//! BEFORE any async/subshell fork (execute_cmd.c:666-668), so
//! execute_coproc runs in the parent, binds COPROC/COPROC_PID there
//! (coproc_setvars, execute_cmd.c:2566) and $! equals $COPROC_PID
//! (make_child FORK_ASYNC). rubash folded the `&` into a
//! background_command wrapper (parse_loop.rs fold_background_commands),
//! so the whole coproc setup ran in the respawned background child
//! and the parent never saw COPROC.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (the issue's
//! regression seed verbatim, plus the `time coproc &` shape).

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
    let dir = std::env::temp_dir().join(format!("issue3949_{}_{}", std::process::id(), seq));
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

fn assert_clean(run: &Run, expected_stdout: &str) {
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, expected_stdout, "stdout: {}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    let _ = &run.dir;
}

#[test]
fn amp_coproc_pid_is_set() {
    // Seed b-amp-pid-set: no $() here — a command substitution would
    // itself be a reap boundary.
    let run = run_script(
        "coproc cat & if [ -n \"${COPROC_PID:-}\" ]; then echo pid=set; else echo pid=unset; fi; exec {COPROC[1]}>&- 2>/dev/null; kill $! 2>/dev/null\n",
    );
    assert_clean(&run, "pid=set\n");
}

#[test]
fn amp_coproc_fds_are_set() {
    // Seed b-amp-fds-set.
    let run = run_script(
        "coproc cat & if [ -n \"${COPROC[0]:-}\" ]; then echo -n f0=set; else echo -n f0=unset; fi; if [ -n \"${COPROC[1]:-}\" ]; then echo \" f1=set\"; else echo \" f1=unset\"; fi; exec {COPROC[1]}>&- 2>/dev/null; kill $! 2>/dev/null\n",
    );
    assert_clean(&run, "f0=set f1=set\n");
}

#[test]
fn amp_coproc_roundtrip() {
    // Seed b-amp-roundtrip.
    let run = run_script(
        "coproc cat & echo hi >&${COPROC[1]}; read -u ${COPROC[0]} x; echo \"x=[$x]\"; exec {COPROC[1]}>&-\n",
    );
    assert_clean(&run, "x=[hi]\n");
}

#[test]
fn amp_coproc_bang_equals_coproc_pid() {
    // Seed b-amp-bang: $! equals $COPROC_PID.
    let run = run_script(
        "coproc cat & if [ \"$!\" = \"${COPROC_PID:-}\" ]; then echo same=yes; else echo same=no; fi; exec {COPROC[1]}>&- 2>/dev/null; kill $! 2>/dev/null\n",
    );
    assert_clean(&run, "same=yes\n");
}

#[test]
fn amp_coproc_wait_reports_body_status() {
    // Seed b-amp-wait: `coproc { exit 3; } &` then wait reports 3.
    let run = run_script("coproc { exit 3; } & wait $COPROC_PID; echo \"wait=$?\"\n");
    assert_clean(&run, "wait=3\n");
}

#[test]
fn amp_coproc_delayed_output_is_readable() {
    // Seed b-amp-delayed.
    let run =
        run_script("coproc { sleep 0.1; echo done; } & read -u ${COPROC[0]} x; echo \"x=[$x]\"\n");
    assert_clean(&run, "x=[done]\n");
}

#[test]
fn amp_coproc_second_replaces_first() {
    // Seed b-amp-twice: the first coproc is explicitly reaped before
    // the second starts, so no SIGCHLD races the second setup.
    let run = run_script(
        "coproc cat & echo one >&${COPROC[1]}; read -u ${COPROC[0]} a; exec {COPROC[1]}>&-; wait $COPROC_PID 2>/dev/null; coproc cat & echo two >&${COPROC[1]}; read -u ${COPROC[0]} b; echo \"a=[$a] b=[$b]\"; exec {COPROC[1]}>&-\n",
    );
    assert_clean(&run, "a=[one] b=[two]\n");
}

#[test]
fn amp_coproc_read_past_eof_is_clean() {
    // Seed b-amp-eof: rc 1, no diagnostic.
    let run = run_script(
        "coproc cat & exec {COPROC[1]}>&-; read -u ${COPROC[0]} x; echo \"rc=$? x=[$x]\"\n",
    );
    assert_clean(&run, "rc=1 x=[]\n");
}

#[test]
fn amp_named_coproc_with_braces() {
    // Seed b-named-amp: named brace form with trailing &.
    let run = run_script(
        "coproc NM { cat; } & if [ -n \"${NM_PID:-}\" ]; then echo pid=set; else echo pid=unset; fi; echo hi >&${NM[1]}; read -u ${NM[0]} x; echo \"x=[$x]\"\n",
    );
    assert_clean(&run, "pid=set\nx=[hi]\n");
}

#[test]
fn amp_time_coproc_pid_is_set() {
    // `time coproc cat &`: GNU (execute_cmd.c:666-668 cm_coproc
    // dispatch, reached through the time frame) still binds COPROC
    // in the parent; verified on the 5.3.20 oracle.
    let run = run_script(
        "time coproc cat & if [ -n \"${COPROC_PID:-}\" ]; then echo pid=set; else echo pid=unset; fi; echo hi >&${COPROC[1]}; read -u ${COPROC[0]} x; echo \"x=[$x]\"; exec {COPROC[1]}>&- 2>/dev/null; kill $! 2>/dev/null\n",
    );
    assert_clean(&run, "pid=set\nx=[hi]\n");
}

#[test]
fn plain_coproc_roundtrip_control() {
    // Seed b-noamp-ctl: control, passes before and after the fix.
    let run = run_script(
        "coproc cat; echo hi >&${COPROC[1]}; read -u ${COPROC[0]} x; echo \"x=[$x]\"; exec {COPROC[1]}>&-\n",
    );
    assert_clean(&run, "x=[hi]\n");
}

#[test]
fn plain_named_coproc_control() {
    // Seed b-named-noamp-ctl: control, passes before and after the fix.
    let run = run_script(
        "coproc NM { cat; }; echo hi >&${NM[1]}; read -u ${NM[0]} x; echo \"x=[$x]\"; exec {NM[1]}>&-\n",
    );
    assert_clean(&run, "x=[hi]\n");
}
