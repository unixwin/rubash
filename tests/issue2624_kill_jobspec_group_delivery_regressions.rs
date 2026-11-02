//! rubash#2624 — `kill %jobspec` on a job-control job must deliver
//! via killpg to the job's process GROUP, not to a single collapsed
//! pid. GNU: kill.def's jobspec arm resolves the spec to
//! `IS_JOBCONTROL (job) ? j->pgrp : j->pipe->pid` and kill_pid's
//! group arm (jobs.c:4008-4020) delivers with one
//! `killpg (jobs[job]->pgrp, sig)` — reaching every pipeline stage
//! and every child a member forked after spawn. The old rubash path
//! resolved the spec to `pids.last()` and signaled that one process:
//! `kill -TERM %1` on a pipeline killed only the last stage.
//!
//! Driven through a util-linux `script(1)` PTY like the #2622 pins
//! (job control needs a terminal); where script(1) is unavailable
//! the tests return without asserting. Process-state assertions read
//! the kernel's own record (/proc on Linux, `ps` elsewhere) for
//! processes whose exact cmdline is the distinctive `sleep N` the
//! test spawned — no transcript parsing for the group faces.
//!
//! UNVERIFIED-BY-CAPTAIN: written to GNU expectations from the C
//! source; not run by the delivering cluster (pace rules).

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct PtySession {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    transcript: Arc<Mutex<String>>,
}

