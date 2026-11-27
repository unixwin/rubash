//! rubash#1129: a shell started with NO `PATH` in the environment
//! installs GNU's compiled-in default instead of an empty string.
//!
//! GNU anchor: `variables.c`: when PATH is absent at startup the
//! shell binds the default path
//! `/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin:.`
//! so command lookup keeps working. rubash bound `PATH=[]`, which
//! silently broke every external command under `env -i`.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::Command;

/// The reported case: `env -i` shows the full compiled-in default.
#[test]
fn missing_path_installs_compiled_in_default() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo \"PATH=[$PATH]\"")
        .env_clear()
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "PATH=[/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin:.]\n"
    );
}

/// Boundary: an inherited PATH is never overwritten by the default.
#[test]
fn inherited_path_wins_over_default() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo \"PATH=[$PATH]\"")
        .env_clear()
        .env("PATH", "/custom")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "PATH=[/custom]\n");
}
