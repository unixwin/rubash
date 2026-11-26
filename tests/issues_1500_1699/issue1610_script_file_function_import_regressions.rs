//! rubash#1610 — A child shell started with a script file argument
//! did not import functions from `BASH_FUNC_*` environment entries at
//! all (the `-c` startup path imported them fine). The in-process
//! script child's serialized environment dropped the exported-function
//! materialization entirely, so the parent-export → child-import →
//! grandchild chain broke at its first link.
//!
//! Expectations verified against GNU bash 5.3.20 with the same
//! two-stage script files (SELF pointing at the shell under test).
//! This is the #1610 companion to the bf-w3-a pin in
//! `issue1275_function_reexport_regressions.rs` (same chain, parent
//! issue number); both must pass once both land.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1610-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn script_file_child_imports_exported_function() {
    let dir = tempdir("direct");
    std::fs::write(dir.join("callf.sh"), "f\n").expect("write callf");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(exe)
        .current_dir(&dir)
        .arg("-c")
        .arg(format!(
            "f() {{ echo F-RAN; }}; export -f f; \"{exe}\" callf.sh"
        ))
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "F-RAN\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn script_file_child_reexports_imported_function_to_grandchildren() {
    let dir = tempdir("chain");
    std::fs::write(
        dir.join("stage1.sh"),
        "f() { echo F-RAN; }\nexport -f f\n\"$SELF\" stage2.sh\n",
    )
    .expect("write stage1");
    std::fs::write(dir.join("stage2.sh"), "f\n\"$SELF\" -c 'f'\n").expect("write stage2");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(exe)
        .current_dir(&dir)
        .env("SELF", exe)
        .arg("stage1.sh")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "F-RAN\nF-RAN\n");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imported_function_does_not_linger_as_a_variable() {
    // GNU variables.c: an imported function becomes a shell function,
    // not a variable — `export -p` in the script child must not print
    // a BASH_FUNC_* declare line (niubash#102 class), while
    // `declare -F f` finds the function.
    let dir = tempdir("clean");
    std::fs::write(
        dir.join("check.sh"),
        "declare -F f\nexport -p | grep -c '^declare -x BASH_FUNC_' || true\n",
    )
    .expect("write check");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(exe)
        .current_dir(&dir)
        .arg("-c")
        .arg(format!(
            "f() {{ echo F-RAN; }}; export -f f; \"{exe}\" check.sh"
        ))
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "f\n0\n", "stdout: {stdout}");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