impl PtySession {
    fn spawn() -> Option<PtySession> {
        if Command::new("script").arg("--version").output().is_err() {
            return None;
        }
        let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
        let mut child = Command::new("script")
            .args(["-qec", &cmdline, "/dev/null"])
            .env("TERM", "xterm")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn script(1)");
        let stdin = child.stdin.take().expect("script stdin");
        let mut stdout = child.stdout.take().expect("script stdout");
        let transcript = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&transcript);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                        sink.lock().expect("transcript lock").push_str(&text);
                    }
                }
            }
        });
        Some(PtySession {
            child,
            stdin,
            transcript,
        })
    }

    fn send(&mut self, bytes: &str) {
        let _ = self.stdin.write_all(bytes.as_bytes());
        let _ = self.stdin.flush();
    }

    fn text(&self) -> String {
        self.transcript.lock().expect("transcript lock").clone()
    }

    fn wait_for(&self, needle: &str, from: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let text = self.text();
                if let Some(pos) = text[from..].find(needle) {
                    return from + pos + needle.len();
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}; transcript so far: {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn finish(mut self) -> std::process::ExitStatus {
        self.send("exit\n");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.child.try_wait().expect("poll script(1)") {
                Some(status) => return status,
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "session did not exit; transcript: {:?}",
                        self.text()
                    );
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}

const LONG: Duration = Duration::from_secs(20);

/// Pids whose whole cmdline is exactly `sleep <secs>` — the test's own
/// stages/children, never the `sh -c 'sleep N; …'` leader (its cmdline
/// carries the whole script text) and never the pgrep process itself.
fn sleep_pids(secs: u32) -> Vec<u32> {
    let pattern = format!("^sleep {secs}$");
    let Ok(output) = Command::new("pgrep").args(["-f", &pattern]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

/// Kernel scheduling state of one process: 'T' stopped, 'S'/'R'
/// runnable, 'Z' zombie; None once the process is gone.
fn process_state(pid: u32) -> Option<char> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The state field follows the LAST ')' (comm may contain
        // spaces and parens).
        let after = stat.rsplit(')').next()?;
        return after.trim_start().chars().next();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .chars()
            .next()
    }
}

fn wait_until(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Boot the session and give the interactive shell a moment to reach
/// its first prompt before typing job commands at it.
fn booted_session() -> Option<PtySession> {
    let session = PtySession::spawn()?;
    std::thread::sleep(Duration::from_millis(800));
    Some(session)
}

#[test]
fn term_jobspec_kills_every_pipeline_stage() {
    let Some(mut session) = booted_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    session.send("sleep 60 | sleep 60 | sleep 60 &\n");
    wait_until("three pipeline stages running", LONG, || {
        sleep_pids(60).len() == 3
    });
    // GNU: one killpg to the job's pgrp — all three stages take
    // SIGTERM. The old single-pid delivery killed only the last
    // stage; the other two kept sleeping and `wait` blocked on them.
    session.send("kill -TERM %1\n");
    wait_until("all stages dead after kill -TERM %1", LONG, || {
        sleep_pids(60).is_empty()
    });
    session.send("wait %1\n");
    session.send("echo W=$?\n");
    // The pipeline's status is its last stage's: 128 + SIGTERM.
    let _ = session.wait_for("W=143", 0, LONG);
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}

#[test]
fn stop_cont_jobspec_reach_unrecorded_child() {
    let Some(mut session) = booted_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    // The job table records only the `sh` leader; the `sleep` child
    // lives in the job's pgrp but in no recorded pid list — the same
    // shape as #2622, through the kill builtin this time.
    session.send("sh -c 'sleep 47; echo DONE47' &\n");
    let mut child = 0;
    wait_until("unrecorded sleep child running", LONG, || {
        let pids = sleep_pids(47);
        if pids.len() == 1 {
            child = pids[0];
            true
        } else {
            false
        }
    });
    // GNU: STOP addresses the group — the unrecorded child stops too.
    // Single-pid delivery stopped only the recorded leader and the
    // child kept running (state S), which is the pre-fix tell.
    session.send("kill -STOP %1\n");
    wait_until("sleep child stopped by kill -STOP %1", LONG, || {
        process_state(child) == Some('T')
    });
    // And CONT addresses the group the same way.
    session.send("kill -CONT %1\n");
    wait_until("sleep child resumed by kill -CONT %1", LONG, || {
        matches!(process_state(child), Some('S') | Some('R'))
    });
    session.send("kill -KILL %1\n");
    session.send("wait %1\n");
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}

#[test]
fn stop_cont_jobspec_stop_whole_pipeline() {
    let Some(mut session) = booted_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    session.send("sleep 45 | sleep 45 &\n");
    wait_until("two pipeline stages running", LONG, || {
        sleep_pids(45).len() == 2
    });
    // GNU: killpg stops BOTH stages. The old delivery stopped only
    // pids.last() — the first stage stayed in state S.
    session.send("kill -STOP %1\n");
    wait_until("both stages stopped by kill -STOP %1", LONG, || {
        let pids = sleep_pids(45);
        pids.len() == 2 && pids.iter().all(|pid| process_state(*pid) == Some('T'))
    });
    session.send("kill -CONT %1\n");
    wait_until("both stages resumed by kill -CONT %1", LONG, || {
        sleep_pids(45)
            .iter()
            .all(|pid| matches!(process_state(*pid), Some('S') | Some('R')))
    });
    session.send("kill -KILL %1\n");
    session.send("wait %1\n");
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}

#[test]
fn plain_pid_kill_hits_exactly_one_process() {
    let Some(mut session) = booted_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    // Distinct durations keep the two stages tellable apart. $! is
    // the pipeline's last stage pid — a PLAIN pid operand, so GNU
    // kill_pid takes the single-process arm (group only for
    // pid < -1): the first stage must survive untouched. This pins
    // the boundary the fix must not move.
    session.send("sleep 51 | sleep 52 &\n");
    wait_until("both stages running", LONG, || {
        sleep_pids(51).len() == 1 && sleep_pids(52).len() == 1
    });
    session.send("kill -TERM $!\n");
    wait_until("last stage dead after plain-pid kill", LONG, || {
        sleep_pids(52).is_empty()
    });
    // Settle, then the survivor check: group delivery on a pid
    // operand would have taken the first stage down too.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        sleep_pids(51).len(),
        1,
        "plain-pid kill must leave the other stage running"
    );
    assert!(
        matches!(process_state(sleep_pids(51)[0]), Some('S') | Some('R')),
        "surviving stage must be running, not stopped"
    );
    session.send("kill -KILL %1\n");
    session.send("wait %1\n");
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}
