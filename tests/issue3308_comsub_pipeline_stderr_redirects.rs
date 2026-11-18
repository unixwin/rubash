//! rubash#3308 — script-mode `$(external | external)`: the pipeline's
//! first process's stderr bypassed every redirection and wrote straight
//! to the console (the `-c` form was reported normal on the Windows host;
//! on Linux both modes leaked). GNU bash 5.3.20 (built from the vendored
//! tree, `third_party/bash/bash`) routes every pipeline element's fd 2
//! through the shell's current fd-2 binding: `execute_cmd.c:2620
//! execute_pipeline` forks each element and `redir.c:237 do_redirections`
//! applies the element's (and the enclosing scope's) redirections in the
//! child, while `subst.c:7143 command_substitute` only dups the capture
//! pipe onto fd 1 and leaves fd 2 inherited.
//!
//! The concurrent external-pipeline spawn loops resolved fd 2 from the
//! process std handle instead of the executor's fd table: non-last stages
//! left stderr unset (process inherit) unless fd 2 was merged onto fd 1,
//! and collected last-stage stderr was written back through the raw
//! process stderr — so a rubash-level `2>/dev/null`, `2>file`, group
//! redirect or `exec 2>...` binding was bypassed for every stage.
//!
//! Expectations below are the GNU 5.3.20 oracle's, probed from script
//! files. Two GNU-correct "leaks" are pinned as guards so a fix cannot
//! over-suppress: a redirect on the assignment itself (`cs=$(...) 2>/dev/null`)
//! does NOT apply to the expansion (the substitution runs before the
//! assignment's redirections), and a redirect on the LAST pipeline stage
//! only (`ls | cat 2>/dev/null`) does not silence the FIRST stage.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const LS_ERROR: &str = "cannot access '/nonexistent_xyz_probe'";
const LS_ERROR_ABC: &str = "cannot access '/nonexistent_abc_probe'";

fn rubash_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rubash")
}

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(rubash_bin())
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Run `script` as a script FILE under rubash. `dir` is a fresh
/// per-test directory the script may use for `>file` / `2>file` targets.
fn rubash_file(dir: &PathBuf, script: &str) -> (String, String, Option<i32>) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash script file");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3308-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn read_back(dir: &PathBuf, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn script_group_devnull_suppresses_comsub_pipeline_first_stage() {
    let dir = test_dir("group-devnull");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "{ cs=$(ls /nonexistent_xyz_probe | cat); } 2>/dev/null\n\
         echo \"len:${#cs}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "first stage stderr leaked past 2>/dev/null");
    cleanup(&dir);
}

#[test]
fn script_exec_devnull_suppresses_comsub_pipeline_first_stage() {
    let dir = test_dir("exec-devnull");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "exec 2>/dev/null\n\
         cs=$(ls /nonexistent_xyz_probe | cat)\n\
         echo \"len:${#cs}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert_eq!(
        stderr, "",
        "first stage stderr leaked past exec 2>/dev/null"
    );
    cleanup(&dir);
}

#[test]
fn script_group_stderr_file_catches_first_stage() {
    let dir = test_dir("group-errfile");
    let err = dir.join("err.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ cs=$(ls /nonexistent_xyz_probe | cat); }} 2>{}\n\
             echo \"len:${{#cs}}\"\n",
            err.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "first stage stderr leaked past 2>file");
    assert!(
        read_back(&dir, "err.txt").contains(LS_ERROR),
        "err.txt missing the ls error: {}",
        read_back(&dir, "err.txt")
    );
    cleanup(&dir);
}

#[test]
fn script_group_stdout_file_and_stderr_devnull() {
    let dir = test_dir("group-outfile-devnull");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ cs=$(ls /nonexistent_xyz_probe | cat); }} 1>{} 2>/dev/null\n\
             echo \"len:${{#cs}}\"\n",
            out.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert_eq!(
        stderr, "",
        "first stage stderr leaked past 1>file 2>/dev/null"
    );
    assert_eq!(read_back(&dir, "out.txt"), "", "out.txt should stay empty");
    cleanup(&dir);
}

