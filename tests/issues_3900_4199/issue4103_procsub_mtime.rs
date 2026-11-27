//! rubash#4103 — two process-substitution pipes created within the
//! same wall-clock second must compare equal under `-nt`/`-ot` on
//! `/dev/fd/N` (and `/proc/self/fd/N`).
//!
//! GNU Bash (`test.c:330-354 filecomp` + `include/stat-time.h:160
//! timespec_cmp`) compares `st_mtim` from `sh_stat()`. Because GNU's
//! `process_substitute` (`subst.c`) forks in-process in sub-millisecond
//! time, consecutive procsub pipes land in the same kernel coarse
//! jiffy and share an identical `st_mtim`. Rubash spawns an external
//! `rubash -c` child per procsub, which crosses jiffy ticks and
//! produces distinct `st_mtime_nsec` within the same second.
//!
//! Fix: truncate `FD_STAT_PREFIX` nanoseconds to `0` for `procsub_pipe`
//! entries in `sync_fd_alias_facts_scoped` (`src/executor/shell_options.rs`).

#![cfg(unix)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn run_rubash(script: &str) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("poll rubash") {
            Some(_) => break,
            None => {
                assert!(Instant::now() < deadline, "rubash timed out: {script}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    let output = child.wait_with_output().expect("collect rubash output");
    assert_eq!(output.status.code(), Some(0), "script failed: {script}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_secs()
}

#[test]
fn same_second_procsub_pipes_compare_equal_under_nt_and_ot() {
    let script = concat!(
        "exec 3< <(echo a); ",
        "exec 5< <(echo b); ",
        "test /dev/fd/3 -nt /dev/fd/5; echo nt35=$?; ",
        "test /dev/fd/3 -ot /dev/fd/5; echo ot35=$?; ",
        "test /dev/fd/5 -nt /dev/fd/3; echo nt53=$?; ",
        "test /dev/fd/5 -ot /dev/fd/3; echo ot53=$?; ",
        "[ /proc/self/fd/3 -nt /proc/self/fd/5 ]; echo b_nt35=$?; ",
        "[ /proc/self/fd/3 -ot /proc/self/fd/5 ]; echo b_ot35=$?; ",
        "[[ /dev/fd/3 -nt /dev/fd/5 ]]; echo c_nt35=$?; ",
        "[[ /dev/fd/3 -ot /dev/fd/5 ]]; echo c_ot35=$?; ",
        "exec 3<&- 5<&-"
    );

    // Retry once if the two procsub spawns happened to straddle a
    // wall-clock second boundary.
    let mut out = String::new();
    for _ in 0..3 {
        let before = current_epoch_secs();
        out = run_rubash(script);
        let after = current_epoch_secs();
        if before == after {
            break;
        }
    }
    assert_eq!(
        out,
        "nt35=1\not35=1\nnt53=1\not53=1\nb_nt35=1\nb_ot35=1\nc_nt35=1\nc_ot35=1\n"
    );
}
