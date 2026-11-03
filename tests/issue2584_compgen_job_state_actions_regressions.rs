//! rubash#2584 — the `running` compgen action returned every job and
//! `stopped` returned nothing, regardless of state: the action
//! dispatch in `apply_completion_actions` mapped `"job" | "running"`
//! to the unfiltered name list and `"stopped"` to an empty arm, and
//! the job names reached it without their process state.
//!
//! GNU ownership: pcomplete.c:570-585 it_init_jobs / it_init_running /
//! it_init_stopped all build their candidate set from it_init_joblist
//! with a status filter (-1 = all jobs, 0 = running only, 1 = stopped
//! only); job candidates are the jobs' command first words (no `%`
//! prefix). The port carries `ProcessState` alongside each job name
//! from both job_names construction sites and filters in
//! `job_completion_candidates`.
//!
//! The script shape needs `set -m` plus a short beat after SIGSTOP:
//! that is when the shell's job table has learned the stop (the same
//! state-learning the GNU oracle needs — without monitor mode GNU's
//! script shell reports the stopped job as running too).

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (String, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash binary");
    let output = child.wait_with_output().expect("wait for rubash binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn compgen_job_actions_filter_by_state() {
    let script = "set -m\n\
                  sleep 30 &\n\
                  tail -f /dev/null &\n\
                  kill -STOP %1\n\
                  sleep 1\n\
                  echo \"=job=\"; compgen -A job\n\
                  echo \"=running=\"; compgen -A running\n\
                  echo \"=stopped=\"; compgen -A stopped\n\
                  kill %1 %2 2>/dev/null\n\
                  wait 2>/dev/null\n";
    let (stdout, stderr, _rc) = run_script(script);
    let section = |name: &str| -> Vec<&str> {
        let start = stdout.find(&format!("={name}=\n")).unwrap_or_else(|| {
            panic!("missing section {name} in stdout {stdout:?} (stderr {stderr:?})")
        }) + name.len()
            + 3;
        let rest = &stdout[start..];
        let end = rest.find('=').unwrap_or(rest.len());
        rest[..end].lines().collect()
    };
    assert_eq!(section("job"), vec!["sleep", "tail"], "all jobs listed");
    assert_eq!(
        section("running"),
        vec!["tail"],
        "only the running job is a `running` candidate"
    );
    assert_eq!(
        section("stopped"),
        vec!["sleep"],
        "only the stopped job is a `stopped` candidate"
    );
}

#[test]
fn compgen_stopped_with_no_stopped_jobs_is_empty() {
    let script = "set -m\n\
                  sleep 30 &\n\
                  compgen -A stopped\n\
                  echo \"rc=$?\"\n\
                  kill %1 2>/dev/null\n\
                  wait 2>/dev/null\n";
    let (stdout, _stderr, _rc) = run_script(script);
    assert_eq!(
        stdout, "rc=1\n",
        "no stopped job exists: no candidates, and compgen reports failure"
    );
}