#[test]
fn script_bare_pipeline_group_stderr_file() {
    let dir = test_dir("bare-pipe-errfile");
    let err = dir.join("err.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ ls /nonexistent_xyz_probe | cat; }} 2>{}\n\
             echo done\n",
            err.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "bare pipeline stderr leaked past 2>file");
    assert!(
        read_back(&dir, "err.txt").contains(LS_ERROR),
        "err.txt missing the ls error: {}",
        read_back(&dir, "err.txt")
    );
    cleanup(&dir);
}

#[test]
fn script_bare_pipeline_group_devnull() {
    let dir = test_dir("bare-pipe-devnull");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "{ ls /nonexistent_xyz_probe | cat; } 2>/dev/null\n\
         echo done\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "bare pipeline stderr leaked past 2>/dev/null");
    cleanup(&dir);
}

#[test]
fn script_assignment_redirect_does_not_suppress() {
    // Guard (GNU-correct leak): the redirect on the assignment itself
    // applies after the expansion runs, so the substitution's stderr
    // still reaches the shell's stderr. GNU 5.3.20 shows the error here.
    let dir = test_dir("assign-redirect");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "cs=$(ls /nonexistent_xyz_probe | cat) 2>/dev/null\n\
         echo \"len:${#cs}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert!(
        stderr.contains(LS_ERROR),
        "assignment-redirect stderr must still surface (GNU shows it): {stderr}"
    );
    cleanup(&dir);
}

