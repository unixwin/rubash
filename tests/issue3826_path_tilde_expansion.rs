//! Regression pins for rubash#3826 — a `~` entry in PATH is not
//! expanded during PATH lookup (GNU expands `~/...` at lookup time).
//!
//! Mechanism (GNU anchors: findcmd.c:544 `find_in_path_element` —
//! `xpath = (posixly_correct == 0 && *path == '~') ? bash_tilde_expand
//! (path, 0) : path`; the per-element expansion happens at lookup
//! time, inside the walk, so `command -v` / `hash -t` print the
//! EXPANDED path). Rubash's unix walk (`find_in_path_walk_unix`,
//! src/executor/path.rs) and Windows walk (`find_in_path_via_listings`
//! via `path_scan_scaffold`) join each PATH element with the name
//! verbatim — a `~/bin` element never matches, so the run, `command
//! -v`, `hash`, `hash -t` and `type` faces all miss. The fix expands
//! each `~`-leading element at the join (reusing
//! `tilde_expand::expand_tilde_segment`), gated on non-POSIX mode.
//!
//! Boundary (measured against GNU bash 5.3.20, seed R133-C): the `~`
//! is NOT expanded when PATH is assigned (`echo $PATH` still shows
//! the literal `~/bin:...` on both shells) — the expansion happens at
//! lookup time only. A bare `~` PATH entry is NOT expanded by GNU
//! either in effect (it expands to $HOME, and the command lives in
//! $HOME/bin, not $HOME — both shells agree: `command not found`).
//! In POSIX mode (`--posix` / `set -o posix`) GNU skips the expansion
//! (findcmd.c:544 `posixly_correct == 0` gate) — the posix cell below
//! pins that gate.
//!
//! Cells 1–6 and 10 are the defect (fail on base, pass fixed); cells
//! 7–9 are controls (pass on both); cell 11 is a POSIX-mode GUARD
//! (passes on both — it pins that the fix must NOT expand under
//! `set -o posix`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn scratch_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-tilde-path-3826-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("bin")).expect("scratch home/bin");
    // The command under test: a shell script in $HOME/bin.
    let mycmd = dir.join("bin").join("mycmd");
    fs::write(&mycmd, "#!/bin/sh\necho TILDE\n").expect("write mycmd");
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mycmd).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mycmd, perms).unwrap();
    }
    dir
}

/// Run `driver` (a shell script) under the rubash test binary with
/// HOME pointed at a fresh scratch home holding bin/mycmd.
/// Returns (stdout, stderr, exit code, scratch home). CRLF is
/// normalized. The scratch home is removed before returning, but its
/// path is what the expanded `~/bin` must print as.
fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>, PathBuf) {
    let home = scratch_home(tag);
    let driver_path = home.join("driver.sh");
    fs::write(&driver_path, driver).expect("write driver");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&home)
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
    let code = out.status.code();
    let _ = fs::remove_dir_all(&home);
    (stdout, stderr, code, home)
}

/// The scratch home in shell display form (`/c/...` on Windows,
/// the path itself on unix) — what `command -v` / `hash -t` print
/// once the `~` is expanded.
fn home_shell_form(home: &Path) -> String {
    let s = home.to_string_lossy().replace('\\', "/");
    // Windows drive `C:/...` displays as `/c/...`.
    if s.len() >= 3 && s.as_bytes()[1] == b':' && s.as_bytes()[2] == b'/' {
        let drive = s.as_bytes()[0].to_ascii_lowercase() as char;
        format!("/{drive}{}", &s[2..])
    } else {
        s
    }
}

fn expanded_mycmd(home: &Path) -> String {
    format!("{}/bin/mycmd", home_shell_form(home))
}

const SETUP_PATH: &str = "PATH='~/bin:/usr/bin:/bin'";

#[test]
fn issue3826_run_via_tilde_path_entry() {
    let (out, _err, code, _home) = run_driver("run", &format!("{SETUP_PATH}; mycmd; echo rc=$?"));
    assert_eq!(out, "TILDE\nrc=0\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_command_v_via_tilde_path_entry() {
    let (out, _err, code, home) = run_driver(
        "cmdv",
        &format!("{SETUP_PATH}; command -v mycmd; echo rc=$?"),
    );
    let expected = expanded_mycmd(&home);
    assert_eq!(
        out,
        format!("{expected}\nrc=0\n"),
        "command -v must print the expanded path"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_hash_via_tilde_path_entry() {
    let (out, _err, code, _home) =
        run_driver("hash", &format!("{SETUP_PATH}; hash mycmd; echo rc=$?"));
    assert_eq!(out, "rc=0\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_hash_t_prints_expanded_path() {
    let (out, _err, code, home) =
        run_driver("hasht", &format!("{SETUP_PATH}; hash mycmd; hash -t mycmd"));
    let expected = expanded_mycmd(&home);
    assert_eq!(
        out,
        format!("{expected}\n"),
        "hash -t must print the expanded path"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_export_path_with_tilde_entry() {
    let (out, _err, code, _home) = run_driver(
        "export",
        "export PATH='~/bin:/usr/bin:/bin'; hash mycmd; echo rc=$?",
    );
    assert_eq!(out, "rc=0\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_path_via_variable_assignment() {
    let (out, _err, code, _home) = run_driver(
        "assign",
        "P='~/bin:/usr/bin:/bin'; PATH=$P; hash mycmd; echo rc=$?",
    );
    assert_eq!(out, "rc=0\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_path_assignment_stays_literal() {
    // Control: the `~` is NOT expanded when PATH is assigned — only
    // at lookup time. Both shells print the literal value.
    let (out, _err, code, _home) =
        run_driver("literal", "PATH='~/bin:/usr/bin:/bin'; echo \"PATH=$PATH\"");
    assert_eq!(out, "PATH=~/bin:/usr/bin:/bin\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_absolute_home_path_entry_works() {
    // Control: an absolute $HOME/bin entry works on both shells.
    let (out, _err, code, _home) =
        run_driver("abs", "PATH=\"$HOME/bin:/usr/bin:/bin\"; mycmd; echo rc=$?");
    assert_eq!(out, "TILDE\nrc=0\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_bare_tilde_entry_not_found() {
    // Control: a bare `~` PATH entry does NOT find the command on
    // GNU either (it expands to $HOME, and mycmd lives in $HOME/bin,
    // not $HOME) — both shells agree: command not found, rc 127.
    let (out, _err, code, _home) = run_driver("bare", "PATH='~:/usr/bin:/bin'; mycmd; echo rc=$?");
    assert_eq!(out, "rc=127\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_type_via_tilde_path_entry() {
    let (out, _err, code, home) = run_driver("type", &format!("{SETUP_PATH}; type mycmd"));
    let expected = expanded_mycmd(&home);
    assert_eq!(
        out,
        format!("mycmd is {expected}\n"),
        "type must report the expanded path"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3826_posix_mode_skips_tilde_expansion() {
    // GUARD (passes on base and fixed): GNU findcmd.c:544 gates the
    // expansion on `posixly_correct == 0` — under `set -o posix` the
    // `~/bin` element stays literal and the command is not found.
    let (out, _err, code, _home) = run_driver(
        "posix",
        &format!("set -o posix; {SETUP_PATH}; mycmd; echo rc=$?"),
    );
    assert_eq!(out, "rc=127\n", "stdout={out:?}");
    assert_eq!(code, Some(0));
}
