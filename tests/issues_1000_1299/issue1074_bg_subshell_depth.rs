//! rubash#1074 remainder: every async `&` body runs one subshell level
//! down, so `BASH_SUBSHELL` inside it is parent depth + 1.
//!
//! GNU anchor: `execute_cmd.c:1588 execute_in_subshell` runs
//! `subshell_level++` unconditionally in the forked child — simple
//! commands, `{ }` groups, `( )` lists and coproc bodies alike. The
//! coproc spawn site already seeded parent + 1; the generic `&` spawn
//! seeded the parent level for non-paren bodies, so `echo $BASH_SUBSHELL
//! &` and `{ ...; } &` reported 0 instead of 1.
//!
//! Baselines verified against GNU bash 5.3.20 (`bash` on this host):
//!   sq:1 / brace:1 / paren:1 / cpbody:1, nested bg inside `( )` reports 2.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1074-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1074 scratch dir");
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

/// `echo ... &` runs one level down: GNU prints `sq:1`.
#[test]
fn async_simple_command_sees_subshell_depth_1() {
    // GNU bash 5.3.20: `sq:1`.
    let (_, stdout, _) = run_script("echo \"sq:$BASH_SUBSHELL\" & wait\n");
    assert_eq!(stdout, "sq:1\n");
}

/// `{ ...; } &` runs one level down: GNU prints `brace:1`.
#[test]
fn async_brace_group_sees_subshell_depth_1() {
    // GNU bash 5.3.20: `brace:1`.
    let (_, stdout, _) = run_script("{ echo \"brace:$BASH_SUBSHELL\"; } & wait\n");
    assert_eq!(stdout, "brace:1\n");
}

/// `( ... ) &` still reports exactly 1 — the spawn already realized the
/// subshell level, so the child must not bump a second time.
#[test]
fn async_paren_list_still_sees_subshell_depth_1() {
    // GNU bash 5.3.20: `paren:1`.
    let (_, stdout, _) = run_script("( echo \"paren:$BASH_SUBSHELL\" ) & wait\n");
    assert_eq!(stdout, "paren:1\n");
}

/// Async depth stacks: background inside an explicit subshell is 2.
#[test]
fn async_inside_subshell_sees_depth_2() {
    // GNU bash 5.3.20: `nested:2`.
    let (_, stdout, _) = run_script("( echo \"nested:$BASH_SUBSHELL\" & wait )\n");
    assert_eq!(stdout, "nested:2\n");
}
