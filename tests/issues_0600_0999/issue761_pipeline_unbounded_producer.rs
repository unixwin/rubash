//! Issue rubash#761 regression: an UNBOUNDED external producer feeding a
//! pipeline whose rightmost stage is served in-shell never terminated — the
//! shell drained the producer into RAM (~250 MB/s) instead of closing the
//! read end and letting it die of SIGPIPE. `yes | true` is the minimal case.
//!
//! Root cause: the rightmost element ran only after the sequential stage loop
//! materialized every upstream element's whole stdout
//! (`execute_external_pipeline_stage_inner` -> `wait_with_output`); an
//! unbounded producer never reaches EOF, so the tail never started and the
//! producer never observed a closed consumer. The materializing
//! `execute_external_prefix_concurrently` path had the same defect one level
//! up: `yes | cat | true` still drained `cat`'s output to completion. The fix
//! runs the plain-external prefix on real OS pipes and hands the last pipe's
//! read end to the in-shell tail as fd 0, so when the tail stops reading its
//! read end closes and the producers are reaped with the SIGPIPE-shaped 141.
//!
//! GNU baseline: execute_cmd.c:2620 execute_pipeline forks every element;
//! the rightmost element's exit closes its read end and the writer dies on
//! SIGPIPE (PIPESTATUS[0]=141, the tail's own status second) — `yes | true`
//! returns 0 with `PIPESTATUS=(141 0)`. This is the same absence as #345
//! (shell-side writer) from the opposite end; the shape was not covered by
//! #723's 190-run negative matrix because none of its 13 shapes used an
//! inline-arm / builtin last stage.
//!
//! Every test is wrapped in a short deadline because the failure mode is a
//! hang, not a wrong byte.

#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Deadline for one pipeline run. The fixed binary finishes every shape here
/// in well under a second; the pre-fix binary never returns.
const RUN_LIMIT: Duration = Duration::from_secs(20);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Run one probe script with a sandboxed CWD (a fresh temp dir), so the probe
/// can never write into the repository root and its relative `> f` shapes stay
/// contained.
fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue761-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue761 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    // LF line endings: the repo's .gitattributes pins *.sh eol=lf.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
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
        "issue761 tests need a real external `{name}` on PATH (the SIGPIPE \
         lives in the OS-pipe producer spawn path)"
    );
}

fn assert_terminates_ok(label: &str, out: &RunOutcome) {
    assert!(
        !out.timed_out,
        "`{label}` did not terminate within {RUN_LIMIT:?} (rubash#761 unbounded-producer hang)"
    );
    assert_eq!(
        out.code,
        Some(0),
        "`{label}` exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The issue's minimal repros: an unbounded external producer feeding a tail
/// that is served in-shell (a builtin / inline arm), which must terminate on
/// the producer's SIGPIPE rather than draining it into RAM.
#[test]
fn unbounded_producer_with_inline_builtin_tail_terminates() {
    let yes = resolve_tool("yes");
    for (label, tail, expected_code) in [
        ("yes | true", "true", 0),
        ("yes | :", ":", 0),
        // `false` is the one tail whose own status is non-zero; the point is
        // that it still terminates (the producer dies, the tail's status wins).
        ("yes | false", "false", 1),
        ("yes | read -r l", "read -r l", 0),
        ("yes | echo hi", "echo hi", 0),
        ("yes | printf x", "printf x", 0),
    ] {
        let out = run_rubash_script(&format!("\"{yes}\" | {tail}\n"));
        assert!(
            !out.timed_out,
            "`{label}` did not terminate within {RUN_LIMIT:?} (rubash#761 unbounded-producer hang)"
        );
        assert_eq!(
            out.code,
            Some(expected_code),
            "`{label}` exit status, stderr: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// The defect is not limited to a single external prefix: two externals
/// before the shell tail (`yes | cat | true`) also hung, because the
/// materializing prefix path drained the last producer to completion.
#[test]
fn unbounded_producer_through_second_external_terminates() {
    let yes = resolve_tool("yes");
    let cat = resolve_tool("cat");
    let out = run_rubash_script(&format!("\"{yes}\" | \"{cat}\" | true\n"));
    assert_terminates_ok("yes | cat | true", &out);
}

/// A compound last stage is served in-shell too; `break` must stop the body
/// and close the producer's read end (`yes | while read; do break; done`).
#[test]
fn unbounded_producer_with_compound_tail_terminates() {
    let yes = resolve_tool("yes");
    let break_loop = run_rubash_script(&format!("\"{yes}\" | while read -r l; do break; done\n"));
    assert_terminates_ok("yes | while read; do break; done", &break_loop);

    let brace = run_rubash_script(&format!(
        "\"{yes}\" | {{ read -r a; read -r b; echo \"a=$a b=$b\"; }}\n"
    ));
    assert_terminates_ok("yes | { read -r a; read -r b; echo ...; }", &brace);
    assert_eq!(
        String::from_utf8_lossy(&brace.stdout),
        "a=y b=y\n",
        "the compound tail must still read the producer's stream"
    );
}

/// The producer must be reported with the SIGPIPE-shaped 141 while the tail
/// reports its own status, matching GNU `PIPESTATUS=(141 0)`.
#[test]
fn unbounded_producer_pipestatus_is_sigpipe() {
    let yes = resolve_tool("yes");
    let out = run_rubash_script(&format!(
        "\"{yes}\" | true\n echo \"PS=${{PIPESTATUS[0]}},${{PIPESTATUS[1]}}\"\n"
    ));
    assert_terminates_ok("yes | true PIPESTATUS", &out);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "PS=141,0\n",
        "GNU anchor: `yes | true` reports PIPESTATUS (141 0)"
    );
}

/// A bounded-but-huge producer is killed as soon as the tail exits instead of
/// being drained (`seq 1 100000000 | true`, the issue's long case).
#[test]
fn huge_bounded_producer_terminates() {
    let seq = resolve_tool("seq");
    let out = run_rubash_script(&format!("\"{seq}\" 1 100000000 | true\n"));
    assert_terminates_ok("seq 1 100000000 | true", &out);
}

/// Negative control: a tail that bounds the producer keeps returning
/// immediately with its data (`yes | head -1` -> "y").
#[test]
fn bounded_head_negative_control_still_returns() {
    let yes = resolve_tool("yes");
    let out = run_rubash_script(&format!("\"{yes}\" | head -1\n"));
    assert_terminates_ok("yes | head -1", &out);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "y\n",
        "the bounding consumer must still receive the producer's first line"
    );
}
