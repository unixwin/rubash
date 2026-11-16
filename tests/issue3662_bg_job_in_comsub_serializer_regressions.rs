//! Issue rubash#3662 regressions.
//!
//! A background job (`&`) inside a command substitution in a defined
//! function's body never ran: the background child is a respawned
//! `rubash -c` whose program is the defined functions serialized
//! `;`-joined plus the job's own command, and the canonical comsub
//! printer (`print_comsub.rs`, GNU print_cmd.c port) emitted `& ;` for
//! the background operator — `&` followed by `;` is a syntax error on
//! both shells (parse.y:1275/1290: only newline_list may follow `&`),
//! so the child died at parse and the job silently never executed
//! (`wait` still returned 0).
//!
//! `declare -f` was already correct (the parse-time printer in
//! ast_print.rs has the background branch); only the executor's
//! ComsubPrinter lacked it. The fix mirrors GNU print_cmd.c:257-272
//! (`&` case prints ` &`, space only before a following command) and
//! ast_print.rs: the background arm prints ` &` with no trailing space,
//! the list connector after a background command prints just a space,
//! and semicolon() then also suppresses after a body ending in `&`.
//!
//! Expected outputs are byte-verified against GNU bash 5.3.20
//! (script-file probes); the seed is the issue's own regression seed.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-3662-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's minimal repro: the function is only defined, never called —
/// the `: &` job alone triggers the prelude serialization, and before the
/// fix the child died with `syntax error near unexpected token ';'`.
#[test]
fn issue_3662_bg_job_in_comsub_in_function_parses() {
    let (stdout, stderr, code) = rubash_c(
        r#"put() { echo "put:$1"; }
_Msh_procsubst() {
  _Msh_FIFO=/tmp/_Msh_FIFO_${$}_${BASHPID:-$( : & put "$!" )}
  echo "fifo=$_Msh_FIFO"
}
: &
wait"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert!(
        !stderr.contains("syntax error"),
        "unexpected syntax error: {stderr}"
    );
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

/// The issue's regression seed (functional half): with the function
/// defined, a plain background job must actually run — before the fix the
/// respawned child died at parse, so `bg-job-done` never landed.
#[test]
fn issue_3662_bg_job_runs_with_comsub_function_defined() {
    let dir = scratch("bg");
    let path = dir.join("bg.txt");
    let script = format!(
        "put() {{ echo \"put:$1\"; }}\n_Msh_procsubst() {{\n  _Msh_FIFO=/tmp/_Msh_FIFO_${{$$}}_${{BASHPID:-$( : & put \"$!\" )}}\n  echo \"fifo=$_Msh_FIFO\"\n}}\necho main-line >> '{}'\nsleep 0.2; echo bg-job-done >> '{}' &\nwait\n",
        path.display(),
        path.display()
    );
    let (stdout, stderr, code) = rubash_c(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert!(
        !stderr.contains("syntax error"),
        "unexpected syntax error: {stderr}"
    );
    let contents = std::fs::read_to_string(&path).expect("read bg.txt");
    assert!(
        contents.contains("main-line"),
        "main line missing: {contents}"
    );
    assert!(
        contents.contains("bg-job-done"),
        "background job never ran: {contents}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Control: the same background job with no comsub function defined runs
/// on both sides of the fix — guards against the nail itself being
/// environment-sensitive rather than defect-sensitive.
#[test]
fn issue_3662_bg_job_runs_without_comsub_function() {
    let dir = scratch("bg-control");
    let path = dir.join("bg.txt");
    let script = format!(
        "echo main-line >> '{}'\nsleep 0.2; echo bg-job-done >> '{}' &\nwait\n",
        path.display(),
        path.display()
    );
    let (stdout, stderr, code) = rubash_c(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    let contents = std::fs::read_to_string(&path).expect("read bg.txt");
    assert!(
        contents.contains("bg-job-done"),
        "background job never ran: {contents}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
