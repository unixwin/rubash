//! rubash#1080: when `command_not_found_handle` is defined, a missing
//! command runs it (with the command name and args) and its return
//! value becomes the command's status — no 127, no diagnostic.
//!
//! GNU anchor: `execute_cmd.c`: the command-search failure path calls
//! the shell function `command_not_found_handle` when it exists.
//! rubash ignored the hook entirely.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1080-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1080 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The reported case, plus the handler-returning-0 form: stdout is
/// exactly the handler's output and the two statuses; stderr empty.
#[test]
fn defined_handler_runs_and_supplies_the_status() {
    let (code, stdout, stderr) = run_script(
        "command_not_found_handle(){ echo \"HANDLED:$1/$2\"; return 42; }\n\
         nosuchcmd a b; echo \"rc=$?\"\n\
         command_not_found_handle(){ return 0; }\n\
         nosuchcmd2; echo \"rc2=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "HANDLED:nosuchcmd/a\nrc=42\nrc2=0\n");
    assert_eq!(stderr, "");
}

/// Control that already agreed: with no handler defined, a missing
/// command is the classic 127 + `command not found` on stderr.
#[test]
fn missing_handler_keeps_classic_127() {
    let (code, stdout, stderr) = run_script("nosuchcmd a b; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains("nosuchcmd: command not found"),
        "stderr: {stderr}"
    );
}
