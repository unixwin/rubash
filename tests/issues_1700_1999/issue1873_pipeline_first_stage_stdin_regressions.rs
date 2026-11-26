//! rubash#1873: a pipeline's FIRST element that runs in-process
//! (function, brace group, subshell) read EOF immediately instead of
//! the shell's stdin: `main | cat` with the script's stdin a file or
//! pipe gave two instant-EOF reads. GNU execute_pipeline
//! (execute_cmd.c:2620) starts prev at pipe_in — NO_PIPE for a
//! pipeline the enclosing context did not itself pipe into — and
//! do_piping (execute_cmd.c:6375) replaces fd 0 only when
//! pipe_in != NO_PIPE, so element 0 is forked with the shell's
//! current fd 0, whatever it designates: the inherited process
//! stdin, or a file description an `exec < file` / compound bind
//! installed. The stage machinery must keep the FUNCTION_STDIN
//! mirror absent for such an element (the #761 streaming treatment)
//! and let its readers use the fd-table transports, whose offset is
//! the descriptor's own — a stage that reads one line leaves the
//! next line for the shell.
//!
//! Oracle: GNU bash 5.3.20, script file, stdin states file + pipe.

#![cfg(unix)]

use std::io::Write as _;
use std::process::{Command, Stdio};

struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

const INPUT: &[u8] = b"alpha\nbeta\n";

fn prepare(script: &str) -> (ScratchDir, std::path::PathBuf) {
    let dir = scratch_tempdir();
    std::fs::write(dir.0.join("in.txt"), INPUT).expect("write in.txt");
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    (dir, script_path)
}

fn base_command(dir: &ScratchDir, script_path: &std::path::Path) -> Command {
    let mut cmd = Command::new(rubash_bin());
    cmd.arg(script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0);
    cmd
}