#[test]
fn script_assignment_stderr_file_stays_empty() {
    // Guard (GNU-correct leak): `2>file` on the assignment itself does
    // not catch the substitution's stderr either — the file stays empty
    // and the error reaches the shell's stderr. GNU 5.3.20 agrees.
    let dir = test_dir("assign-errfile");
    let err = dir.join("err.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "cs=$(ls /nonexistent_xyz_probe | cat) 2>{}\n\
             echo \"len:${{#cs}}\"\n",
            err.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert!(
        stderr.contains(LS_ERROR),
        "assignment-redirect stderr must still surface: {stderr}"
    );
    assert_eq!(
        read_back(&dir, "err.txt"),
        "",
        "assignment 2>file must not catch the substitution's stderr"
    );
    cleanup(&dir);
}

#[test]
fn c_mode_group_devnull_matches_script_mode() {
    let (stdout, stderr, code) =
        rubash_c("{ cs=$(ls /nonexistent_xyz_probe | cat); } 2>/dev/null; echo \"len:${#cs}\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert_eq!(
        stderr, "",
        "first stage stderr leaked past 2>/dev/null in -c mode"
    );
}

#[test]
fn script_capture_clean_thirty_runs() {
    // Acceptance criterion (3): 30 repeated runs, zero capture pollution
    // and zero stderr leakage under the group redirect.
    let dir = test_dir("thirty-runs");
    let script = "{ cs=$(ls /nonexistent_xyz_probe | cat); } 2>/dev/null\n\
         echo \"len:${#cs}\"\n";
    for run in 0..30 {
        let (stdout, stderr, code) = rubash_file(&dir, script);
        assert_eq!(code, Some(0), "run {run}: stderr: {stderr}");
        assert_eq!(stdout, "len:0\n", "run {run}: capture polluted: {stdout:?}");
        assert_eq!(stderr, "", "run {run}: stderr leaked");
    }
    cleanup(&dir);
}

#[test]
fn script_last_stage_stderr_devnull() {
    // The last stage's collected stderr must also respect the binding
    // (the write-back path), not just the first stage's spawn wiring.
    let dir = test_dir("last-stage-devnull");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "{ echo hi | ls /nonexistent_xyz_probe; } 2>/dev/null\n\
         echo done\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "last stage stderr leaked past 2>/dev/null");
    cleanup(&dir);
}

#[test]
fn script_last_stage_stderr_file() {
    let dir = test_dir("last-stage-errfile");
    let err = dir.join("err.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ echo hi | ls /nonexistent_xyz_probe; }} 2>{}\n\
             echo done\n",
            err.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "last stage stderr leaked past 2>file");
    assert!(
        read_back(&dir, "err.txt").contains(LS_ERROR),
        "err.txt missing the ls error: {}",
        read_back(&dir, "err.txt")
    );
    cleanup(&dir);
}

#[test]
fn script_three_stage_stderr_file_gets_both_errors() {
    // Both non-last stages error; both must land in the file (order is
    // concurrent write timing, so assert set membership, not order).
    let dir = test_dir("three-stage-errfile");
    let err = dir.join("err.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ ls /nonexistent_xyz_probe | ls /nonexistent_abc_probe | cat; }} 2>{}\n\
             echo done\n",
            err.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "pipeline stderr leaked past 2>file");
    let err_text = read_back(&dir, "err.txt");
    assert!(err_text.contains(LS_ERROR), "err.txt: {err_text}");
    assert!(err_text.contains(LS_ERROR_ABC), "err.txt: {err_text}");
    cleanup(&dir);
}

#[test]
fn script_stderr_closed_drops_silently() {
    // `2>&-`: the child writes to a closed fd 2; GNU shows nothing.
    let dir = test_dir("closed-stderr");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "{ cs=$(ls /nonexistent_xyz_probe | cat); } 2>&-\n\
         echo \"len:${#cs}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "len:0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr leaked past 2>&-");
    cleanup(&dir);
}

#[test]
fn script_stderr_merged_onto_stdout_goes_to_stdout_not_capture() {
    // `{ ...; } 2>&1`: the dup ran OUTSIDE the substitution, so fd 2 is a
    // snapshot of the real stdout — the error goes to the script's stdout
    // and must NOT pollute the `cs` capture (GNU 5.3.20: error line, then
    // `len:0`). This pins the dup2-snapshot rule (redir.c:1170) against
    // the live-fd-1 resolution.
    let dir = test_dir("merged-stdout");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "{ cs=$(ls /nonexistent_xyz_probe | cat); } 2>&1\n\
         echo \"len:${#cs}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(LS_ERROR),
        "merged stderr missing from stdout: {stdout}"
    );
    assert!(stdout.contains("len:0\n"), "capture polluted: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    cleanup(&dir);
}

#[test]
fn script_stderr_merged_onto_stdout_inside_comsub_is_captured() {
    // The dup runs INSIDE the substitution here, so fd 2's target IS the
    // capture pipe and the error belongs in `x` (GNU 5.3.20 agrees).
    let (stdout, stderr, code) =
        rubash_c("x=$( { ls /nonexistent_xyz_probe | cat; } 2>&1 ); echo \"x=[$x]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(LS_ERROR),
        "merged stderr missing from capture: {stdout}"
    );
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn script_prefix_streaming_tail_group_devnull() {
    // External prefix + builtin tail (`ls | echo`) takes the
    // prefix-streaming-tail path: the prefix spawn never set stderr,
    // so `ls` inherited the process stderr past the group binding
    // (GNU 5.3.20: suppressed, `hi` on stdout).
    let dir = test_dir("prefix-tail-devnull");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        "{ ls /nonexistent_xyz_probe | echo hi; } 2>/dev/null\n\
         echo done\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\ndone\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "prefix stderr leaked past 2>/dev/null");
    cleanup(&dir);
}

#[test]
fn script_prefix_streaming_tail_group_stderr_file() {
    // Same shape with `2>file`: the prefix error must land in the file.
    let dir = test_dir("prefix-tail-errfile");
    let err = dir.join("err.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ ls /nonexistent_xyz_probe | echo hi; }} 2>{}\n\
             echo done\n",
            err.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\ndone\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "prefix stderr leaked past 2>file");
    assert!(
        read_back(&dir, "err.txt").contains(LS_ERROR),
        "err.txt missing the ls error: {}",
        read_back(&dir, "err.txt")
    );
    cleanup(&dir);
}
