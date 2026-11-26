//! rubash#3825 — `hash` and `type -a` accept a non-executable file
//! found via PATH; GNU reports "not found" (rc 1).
//!
//! GNU anchors (bash 5.3):
//! - `hash.def:214-230` `add_hashed_command`: the bare-name path runs
//!   `full_path = find_user_command (w)` and then applies an
//!   `executable_file (full_path)` post-check
//!   (`findcmd.c:214-223`: FS_EXECABLE **and** not a directory); a
//!   non-executable hit is not stored and the builtin reports
//!   `sh_notfound`.
//! - `type.def:381` `describe_command`: the `-a` enumeration uses
//!   `user_command_matches (command, FS_EXEC_ONLY, ...)` — only
//!   executable files are enumerated at all.
//! - The PATH walk itself stays FS_EXEC_PREFERRED (`findcmd.c:580-586`):
//!   a non-executable hit is the `file_to_lose_on` fallback, returned
//!   only when no executable exists. That fallback is CORRECT for the
//!   execution face (the exec then fails EACCES, rc 126) and for the
//!   lenient faces — plain `type` (`type.def:379` `find_user_command`)
//!   and `command -v` both report it on GNU. So the fix adds the
//!   strict post-check at the `hash` and `type -a` consumption points
//!   only; `find_user_command` itself is untouched.
//!
//! Expectations are GNU bash 5.3.20 behaviour for the verbatim seed
//! scripts (R133-B), run from a file in a fresh directory. The defect
//! is unix-only: executability as a mode bit does not exist on the
//! Windows face.
//!
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("rubash-3825-{tag}-{pid}", pid = std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const SETUP_NONEXEC: &str = "printf 'x' > mycmd; chmod 644 mycmd; PATH=.:/usr/bin:/bin";

#[test]
fn issue3825_hash_nonexecutable_path_file_is_not_found() {
    let scratch = Scratch::new("hash644");
    let script = format!("{SETUP_NONEXEC}; hash mycmd; echo rc=$?\n");
    let (stdout, stderr, code) = scratch.rubash_file(&script);
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("hash: mycmd: not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_hash_nonexecutable_path_file_is_not_stored() {
    let scratch = Scratch::new("nostore");
    let script = format!(
        "{SETUP_NONEXEC}; hash mycmd 2>/dev/null; echo hash-rc=$?; hash -t mycmd 2>/dev/null; echo t-rc=$?\n"
    );
    let (stdout, stderr, code) = scratch.rubash_file(&script);
    assert_eq!(stdout, "hash-rc=1\nt-rc=1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_hash_mode_000_path_file_is_not_found() {
    let scratch = Scratch::new("hash000");
    let script =
        "printf 'x' > mycmd; chmod 000 mycmd; PATH=.:/usr/bin:/bin; hash mycmd; echo rc=$?\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("hash: mycmd: not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_type_a_nonexecutable_path_file_is_not_found() {
    let scratch = Scratch::new("typea");
    let script = format!("{SETUP_NONEXEC}; type -a mycmd; echo rc=$?\n");
    let (stdout, stderr, code) = scratch.rubash_file(&script);
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("type: mycmd: not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_hash_executable_path_file_is_stored() {
    let scratch = Scratch::new("exec");
    let script = "printf '#!/bin/sh\\necho OK\\n' > mycmd; chmod +x mycmd; PATH=.:/usr/bin:/bin; hash mycmd; echo rc=$?; hash -t mycmd\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "rc=0\n./mycmd\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_hash_skips_nonexecutable_to_later_executable() {
    let scratch = Scratch::new("skip");
    let script = "mkdir -p a b; printf 'x' > a/mycmd; chmod 644 a/mycmd; printf '#!/bin/sh\\necho LATER\\n' > b/mycmd; chmod +x b/mycmd; PATH=a:b:/usr/bin:/bin; hash mycmd; echo rc=$?; hash -t mycmd\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "rc=0\n./b/mycmd\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_command_v_nonexecutable_path_file_stays_lenient() {
    let scratch = Scratch::new("cmdv");
    let script = format!("{SETUP_NONEXEC}; command -v mycmd; echo rc=$?\n");
    let (stdout, stderr, code) = scratch.rubash_file(&script);
    assert_eq!(stdout, "./mycmd\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_hash_rehash_after_delete_restores_entry() {
    let scratch = Scratch::new("rehash");
    let script = "printf '#!/bin/sh\\necho OK\\n' > mycmd; chmod +x mycmd; PATH=.:/usr/bin:/bin; hash mycmd; hash -d mycmd; mycmd; hash -t mycmd\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "OK\n./mycmd\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3825_type_plain_nonexecutable_path_file_stays_lenient() {
    let scratch = Scratch::new("typeplain");
    let script = format!("{SETUP_NONEXEC}; type mycmd; echo rc=$?\n");
    let (stdout, stderr, code) = scratch.rubash_file(&script);
    assert_eq!(stdout, "mycmd is ./mycmd\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