/// Run the script with its stdin a regular file holding INPUT.
fn run_stdin_file(script: &str) -> (String, String, bool) {
    let (dir, script_path) = prepare(script);
    let in_file = std::fs::File::open(dir.0.join("in.txt")).expect("open in.txt");
    let output = base_command(&dir, &script_path)
        .stdin(Stdio::from(in_file))
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

/// Run the script with its stdin a pipe fed INPUT.
fn run_stdin_pipe(script: &str) -> (String, String, bool) {
    let (dir, script_path) = prepare(script);
    let mut child = base_command(&dir, &script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(INPUT)
        .expect("feed stdin pipe");
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

/// Run the script with stdin /dev/null (for shapes whose input is
/// bound inside the script: exec / compound redirects).
fn run_stdin_null(script: &str) -> (String, String, bool) {
    let (dir, script_path) = prepare(script);
    let output = base_command(&dir, &script_path)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

const FN_STAGE: &str = "main() {\n  read -r a; echo \"read1: [$a] rc=$?\"\n  read -r b; echo \"read2: [$b] rc=$?\"\n}\nmain | cat\n";
const FN_EXPECTED: &str = "read1: [alpha] rc=0\nread2: [beta] rc=0\n";

#[test]
fn issue1873_function_first_stage_reads_stdin_file() {
    let (stdout, stderr, ok) = run_stdin_file(FN_STAGE);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, FN_EXPECTED);
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_function_first_stage_reads_stdin_pipe() {
    let (stdout, stderr, ok) = run_stdin_pipe(FN_STAGE);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, FN_EXPECTED);
    assert_eq!(stderr, "");
}

const GROUP_STAGE: &str = "{ read -r a; echo \"group read1: [$a] rc=$?\"; read -r b; echo \"group read2: [$b] rc=$?\"; } | cat\n";
const GROUP_EXPECTED: &str = "group read1: [alpha] rc=0\ngroup read2: [beta] rc=0\n";

#[test]
fn issue1873_group_first_stage_reads_stdin_file() {
    let (stdout, stderr, ok) = run_stdin_file(GROUP_STAGE);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, GROUP_EXPECTED);
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_group_first_stage_reads_stdin_pipe() {
    let (stdout, stderr, ok) = run_stdin_pipe(GROUP_STAGE);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, GROUP_EXPECTED);
    assert_eq!(stderr, "");
}

const SUBSHELL_STAGE: &str = "( read -r a; echo \"subshell read1: [$a] rc=$?\"; read -r b; echo \"subshell read2: [$b] rc=$?\" ) | cat\n";
const SUBSHELL_EXPECTED: &str = "subshell read1: [alpha] rc=0\nsubshell read2: [beta] rc=0\n";

#[test]
fn issue1873_subshell_first_stage_reads_stdin_file() {
    let (stdout, stderr, ok) = run_stdin_file(SUBSHELL_STAGE);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, SUBSHELL_EXPECTED);
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_subshell_first_stage_reads_stdin_pipe() {
    let (stdout, stderr, ok) = run_stdin_pipe(SUBSHELL_STAGE);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, SUBSHELL_EXPECTED);
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_while_loop_first_stage_reads_stdin_file() {
    // The junit/bats consumer shape: a while-read loop as element 0
    // drains the shell's stdin line by line.
    let script = "main() { while IFS= read -r l; do echo \"line: $l\"; done; }\nmain | cat\n";
    let (stdout, stderr, ok) = run_stdin_file(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "line: alpha\nline: beta\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_first_stage_junit_header_shape() {
    // Minimal junit-formatter shape (w3-1720-final evidence): the
    // first-stage function reads a header line, a downstream external
    // consumer transforms the element's output.
    let script =
        "main() { IFS= read -r header; echo \"header: [$header]\"; }\nmain | tr 'a-z' 'A-Z'\n";
    let (stdout, stderr, ok) = run_stdin_pipe(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "HEADER: [ALPHA]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_first_stage_read_shares_offset_with_shell_file() {
    // GNU's shared open file description: the stage consumes the
    // first line; the shell's next read continues at the second.
    let script = "main() { IFS= read -r l; printf 'stage:[%s]\\n' \"$l\"; }\nmain | cat\nIFS= read -r rest\nprintf 'shell:[%s]\\n' \"$rest\"\n";
    let (stdout, stderr, ok) = run_stdin_file(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "stage:[alpha]\nshell:[beta]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_first_stage_read_shares_offset_with_shell_pipe() {
    let script = "main() { IFS= read -r l; printf 'stage:[%s]\\n' \"$l\"; }\nmain | cat\nIFS= read -r rest\nprintf 'shell:[%s]\\n' \"$rest\"\n";
    let (stdout, stderr, ok) = run_stdin_pipe(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "stage:[alpha]\nshell:[beta]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_exec_bound_function_first_stage() {
    // `exec < file` installs fd 0 as a file description; a first-stage
    // function reads it (at its shared offset), not an empty buffer.
    let script =
        "main() { IFS= read -r l; printf 'got:[%s]\\n' \"$l\"; }\nexec < in.txt\nmain | cat\n";
    let (stdout, stderr, ok) = run_stdin_null(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "got:[alpha]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_exec_bound_continuity() {
    let script = "main() { IFS= read -r l; printf 'stage:[%s]\\n' \"$l\"; }\nexec < in.txt\nmain | cat\nIFS= read -r rest\nprintf 'shell:[%s]\\n' \"$rest\"\n";
    let (stdout, stderr, ok) = run_stdin_null(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "stage:[alpha]\nshell:[beta]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_compound_bound_pipeline_first_stage() {
    // The pipeline itself carries the `< in.txt` bind: the bind lands
    // on fd 0 as a file endpoint inside the pipeline's subshell.
    let script =
        "main() { IFS= read -r l; printf 'got:[%s]\\n' \"$l\"; }\n{ main | cat; } < in.txt\n";
    let (stdout, stderr, ok) = run_stdin_null(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "got:[alpha]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_subshell_bound_pipeline_first_stage() {
    let script =
        "main() { IFS= read -r l; printf 'got:[%s]\\n' \"$l\"; }\n( main | cat ) < in.txt\n";
    let (stdout, stderr, ok) = run_stdin_null(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "got:[alpha]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_middle_stage_still_reads_pipe_not_shell_stdin() {
    // Control: a non-first element's fd 0 is the pipe (do_piping with
    // pipe_in != NO_PIPE) — an empty upstream yields EOF even though
    // the shell's own stdin holds lines.
    let script = "main() { IFS= read -r l; printf 'mid:[%s] rc=%s\\n' \"$l\" \"$?\"; }\nprintf '' | main | cat\n";
    let (stdout, stderr, ok) = run_stdin_file(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "mid:[] rc=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_middle_and_last_stage_controls() {
    // Controls: pipe-fed middle and last elements read the pipe
    // payload (unchanged by the stage-0 classification).
    let mid = "main() { IFS= read -r l; printf 'mid:[%s]\\n' \"$l\"; }\nprintf 'alpha\\nbeta\\n' | main | cat\n";
    let (stdout, stderr, ok) = run_stdin_null(mid);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "mid:[alpha]\n");
    assert_eq!(stderr, "");

    let last = "main() { IFS= read -r l; printf 'last:[%s]\\n' \"$l\"; }\nprintf 'alpha\\nbeta\\n' | main\n";
    let (stdout, stderr, ok) = run_stdin_null(last);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "last:[alpha]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1873_first_stage_own_redirect_still_wins() {
    // Adjacent #1823 shape: the element's own `< file` binding is a
    // payload source and must keep winning over the shell's stdin.
    let script = "main() { IFS= read -r l; printf 'own:[%s]\\n' \"$l\"; }\nmain < in.txt | cat\n";
    let (stdout, stderr, ok) = run_stdin_null(script);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "own:[alpha]\n");
    assert_eq!(stderr, "");
}
