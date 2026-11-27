//! Nested same-shell child signal death (rubash#2236; GNU bash 5.3.20
//! oracle-verified 2026-10-30).
//!
//! A script child invoked through the shell's own binary runs
//! in-process in rubash's emulation. When the child killed ITSELF
//! with an untrapped signal (`kill -TERM $$`), the delivery used to
//! fall through to raise(2) on the hosting process: the OUTER shell
//! died with the child's signal and none of its remaining commands
//! ran. GNU runs the child as its own process (execute_cmd.c), so
//! the kill can only kill the child: the parent observes 128+signal,
//! prints the foreground job-death notice (jobs.c), and continues.
//!
//! These pins cover the containment: the child is judged by its own
//! trap table (trap.c run_pending_traps) — untrapped deaths end only
//! the child with 128+signal plus the parent notice; trapped and
//! ignored deliveries run the child's action / are dropped and the
//! child continues; a lone (non-nested) shell still dies by a real
//! untrapped delivery.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

fn run_script(script_path: &PathBuf) -> (String, String, Option<i32>) {
    let output = Command::new(rubash_bin())
        .arg(script_path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_dash_c(text: &str) -> (String, String, Option<i32>, Option<i32>) {
    let output = Command::new(rubash_bin())
        .arg("-c")
        .arg(text)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    use std::os::unix::process::ExitStatusExt;
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        output.status.signal(),
    )
}

/// Write `inner` and `outer` scripts into a fresh temp dir; the outer
/// template's `{IN}` is replaced by "<rubash> <inner path>".
fn write_pair(tag: &str, inner: &str, outer_template: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("issue2236-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let inner_path = dir.join("inner.sh");
    fs::write(&inner_path, inner).expect("write inner");
    let invocation = format!("{} {}", rubash_bin(), inner_path.display());
    let outer_path = dir.join("outer.sh");
    fs::write(&outer_path, outer_template.replace("{IN}", &invocation)).expect("write outer");
    (dir, outer_path)
}

const SUICIDE: &str = "kill -TERM $$\necho unreachable\n";

fn terminated_notice(invocation: &str) -> String {
    // jobs.c foreground death notice: the description padded to
    // column 27, then the command text as invoked.
    format!("{:<27}{}\n", "Terminated", invocation)
}

#[test]
fn issue2236_nested_child_suicide_parent_survives() {
    let (dir, outer) = write_pair("plain", SUICIDE, "{IN}\necho \"inner-rc=$?\"\necho after\n");
    let (stdout, stderr, code) = run_script(&outer);
    let invocation = format!("{} {}", rubash_bin(), dir.join("inner.sh").display());
    assert_eq!(stdout, "inner-rc=143\nafter\n");
    assert_eq!(stderr, terminated_notice(&invocation));
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2236_nested_child_suicide_as_last_command() {
    let (dir, outer) = write_pair("last", SUICIDE, "{IN}\n");
    let (stdout, stderr, code) = run_script(&outer);
    let invocation = format!("{} {}", rubash_bin(), dir.join("inner.sh").display());
    assert_eq!(stdout, "");
    assert_eq!(stderr, terminated_notice(&invocation));
    assert_eq!(code, Some(143));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2236_nested_child_in_subshell_parent_survives() {
    let (dir, outer) = write_pair(
        "subshell",
        SUICIDE,
        "( {IN} )\necho \"inner-rc=$?\"\necho after\n",
    );
    let (stdout, stderr, code) = run_script(&outer);
    let invocation = format!("( {} {} )", rubash_bin(), dir.join("inner.sh").display());
    assert_eq!(stdout, "inner-rc=143\nafter\n");
    assert_eq!(stderr, terminated_notice(&invocation));
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2236_nested_dash_c_child_suicide_parent_survives() {
    let bin = rubash_bin();
    let text = format!("{bin} -c 'kill -TERM $$; echo unreachable'; echo inner-rc=$?; echo after");
    let (stdout, stderr, code, signal) = run_dash_c(&text);
    assert_eq!(stdout, "inner-rc=143\nafter\n");
    assert_eq!(
        stderr,
        terminated_notice(&format!("{bin} -c 'kill -TERM $$; echo unreachable'"))
    );
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue2236_nested_child_trapped_signal_child_survives() {
    let (dir, outer) = write_pair(
        "trapped",
        "trap 'echo caught' TERM\nkill -TERM $$\necho inner-after\n",
        "{IN}\necho \"inner-rc=$?\"\necho after\n",
    );
    let (stdout, stderr, code) = run_script(&outer);
    assert_eq!(stdout, "caught\ninner-after\ninner-rc=0\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2236_nested_child_ignored_signal_child_survives() {
    let (dir, outer) = write_pair(
        "ignored",
        "trap '' TERM\nkill -TERM $$\necho inner-after\n",
        "{IN}\necho \"inner-rc=$?\"\necho after\n",
    );
    let (stdout, stderr, code) = run_script(&outer);
    assert_eq!(stdout, "inner-after\ninner-rc=0\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2236_nested_child_exit_trap_runs_and_notice_prints() {
    // GNU: a child dying by an untrapped signal runs its EXIT trap
    // and still dies by the signal, so the parent notice prints.
    let (dir, outer) = write_pair(
        "exit-trap",
        "trap 'echo child-exit-trap' EXIT\nkill -TERM $$\necho unreachable\n",
        "{IN}\necho \"inner-rc=$?\"\necho after\n",
    );
    let (stdout, stderr, code) = run_script(&outer);
    let invocation = format!("{} {}", rubash_bin(), dir.join("inner.sh").display());
    assert_eq!(stdout, "child-exit-trap\ninner-rc=143\nafter\n");
    assert_eq!(stderr, terminated_notice(&invocation));
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2236_nested_child_usr1_death_notice() {
    let (dir, outer) = write_pair(
        "usr1",
        "kill -USR1 $$\necho unreachable\n",
        "{IN}\necho \"inner-rc=$?\"\necho after\n",
    );
    let (stdout, stderr, code) = run_script(&outer);
    assert_eq!(stdout, "inner-rc=138\nafter\n");
    // Branch-1 notice carries the (run-specific) child pid, so pin
    // the stable parts: the description and the command text.
    assert!(
        stderr.contains("User defined signal 1"),
        "usr1 notice description: {stderr:?}"
    );
    assert!(
        stderr.contains(&format!(
            "{} {}",
            rubash_bin(),
            dir.join("inner.sh").display()
        )),
        "usr1 notice command text: {stderr:?}"
    );
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2236_single_shell_untrapped_suicide_still_dies() {
    // No nesting: containment must not leak — a lone shell that kills
    // itself with an untrapped signal dies by that signal.
    let (stdout, _stderr, code, signal) = run_dash_c("kill -TERM $$; echo alive");
    assert_eq!(stdout, "");
    assert_eq!(code, None);
    assert_eq!(signal, Some(15));
}
