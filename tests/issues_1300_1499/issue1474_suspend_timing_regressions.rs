//! rubash#1474 — `suspend -f` took effect only after the REST of the
//! script had run (the stop was delivered outside the execution
//! stream); GNU stops the process AT the builtin: output produced
//! before SIGCONT ends at the line before `suspend -f`.
//!
//! Probe: clusters/x2/repro/1474-sup.sh + 1474-target.sh — a session
//! supervisor (setsid) observes the stopped state and the partial
//! output, then sends SIGCONT. This test reproduces that harness:
//! the child runs in its own session, we poll /proc for state T,
//! check the partial output, then continue it.
//! Expectations verified against GNU bash 5.3.20 (same harness).
//!
//! rubash#1637 — the original carrier could hang the whole suite in
//! a sandbox: it sent SIGCONT exactly once and then blocked in an
//! unbounded `wait()`. Under load that one SIGCONT did not wake the
//! stopped child (an externally-sent SIGCONT later did), the test
//! hung for 815s, and the stopped process leaked for manual cleanup.
//! The carrier is now bounded end to end: every wait has a deadline,
//! SIGCONT is re-sent while the child remains stopped, and a Drop
//! guard SIGKILLs + reaps the child session on every exit path, so
//! a failure here can never wedge the suite or leak a stopped
//! process. The assertions and expectations are unchanged.

#![cfg(target_os = "linux")]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1474-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn proc_state(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm may contain spaces/parens; state is the field after ") ".
    let after = stat.rsplit(") ").next()?;
    after.chars().next()
}

fn read_file(path: &PathBuf) -> String {
    let mut s = String::new();
    let _ = std::fs::File::open(path).map(|mut f| f.read_to_string(&mut s));
    s
}

/// Supervised child (rubash#1637): owns the spawned session leader
/// and guarantees the session is killed and reaped on every path —
/// success, assertion panic, or timeout — so no stopped rubash can
/// outlive the test.
struct SupervisedChild {
    child: std::process::Child,
    pid: u32,
}

impl SupervisedChild {
    /// Poll for exit until `timeout`; returns the status if the
    /// child finished in time, `None` on deadline.
    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn send_sigcont(&self) {
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGCONT);
            libc::kill(self.pid as i32, libc::SIGCONT);
        }
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        // Reap, itself bounded so Drop can never hang.
        let _ = self.wait_for_exit(Duration::from_secs(5));
    }
}

#[test]
fn suspend_f_stops_before_following_commands() {
    let dir = tempdir("suspend");
    let target = dir.join("t.sh");
    std::fs::write(
        &target,
        "suspend\necho \"suspend rc=$?\"\nsuspend -f\necho \"suspend -f rc=$?\"\nlogout\necho \"logout rc=$?\"\n",
    )
    .expect("write target");
    let out_path = dir.join("out.txt");
    let err_path = dir.join("err.txt");
    let out_file = std::fs::File::create(&out_path).expect("create out");
    let err_file = std::fs::File::create(&err_path).expect("create err");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("t.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
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

    // Poll up to 15s for the stopped state.
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
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(stopped, "process never entered stopped state T");

    // Give any (wrongly) deferred commands a chance to run, then the
    // output must still end right before `suspend -f`.
    std::thread::sleep(Duration::from_millis(500));
    let partial = read_file(&out_path);
    assert_eq!(
        partial, "suspend rc=1\n",
        "commands after suspend -f ran before SIGCONT"
    );

    // Continue the session and let it finish. SIGCONT is re-sent for
    // as long as the child remains stopped (rubash#1637: a single
    // send proved losable under sandbox load), and the wait is
    // bounded: on deadline the guard kills the session and this
    // assertion fails with diagnostics instead of hanging the suite.
    let resume_deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Ok(Some(status)) = sup.child.try_wait() {
            break Some(status);
        }
        if Instant::now() >= resume_deadline {
            break None;
        }
        sup.send_sigcont();
        std::thread::sleep(Duration::from_millis(100));
    };
    let status = status.unwrap_or_else(|| {
        panic!(
            "child did not exit within 30s of SIGCONT (state: {:?}); out: {:?}; err: {:?}",
            proc_state(pid),
            read_file(&out_path),
            read_file(&err_path)
        )
    });
    assert!(status.success(), "final status: {status}");
    let full = read_file(&out_path);
    assert_eq!(
        full, "suspend rc=1\nsuspend -f rc=0\nlogout rc=1\n",
        "final output after SIGCONT"
    );
    let err = read_file(&err_path);
    assert!(
        err.contains("suspend: cannot suspend: no job control"),
        "stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
