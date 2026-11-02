//! rubash#1648 — `command -p` consulted the hash table, so a
//! poisoned entry hijacked it: `hash -p /bin/false myls; command -p
//! myls` executed /bin/false (rc 1) instead of reporting
//! `myls: command not found` (rc 127).
//!
//! GNU ownership: third_party/bash CHANGES ("`command -p' no longer
//! looks in the hash table for the specified command"),
//! findcmd.c:364 (CMDSRCH_STDPATH skips the phash_search consult)
//! and execute_cmd.c:5804-5808 (CMDSRCH_HASH is still passed, so a
//! -p hit is *inserted* into the table, overwriting a poisoned
//! entry with times_found=1). The fix skips the hashed-mirror
//! consult in the standard-path lookup (path.rs) and records -p
//! resolutions as unconditional inserts (hash.rs) instead of the
//! hit-bump-only record the plain path uses.
//!
//! Expectations verified against GNU bash.

#![cfg(unix)]

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn command_p_skips_hash_table() {
    let (stdout, stderr, _) = rubash("hash -p /bin/false myls; command -p myls; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains("myls: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn plain_command_still_uses_hash_table() {
    let (stdout, _, _) = rubash(
        "hash -p /bin/false myls; command myls; echo \"rc=$?\"; command -v myls; hash -t myls",
    );
    assert_eq!(stdout, "rc=1\n/bin/false\n/bin/false\n");
}

#[test]
fn command_p_hit_overwrites_poisoned_entry() {
    let (stdout, _, _) = rubash(
        "hash -p /bin/false ls; command -p ls /tmp >/dev/null; echo \"p-rc=$?\"; hash; command ls /tmp >/dev/null; echo \"plain-rc=$?\"; hash",
    );
    assert_eq!(
        stdout,
        "p-rc=0\nhits\tcommand\n   1\t/bin/ls\nplain-rc=0\nhits\tcommand\n   2\t/bin/ls\n"
    );
}

#[test]
fn command_p_skips_hash_under_bogus_path_too() {
    let (stdout, _, _) = rubash(
        "PATH=/nonexistent_xyz; hash -p /bin/false ls; command -p ls / >/dev/null; echo \"rc=$?\"",
    );
    assert_eq!(stdout, "rc=0\n");
}
