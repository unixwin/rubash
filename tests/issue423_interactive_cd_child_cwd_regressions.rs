//! Issue rubash#423 (themesweep P1 wave, lane wt54/quotecd): after `cd` in
//! an interactive (piped `-i`) session, external children still spawned
//! with the shell's STARTUP cwd — `cd dir` then `ls ..` listed the OLD
//! directory — while the shell's own PWD had already moved. Script mode was
//! unaffected; only cross-line interactive runs diverged.
//!
//! Root cause: `Executor::execute_ast` (executor/ast_exec.rs) saved the
//! process cwd before each reader-level run and restored it afterwards
//! (ced287c4 "isolate executor cwd during runs"). The interactive driver
//! enters execute_ast once per accepted line, so the undo fired between
//! lines and reverted the `cd`. GNU spec: `cd` runs chdir(2) in the SHELL
//! process itself (builtins/cd.def cd_builtin -> chdir, then bindpwd), and
//! the process cwd IS the session state every later command — the reader
//! never restores it (execute_cmd.c children inherit the live cwd via
//! fork/execve). The per-run restore was removed; the process cwd is live.
//!
//! Verified against WSL GNU Bash 5.3.0: `printf 'cd SUB\nls ..\n' | bash -i`
//! lists SUB's parent, and a same-line `cd SUB && ls ..` always did.

use std::io::Write;
use std::process::{Command, Stdio};

/// An external child (this repo's own `bash` bin, run as a fresh process)
/// reports the cwd it was spawned with, after `cd` on a PREVIOUS
/// interactive line.
#[test]
fn interactive_cd_persists_into_later_external_children() {
    let base = std::env::temp_dir().join("rubash-issue423-cwd");
    let sub = base.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    // Marker files: only visible from the right directory.
    std::fs::write(base.join("PARENT-MARK"), "").unwrap();
    std::fs::write(sub.join("CHILD-MARK"), "").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .arg("--norc")
        .current_dir(&base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash -i");
    // The child command must be a REAL process (not a builtin): the `bash`
    // bin of this crate prints its own process cwd.
    let bash = env!("CARGO_BIN_EXE_bash").replace('\\', "/");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("cd sub\n{bash} -c pwd\nexit\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The echoed interactive transcript carries prompts; the cwd line must
    // be the SUBdirectory (previously it printed the startup base dir).
    let reported = stdout
        .lines()
        .find(|line| line.contains("sub"))
        .unwrap_or(&stdout);
    assert!(
        reported.ends_with("sub"),
        "external child cwd was stale; stdout: {stdout:?} stderr: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// Same-line `cd && child` worked before the fix and must keep working.
#[test]
fn interactive_cd_and_child_on_one_line_reports_new_dir() {
    let base = std::env::temp_dir().join("rubash-issue423-sameline");
    let sub = base.join("sub2");
    std::fs::create_dir_all(&sub).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .arg("--norc")
        .current_dir(&base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash -i");
    let bash = env!("CARGO_BIN_EXE_bash").replace('\\', "/");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("cd sub2 && {bash} -c pwd\nexit\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| line.ends_with("sub2")),
        "same-line cd chain broke; stdout: {stdout:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}
