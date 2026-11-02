//! rubash#2483 — pipeline stream ORDER: a member's output must
//! interleave with the other streams in production order, not in
//! stage-completion order.
//!
//! GNU contract (bash 5.3 C source, read first): execute_cmd.c:2620
//! `execute_pipeline` forks EVERY element onto real pipes
//! (do_piping, execute_cmd.c:2741 binds the element's fd 1 to the
//! pipe), so bytes flow as produced and a member's direct fd-2
//! writes land on the shared stream when they happen; a builtin or
//! function running as an element flushes its stdout into the bound
//! fd at every simple-command boundary (execute_cmd.c:5550
//! `fflush (stdout)` in execute_subshell_builtin_or_function;
//! execute_cmd.c:5648 `fflush (stdout); fpurge (stdout);` in
//! execute_builtin_or_function). There is no capture-then-emit step.
//!
//! The bats cells 115/116 shape that filed the issue: an external
//! producer | a shell FUNCTION validator | an external formatter,
//! where the producer's late direct stderr line (`Marking test run
//! as failed...`) must land AFTER the TAP stream it brackets. The
//! sequential stage loop handed the function the producer's complete
//! captured output, so the late stderr line (written live) overtook
//! the pipeline's first bytes.
//!
//! Every pin below merges the shell's stdout and stderr onto ONE
//! file (both Stdio handles clone the same File) so the recorded
//! line order is the true write order, and separates the events by
//! 0.4s sleeps so the expected order is scheduling-independent.
//! Expectations were measured against GNU bash 5.3.20 running the
//! same script files.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2483-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Run `body` as a script file under rubash with stdout and stderr
/// merged onto one file; return the merged lines in write order.
fn run_merged(dir: &Path, name: &str, body: &str) -> Vec<String> {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    let merged_path = dir.join("merged.txt");
    let merged = fs::File::create(&merged_path).expect("create merged capture");
    let merged_err = merged.try_clone().expect("clone merged capture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT")
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .stdin(Stdio::null())
        .stdout(Stdio::from(merged))
        .stderr(Stdio::from(merged_err))
        .output()
        .expect("run script");
    assert!(
        output.status.success(),
        "script failed: {output:?}; merged: {}",
        fs::read_to_string(&merged_path).unwrap_or_default()
    );
    fs::read_to_string(&merged_path)
        .expect("read merged capture")
        .lines()
        .map(str::to_string)
        .collect()
}

fn write_helper(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, body).expect("write helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod helper");
    }
    shell_test_path(&path)
}

/// The bats-shaped producer: a direct stderr line first, two TAP
/// lines spaced out, then the late direct stderr line that must NOT
/// overtake the TAP stream.
const PRODUCER: &str = "#!/bin/sh\necho err-early >&2\nsleep 0.4\necho tap1\nsleep 0.4\necho tap2\nsleep 0.4\necho err-late >&2\n";

/// The bats-shaped formatter: prefix each line as it arrives.
const FORMATTER: &str = "#!/bin/sh\nwhile IFS= read -r line; do echo \"V:$line\"; done\n";

// 01 — ext | fn | ext (the t115/t116 kernel): the validator function
// sits between two external members. GNU order: the producer's
// early stderr, both transformed TAP lines, then the late stderr.
#[test]
fn issue2483_01_external_function_external_interleave() {
    let dir = test_dir("01");
    let producer = write_helper(&dir, "producer.sh", PRODUCER);
    let formatter = write_helper(&dir, "formatter.sh", FORMATTER);
    let body = format!(
        "validate() {{ while IFS= read -r line; do echo \"F:$line\"; done; }}\n{producer} | validate | {formatter}\n"
    );
    let lines = run_merged(&dir, "case.sh", &body);
    assert_eq!(
        lines,
        ["err-early", "V:F:tap1", "V:F:tap2", "err-late"],
        "merged stream order"
    );
}

// 02 — ext | fn tail: the function's output is the pipeline's
// output; the producer's late stderr still lands after both TAP
// lines (the streaming prefix must not finish-and-dump first).
#[test]
fn issue2483_02_external_function_tail_interleave() {
    let dir = test_dir("02");
    let producer = write_helper(&dir, "producer.sh", PRODUCER);
    let body = format!(
        "validate() {{ while IFS= read -r line; do echo \"F:$line\"; done; }}\n{producer} | validate\n"
    );
    let lines = run_merged(&dir, "case.sh", &body);
    assert_eq!(
        lines,
        ["err-early", "F:tap1", "F:tap2", "err-late"],
        "merged stream order"
    );
}

