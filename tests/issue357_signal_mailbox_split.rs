//! Issue rubash#357 — cross-process `kill -TERM` to a trap-bearing rubash
//! child must run the trap, not hard-kill the process.
//!
//! Root cause (Windows mailbox): `signal_mailbox_dir()` resolved
//! `std::env::temp_dir()` per call. An MSYS parent forwards raw POSIX values
//! (`TEMP=/tmp`, `TMP=/tmp`), which Win32 file APIs resolve against the
//! CURRENT DRIVE — so two rubash processes (victim with a raw value or a
//! different cwd drive, killer with the native `%TEMP%`) split into different
//! mailbox boxes. The killer missed the victim's `{pid}.alive` marker, and
//! the kill fell back to `TerminateProcess(handle, 128+sig)` — exit 143 with
//! the trap never running (143/NOTRAPPED). A rubash child's env restore could
//! also re-apply the raw spawn value MID-RUN, flipping one process between
//! boxes after registration. Fixed by pinning one canonical box
//! (`%LOCALAPPDATA%\rubash-signals`, OnceLock-cached) that ignores
//! TMP/TEMP entirely (kill.rs signal_mailbox_dir).
//!
//! GNU anchors for the dispatch model these tests lock:
//! - trap.c:553 `trap_handler`: a caught signal during a foreground wait is
//!   recorded via `set_trap_state` (trap.c:537 `pending_traps[sig]++`) and
//!   dispatched at the next command boundary — execute_cmd.c:643
//!   `run_pending_traps` (bash manual, SIGNALS: the trap "will not be
//!   executed until the command completes"). Only the `wait` builtin
//!   longjmps out of the blocking poll (trap.c:603-611, jobs.c CHECK_WAIT_INTR).
//! - jobs.c:3064 `wait_for`: the foreground wait stays interruptible; an
//!   UNtrapped terminating signal has no handler in a non-interactive shell
//!   (sig.c:315-316) and kills it through SIG_DFL.
//!
//! Expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-30): busy-loop trap →
//! `TRAPPED` + rc 7; deferred (sleep) trap → fires only after the sleep
//! completes, then `TRAPPED` + rc 7; no trap → 143; `kill -9` → 137.
//!
//! The tests spawn REAL processes (CreateProcess via std::process::Command):
//! a victim rubash script child and a separate killer rubash process, so the
//! delivery crosses a real process boundary through the shared mailbox.

#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A child that exits with a bounded wait instead of hanging the test run
/// forever (AGENTS: every possibly-hanging probe runs under a timeout).
struct WatchedChild(std::process::Child);

impl WatchedChild {
    fn spawn(command: &mut Command) -> Self {
        Self(
            command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn rubash child"),
        )
    }

