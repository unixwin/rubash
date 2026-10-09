//! Issue rubash#440 regression: a broken pipe (SIGPIPE analogue) hitting a
//! command's output must not abort the whole interpreter.
//!
//! GNU spec: bash's parent shell keeps SIGPIPE at SIG_IGN — only forked
//! external commands die of it. A builtin whose write hits EPIPE reports
//! ``write error: Broken pipe'', finishes with status 1, and the script
//! keeps running; the shell itself never exits 128+13=141 because some
//! upstream pipeline member lost its reader. On Windows the same failure
//! surfaces as ERROR_BROKEN_PIPE (109) / ERROR_NO_DATA (232) from the
//! write, and the executor used to propagate it as a fatal ExecuteError,
//! printing ``rubash: Broken pipe'' and abandoning the remaining commands.
//!
//! Rust semantic owner: executor/ast_exec.rs command-list error arms (a
//! closed output is the command's own write failure: report, status 1,
//! continue) and executor/external_finish.rs child_process_exit_status
//! (a subshell's broken stdout is the child's status 1, not fatal).
//!
//! Every run is wrapped in a deadline: the pre-fix failure mode is an
//! early abort (and a regression could hang on a full pipe), never a
//! deadlock-tolerant pass.

#![cfg(windows)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(60);

struct BrokenPipeRun {
    timed_out: bool,
    code: Option<i32>,
    stderr: String,
}

/// Runs `script` with rubash's stdout attached to a pipe this harness
/// reads exactly one line from and then closes — the reader-gone shape
/// GNU models as SIGPIPE. stderr is drained concurrently so the child
/// can never block on a full pipe.
fn run_with_stdout_closed_after_first_line(script: &str) -> BrokenPipeRun {
    let dir = std::env::temp_dir().join(format!("rubash-issue440-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue440 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");

    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    // Consume the first line so at least one write has succeeded, then
    // drop the reader: every later write hits the broken pipe.
    let mut first_line = [0u8; 1];
    let _ = stdout_pipe.read(&mut first_line);
    drop(stdout_pipe);

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
    let stderr = stderr_reader.join().expect("join stderr reader");
    let _ = std::fs::remove_dir_all(&dir);
    BrokenPipeRun {
        timed_out,
        code: status.and_then(|status| status.code()),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

/// Core rubash#440: after the outer reader goes away, the builtin write
/// errors are reported (GNU ``write error: Broken pipe'' shape) and the
/// script's REMAINING commands still run — the interpreter does not die.
#[test]
fn broken_stdout_does_not_abort_the_script() {
    let out = run_with_stdout_closed_after_first_line(concat!(
        "echo READY\n",
        "i=0\n",
        "while [ $i -lt 2000 ]; do\n",
        "  echo line$i\n",
        "  i=$((i+1))\n",
        "done\n",
        "echo >&2 AFTER_LOOP\n",
    ));
    assert!(!out.timed_out, "script hung after broken stdout");
    assert!(
        out.stderr.contains("AFTER_LOOP"),
        "script aborted at the broken pipe (rubash#440); stderr: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("Broken pipe"),
        "the failed writes were not reported GNU-style; stderr: {}",
        out.stderr
    );
    assert_eq!(
        out.code,
        Some(0),
        "the trailing `echo >&2` succeeded, so the shell exits 0; stderr: {}",
        out.stderr
    );
}

/// A subshell (in-process child script) whose shared stdout breaks is its
/// own write failure with status 1 — the parent keeps running.
#[test]
fn broken_stdout_in_subshell_keeps_parent_running() {
    let out = run_with_stdout_closed_after_first_line(concat!(
        "echo READY\n",
        "(i=0\n",
        "while [ $i -lt 2000 ]; do\n",
        "  echo line$i\n",
        "  i=$((i+1))\n",
        "done)\n",
        "echo >&2 PARENT_ALIVE\n",
    ));
    assert!(!out.timed_out, "subshell hung after broken stdout");
    assert!(
        out.stderr.contains("PARENT_ALIVE"),
        "parent aborted by the subshell's broken stdout (rubash#440); stderr: {}",
        out.stderr
    );
}

/// Under `set -e` the failed builtin write is an ordinary command failure:
/// errexit stops the script with status 1 — a clean exit, not a fatal
/// interpreter error and not 141.
#[test]
fn errexit_promotes_broken_write_to_status_one() {
    let out = run_with_stdout_closed_after_first_line(concat!(
        "echo READY\n",
        "set -e\n",
        "i=0\n",
        "while [ $i -lt 2000 ]; do\n",
        "  echo line$i\n",
        "  i=$((i+1))\n",
        "done\n",
        "echo >&2 NEVER_REACHED\n",
    ));
    assert!(!out.timed_out, "script hung under errexit");
    assert!(
        !out.stderr.contains("NEVER_REACHED"),
        "errexit did not stop the script; stderr: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("Broken pipe"),
        "expected the GNU write-error diagnostic; stderr: {}",
        out.stderr
    );
    assert_eq!(
        out.code,
        Some(1),
        "errexit must surface the failed write as status 1, not 141/abort"
    );
}
