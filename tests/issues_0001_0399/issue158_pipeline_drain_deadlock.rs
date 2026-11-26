//! Issue niubash#158 regression: `seq 20000 | cat` hung forever (P0, exposed
//! by the 2c781657 patient-wait change that replaced the non-final member's
//! 100ms hard kill). Two staged orderings deadlocked once a pass-through
//! member moved more data than the pipe buffers could hold:
//!
//! 1. Concurrent OS-pipe path (`spawn_wait_concurrent_external_stages`):
//!    the last member's stdout is a parent-side capture pipe, but the drain
//!    (`wait_with_output`) only started AFTER every upstream member was
//!    reaped. A pass-through last member (cat/nl/rev) that fills its stdout
//!    pipe stops reading its input, the producer then fills the inter-member
//!    pipe and never exits, and the forward wait polls forever.
//! 2. Sequential stage path (`execute_external_pipeline_stage_inner`): the
//!    stage's stdin payload was written synchronously before any drain, so a
//!    payload larger than the stdin pipe capacity blocked `write_all` while
//!    the child had already filled its own (undrained) stdout pipe.
//!
//! GNU baseline: `execute_cmd.c:2620 execute_pipeline` leaves the rightmost
//! element's fd 1 on the shell's own stdout, and the parent closes its pipe
//! write copies immediately after each fork (`close(prev)` 2707-2708,
//! `close(fildes[1])` 2711); inside `$(...)` `subst.c:7143
//! command_substitute` closes the write end (7428) and drains the capture
//! pipe (`read_comsub`, 7437) BEFORE `wait_for(pid)` (7441) — the drain runs
//! concurrently with the members, never staged behind producer waits. The
//! fix mirrors that ordering (drain threads started before the waits, stdin
//! payload fed from a writer thread); no wall-clock timeout is involved.
//!
//! Every test here is wrapped in a short deadline because the failure mode
//! is a hang, not a wrong byte.

#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Deadline for one pipeline run. The fixed binary finishes every shape here
/// in well under a second; the pre-fix binary never returns.
const RUN_LIMIT: Duration = Duration::from_secs(30);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue158-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue158 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    // LF line endings: the repo's .gitattributes pins *.sh eol=lf and WSL
    // GNU-side comparisons choke on CR.
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

    // Drain the captured streams while the shell runs — reading them only
    // after exit would re-create the very write-before-drain deadlock under
    // test whenever the probe's output exceeds the pipe capacity.
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
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

/// Resolve an external tool to an absolute, quoteable script word so the
/// probe does not depend on the child's PATH. Windows path separators are
/// normalized to `/` (CreateProcess accepts them and the shell will not
/// treat `\` as an escape).
fn resolve_tool(name: &str) -> String {
    let with_exe = format!("{name}.exe");
    for dir in std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .filter(|entry| !entry.is_empty())
    {
        for candidate in [format!("{dir}\\{with_exe}"), format!("{dir}\\{name}")] {
            let path = PathBuf::from(&candidate);
            if path.is_file() {
                return path.to_string_lossy().replace('\\', "/");
            }
        }
    }
    panic!(
        "issue158 tests need a real external `{name}` on PATH (the deadlock \
         lives in the OS-pipe spawn paths, which internal emulations bypass)"
    );
}

fn seq_stream() -> String {
    (1..=20000)
        .map(|line| format!("{line}\n"))
        .collect::<String>()
}

#[test]
fn concurrent_path_drains_last_member_while_producers_run() {
    let seq = resolve_tool("seq");
    let cat = resolve_tool("cat");
    // 108,894 bytes exceed the stdin+stdout pipe capacities, so the pre-fix
    // binary hangs here (the repro shape from the external report).
    let out = run_rubash_script(&format!("\"{seq}\" 20000 | \"{cat}\"\n"));
    assert!(
        !out.timed_out,
        "`seq 20000 | cat` did not terminate within {RUN_LIMIT:?} (niubash#158 concurrent-path drain deadlock)"
    );
    assert_eq!(
        out.code,
        Some(0),
        "exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        seq_stream(),
        "`seq 20000 | cat` must pass the producer's bytes through unchanged"
    );
}

