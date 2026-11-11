//! rubash#2631 — test-hygiene umbrella: the part_023 `builtin cd` pin
//! lowercased its whole expected path (`.to_ascii_lowercase()`), so a
//! worktree path containing uppercase letters (e.g. `fix-B5`)
//! false-redded: the expectation became `fix-b5` while `pwd` prints
//! the on-disk spelling `fix-B5`. The pin in
//! `tests/executor_command_chaining/part_023.rs` now normalizes only
//! the drive-letter segment. This file locks the product side of that
//! expectation: `pwd` reports the real-cased path, exercised in a
//! directory whose name contains uppercase letters.

use std::process::Command;

#[test]
fn issue2631_pwd_keeps_uppercase_path_spelling() {
    let dir = std::env::temp_dir().join(format!("rubash-2631-Upper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create uppercase scratch dir");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("pwd")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        dir.to_string_lossy().as_ref(),
        "pwd must print the on-disk spelling, not a lowercased one"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
