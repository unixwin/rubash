//! Issue unixwin/rubash#2184: an output process substitution in
//! REDIRECT position on a pipeline stage delivered zero bytes,
//! deterministically, at any payload size:
//! `echo hi | cat > >(wc -c)` printed 0 where GNU bash 5.3.20 prints 3.
//!
//! GNU owner: subst.c process_substitute() materializes `>(...)` as a
//! pipe and execute_cmd.c execute_pipeline runs each element with its
//! own redirections applied, so the element's redirected fd IS the
//! substitution pipe and the payload flows during the element. Rubash's
//! carrier model creates an empty temp carrier at materialization and
//! spawns the consumer at stage end to read it — but the pipeline stage
//! runners capture the stage's streams as strings and leave placement
//! to the driver loop's routing walk, which re-resolves the ORIGINAL
//! stage node: the raw `>(...)` target text resolved to a literal file
//! of that name in the cwd (verified: a 3-byte file named `>(wc -c)`
//! after the base probe) and the consumer read the still-empty
//! carrier. The fix routes the captured streams inside
//! execute_pipeline_stage, through the MATERIALIZED node's redirect
//! state, before the consumer spawns — for exactly the stages the
//! driver would have routed (external/builtin capture-model stages;
//! function and compound stages place their own streams and are
//! untouched).
//!
//! Boundary, probed on master and after the fix: input process
//! substitutions `<(...)` on stages (redirect and word position)
//! already delivered and are pinned below as boundary guards.
//! A `2>&1` dup ONTO a carrier (`> >(X) 2>&1`) loses the payload on
//! EVERY path including simple commands — a separate pre-existing
//! carrier/dup defect, deliberately not pinned here; the reverse
//! order (`2>&1 > >(X)`) works and is pinned. Same-fd multi-redirect
//! shapes (`> >(X) > file`, `> file > >(X)`) are likewise excluded:
//! GNU's consumer inherits the fd state as of its position in the
//! redirect sequence, which rubash's consumer spawn does not model
//! on any path (pre-existing, simple commands included).
//!
//! Expectations verified against GNU bash 5.3.20 (script-file probes).

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn rubash_c(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c probe");
    (output.stdout, output.stderr, output.status.code())
}

fn assert_stdout(script: &str, expected: &[u8]) {
    let (stdout, stderr, code) = rubash_c(script);
    assert_eq!(code, Some(0), "script {script:?}");
    assert_eq!(stdout, expected, "script {script:?}");
    assert_eq!(
        stderr,
        b"",
        "script {script:?} stderr {:?}",
        String::from_utf8_lossy(&stderr)
    );
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-2184-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run with a hard deadline; a hang is a test failure, never a frozen
/// suite.
fn rubash_deadline(
    dir: &PathBuf,
    script: &str,
    deadline: Duration,
) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let t0 = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if t0.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash did not finish within {deadline:?} — HANG regression for script: {script}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_end(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        let _ = se.read_to_end(&mut err);
    }
    let status = child.wait().expect("wait");
    (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
        status.code(),
    )
}