#[test]
fn concurrent_path_capture_form_drains_last_member() {
    let seq = resolve_tool("seq");
    let cat = resolve_tool("cat");
    let out = run_rubash_script(&format!("x=$(\"{seq}\" 20000 | \"{cat}\")\necho ${{#x}}\n"));
    assert!(
        !out.timed_out,
        "`$(seq 20000 | cat)` did not terminate within {RUN_LIMIT:?} (niubash#158 capture form)"
    );
    assert_eq!(
        out.code,
        Some(0),
        "exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Command substitution strips the one trailing newline: 108,894 - 1.
    assert_eq!(out.stdout, b"108893\n", "captured length");
}

#[test]
fn concurrent_path_redirect_tail_form_terminates() {
    let seq = resolve_tool("seq");
    let cat = resolve_tool("cat");
    let dir = std::env::temp_dir().join(format!("rubash-issue158-redir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue158 redir scratch");
    let parent = dir.to_string_lossy().replace('\\', "/");
    let out = run_rubash_script(&format!(
        "cd '{parent}'\n\"{seq}\" 20000 | \"{cat}\" > out.bin\necho RC=$?\n"
    ));
    let written = std::fs::read(dir.join("out.bin")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.timed_out,
        "`seq 20000 | cat > f` did not terminate within {RUN_LIMIT:?}"
    );
    assert_eq!(
        out.stdout,
        b"RC=0\n",
        "status line, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&written),
        seq_stream(),
        "the redirected tail must hold the producer's bytes unchanged"
    );
}

#[test]
fn sequential_path_stdin_payload_write_does_not_block_drain() {
    let cat = resolve_tool("cat");
    // `cat <file>` on member 0 carries a redirect_in, so the pipeline takes
    // the sequential stage executor; stage 1 receives the whole 108,894-byte
    // payload through its stdin pipe (the write that used to block before
    // any drain started).
    let dir = std::env::temp_dir().join(format!("rubash-issue158-seq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue158 seq scratch");
    std::fs::write(dir.join("big.txt"), seq_stream()).expect("write big.txt");
    let parent = dir.to_string_lossy().replace('\\', "/");
    let out = run_rubash_script(&format!("cd '{parent}'\n\"{cat}\" big.txt | \"{cat}\"\n"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.timed_out,
        "`cat big.txt | cat` did not terminate within {RUN_LIMIT:?} (niubash#158 sequential-path stdin write deadlock)"
    );
    assert_eq!(
        out.code,
        Some(0),
        "exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        seq_stream(),
        "`cat big.txt | cat` must pass the bytes through unchanged"
    );
}

#[test]
fn early_exit_reader_keeps_clean_status_after_payload_write() {
    let cat = resolve_tool("cat");
    let head = resolve_tool("head");
    // The payload writer thread meets a broken pipe when the child exits
    // early (GNU: the upstream element dies on SIGPIPE); the stage must
    // still report the reader's status with no diagnostics.
    let dir = std::env::temp_dir().join(format!("rubash-issue158-h1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue158 h1 scratch");
    std::fs::write(dir.join("big.txt"), seq_stream()).expect("write big.txt");
    let parent = dir.to_string_lossy().replace('\\', "/");
    let out = run_rubash_script(&format!(
        "cd '{parent}'\n\"{cat}\" big.txt | \"{head}\" -3\necho RC=$?\n"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.timed_out,
        "`cat big.txt | head -3` did not terminate within {RUN_LIMIT:?}"
    );
    assert_eq!(
        out.stdout,
        b"1\n2\n3\nRC=0\n",
        "stdout, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "",
        "a broken-pipe payload write must stay silent like SIGPIPE"
    );
}
