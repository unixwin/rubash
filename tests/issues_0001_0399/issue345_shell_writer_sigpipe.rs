//! Issue niubash#345 regression: a SHELL-SIDE pipeline producer — a compound
//! command (`while ...; do ...; done`, `for`, `until`, a brace group), a
//! function, or an external writer whose stderr is redirected like
//! `yes 2>/dev/null` — hung forever instead of dying with SIGPIPE when its
//! downstream consumer (`head -1`) exited after taking its fill.
//!
//! Root cause: `concurrent_external_stage_spec` only knows how to wire the
//! concurrent OS-pipe path for plain external members. A compound / builtin /
//! function member fell to the sequential stage loop, which buffered the
//! member's whole stdout in memory (`stdout_capture`), so an unbounded writer
//! never reached the consumer and never observed the broken pipe.
//!
//! GNU baseline: `execute_cmd.c:2620 execute_pipeline` forks every member with
//! fd 1 on a real pipe to the next; the writer is killed by SIGPIPE (status
//! 141) once the reader has exited (the reader closing the last read end makes
//! the write fail with EPIPE). The fix streams the in-process producer's fd 1
//! into a real OS pipe held by the concurrently-spawned downstream consumers.
//!
//! Every test is wrapped in a short deadline because the failure mode is a
//! hang, not a wrong byte.

#![cfg(any(windows, unix))]

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
    let dir = std::env::temp_dir().join(format!("rubash-issue345-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue345 scratch dir");
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
    // after exit would re-create a write-before-drain deadlock whenever the
    // probe's output exceeds the pipe capacity.
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
    #[cfg(windows)]
    let with_exe = format!("{name}.exe");
    #[cfg(windows)]
    let separator = ';';
    #[cfg(unix)]
    let separator = ':';
    for dir in std::env::var("PATH")
        .unwrap_or_default()
        .split(separator)
        .filter(|entry| !entry.is_empty())
    {
        #[cfg(windows)]
        let candidates = [format!("{dir}\\{with_exe}"), format!("{dir}\\{name}")];
        #[cfg(unix)]
        let candidates = [format!("{dir}/{name}")];
        for candidate in candidates {
            let path = PathBuf::from(&candidate);
            if path.is_file() {
                #[cfg(windows)]
                return path.to_string_lossy().replace('\\', "/");
                #[cfg(unix)]
                return path.to_string_lossy().into_owned();
            }
        }
    }
    panic!(
        "issue345 tests need a real external `{name}` on PATH (the SIGPIPE \
         lives in the OS-pipe consumer spawn path)"
    );
}

/// Assert a probe terminates promptly with a zero status and the given stdout.
fn assert_terminates(label: &str, out: &RunOutcome, expected_stdout: &str) {
    assert!(
        !out.timed_out,
        "`{label}` did not terminate within {RUN_LIMIT:?} (niubash#345 shell-writer pipeline hang)"
    );
    assert_eq!(
        out.code,
        Some(0),
        "`{label}` exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        expected_stdout,
        "`{label}` stdout"
    );
}

/// The three minimal repros from the issue body: an external writer, a `while`
/// loop, and a C-style `for` loop, each terminated early by `head -1`.
#[test]
fn minimal_repros_terminate_on_sigpipe() {
    let head = resolve_tool("head");
    let yes = resolve_tool("yes");

    let external = run_rubash_script(&format!("\"{yes}\" 2>/dev/null | \"{head}\" -1\n"));
    assert_terminates("yes 2>/dev/null | head -1", &external, "y\n");

    let while_loop = run_rubash_script(&format!(
        "while :; do echo x; done 2>/dev/null | \"{head}\" -1\n"
    ));
    assert_terminates(
        "while :; do echo x; done 2>/dev/null | head -1",
        &while_loop,
        "x\n",
    );

    let for_loop = run_rubash_script(&format!(
        "for ((i=0;;i++)); do echo x; done 2>/dev/null | \"{head}\" -1\n"
    ));
    assert_terminates(
        "for ((i=0;;i++)); do echo x; done 2>/dev/null | head -1",
        &for_loop,
        "x\n",
    );
}

/// The other compound shapes that funnel through the same producer path: an
/// `until` loop, a brace group, and a shell function.
#[test]
fn compound_and_function_producers_terminate() {
    let head = resolve_tool("head");

    let until_loop = run_rubash_script(&format!("until false; do echo x; done | \"{head}\" -1\n"));
    assert_terminates("until false; do echo x; done | head -1", &until_loop, "x\n");

    let brace = run_rubash_script(&format!(
        "{{ while true; do echo x; done; }} | \"{head}\" -1\n"
    ));
    assert_terminates("{ while true; do echo x; done; } | head -1", &brace, "x\n");

    let function = run_rubash_script(&format!(
        "f() {{ while true; do echo x; done; }}; f | \"{head}\" -1\n"
    ));
    assert_terminates(
        "f() { while true; do echo x; done; }; f | head -1",
        &function,
        "x\n",
    );
}

/// A nested pipeline inside the producing loop still terminates, and the
/// inner pipeline's own pipes are honoured.
///
/// The inner pipeline is builtin-only (`echo x | echo y`) on purpose. It
/// proves two things at once without depending on any external tool: the loop
/// terminates on the consumer's exit, and the inner FIRST stage's `echo x`
/// does NOT leak to the outer pipe — the outer `head -1` must see `y` (the
/// inner last stage), not `x`. A real external child spawned inside the
/// producer body is a separate, pre-existing Windows limitation (it too hung
/// before this fix) and is out of scope for the shell-writer streaming path.
#[test]
fn nested_pipeline_producer_terminates() {
    let head = resolve_tool("head");
    let out = run_rubash_script(&format!(
        "while :; do echo x | echo y; done | \"{head}\" -1\n"
    ));
    assert_terminates("while :; do echo x | echo y; done | head -1", &out, "y\n");
}

/// The producing member must report the SIGPIPE-shaped 141 while the consumer
/// reports 0, matching GNU `PIPESTATUS=(141 0)`.
#[test]
fn compound_producer_pipestatus_is_sigpipe() {
    let head = resolve_tool("head");
    let out = run_rubash_script(&format!(
        "while :; do echo x; done 2>/dev/null | \"{head}\" -1 >/dev/null\n\
         echo \"PS=${{PIPESTATUS[0]}},${{PIPESTATUS[1]}}\"\n"
    ));
    assert_terminates("compound producer PIPESTATUS", &out, "PS=141,0\n");
}

/// A shell-side consumer written as a trivial brace group (`{ head -1; }`) is
/// reduced to its single external command so it joins the concurrent consumer
/// chain instead of the sequential loop re-buffering the producer.
#[test]
fn brace_group_consumer_terminates() {
    let head = resolve_tool("head");
    let cat = resolve_tool("cat");

    let plain = run_rubash_script(&format!(
        "while :; do echo x; done 2>/dev/null | {{ \"{head}\" -1; }}\n"
    ));
    assert_terminates("while :; do echo x; done | { head -1; }", &plain, "x\n");

    let chained = run_rubash_script(&format!(
        "while :; do echo x; done 2>/dev/null | {{ \"{head}\" -1; }} | \"{cat}\"\n"
    ));
    assert_terminates(
        "while :; do echo x; done | { head -1; } | cat",
        &chained,
        "x\n",
    );
}
