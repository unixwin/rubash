//! rubash#2142 — no-argument `caller` when the calling frame is the
//! top-level frame: GNU prints a literal NULL for the source field;
//! rubash filled in the shell's own path.
//!
//! GNU builtins/caller.def:89-105: the no-argument form prints
//! BASH_LINENO[0] and BASH_SOURCE[1], a literal NULL for an absent
//! element. The arrays carry exactly one element per FUNCTION frame;
//! the synthetic bottom frames (FUNCNAME "main", the script's
//! BASH_SOURCE entry, BASH_LINENO "0") exist ONLY when the shell reads
//! a script FILE (shell.c:1635-1650 shell_execscript). Under `bash -c`
//! or a stdin script there is no bottom frame: BASH_SOURCE has no [1]
//! element, so the field renders NULL (and `caller 0` fails, because
//! FUNCNAME[1] is absent too).
//!
//! rubash's variable views already apply that gating
//! (dynamic_stack_array_values pops the $0 bottom entry when there is
//! no script main frame), but `execute_caller` read the raw
//! bash_source_stack through indexed_array_stack, whose BASH_SOURCE
//! bottom entry is the shell's own path — so the raw stack leaked the
//! path where GNU has no element at all.
//!
//! Control shapes that already matched and must not move: a script
//! file's main frame reports the script path; a nested `-c` call
//! reports the caller frame's own source element.
//!
//! All expectations measured against GNU bash 5.3.20 (self-compiled
//! oracle), 2026-10-30.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2142-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn base_command(dir: &PathBuf) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir);
    command
}

const LIB_SH: &str = "sf(){\ncaller 0; echo rc0=$?\ncaller; echo rcN=$?\n}\n";

#[test]
fn caller_top_frame_source_is_null_c_sourced_issue2142() {
    // The issue's regression seed, driven through `-c` (the top-level
    // frame is the -c string: no script main frame exists).
    let dir = tempdir("csourced");
    std::fs::write(dir.join("lib.sh"), LIB_SH).expect("write lib");
    let output = base_command(&dir)
        .args(["-c", "source ./lib.sh; sf"])
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc0=1\n1 NULL\nrcN=0\n", "stdout: {stdout:?}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn caller_top_frame_source_is_null_c_defined_issue2142() {
    let dir = tempdir("cdefined");
    let output = base_command(&dir)
        .args(["-c", "f(){ caller; echo rc=$?; }; f"])
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "1 NULL\nrc=0\n", "stdout: {stdout:?}");
}

#[test]
fn caller_top_frame_source_is_null_stdin_issue2142() {
    // A stdin script has no script main frame either (shell.c:780-786
    // read_from_stdin never enters shell_execscript).
    let dir = tempdir("stdin");
    let mut child = base_command(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"f(){ caller; echo rc=$?; }\nf\n")
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2 NULL\nrc=0\n", "stdout: {stdout:?}");
}

#[test]
fn caller_script_file_main_frame_control_issue2142() {
    // Control: with a script file the main frame EXISTS — caller 0
    // reports it (line 2, "main", the script path) and the no-argument
    // form reports the script path, exactly as before the fix.
    let dir = tempdir("scriptfile");
    std::fs::write(dir.join("lib.sh"), LIB_SH).expect("write lib");
    std::fs::write(dir.join("main.sh"), "source ./lib.sh\nsf\n").expect("write main");
    let output = base_command(&dir)
        .arg("main.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "2 main main.sh\nrc0=0\n2 main.sh\nrcN=0\n",
        "stdout: {stdout:?}"
    );
}

#[test]
fn caller_nested_c_frame_source_control_issue2142() {
    // Control: nested `-c` calls — the caller frame is g's frame,
    // whose source element EXISTS (the -c frame's own name, i.e. the
    // name the shell was invoked as); it must keep printing. The
    // binary is invoked through a `bash` symlink so the invocation
    // name is exactly "bash" for both shells (GNU renders
    // dollar_vars[0] verbatim here, so the spelling follows the
    // invocation, not the binary's location).
    let dir = tempdir("nested");
    let bin_link = dir.join("bash");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_rubash"), &bin_link).expect("symlink binary");
    let output = Command::new(&bin_link)
        .current_dir(&dir)
        .args(["-c", "f(){ caller; }; g(){ f; }; g"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "1 bash\n", "stdout: {stdout:?}");
}
