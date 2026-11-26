//! Issue rubash#1798 regression: output produced while `compgen`
//! resolves its -F function / -C command escaped the compgen
//! command's own redirections — a nested `compopt`'s `not currently
//! executing completion function` diagnostic reached the real stderr
//! even under `compgen -F f foo 2>/dev/null` (the diff-hunt cc08
//! shape), and the -F warning itself was erased by the final flush
//! re-opening the target.
//!
//! GNU runs a builtin under its redirections (execute_cmd.c
//! do_redirections precedes the builtin body), so for the whole
//! resolution the shell's fd 1/2 ARE the command's redirected
//! descriptors: the warning, the function's own stdout/stderr
//! writes, nested builtin diagnostics and the -C command's stderr
//! all travel through the frame, in execution order, while the -C
//! command's stdout is still captured as candidates. The fix binds
//! the command's fd 1/2 redirects into the executor's fd table for
//! the resolution and the final output (the frame
//! bind_conditional_stdio_redirects installs), writes the warning
//! through the frame, and skips the buffered flush, whose re-open
//! would truncate the frame's writes.
//!
//! Oracle: GNU bash 5.3.20 — every expectation below was produced by
//! the oracle binary (script-file probes, streams captured
//! separately).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-1798-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run a script file in its own directory (redirect targets inside
/// the script resolve there); return (rc, stdout, stderr, files).
fn run_script(tag: &str, script: &str, files: &[&str]) -> (i32, String, String, Vec<String>) {
    let dir = scratch_dir(tag);
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(shell_bin())
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let contents = files
        .iter()
        .map(|name| {
            std::fs::read_to_string(dir.join(name))
                .unwrap_or_else(|_| "<missing>".to_string())
                .replace("\r", "")
        })
        .collect();
    let result = (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).replace("\r", ""),
        String::from_utf8_lossy(&output.stderr).replace("\r", ""),
        contents,
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn compopt_diagnostic_inside_f_function_honors_compgen_redirect() {
    // The cc08 shape itself: stderr of the whole script must be
    // empty; the diagnostic is swallowed by the 2>/dev/null frame.
    let (rc, stdout, stderr, _) = run_script(
        "cc08",
        "f(){ COMPREPLY=(z); compopt +o default; echo \"compopt rc:$?\"; }\n\
         compgen -F f foo 2>/dev/null\n\
         echo rc:$?\n",
        &[],
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "compopt rc:1\nz\nrc:0\n");
    assert_eq!(stderr, "", "compopt diagnostic must not escape the frame");
}

#[test]
fn f_function_stderr_and_warning_land_in_the_redirect_target() {
    let (rc, stdout, stderr, files) = run_script(
        "fnstderr",
        "f(){ COMPREPLY=(z); echo INNER-ERR >&2; }\n\
         compgen -F f foo 2>err.txt\n",
        &["err.txt"],
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "z\n");
    assert_eq!(stderr, "");
    let lines: Vec<&str> = files[0].lines().collect();
    assert_eq!(lines.len(), 2, "warning then INNER-ERR: {:?}", files[0]);
    assert!(
        lines[0].ends_with("compgen: warning: -F option may not work as you expect"),
        "first line is the -F warning: {}",
        lines[0]
    );
    assert_eq!(lines[1], "INNER-ERR");
}

#[test]
fn f_function_stdout_precedes_matches_in_the_redirect_target() {
    let (rc, stdout, stderr, files) = run_script(
        "fnstdout",
        "f(){ COMPREPLY=(z); echo F-OUT; }\n\
         compgen -F f foo >out.txt 2>err.txt\n\
         echo \"--- out:\"; cat out.txt\n",
        &["out.txt", "err.txt"],
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "--- out:\nF-OUT\nz\n");
    assert_eq!(stderr, "");
    assert_eq!(files[0], "F-OUT\nz\n");
    assert!(files[1].contains("compgen: warning: -F option"));
}

#[test]
fn c_command_stderr_uses_the_frame_but_stdout_stays_captured() {
    let (rc, _, stderr, files) = run_script(
        "ccmd",
        "compgen -C 'echo C-ERR >&2; echo C-OUT' foo >out.txt 2>err.txt\n",
        &["out.txt", "err.txt"],
    );
    assert_eq!(rc, 0);
    assert_eq!(stderr, "");
    // The -C command's stdout is the candidate list (with
    // build_arg_list's appended arguments), not frame output.
    assert_eq!(files[0], "C-OUT compgen foo \n");
    assert!(files[1].contains("compgen: warning: -C option"));
    assert!(files[1].contains("C-ERR"));
}

#[test]
fn merged_stream_keeps_execution_order() {
    let (rc, stdout, stderr, files) = run_script(
        "merged",
        "f(){ COMPREPLY=(z); echo F-OUT; echo INNER-ERR >&2; }\n\
         compgen -F f foo >both.txt 2>&1\n\
         echo \"--- both:\"; cat both.txt\n",
        &["both.txt"],
    );
    assert_eq!(rc, 0);
    assert_eq!(stderr, "");
    let both = &files[0];
    let warning_at = both
        .find("compgen: warning")
        .expect("warning in merged file");
    let fout_at = both.find("F-OUT").expect("function stdout in merged file");
    let ferr_at = both
        .find("INNER-ERR")
        .expect("function stderr in merged file");
    let match_at = both.rfind("\nz\n").expect("matches in merged file");
    assert!(
        warning_at < fout_at && fout_at < ferr_at && ferr_at < match_at,
        "merged order must be warning, function output, matches: {both}"
    );
    assert!(stdout.contains("--- both:"));
}

#[test]
fn compgen_inside_command_substitution_keeps_the_frame() {
    let (rc, stdout, stderr, files) = run_script(
        "subst",
        "f(){ COMPREPLY=(z); }\n\
         x=$(compgen -F f foo 2>err.txt)\n\
         echo \"x=[$x]\"\n",
        &["err.txt"],
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "x=[z]\n");
    assert_eq!(stderr, "");
    assert!(files[0].contains("compgen: warning: -F option"));
}

#[test]
fn bad_redirect_target_fails_before_the_function_runs() {
    // GNU do_redirections precedes the builtin: the open fails, the
    // diagnostic names the target on the AMBIENT stderr, status 1,
    // and the function never runs (no candidate output).
    let (rc, stdout, stderr, _) = run_script(
        "badtarget",
        "f(){ COMPREPLY=(z); }\n\
         compgen -F f foo 2>/nonexistent-dir-1798/e\n\
         echo \"rc=$?\"\n",
        &[],
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent-dir-1798/e: No such file or directory"),
        "redirect failure must be diagnosed on ambient stderr: {stderr}"
    );
    assert!(
        !stderr.contains("may not work as you expect"),
        "the -F warning is part of the builtin, which never ran: {stderr}"
    );
}