    fn wait_ok(&mut self, label: &str) -> (String, Option<i32>) {
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            if let Some(status) = self.0.try_wait().expect("try_wait child") {
                let mut output = self.0.stdout.take().expect("child stdout");
                let mut text = String::new();
                use std::io::Read;
                output.read_to_string(&mut text).expect("read child stdout");
                return (text, status.code());
            }
            if Instant::now() > deadline {
                let _ = self.0.kill();
                panic!("{label}: child did not exit within 25s");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for WatchedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Write a victim script under the test's own temp area and return its path.
fn victim_script(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue357-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    // LF line endings: the repo's .sh contract (CRLF breaks dollar-quote and
    // trap parsing identically to the WSL-side baseline runs).
    let _ = file.write_all(body.replace("\r\n", "\n").as_bytes());
    path
}

/// A killer rubash process: `kill -TERM <pid>` from its OWN process, with the
/// given TMP/TEMP forwarded raw (the MSYS-passthrough shape) or native.
fn killer(signal: &str, victim_pid: u32, tmp: &str, temp: &str, cwd: &str) -> WatchedChild {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("-c")
        .arg(format!("kill -{signal} {victim_pid}"))
        .env("TMP", tmp)
        .env("TEMP", temp)
        .env_remove("RUBASH_SIG_TRACE")
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    WatchedChild::spawn(&mut command)
}

const RAW: &str = "/tmp";
const NATIVE_TEMP: &str = "C:\\Windows\\Temp";

fn native_temp() -> String {
    std::env::var("TEMP").unwrap_or_else(|_| NATIVE_TEMP.to_string())
}

/// Primary #357 shape: victim spawned with RAW MSYS TMP/TEMP (drive-relative
/// box pre-fix), killer with native TEMP and a DIFFERENT cwd drive. GNU:
/// `TRAPPED`, rc 7.
#[test]
fn term_trap_runs_across_raw_msys_mailbox_split() {
    let script = victim_script(
        "busy-trap.sh",
        "trap 'echo TRAPPED; exit 7' TERM\nwhile :; do :; done\necho NOTRAPPED\n",
    );
    let mut victim = WatchedChild::spawn(
        Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script)
            .env("TMP", RAW)
            .env("TEMP", RAW)
            .current_dir("D:\\")
            .env_remove("RUBASH_SIG_TRACE"),
    );
    // Give the victim's (async) mailbox registration and the `trap` line a
    // moment — the pre-mailbox window is the documented sub-millisecond
    // startup21 tradeoff; 1s covers it by three orders of magnitude.
    std::thread::sleep(Duration::from_secs(1));
    let mut killer = killer("TERM", victim.0.id(), RAW, RAW, "C:\\");
    let (killer_out, killer_code) = killer.wait_ok("killer");
    assert_eq!(killer_out, "");
    assert_eq!(killer_code, Some(0), "kill -TERM must succeed");
    let (victim_out, victim_code) = victim.wait_ok("victim busy-loop");
    assert_eq!(victim_out, "TRAPPED\n");
    assert_eq!(victim_code, Some(7));
}

/// Symmetric direction: victim with native TEMP, killer with raw MSYS TMP on
/// another cwd drive. Same GNU answer: `TRAPPED`, rc 7.
#[test]
fn term_trap_runs_when_killer_resolves_raw_msys_mailbox() {
    let script = victim_script(
        "busy-trap2.sh",
        "trap 'echo TRAPPED; exit 7' TERM\nwhile :; do :; done\n",
    );
    let mut victim = WatchedChild::spawn(
        Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script)
            .env("TMP", &native_temp())
            .env("TEMP", &native_temp())
            .current_dir("C:\\")
            .env_remove("RUBASH_SIG_TRACE"),
    );
    std::thread::sleep(Duration::from_secs(1));
    let mut killer = killer("TERM", victim.0.id(), RAW, RAW, "D:\\");
    let (_, killer_code) = killer.wait_ok("killer");
    assert_eq!(killer_code, Some(0));
    let (victim_out, victim_code) = victim.wait_ok("victim busy-loop");
    assert_eq!(victim_out, "TRAPPED\n");
    assert_eq!(victim_code, Some(7));
}

/// GNU deferral (trap.c:553 set_trap_state + execute_cmd.c:643
/// run_pending_traps): the trap for a signal that arrives while the shell
/// waits for a foreground command runs only AFTER the command completes —
/// verified byte-for-byte against GNU 5.3.0 (busy deferral probe: the child
/// stays alive past the signal; sleep-2 shape: TRAPPED appears at the sleep's
/// end, not before). Locks that the mailbox port keeps deferring instead of
/// hard-killing.
#[test]
fn term_trap_defers_until_foreground_command_completes() {
    let script = victim_script(
        "sleep-trap.sh",
        "trap 'echo TRAPPED; exit 7' TERM\nsleep 3\necho NOTRAPPED\n",
    );
    let started = Instant::now();
    let mut victim = WatchedChild::spawn(
        Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script)
            .env("TMP", RAW)
            .env("TEMP", RAW)
            .env_remove("RUBASH_SIG_TRACE"),
    );
    std::thread::sleep(Duration::from_millis(800));
    let mut killer = killer("TERM", victim.0.id(), RAW, RAW, "C:\\");
    let (_, killer_code) = killer.wait_ok("killer");
    assert_eq!(killer_code, Some(0));
    let (victim_out, victim_code) = victim.wait_ok("victim sleep");
    // The trap fires at the sleep's completion: no earlier than ~2s after the
    // kill, and the sleep body never echoes NOTRAPPED (exit 7 preempts it).
    assert!(
        started.elapsed() >= Duration::from_millis(1800),
        "trap fired before the foreground sleep completed: {:?}",
        started.elapsed()
    );
    assert_eq!(victim_out, "TRAPPED\n");
    assert_eq!(victim_code, Some(7));
}

