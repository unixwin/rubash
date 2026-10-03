//! Issue rubash#421 (themesweep P1 wave, lane wt54/quotecd): `\u` in PS1
//! honored the USER environment variable, so a session running with
//! `USER=""` rendered the prompt with NO username (`@host`) where GNU shows
//! the account name (`root@host`).
//!
//! GNU spec: parse.y:6542-6551 (decode_prompt_string `case 'u'`) renders
//! `current_user.user_name`, which get_current_user_info (shell.c:1877-1915)
//! fills ONCE per process from getpwuid(geteuid()) — the passwd database.
//! The USER environment variable is never consulted for `\u`; a userless
//! lookup renders the literal "I have no name!".
//!
//! Rust semantic owner: executor/parameter_case.rs prompt_username — now
//! env-independent (GetUserNameW on Windows, getpwuid on Unix), cached once
//! per process like GNU's current_user.
//!
//! Verified against WSL GNU Bash 5.3.0: with USER empty, unset and spoofed,
//! `\u` always renders the real account name (probe target/probe/p421*.sh,
//! 2026-10-02).

use std::process::Command;

fn prompt_username_with(user_env: Option<&str>) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--norc")
        .arg("-i")
        .env_remove("PS1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run rubash -i");
    match user_env {
        Some(value) => {
            // Applied inside the session so the child environment of the
            // prompt-rendering shell is exactly what the sweep hit.
            use std::io::Write;
            command
                .stdin
                .as_mut()
                .unwrap()
                .write_all(format!("USER={value:?}\nPS1='\\u@mark\\n'\nexit\n").as_bytes())
                .unwrap();
        }
        None => {
            use std::io::Write;
            command
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"unset USER\nPS1='\\u@mark\\n'\nexit\n")
                .unwrap();
        }
    }
    let output = command.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&output.stderr);
    // The rendered PS1 goes to stderr (readline prompt channel).
    text.lines()
        .find(|line| line.ends_with("@mark"))
        .map(|line| line.trim_end_matches("@mark").to_string())
        .unwrap_or_default()
}

/// GNU \u is the passwd/token account name: USER="" must not blank it.
#[test]
fn prompt_u_renders_account_name_with_empty_user() {
    let username = prompt_username_with(Some(""));
    assert!(
        !username.is_empty(),
        "\\u rendered empty with USER=\"\" (rubash#421 regression)"
    );
}

/// USER=spoofed must not leak into \u (env never consulted).
#[test]
fn prompt_u_ignores_spoofed_user_env() {
    let username = prompt_username_with(Some("spoofed"));
    assert_ne!(username, "spoofed", "\\u honored a spoofed USER env");
    assert!(!username.is_empty());
}

/// USER unset keeps \u non-empty (the pre-existing behavior must hold).
#[test]
fn prompt_u_renders_account_name_with_user_unset() {
    let username = prompt_username_with(None);
    assert!(!username.is_empty());
}
