//! rubash#2212: bats-core own-suite setup/teardown + file-evaluation
//! machinery — miniature end-to-end chain guards for the family that
//! aborted whole .bats files at the 2026-10 triage (bats-gather-tests
//! trace ENOENT; `bats_run_setup_file` failing inside `source`).
//! Expectations are GNU bash 5.3.20 behavior, verified cell by cell
//! against the oracle before being encoded here. The full-suite chain
//! results live in the ecosuite harness (run-bats-suite.sh); these
//! cells pin the mechanism shapes that chain depends on:
//!   01 — an EXIT trap can play back a trace file the same script
//!        created with `touch` at startup (gather-tests shape)
//!   02 — `source` of a failing setup file inside a function reports
//!        the failure without aborting the caller (bats checks the
//!        status itself; the shell must propagate it faithfully)
//!   03 — setup_file/teardown_file ordering across a sourced fixture,
//!        with teardown_file reached after a failing test function
//!   04 — the same chain driven through a nested rubash invocation,
//!        state passed across processes via a shared log file

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2212-chain-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    rubash_command()
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .output()
        .expect("run script")
}

// 01 — bats-gather-tests touches $BATS_TRACE at startup and its EXIT
// trap plays the file back; at triage the trap reported ENOENT on the
// just-created file and the whole file aborted before any test ran.
#[test]
fn issue2212_chain_01_exit_trap_plays_back_touched_trace_file() {
    let dir = test_dir("trap-trace");
    let output = run_script(
        &dir,
        "gather.sh",
        "d=$(mktemp -d)\n\
         trap 'echo trap-saw-$(wc -l < \"$d/trace\")' EXIT\n\
         touch \"$d/trace\"\n\
         echo line1 >> \"$d/trace\"\n\
         echo line2 >> \"$d/trace\"\n\
         false\n",
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "trap-saw-2\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1));
}

// 02 — `bats_run_setup_file` sources the setup file from inside a
// function; a nonzero `source` must surface as the function's status
// (so bats can abort the file itself), never as a shell-level abort
// and never swallowed.
#[test]
fn issue2212_chain_02_source_failure_inside_setup_function_propagates() {
    let dir = test_dir("source-fail");
    fs::write(dir.join("setup_fails.bash"), "return 1\n").expect("write setup");
    let output = run_script(
        &dir,
        "caller.sh",
        "setup() { source ./setup_fails.bash; }\n\
         echo before\n\
         setup\n\
         echo setup-rc=$?\n\
         echo after\n",
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "before\nsetup-rc=1\nafter\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
}

// 03 — the file-evaluation shape bats-exec-file relies on: source a
// fixture defining setup_file/teardown_file/test functions, run the
// tests, and teardown_file still runs after a failing test.
#[test]
fn issue2212_chain_03_setup_teardown_ordering_with_failing_test() {
    let dir = test_dir("ordering");
    fs::write(
        dir.join("fixture.bash"),
        "setup_file() { echo setup_file >> \"$CHAIN_LOG\"; }\n\
         teardown_file() { echo teardown_file >> \"$CHAIN_LOG\"; }\n\
         test_one() { echo test_one >> \"$CHAIN_LOG\"; }\n\
         test_two() { echo test_two >> \"$CHAIN_LOG\"; return 1; }\n",
    )
    .expect("write fixture");
    let output = run_script(
        &dir,
        "driver.sh",
        "export CHAIN_LOG=\"$PWD/chain.log\"\n\
         : > \"$CHAIN_LOG\"\n\
         source ./fixture.bash\n\
         setup_file\n\
         test_one || echo one-failed >> \"$CHAIN_LOG\"\n\
         test_two || echo two-failed >> \"$CHAIN_LOG\"\n\
         teardown_file\n\
         cat \"$CHAIN_LOG\"\n",
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "setup_file\ntest_one\ntest_two\ntwo-failed\nteardown_file\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
}

// 04 — two-level chain: an outer script invokes a fresh rubash on the
// driver of cell 03; the teardown marker must land in the shared log
// across the process boundary (the nested-bats shape).
#[test]
fn issue2212_chain_04_nested_invocation_chain() {
    let dir = test_dir("nested");
    fs::write(
        dir.join("fixture.bash"),
        "setup_file() { echo setup_file >> \"$CHAIN_LOG\"; }\n\
         teardown_file() { echo teardown_file >> \"$CHAIN_LOG\"; }\n\
         test_one() { echo test_one >> \"$CHAIN_LOG\"; }\n",
    )
    .expect("write fixture");
    fs::write(
        dir.join("driver.sh"),
        "export CHAIN_LOG=\"$PWD/chain.log\"\n\
         source ./fixture.bash\n\
         setup_file\n\
         test_one\n\
         teardown_file\n",
    )
    .expect("write driver");
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let outer = format!(
        "export CHAIN_LOG=\"$PWD/chain.log\"\n\
         : > \"$CHAIN_LOG\"\n\
         \"{}\" ./driver.sh\n\
         echo outer-rc=$?\n\
         echo teardown-present=$(grep -c teardown_file \"$CHAIN_LOG\")\n",
        rubash.replace('\\', "/")
    );
    let output = run_script(&dir, "outer.sh", &outer);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "outer-rc=0\nteardown-present=1\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
}
