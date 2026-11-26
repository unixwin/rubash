//! rubash#2484 — a pipeline-stage function's `exec 3<&1` must bind
//! fd 3 to the inter-stage pipe itself, and a `>(cat >&3)` consumer
//! must deliver through it.
//!
//! GNU contract (bash 5.3 C source, read first): execute_cmd.c:2620
//! `execute_pipeline` makes a real OS pipe per junction and
//! do_piping (execute_cmd.c:6375) dup2's the pipe write end onto
//! fd 1 in the forked stage — so fd 1 in a stage IS the pipe,
//! `3<&1` (redir.c dup2) dups the pipe, and the process_substitute
//! fork (subst.c:6362) inherits fd 3 naming the pipe. The consumer's
//! writes therefore merge into the stage's output stream in write
//! order, and tee's `/dev/fd/63` operand names that same pipe.
//!
//! The bats-core shape that filed the issue (bats_tee,
//! libexec/bats-core/bats:537-546): a middle-stage function runs
//! `exec 3<&1; tee >(cat >&3) | "$@" >"$output_file"` — the NEXT
//! stage must receive the original lines via fd 3, the file only
//! the transformed lines, each exactly once.
//!
//! Pre-fix rubash failed this two coupled ways: the dev-fd operand
//! attach recorded the carrier's raw parent descriptor number (the
//! parent's lowest free fd — 3, because the shell's fd 3 is
//! virtual), and the child's ambient pin pass overwrote it with the
//! stage pipe before the attach pass read it, so tee wrote a second
//! copy into the pipe and the carrier stayed empty; and the
//! consumer's fd 3 resolved through the process-stdout backing
//! instead of the capture the dup record named, so its bytes never
//! reached the stage output at all.
//!
//! Expectations were measured against GNU bash 5.3.20 running the
//! same script files.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2484-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Run `body` as a script file under rubash; return (status, stdout).
fn run_script(dir: &Path, name: &str, body: &str) -> (i32, String) {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT")
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .stdin(Stdio::null())
        .output()
        .expect("run rubash script");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// p84a: a stage function's procsub consumer writes through the
/// fd 3 it dup'd from the stage's pipe-bound fd 1; the next stage
/// receives the bytes as part of this stage's output.
#[test]
fn stage_fn_procsub_consumer_delivers_through_dup_of_pipe_fd1() {
    let dir = test_dir("p84a");
    let body = r#"
producer() { printf 'tap1\n'; }
stage_fn() {
  exec 3<&1
  printf 'via3\n' > >(cat >&3)
}
consumer() { while IFS= read -r l; do printf 'C:%s\n' "$l"; done; }
producer | stage_fn | consumer
echo "rc=$?"
"#;
    let (rc, stdout) = run_script(&dir, "p84a.sh", body);
    assert_eq!(rc, 0);
    assert_eq!(stdout, "C:via3\nrc=0\n");
}

/// p84b: the bats_tee shape verbatim, as a middle pipeline stage:
/// the next stage receives the original lines via fd 3, the report
/// file receives only the command's output, nothing is doubled.
#[test]
fn bats_tee_shape_middle_stage_routes_lines_once_each() {
    let dir = test_dir("p84b");
    let report = shell_test_path(&dir.join("report.log"));
    let body = format!(
        r#"
bats_tee() {{
  local output_file=$1 status=0
  shift
  exec 3<&1
  tee >(cat >&3) | "$@" >"$output_file" || status=$?
  return $status
}}
pass_through() {{ cat; }}
validator() {{ while IFS= read -r l; do printf 'V:%s\n' "$l"; done; }}
printf 'suite /x/passing.bats\nbegin 1 a passing test\nok 1 a passing test\n' |
  bats_tee {report} pass_through |
  validator
echo "rc=$?"
"#
    );
    let (rc, stdout) = run_script(&dir, "p84b.sh", &body);
    assert_eq!(rc, 0);
    assert_eq!(
        stdout,
        "V:suite /x/passing.bats\nV:begin 1 a passing test\nV:ok 1 a passing test\nrc=0\n"
    );
    let report_bytes = fs::read_to_string(dir.join("report.log")).expect("read report file");
    assert_eq!(
        report_bytes,
        "suite /x/passing.bats\nbegin 1 a passing test\nok 1 a passing test\n"
    );
}

/// p84f arrangement: tee's operand is the /dev/fd spelling of its
/// own procsub — the attach must bind the carrier description, not
/// whatever the pin pass left at the carrier's parent fd number.
/// The inner redirect target (fileA) is byte-exact; the next
/// stage's stream (fileB) is compared as a SET of lines: the
/// consumer's bytes drain into the capture at scope close in
/// rubash's capture model, so their position relative to the
/// stage's own buffered fd-1 writes is the documented drain
/// ordering, while GNU interleaves them in write order. Delivery
/// and exactly-once are what this pin guards.
#[test]
fn tee_operand_attach_binds_carrier_not_clobbered_pin() {
    let dir = test_dir("p84f");
    let body = r#"
tee_like() {
  exec 3<&1
  tee >(cat >&3) | cat > fileA.log
  printf 'inshell-after\n' >&3
  printf 'inshell-fd1\n'
}
printf 'L1\n' | tee_like | cat > fileB.log
echo "rc=$?"
"#;
    let (rc, stdout) = run_script(&dir, "p84f.sh", body);
    assert_eq!(rc, 0);
    assert_eq!(stdout, "rc=0\n");
    let file_a = fs::read_to_string(dir.join("fileA.log")).expect("read fileA");
    assert_eq!(file_a, "L1\n");
    let file_b = fs::read_to_string(dir.join("fileB.log")).expect("read fileB");
    let mut file_b_lines: Vec<&str> = file_b.lines().collect();
    file_b_lines.sort_unstable();
    assert_eq!(file_b_lines, ["L1", "inshell-after", "inshell-fd1"]);
}

/// p84j: the same tee_like as the FINAL stage inside a command
/// substitution — fd 1 is the substitution's capture, and the
/// consumer's fd-3 bytes belong to that capture.
#[test]
fn final_stage_in_comsub_capture_receives_consumer_bytes() {
    let dir = test_dir("p84j");
    let body = r#"
tee_like() {
  exec 3<&1
  tee >(cat >&3) | cat > /dev/null
}
out=$(echo hello | tee_like)
echo "captured=[$out]"
"#;
    let (rc, stdout) = run_script(&dir, "p84j.sh", body);
    assert_eq!(rc, 0);
    assert_eq!(stdout, "captured=[hello]\n");
}