/// Untrapped TERM: GNU leaves the disposition SIG_DFL for a non-interactive
/// script (sig.c:315-316 initialize_terminating_signals is interactive-only),
/// so the shell dies with 128+15. The mailbox port ends the blocked shell the
/// same way (trap_exec observe_signals_while_blocked ExitCode path).
#[test]
fn untrapped_term_kills_child_with_143() {
    let script = victim_script("busy-notrap.sh", "while :; do :; done\n");
    let mut victim = WatchedChild::spawn(
        Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script)
            .env("TMP", RAW)
            .env("TEMP", RAW)
            .env_remove("RUBASH_SIG_TRACE"),
    );
    std::thread::sleep(Duration::from_secs(1));
    let native = native_temp();
    let mut killer = killer("TERM", victim.0.id(), &native, &native, "D:\\");
    let (_, killer_code) = killer.wait_ok("killer");
    assert_eq!(killer_code, Some(0));
    let (victim_out, victim_code) = victim.wait_ok("victim untrapped");
    assert_eq!(victim_out, "");
    assert_eq!(victim_code, Some(143));
}

/// SIGKILL is uncatchable by design (kill.rs deliver_rubash_signal skips
/// mailbox routing for 9) — the native TerminateProcess fallback keeps
/// reporting 137.
#[test]
fn kill_nine_remains_a_hard_kill() {
    let script = victim_script(
        "busy-k9.sh",
        "trap 'echo TRAPPED; exit 7' TERM\nwhile :; do :; done\n",
    );
    let mut victim = WatchedChild::spawn(
        Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script)
            .env("TMP", RAW)
            .env("TEMP", RAW)
            .env_remove("RUBASH_SIG_TRACE"),
    );
    std::thread::sleep(Duration::from_secs(1));
    let mut killer = killer("9", victim.0.id(), RAW, RAW, "C:\\");
    let (_, killer_code) = killer.wait_ok("killer");
    assert_eq!(killer_code, Some(0));
    let (victim_out, victim_code) = victim.wait_ok("victim k9");
    assert_eq!(victim_out, "");
    assert_eq!(victim_code, Some(137));
}

/// Direct split-brain guard: a victim whose TMP/TEMP are RAW MSYS values must
/// register its `{pid}.alive` marker in the canonical `%LOCALAPPDATA%`
/// box — never under a drive-relative `\tmp` box (the two-directory
/// coexistence the issue reported).
#[test]
fn raw_msys_env_registers_marker_in_canonical_box() {
    let script = victim_script("busy-marker.sh", "while :; do :; done\n");
    let mut victim = WatchedChild::spawn(
        Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script)
            .env("TMP", RAW)
            .env("TEMP", RAW)
            .current_dir("D:\\")
            .env_remove("RUBASH_SIG_TRACE"),
    );
    let pid = victim.0.id();
    // Registration is async (startup21): poll for the marker briefly. The
    // deadline assert inside the loop is the failure mode (marker never
    // appears in the canonical box).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let canonical = std::env::var("LOCALAPPDATA")
            .map(|local| {
                PathBuf::from(local)
                    .join("rubash-signals")
                    .join(format!("{pid}.alive"))
            })
            .ok();
        if canonical.is_some_and(|path| path.is_file()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no {pid}.alive marker in the canonical LOCALAPPDATA box"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // The drive-relative boxes must NOT gain this pid's marker.
    for stray in ["D:\\tmp\\rubash-signals", "C:\\tmp\\rubash-signals"] {
        let path = PathBuf::from(stray).join(format!("{pid}.alive"));
        assert!(!path.is_file(), "marker leaked into the split box {stray}");
    }
    let _ = victim.0.kill();
    let _ = victim.0.wait();
}