// 03 — fn | ext: the shell-side producer's own stderr is live too.
// Its early line precedes the first formatted line; its late line
// (written between the two TAP lines) lands between them — under
// capture-then-emit both stderr lines used to arrive at pipeline
// end, after the whole formatted stream.
#[test]
fn issue2483_03_function_producer_live_stderr() {
    let dir = test_dir("03");
    let formatter = write_helper(&dir, "formatter.sh", FORMATTER);
    let body = format!(
        "produce() {{ echo err-early >&2; echo tap1; sleep 0.4; echo err-late >&2; echo tap2; }}\nproduce | {formatter}\n"
    );
    let lines = run_merged(&dir, "case.sh", &body);
    assert_eq!(
        lines,
        ["err-early", "V:tap1", "err-late", "V:tap2"],
        "merged stream order"
    );
}

// 04 — ext | compound | ext: a brace-group validator streams the
// same way a function does (one shell-side stage, either funnel).
#[test]
fn issue2483_04_external_compound_external_interleave() {
    let dir = test_dir("04");
    let producer = write_helper(&dir, "producer.sh", PRODUCER);
    let formatter = write_helper(&dir, "formatter.sh", FORMATTER);
    let body = format!(
        "{producer} | {{ while IFS= read -r line; do echo \"C:$line\"; done; }} | {formatter}\n"
    );
    let lines = run_merged(&dir, "case.sh", &body);
    assert_eq!(
        lines,
        ["err-early", "V:C:tap1", "V:C:tap2", "err-late"],
        "merged stream order"
    );
}

// 05 — control: fn | fn keeps correct CONTENT and PIPESTATUS. Two
// shell-side stages still run on the sequential stage loop (the
// executor is not movable across threads — its fd endpoints are
// Rc-based), so no interleave order is claimed here; the streams
// and statuses must stay exact.
#[test]
fn issue2483_05_function_function_content_and_status() {
    let dir = test_dir("05");
    let body = "produce() { echo tap1; echo tap2; }\n\
                validate() { while IFS= read -r line; do echo \"F:$line\"; done; }\n\
                produce | validate\n\
                echo \"pipestatus=${PIPESTATUS[*]}\"\n";
    let script = dir.join("case.sh");
    fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT")
        .current_dir(&dir)
        .arg(shell_test_path(&script))
        .stdin(Stdio::null())
        .output()
        .expect("run script");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "F:tap1\nF:tap2\npipestatus=0 0\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

// 06 — exec | fn | ext: the bats pipeline itself is spelled with an
// `exec` prefix on its first element (`exec bats-exec-suite |
// validator | formatter`). GNU forks the element before its command
// runs (execute_cmd.c:2620), so the element IS the exec'd program
// with the pipe already bound; the stream order is the plain
// sandwich order and PIPESTATUS reports the real member statuses.
#[test]
fn issue2483_06_exec_prefixed_producer_interleave() {
    let dir = test_dir("06");
    let producer = write_helper(&dir, "producer.sh", PRODUCER);
    let formatter = write_helper(&dir, "formatter.sh", FORMATTER);
    let body = format!(
        "validate() {{ while IFS= read -r line; do echo \"F:$line\"; done; }}\nexec {producer} | validate | {formatter}\n"
    );
    let lines = run_merged(&dir, "case.sh", &body);
    assert_eq!(
        lines,
        ["err-early", "V:F:tap1", "V:F:tap2", "err-late"],
        "merged stream order"
    );

    let body = format!(
        "validate() {{ while IFS= read -r line; do echo \"F:$line\"; done; }}\nexec {producer} | validate | {formatter}\necho \"pipestatus=${{PIPESTATUS[*]}}\"\n"
    );
    let script = dir.join("status.sh");
    fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT")
        .current_dir(&dir)
        .arg(shell_test_path(&script))
        .stdin(Stdio::null())
        .output()
        .expect("run script");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).ends_with("pipestatus=0 0 0\n"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
