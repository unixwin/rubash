//! Issue rubash#324: `mapfile -t arr < missing-file` silently swallowed
//! the redirect-open failure — no diagnostic, rc=0, and the array was
//! CLOBBERED to empty. GNU prints `script: line N: missing: No such file
//! or directory`, returns 1, and leaves the array untouched (pbb sections
//! 030/032 lost one error line per call and scripts proceeded as if the
//! file were empty).
//!
//! GNU contract (vendored third_party/bash): execute_builtin
//! (execute_cmd.c:4787+) applies the command's redirections via
//! do_redirections BEFORE the builtin function runs; a redir_open failure
//! (redir.c redir_error) reports the shell-prefixed diagnostic and
//! returns 1 — mapfile.def never executes, so the target array keeps its
//! previous value and option/identifier validation never happens
//! (`mapfile -t @bad < missing` reports the MISSING FILE, not `not a
//! valid identifier`).
//!
//! Root cause: rubash's mapfile dispatch (three sites) called
//! execute_mapfile directly, skipping the apply_no_output_builtin_redirects
//! preflight every other builtin gets; the raw-byte fd-0 file reader then
//! fell through to the empty-input path, which stored an EMPTY array and
//! returned 0.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28). stderr text is
//! asserted by substring: the diagnostic's script-path prefix is
//! environment-dependent, and stdout/stderr INTERLEAVING when merged into
//! one file is a documented known limitation (GNU orders differently) —
//! content, rc and array state are the contract here.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The reporter's shape: rc=1, diagnostic, array untouched.
#[test]
fn mapfile_missing_input_redirect_reports_and_keeps_array() {
    let (stdout, stderr, code) = rubash(
        "arr=(old1 old2)\nmapfile -t arr < /definitely/missing_324a\necho \"rc=$? n=${#arr[@]} a0=${arr[0]}\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 n=2 a0=old1\n");
    assert!(
        stderr.contains("missing_324a: No such file or directory"),
        "stderr: {stderr}"
    );
}

/// readarray (the alias) behaves identically.
#[test]
fn readarray_missing_input_redirect_reports() {
    let (stdout, stderr, code) =
        rubash("readarray arr2 < /definitely/missing_324b\necho \"rc=$? n=${#arr2[@]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 n=0\n");
    assert!(
        stderr.contains("missing_324b: No such file or directory"),
        "stderr: {stderr}"
    );
}

/// GNU precedence: the redirect-open failure fires BEFORE option and
/// identifier validation (`mapfile -t @bad < missing` reports the missing
/// file, never `not a valid identifier`).
#[test]
fn redirect_error_precedes_identifier_validation() {
    let (stdout, stderr, code) =
        rubash("mapfile -t @badname < /definitely/missing_324c\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("missing_324c: No such file or directory"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
}

/// A readable empty source still succeeds with an empty array
/// (`/dev/null` — the control for the silent-empty path).
#[test]
fn mapfile_dev_null_still_succeeds() {
    let (stdout, stderr, code) = rubash("mapfile -t a < /dev/null\necho \"rc=$? n=${#a[@]}\"");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 n=0\n");
}

/// The `read` control from the issue: unchanged behavior (already
/// correct before the fix).
#[test]
fn read_missing_input_redirect_still_reports() {
    let (stdout, stderr, code) = rubash("read -r x < /definitely/missing_324d\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("missing_324d: No such file or directory"),
        "stderr: {stderr}"
    );
}

/// mapfile with a here-string and with an existing file still work
/// (no preflight side effects on the working paths).
#[test]
fn mapfile_working_paths_unaffected() {
    let (stdout, stderr, code) = rubash(
        "mapfile -t h <<< $'one\\ntwo'\necho \"h=${#h[@]}:${h[0]}:${h[1]}\"\n\
         printf 'x\\ny\\n' > /tmp/rubash324f.txt\nmapfile -t f < /tmp/rubash324f.txt\necho \"f=${#f[@]}:${f[0]}:${f[1]}\"\nrm -f /tmp/rubash324f.txt",
    );
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "h=2:one:two\nf=2:x:y\n");
}
