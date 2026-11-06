//! rubash#2753 (Stage-2 ④ Phase 1 scope) — the pilot family
//! (`wait`, `times`, `suspend`, `fg`, `bg`, `disown`, `jobs`)
//! did not recognise `--help` in option position: every one
//! reported `--: invalid option` (or, for `fg`/`bg`, the
//! job-control refusal) instead of the long help with status 2.
//! The expected texts below are byte-exact copies of GNU bash
//! 5.3.20's `--help` output (Stage-2 ④ Phase 1 E1 differential).
//!
//! Also pinned: `times -` (a bare `-` is an operand, not an
//! option word — GNU prints the two time lines, rc 0).

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
    let dir = std::env::temp_dir().join(format!("issue2753_{}_{}", std::process::id(), seq));
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
fn wait_help_prints_long_help() {
    let run = run_script("wait --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_WAIT, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn jobs_help_prints_long_help() {
    let run = run_script("jobs --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_JOBS, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn disown_help_prints_long_help() {
    let run = run_script("disown --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_DISOWN, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn suspend_help_prints_long_help() {
    let run = run_script("suspend --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_SUSPEND, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn fg_help_prints_long_help() {
    let run = run_script("fg --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_FG, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn bg_help_prints_long_help() {
    let run = run_script("bg --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_BG, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn times_help_prints_long_help() {
    let run = run_script("times --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_TIMES, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn wait_n_help_prints_long_help() {
    let run = run_script("wait -n --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_WAIT, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn jobs_l_help_prints_long_help() {
    let run = run_script("jobs -l --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_JOBS, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn suspend_f_help_prints_long_help() {
    let run = run_script("suspend -f --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_SUSPEND, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn times_bare_dash_is_an_operand() {
    let run = run_script("times -\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {}", run.stdout);
    for line in lines {
        assert!(is_time_line(line), "not a time line: {line}");
    }
    let _ = run.dir;
}

/// One `NmN.NNNs NmN.NNNs` line (GNU `times` shape; the values
/// are real CPU times on GNU and stub zeros on rubash — #2932 —
/// so only the shape is pinned here).
fn is_time_line(line: &str) -> bool {
    let mut parts = line.split(' ');
    matches!((parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), None) if is_time_field(a) && is_time_field(b))
}

fn is_time_field(field: &str) -> bool {
    let Some((min, rest)) = field.split_once('m') else {
        return false;
    };
    let Some(sec) = rest.strip_suffix('s') else {
        return false;
    };
    !min.is_empty()
        && min.bytes().all(|b| b.is_ascii_digit())
        && sec.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && sec.contains('.')
}

const EXPECTED_WAIT: &str = r#"wait: wait [-fn] [-p var] [id ...]
    Wait for job completion and return exit status.
    
    Waits for each process identified by an ID, which may be a process ID or a
    job specification, and reports its termination status.  If ID is not
    given, waits for all currently active child processes, and the return
    status is zero.  If ID is a job specification, waits for all processes
    in that job's pipeline.
    
    If the -n option is supplied, waits for a single job from the list of IDs,
    or, if no IDs are supplied, for the next job to complete and returns its
    exit status.
    
    If the -p option is supplied, the process or job identifier of the job
    for which the exit status is returned is assigned to the variable VAR
    named by the option argument. The variable will be unset initially, before
    any assignment. This is useful only when the -n option is supplied.
    
    If the -f option is supplied, and job control is enabled, waits for the
    specified ID to terminate, instead of waiting for it to change status.
    
    Exit Status:
    Returns the status of the last ID; fails if ID is invalid or an invalid
    option is given, or if -n is supplied and the shell has no unwaited-for
    children.
"#;

const EXPECTED_JOBS: &str = r#"jobs: jobs [-lnprs] [jobspec ...] or jobs -x command [args]
    Display status of jobs.
    
    Lists the active jobs.  JOBSPEC restricts output to that job.
    Without options, the status of all active jobs is displayed.
    
    Options:
      -l	lists process IDs in addition to the normal information
      -n	lists only processes that have changed status since the last
    		notification
      -p	lists process IDs only
      -r	restrict output to running jobs
      -s	restrict output to stopped jobs
    
    If -x is supplied, COMMAND is run after all job specifications that
    appear in ARGS have been replaced with the process ID of that job's
    process group leader.
    
    Exit Status:
    Returns success unless an invalid option is given or an error occurs.
    If -x is used, returns the exit status of COMMAND.
"#;

const EXPECTED_DISOWN: &str = r#"disown: disown [-h] [-ar] [jobspec ... | pid ...]
    Remove jobs from current shell.
    
    Removes each JOBSPEC argument from the table of active jobs.  Without
    any JOBSPECs, the shell uses its notion of the current job.
    
    Options:
      -a	remove all jobs if JOBSPEC is not supplied
      -h	mark each JOBSPEC so that SIGHUP is not sent to the job if the
    		shell receives a SIGHUP
      -r	remove only running jobs
    
    Exit Status:
    Returns success unless an invalid option or JOBSPEC is given.
"#;

const EXPECTED_SUSPEND: &str = r#"suspend: suspend [-f]
    Suspend shell execution.
    
    Suspend the execution of this shell until it receives a SIGCONT signal.
    Unless forced, login shells and shells without job control cannot be
    suspended.
    
    Options:
      -f	force the suspend, even if the shell is a login shell or job
    		control is not enabled.
    
    Exit Status:
    Returns success unless job control is not enabled or an error occurs.
"#;

const EXPECTED_FG: &str = r#"fg: fg [job_spec]
    Move job to the foreground.
    
    Place the job identified by JOB_SPEC in the foreground, making it the
    current job.  If JOB_SPEC is not present, the shell's notion of the
    current job is used.
    
    Exit Status:
    Status of command placed in foreground, or failure if an error occurs.
"#;

const EXPECTED_BG: &str = r#"bg: bg [job_spec ...]
    Move jobs to the background.
    
    Place the jobs identified by each JOB_SPEC in the background, as if they
    had been started with `&'.  If JOB_SPEC is not present, the shell's notion
    of the current job is used.
    
    Exit Status:
    Returns success unless job control is not enabled or an error occurs.
"#;

const EXPECTED_TIMES: &str = r#"times: times
    Display process times.
    
    Prints the accumulated user and system times for the shell and all of its
    child processes.
    
    Exit Status:
    Always succeeds.
"#;
