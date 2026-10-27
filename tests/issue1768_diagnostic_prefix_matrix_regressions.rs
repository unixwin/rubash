//! Issue rubash#1768 (characterization pins): the filing claimed
//! diagnostics — a SIGHUP job death notice in particular — are
//! prefixed with the face name `bash` instead of the current
//! input source's name while executing a named script.
//!
//! The w2-parser-diag cluster's adjudication matrix (diagnostic
//! sites x invocation modes, run against the GNU bash 5.3.20
//! oracle and this tree, script-file / -c / stdin) found the
//! claim NOT reproducible in any cell: the shared prefix source
//! (error.c get_name_for_error's analogue: the invocation's
//! script name, else the shell name) already produces the
//! script name in script mode for every site probed — builtin
//! errors, command-not-found, and foreground/background signal
//! death notices alike. The filing's originating observation
//! came from the niubash#528 device session, whose Hangup form
//! bf-w5 adjudicated a real-SIGHUP session artifact (see
//! niubash tests/issue528_lastpipe_empty_read_baseline_residual.rs:
//! under baseline conditions neither shell prints that notice).
//! These pins hold the verified matrix cells so the prefix
//! source cannot silently regress; the closure recommendation
//! for #1768 itself is in the cluster REPORT.

use std::process::Command;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1768-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("file");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1768_script_command_not_found_uses_script_name() {
    let (_, stderr, code) = rubash_file("/nonexistent-cmd-xyz\n");
    assert!(
        stderr.starts_with("case.sh: line 1: /nonexistent-cmd-xyz: "),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(127));
}

#[test]
fn issue1768_script_builtin_error_uses_script_name() {
    let (_, stderr, code) = rubash_file("cd /nonexistent-dir-xyz\n");
    assert!(
        stderr.starts_with("case.sh: line 1: cd: /nonexistent-dir-xyz: "),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[cfg(unix)]
#[test]
fn issue1768_script_background_death_notice_uses_script_name() {
    let (stdout, stderr, code) = rubash_file("sleep 30 &\nkill -HUP $!\nwait $!\necho done\n");
    assert_eq!(stdout, "done\n");
    assert!(stderr.contains("case.sh: line 3: "), "stderr: {stderr}");
    assert!(stderr.contains(" Hangup "), "stderr: {stderr}");
    assert!(stderr.contains("sleep 30"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue1768_script_foreground_death_notice_uses_script_name() {
    // Single quotes keep `$$` unexpanded by this shell: the
    // child sh expands its OWN pid and kills itself. (The
    // double-quoted form would expand `$$` to this shell's pid
    // and HUP the shell under test instead.)
    let (stdout, stderr, code) = rubash_file("/bin/sh -c 'kill -HUP $$'\necho after\n");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("case.sh: line 1: "), "stderr: {stderr}");
    assert!(stderr.contains(" Hangup "), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1768_dash_c_command_not_found_uses_shell_name() {
    // No script operand: get_name_for_error is the shell name
    // (GNU invoked as `bash` prints `bash: line 1: ...`).
    let dir = scratch("dashc");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "/nonexistent-cmd-xyz"])
        .current_dir(&dir)
        .output()
        .expect("run rubash -c");
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.starts_with("bash: line 1: /nonexistent-cmd-xyz: "),
        "stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(127));
}

#[test]
fn issue1768_stdin_command_not_found_uses_invocation_name() {
    // GNU never rebinds $0 for a stdin script: the prefix is the
    // invocation name verbatim (the full path when invoked by
    // one — shell.c:1613's rebind is the script-FILE path only).
    use std::io::Write;
    let dir = scratch("stdin");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(b"/nonexistent-cmd-xyz\n")
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash stdin");
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let invocation = env!("CARGO_BIN_EXE_rubash");
    assert!(
        stderr.starts_with(&format!("{invocation}: line 1: /nonexistent-cmd-xyz: ")),
        "stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(127));
}
