//! rubash#2579 — `wait -f` was parsed by every option scanner and then
//! dropped (`'f' => {}`), so a wait on a stopped job returned
//! 128+stopsig immediately even with -f.
//!
//! GNU ownership: builtins/wait.def:134-136 maps `-f` to JWAIT_FORCE;
//! jobs.c wait_for_job (3423) and wait_for_single_pid (2764) then loop
//! until the job is JDEAD / the child is no longer alive instead of
//! returning at the state change. Expectations probed against the GNU
//! bash 5.3.20 oracle with `set -m` (job control on — without it GNU's
//! plain wait also blocks on a stopped child):
//!   plain `wait $pid` on a SIGSTOPped job -> 147 (128+19), at once
//!   `wait -f $pid`, job CONT'd then TERM'd  -> 143 (128+15)
//!
//! The machinery limit, honestly: rubash polls std Child handles (no
//! WUNTRACED waitpid in the background path, #2398), so -f is honored
//! by not short-circuiting on the recorded Stopped state and polling
//! the handle until real termination — exactly the GNU outcome for
//! every state sequence the job table can observe.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_timeout(script: &str, timeout: Duration) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2579-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status.code()),
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_string(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        let _ = se.read_to_string(&mut err);
    }
    let _ = std::fs::remove_dir_all(&dir);
    (out, err, status)
}

#[test]
fn issue_2579_plain_wait_on_stopped_job_returns_stop_status() {
    let (stdout, stderr, code) = run_script_timeout(
        "set -m\nsleep 30 & pid=$!\nkill -STOP $pid\nsleep 0.2\nwait $pid; echo \"plain=$?\"\nkill -KILL $pid\nwait $pid 2>/dev/null; true\n",
        Duration::from_secs(20),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "plain=147\n", "stderr: {stderr}");
}

#[test]
fn issue_2579_wait_force_waits_for_termination() {
    let (stdout, stderr, code) = run_script_timeout(
        "set -m\nsleep 30 & pid=$!\nkill -STOP $pid\nsleep 0.2\n{ sleep 0.3; kill -CONT $pid; sleep 0.3; kill $pid; } &\nwait -f $pid; echo \"force=$?\"\n",
        Duration::from_secs(20),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "force=143\n", "stderr: {stderr}");
}

#[test]
fn issue_2579_wait_force_on_exiting_job_returns_its_status() {
    let (stdout, stderr, code) = run_script_timeout(
        "(exit 7) & pid=$!\nwait -f $pid; echo \"force=$?\"\n",
        Duration::from_secs(20),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "force=7\n", "stderr: {stderr}");
}

#[test]
fn issue_2579_bare_wait_force_waits_for_all_termination() {
    let (stdout, stderr, code) = run_script_timeout(
        "set -m\nsleep 30 & pid=$!\nkill -STOP $pid\nsleep 0.2\n{ sleep 0.3; kill -CONT $pid; sleep 0.3; kill $pid; } &\nwait -f; echo \"bare=$?\"\n",
        Duration::from_secs(20),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "bare=0\n", "stderr: {stderr}");
}
