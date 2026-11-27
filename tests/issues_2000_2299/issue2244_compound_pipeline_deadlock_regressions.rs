//! Issue unixwin/rubash#2244: compound pipeline element feeding an
//! external consumer deadlocked past ~128 KiB of output.
//!
//! GNU forks every pipeline element (execute_cmd.c:2620
//! execute_pipeline), so a producer's output streams through real pipes
//! and is drained concurrently by construction. Rubash's shell-writer
//! pipeline runs the compound producer in-process into pipe[0] while the
//! external consumers are real processes; the last member's captured
//! stdout/stderr pipes must therefore be drained BEFORE the producer
//! runs (the niubash#158 discipline). The pre-fix code spawned those
//! drain threads only after the producer returned: past pipe[0]'s
//! 64 KiB plus the capture pipe's 64 KiB the consumer blocked in pipe
//! write, stopped reading, and the producer blocked forever.
//!
//! Verified against GNU Bash 5.3.20 (script-file probes in the cluster
//! workdir): every shape below completes there byte-identically; on the
//! pre-fix master every compound/function shape hangs (timeout-killed,
//! zero bytes delivered) while the single-builtin shape passes.
//!
//! Harness note: every pin runs rubash as a subprocess on a script FILE
//! under a hard deadline; a child that misses the deadline is killed
//! and the pin fails — the pre-fix failure mode is a hang, not a wrong
//! byte, so no pin may wait unboundedly.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(60);

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2244-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// Run `script` as a file under the rubash binary; returns
/// (stdout, stderr, status) or panics after killing a child that
/// exceeds DEADLINE (the deadlock shape).
fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = test_dir(tag);
    let file = dir.join("case.sh");
    std::fs::write(&file, script).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait().expect("try_wait rubash") {
            Some(status) => break status,
            None => {
                if started.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "issue2244 pin {tag}: rubash did not finish within {DEADLINE:?} — \
                         compound-pipeline deadlock (pre-fix shape)"
                    );
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out_thread.join().expect("stdout drain")).into_owned();
    let stderr = String::from_utf8_lossy(&err_thread.join().expect("stderr drain")).into_owned();
    (stdout, stderr, status.code())
}

/// One producer line: `line-%04d-` + 87 x's + '\n' = exactly 98 bytes.
fn line(i: usize) -> String {
    format!("line-{i:04}-{}", "x".repeat(87))
}

fn expected_output(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 98);
    for i in 0..lines {
        out.push_str(&line(i));
        out.push('\n');
    }
    out
}

/// The loop body shared by the compound producer shapes (4000 lines =
/// 392,000 bytes, ~3x the old 131,072-byte wedge point).
fn loop_body() -> String {
    "i=0; while [ $i -lt 4000 ]; do printf 'line-%04d-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' \"$i\"; i=$((i+1)); done"
        .to_string()
}

fn assert_shape(tag: &str, script: &str) {
    let (stdout, stderr, code) = run_script(tag, script);
    assert_eq!(
        code,
        Some(0),
        "issue2244 {tag}: exit code (stderr: {stderr})"
    );
    assert_eq!(
        stdout,
        expected_output(4000),
        "issue2244 {tag}: payload must match GNU byte-for-byte ({} bytes)",
        stdout.len()
    );
}

#[test]
fn issue2244_brace_group_producer_over_128kib_completes() {
    let script = format!("{{ {}; }} | cat\n", loop_body());
    assert_shape("brace", &script);
}

#[test]
fn issue2244_subshell_producer_over_128kib_completes() {
    let script = format!("( {} ) | cat\n", loop_body());
    assert_shape("subshell", &script);
}

#[test]
fn issue2244_function_producer_over_128kib_completes() {
    let script = format!("f() {{ {}; }}; f | cat\n", loop_body());
    assert_shape("function", &script);
}

#[test]
fn issue2244_while_loop_stage_producer_over_128kib_completes() {
    let script = format!("{} | cat\n", loop_body());
    assert_shape("whileloop", &script);
}

#[test]
fn issue2244_three_stage_compound_producer_over_128kib_completes() {
    let script = format!("{{ {}; }} | cat | cat\n", loop_body());
    assert_shape("threestage", &script);
}

#[test]
fn issue2244_compound_producer_near_1mib_completes() {
    // 11000 lines x 98 bytes = 1,078,000 bytes: the fix must stream at
    // any size, not merely raise the wedge threshold.
    let body = loop_body().replace("4000", "11000");
    let script = format!("{{ {}; }} | cat\n", body);
    let (stdout, stderr, code) = run_script("nearmib", &script);
    assert_eq!(
        code,
        Some(0),
        "issue2244 near-1MiB: exit code (stderr: {stderr})"
    );
    assert_eq!(
        stdout,
        expected_output(11000),
        "issue2244 near-1MiB: payload must match GNU byte-for-byte ({} bytes)",
        stdout.len()
    );
}

#[test]
fn issue2244_compound_producer_inside_comsub_collects_over_128kib() {
    // The Collect drain face: inside $(...) the last member's payload is
    // collected, not streamed to the host — it wedged the same way.
    // Command substitution strips the one trailing newline, so the
    // captured length is 392,000 - 1.
    let script = format!("out=$( {{ {}; }} | cat ); echo ${{#out}}\n", loop_body());
    let (stdout, stderr, code) = run_script("comsub", &script);
    assert_eq!(
        code,
        Some(0),
        "issue2244 comsub: exit code (stderr: {stderr})"
    );
    assert_eq!(stdout, "391999\n", "issue2244 comsub: captured length");
}

#[test]
fn issue2244_unbounded_producer_still_dies_sigpipe_shaped() {
    // The EPIPE arm of the same producer path: an unbounded compound
    // producer feeding a bounding consumer must terminate with the
    // SIGPIPE-shaped producer status 141, not hang (GNU jobs.c:2958).
    let script = "{ while :; do echo x; done; } | head -3; echo \"ps=${PIPESTATUS[*]}\"\n";
    let (stdout, stderr, code) = run_script("sigpipe", script);
    assert_eq!(
        code,
        Some(0),
        "issue2244 sigpipe: exit code (stderr: {stderr})"
    );
    assert_eq!(stdout, "x\nx\nx\nps=141 0\n", "issue2244 sigpipe: shape");
}
