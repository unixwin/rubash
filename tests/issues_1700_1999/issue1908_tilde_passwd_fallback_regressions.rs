//! Issue rubash#1908: with HOME *unset*, the bare `~` did not fall back
//! to the passwd home directory the way GNU does — word context expanded
//! it to the empty string and assignment context kept the literal `~`.
//!
//! GNU semantics: lib/tilde/tilde.c:344-356 (tilde_expand_word) — a
//! leading `~` or `~/` translates to $HOME; only a MISSING HOME reaches
//! the fallback, sh_get_home_dir (variables.c:736), which returns
//! current_user.home_dir as filled by get_current_user_info
//! (shell.c:1878) from `getpwuid (current_user.uid)` — pw_dir, or "/"
//! when the uid has no passwd entry. A set-but-empty HOME is a value
//! and never triggers the fallback (both shells agree there).
//!
//! Root cause: `tilde_home` in src/expand/tilde/tilde.rs — the single
//! home source every tilde context reads — had no unix fallback at all:
//! its non-Windows arm returned None, which word expansion flattened to
//! "" and assignment expansion treated as "keep verbatim". Fix: the
//! unix arm performs the same getpwuid_r lookup GNU performs (sharing
//! the reentrant buffer dance with the `~user` getpwnam_r path), with
//! GNU's "/" for a uid without an entry.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-10-28); the expected
//! home is derived from the live passwd database via getpwuid_r on the
//! current uid, exactly the lookup under test's specification.
//!
//! Unix-only: the passwd fallback is the unix arm of the GNU chain
//! (Windows falls back to USERPROFILE instead, covered by the tilde
//! module's own unit tests).

#![cfg(unix)]

use std::process::Command;

/// The current uid's passwd home, via the same reentrant lookup GNU's
/// get_current_user_info performs (spelled like the precedent in
/// tests/cli_tests.rs tilde_user_expands_system_passwd_home_on_unix).
#[cfg(unix)]
fn current_passwd_home() -> String {
    let mut buffer = vec![0u8; 16384];
    loop {
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        assert_eq!(status, 0, "system passwd lookup failed");
        assert!(!result.is_null(), "current uid has no passwd entry");
        break unsafe {
            std::ffi::CStr::from_ptr(entry.pw_dir)
                .to_str()
                .unwrap()
                .to_owned()
        };
    }
}

fn rubash(script: &str, home: Option<&str>) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.arg("-c").arg(script);
    match home {
        Some(value) => {
            command.env("HOME", value);
        }
        None => {
            command.env_remove("HOME");
        }
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The issue's exact seed (word context): bare `~` and `~/file` with
/// HOME unset resolve to the passwd home.
#[test]
#[cfg(unix)]
fn bare_tilde_falls_back_to_passwd_home_when_home_unset() {
    let home = current_passwd_home();
    let (stdout, stderr, _) = rubash("echo ~; echo ~/file", None);
    assert_clean(&stderr);
    assert_eq!(stdout, format!("{home}\n{home}/file\n"));
}

/// The issue's assignment-context shape: `V=~` binds the passwd home
/// instead of the literal tilde.
#[test]
#[cfg(unix)]
fn assignment_tilde_falls_back_to_passwd_home_when_home_unset() {
    let home = current_passwd_home();
    let (stdout, stderr, _) = rubash("V=~; W=~/docs; echo \"V=$V W=$W\"", None);
    assert_clean(&stderr);
    assert_eq!(stdout, format!("V={home} W={home}/docs\n"));
}

/// Set-but-empty HOME is a value: no fallback, in either shell.
#[test]
#[cfg(unix)]
fn empty_home_does_not_trigger_passwd_fallback() {
    let (stdout, stderr, _) = rubash("printf '[%s]\n' ~; echo ~/file", Some(""));
    assert_clean(&stderr);
    assert_eq!(stdout, "[]\n/file\n");
}

/// A set HOME wins over the passwd entry (no fallback consulted).
#[test]
#[cfg(unix)]
fn set_home_wins_over_passwd() {
    let (stdout, stderr, _) = rubash("echo ~; echo ~/file", Some("/custom/home"));
    assert_clean(&stderr);
    assert_eq!(stdout, "/custom/home\n/custom/home/file\n");
}
