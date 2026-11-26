//! Issue rubash#3570 — the startup PWD repair must not rewrite literal
//! backslashes in the physical cwd on Unix: a backslash is an ordinary
//! filename byte there (general.h:264-270 ABSPATH), and GNU set_pwd
//! (variables.c:907) binds the get_working_directory spelling verbatim
//! when the inherited PWD is invalid. The unconditional `\`→`/` replace
//! turned `cwd\backslash` into the nonexistent `cwd/backslash`.

#![cfg(unix)]

use std::process::Command;

#[test]
fn startup_pwd_keeps_literal_backslash() {
    let dir = std::env::temp_dir()
        .join("rubash3570")
        .join("cwd\\backslash");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("p.sh");
    std::fs::write(&script, "printf 'PWD=<%s>\\n' \"$PWD\"\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(&dir)
        .env("PWD", "/nonexistent-stale-pwd")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        format!("PWD=<{}>\n", dir.to_string_lossy()),
        "stdout={stdout:?}"
    );
    assert!(stdout.contains("cwd\\backslash"), "stdout={stdout:?}");
}