/// The substitution body is a real child process; its file write can
/// land a moment after the shell exits. Poll briefly for the content.
fn wait_file(dir: &PathBuf, name: &str, expected: &str) -> String {
    let path = dir.join(name);
    let t0 = Instant::now();
    loop {
        let got = std::fs::read_to_string(&path).unwrap_or_default();
        if got == expected || t0.elapsed() > Duration::from_secs(5) {
            return got;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_file_contains(dir: &PathBuf, name: &str, needle: &str) -> String {
    let path = dir.join(name);
    let t0 = Instant::now();
    loop {
        let got = std::fs::read_to_string(&path).unwrap_or_default();
        if got.contains(needle) || t0.elapsed() > Duration::from_secs(5) {
            return got;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn issue2184_last_stage_output_procsub_receives_stage_stdout() {
    assert_stdout("echo hi | cat > >(wc -c)", b"3\n");
}

#[test]
fn issue2184_middle_stage_output_procsub_receives_stage_stdout() {
    assert_stdout("printf 'abc\\n' | grep b > >(wc -c) | tr a-z A-Z", b"4\n");
}

#[test]
fn issue2184_first_stage_output_procsub_receives_stage_stdout() {
    assert_stdout("cat > >(wc -c) <<< 'hi' | tr a-z A-Z", b"3\n");
}

#[test]
fn issue2184_append_form_output_procsub_on_stage() {
    assert_stdout("echo hi | cat >> >(wc -c)", b"3\n");
}

#[test]
fn issue2184_builtin_stage_output_procsub() {
    assert_stdout("echo zz | echo stageout > >(wc -c)", b"9\n");
}

#[test]
fn issue2184_function_stage_output_procsub() {
    assert_stdout("f() { cat; }; echo hi | f > >(wc -c)", b"3\n");
}

#[test]
fn issue2184_compound_stage_output_procsub() {
    assert_stdout("echo hi | { cat; } > >(wc -c)", b"3\n");
    assert_stdout("echo hi | (cat) > >(wc -c)", b"3\n");
}

#[test]
fn issue2184_stage_with_additional_stderr_redirect() {
    assert_stdout("echo hi | cat > >(wc -c) 2>/dev/null", b"3\n");
}

#[test]
fn issue2184_word_and_redirect_procsubs_on_one_stage() {
    // The word-position consumer and the redirect-position consumer
    // each receive the full stage output; both print "3", so the
    // capture is order-insensitive by construction.
    assert_stdout("echo hi | tee >(wc -c) > >(wc -c)", b"3\n3\n");
}

#[test]
fn issue2184_dup_before_procsub_redirect_on_stage() {
    // `2>&1` BEFORE the procsub redirect dups the pipe end; the
    // consumer still receives the stage's stdout in full.
    assert_stdout("echo hi | cat 2>&1 > >(wc -c)", b"3\n");
}

#[test]
fn issue2184_function_stage_stderr_procsub() {
    assert_stdout("f() { echo E >&2; }; echo hi | f 2> >(wc -c)", b"2\n");
}

#[test]
fn issue2184_large_payload_crosses_pipe_buffer() {
    // 108890 bytes: past the 65536-byte pipe buffer that was the
    // abandoned-feed signature in #2179, so a partial transfer
    // cannot pass.
    assert_stdout(
        "i=0; while [ $i -lt 12000 ]; do echo line$i; i=$((i+1)); done | cat > >(wc -c) | cat",
        b"108890\n",
    );
}

#[test]
fn issue2184_input_procsub_on_stage_boundary() {
    // Boundary guards: input substitutions on stages already
    // delivered before this fix and must keep delivering.
    assert_stdout("echo pipe | cat < <(printf 'in\\n')", b"in\n");
    assert_stdout("cat < <(printf 'in\\n') | wc -c", b"3\n");
}

#[test]
fn issue2184_no_literal_procsub_file_created() {
    // The pre-fix routing walk wrote the payload into a literal file
    // named after the raw `>(...)` text in the cwd.
    let dir = scratch("literal");
    let (stdout, stderr, code) = rubash_deadline(
        &dir,
        "echo hi | cat > >(wc -c)",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "3\n");
    let entries: Vec<String> = std::fs::read_dir(&dir)
        .expect("read scratch dir")
        .map(|entry| entry.expect("dir entry").file_name().to_string_lossy().into_owned())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        entries.is_empty(),
        "stage routing must not create files in the cwd, found: {entries:?}"
    );
}

#[test]
fn issue2184_consumer_file_receives_stage_payload() {
    let dir = scratch("consumer-file");
    let (stdout, stderr, code) = rubash_deadline(
        &dir,
        "echo hi | cat > >(cat > got.txt) | tr a-z A-Z",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "", "payload must not leak to the next stage");
    let got = wait_file(&dir, "got.txt", "hi\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got, "hi\n");
}

#[test]
fn issue2184_external_stage_stderr_procsub_delivers() {
    let dir = scratch("stderr");
    let (stdout, stderr, code) = rubash_deadline(
        &dir,
        "echo x | cat /nonexistent-2184 2> >(cat > err.txt) | tr a-z A-Z",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "", "the diagnostic belongs to the consumer");
    let got = wait_file_contains(&dir, "err.txt", "No such file or directory");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        got.contains("No such file or directory"),
        "consumer must receive the stage's stderr, got: {got:?}"
    );
}
