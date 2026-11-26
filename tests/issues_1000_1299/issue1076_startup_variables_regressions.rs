//! rubash#1076 (startup-environment facets a and c, plus the COLUMNS
//! facet from the same report): a fresh shell binds the same startup
//! variables GNU binds.
//!
//! GNU anchors: `variables.c` `initialize_shell_variables` binds the
//! compiled-in `BASH_LOADABLES_PATH` search list for every fresh
//! shell; `COLUMNS` is only bound when the shell can learn a window
//! size (or inherits one), so an `env -i` non-tty `-c` run leaves it
//! unset; `shell.c` seeds `$_` with the pathname used to invoke the
//! shell when the environment did not supply one.
//!
//! rubash preset none of these faithfully: BASH_LOADABLES_PATH was
//! missing, COLUMNS was preset to 80, and `$_` started empty in
//! script files.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("BASH_LOADABLES_PATH")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// (a) `declare -p BASH_LOADABLES_PATH` prints the compiled-in list.
#[test]
fn bash_loadables_path_is_bound_at_startup() {
    let (stdout, _, code) = rubash_c("declare -p BASH_LOADABLES_PATH");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -- BASH_LOADABLES_PATH=\"/usr/local/lib/bash:/usr/lib/bash:/opt/local/lib/bash:/usr/pkg/lib/bash:/opt/pkg/lib/bash:.\"\n"
    );
}

/// (b) Under `env -i` with no tty, COLUMNS stays unset (GNU prints
/// `[unset]`; rubash used to preset `[80]`).
#[test]
fn columns_stays_unset_without_tty_or_inheritance() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo \"[${COLUMNS-unset}]\"")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/tmp")
        .env("TERM", "dumb")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "[unset]\n");
}

/// (c) A script's first command already sees `$_` as the pathname
/// used to invoke the shell (GNU prints the binary path; rubash
/// printed `[]` until after the first command).
#[test]
fn dollar_underscore_is_set_on_the_first_line() {
    let dir = std::env::temp_dir().join(format!("rubash-issue1076-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1076 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, "echo \"[$_]\"\n").expect("write probe script");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(exe)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("_")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("[{exe}]\n")
    );
}
