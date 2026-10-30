//! rubash#2203 — `suspend -f` must resume on a SINGLE SIGCONT.
//!
//! The suspend builtin stopped the group with killpg and then, as the
//! #1474 promptness clamp, queued a second thread-directed SIGSTOP on
//! the executing thread. When that stop was still pending as the group
//! stop completed, the first SIGCONT released the group stop, the
//! executing thread dequeued its pending SIGSTOP, and the group
//! re-stopped: one SIGCONT — pid- or group-addressed, i.e. exactly
//! what `fg` sends — did not resume the shell. Measured on the
//! pre-fix build under load: 15/40 supervised runs needed a second
//! SIGCONT (group-addressed first: 11/30). The cli_tests carrier
//! `scripts::suspend_force_stops_own_group_until_continued` sends
//! exactly one SIGCONT and flaked on CI ten times with "child did not
//! exit after CONT".
//!
//! This pin supervises the child exactly like the #1474 harness
//! (own session via setsid, /proc state polling, Drop guard that
//! SIGKILLs + reaps the session on every path so a failure can never
//! leak a stopped process), but sends precisely ONE SIGCONT per
//! round — alternating pid-addressed and group-addressed — and
//! requires the child to exit on it. GNU bash 5.3.20 passes the same
//! protocol trivially (single-threaded, one stop generation).

#![cfg(target_os = "linux")]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2203-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn proc_state(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.rsplit(") ").next()?;
    after.chars().next()
}

fn read_file(path: &PathBuf) -> String {
    let mut s = String::new();
    let _ = std::fs::File::open(path).map(|mut f| f.read_to_string(&mut s));
    s
}

/// Supervised session leader (the #1474/#1637 carrier shape): Drop
/// SIGKILLs the whole session and reaps, so no stopped rubash can
/// outlive this test on any path.
struct SupervisedChild {
    child: std::process::Child,
    pid: u32,
}

impl SupervisedChild {
    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        let _ = self.wait_for_exit(Duration::from_secs(5));
    }
}

#[test]
fn suspend_f_resumes_on_single_sigcont() {
    let dir = tempdir("single-cont");
    let target = dir.join("t.sh");
    std::fs::write(&target, "suspend -f\necho \"rc=$?\"\n").expect("write target");

    // Twelve rounds alternate the two resume addressings `fg` and a
    // supervising parent use. The pre-fix failure rate under load was
    // ~1/3 per round, so twelve strict rounds make a pre-fix pass
    // vanishingly unlikely while a correct build is deterministic.
    for round in 0..12 {
        let group_addressed = round % 2 == 1;
        let out_path = dir.join(format!("out{round}.txt"));
        let out_file = std::fs::File::create(&out_path).expect("create out");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
        cmd.arg("t.sh")
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::null());
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().expect("spawn rubash");
        let pid = child.id();
        let mut sup = SupervisedChild { child, pid };

        // Wait for the stopped state.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut stopped = false;
        while Instant::now() < deadline {
            if proc_state(pid) == Some('T') {
                stopped = true;
                break;
            }
            if let Ok(Some(_)) = sup.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            stopped,
            "round {round}: process never entered stopped state T"
        );

        // Exactly ONE SIGCONT — the assertion of this pin.
        unsafe {
            if group_addressed {
                libc::kill(-(pid as i32), libc::SIGCONT);
            } else {
                libc::kill(pid as i32, libc::SIGCONT);
            }
        }
        let status = sup.wait_for_exit(Duration::from_secs(5));
        assert!(
            status.is_some(),
            "round {round} ({}-addressed): child did not exit on a single \
             SIGCONT after suspend -f (state: {:?}) — a second stop \
             generation survived the group stop (rubash#2203)",
            if group_addressed { "group" } else { "pid" },
            proc_state(pid),
        );
        let status = status.unwrap();
        assert!(status.success(), "round {round}: final status: {status}");
        assert_eq!(
            read_file(&out_path),
            "rc=0\n",
            "round {round}: output after resume"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
