//! rubash#387: a script whose NAME contains "coproc" leaked one blank
//! line to the real stdout after the script body ran
//! (`coproc GREP { echo co-value; }; echo "pid:${GREP_PID:+set}"`
//! printed `pid:set` + an empty line; GNU prints `pid:set` only).
//!
//! Root cause: script_driver.rs carried a "KNOWN WORKAROUND" that printed
//! an unconditional newline at exit for every non-interactive rc-0 script
//! whose `__RUBASH_SCRIPT_NAME` contains "coproc" — a script-NAME-gated
//! symptom guard (rubash#117 family) aimed at the vendored
//! `tests/coproc.right` tail.
//!
//! That golden file is defective, not rubash: GNU 5.3.0's own output for
//! the suite ends with the final `echo $foo >&2` newline, which the
//! vendored `.right` lacks (captured without the final byte). Rubash's
//! fd-2 write path emits that newline correctly in every reproducible
//! context — direct script run, Git-Bash PATH (cat reads /etc/passwd),
//! and the faithful three-coproc sequence including a failing third
//! coproc — verified against WSL GNU Bash 5.3.0 script-file probes
//! (target/probe386/, target/issue-suites/results/wt37-gapfix1/,
//! 2026-10-02). The guard is deleted; the suite diff is byte-closer to
//! GNU (true-baseline coproc slice 10 -> 8 diff lines, upstream runner
//! slice 15 -> 11; both keep failing only on pre-existing
//! environment-bound lines — no /etc/passwd under WinuxCmd `cat`,
//! missing `xcase` helper, and the golden's own env shape).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(20);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_rubash_script_at(script_path: &std::path::Path, script: &str) -> RunOutcome {
    let mut file = std::fs::File::create(script_path).expect("create probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");

    let stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stdout_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None => {
                if started.elapsed() >= RUN_LIMIT {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = stdout_reader.join().expect("join stdout reader");
    let stderr = stderr_reader.join().expect("join stderr reader");
    RunOutcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

#[test]
fn named_coproc_script_does_not_leak_blank_stdout() {
    // The #387 reproducer, at a path whose NAME contains "coproc" — the
    // old workaround's trigger. GNU: `pid:set\n`, rc 0, empty stderr.
    let dir = std::env::temp_dir().join(format!("rubash-issue387-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let outcome = run_rubash_script_at(
        &dir.join("B09_coproc_leak.sh"),
        "coproc GREP { echo co-value; }\necho \"pid:${GREP_PID:+set}\"\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"pid:set\n");
    assert!(outcome.stderr.is_empty());
}

#[test]
fn unnamed_and_silent_coproc_bodies_stay_clean() {
    let dir = std::env::temp_dir().join(format!("rubash-issue387b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    for (name, script, expected) in [
        (
            "coproc_unnamed.sh",
            "coproc { echo unnamed; }\necho \"pid:${COPROC_PID:+set}\"\n",
            &b"pid:set\n"[..],
        ),
        (
            "coproc_silent.sh",
            "coproc G { :; }\necho \"pid:${G_PID:+set}\"\n",
            &b"pid:set\n"[..],
        ),
        (
            "coproc_two_writes.sh",
            "coproc GREP { echo one; echo two; }\necho \"pid:${GREP_PID:+set}\"\n",
            &b"pid:set\n"[..],
        ),
    ] {
        let outcome = run_rubash_script_at(&dir.join(name), script);
        assert_eq!(outcome.code, Some(0), "rc for {name}");
        assert_eq!(outcome.stdout, expected, "stdout for {name}");
        assert!(outcome.stderr.is_empty(), "stderr for {name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coproc_fd2_final_newline_is_emitted() {
    // The sequence the workaround claimed to stand in for: after moving
    // the coproc fds, `echo $foo >&2` with unset foo emits one newline.
    // Rubash emits it (verified vs WSL GNU 5.3.0 script-file probe).
    let dir = std::env::temp_dir().join(format!("rubash-issue387c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let outcome = run_rubash_script_at(
        &dir.join("fd2_tail_coproc_seq.sh"),
        "coproc CC { echo FOO; }\nread LINE <&${CC[0]}\nwait $CC_PID\n\
         exec 4<&${CC[0]}-\nexec >&${CC[1]}-\nread foo <&4\necho $foo >&2\nexit 0\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    // stdout was closed by `exec >&${CC[1]}-`, so the fd-2 write lands on
    // stderr: it must end with exactly the one newline, and nothing may
    // surface on the closed stdout.
    assert!(
        outcome.stdout.is_empty(),
        "closed stdout received bytes: {:?}",
        outcome.stdout
    );
    assert!(
        outcome.stderr.ends_with(b"\n"),
        "fd-2 newline missing: {:?}",
        outcome.stderr
    );
    // GNU's own output for this sequence ends `Bad file descriptor\n\n`
    // (the read diagnostic's newline, then the fd-2 echo's); a THIRD
    // trailing newline would be the leak this issue is about.
    assert!(
        !outcome.stderr.ends_with(b"\n\n\n"),
        "extra trailing newline leaked: {:?}",
        outcome.stderr
    );
}
