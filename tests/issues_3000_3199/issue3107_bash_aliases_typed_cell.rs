//! rubash#3107: dynamic associative variables are rebuilt from their
//! owning tables in GNU (`variables.c:1745 build_aliasvar` and
//! `variables.c:1675 build_hashcmd`); an owner-table write must not leave
//! a previously materialized typed cell disagreeing with that rebuild.
//!
//! These cases run script files in the debug test binary, where the
//! command-boundary typed-cell verifier is active.

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3107-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

fn assert_success(stdout: &str, stderr: &str, code: Option<i32>) {
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
}

#[test]
fn issue_script_alias_after_declare_does_not_panic() {
    let (stdout, stderr, code, dir) = run_script(
        "original",
        "declare -p BASH_VERSINFO BASH_ALIASES BASH_CMDS DIRSTACK\n\
         alias ll=\"ls -l\"\n\
         pushd / >/dev/null\n\
         declare -p BASH_ALIASES DIRSTACK\n",
    );
    assert_success(&stdout, &stderr, code);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 6, "stdout={stdout:?}");
    assert!(
        lines[0].starts_with("declare -ar BASH_VERSINFO=([0]=\"5\" [1]=\"3\" [2]=\"20\""),
        "stdout={stdout:?}"
    );
    assert_eq!(lines[1], "declare -A BASH_ALIASES=()");
    assert_eq!(lines[2], "declare -A BASH_CMDS=()");
    assert_eq!(
        lines[3],
        format!("declare -a DIRSTACK=([0]=\"{}\")", dir.to_string_lossy())
    );
    assert_eq!(lines[4], "declare -A BASH_ALIASES=([ll]=\"ls -l\" )");
    assert_eq!(
        lines[5],
        format!(
            "declare -a DIRSTACK=([0]=\"/\" [1]=\"{}\")",
            dir.to_string_lossy()
        )
    );
}

#[test]
fn alias_overwrite_and_unalias_resync_materialized_cell() {
    let (stdout, stderr, code, _) = run_script(
        "alias-writes",
        "declare -p BASH_ALIASES\n\
         alias ll='ls -l'\n\
         alias ll='ls -la'\n\
         printf '%s\\n' \"${BASH_ALIASES[ll]}\"\n\
         unalias ll\n\
         declare -p BASH_ALIASES\n\
         alias a='echo a'\n\
         alias b='echo b'\n\
         unalias -a\n\
         declare -p BASH_ALIASES\n",
    );
    assert_success(&stdout, &stderr, code);
    assert_eq!(
        stdout,
        "declare -A BASH_ALIASES=()\n\
         ls -la\n\
         declare -A BASH_ALIASES=()\n\
         declare -A BASH_ALIASES=()\n"
    );
}

#[test]
fn command_lookup_resyncs_materialized_bash_cmds_cell() {
    // An ordinary external-command lookup inserts into hashed_filenames
    // (hashcmd.c phash_insert) without any hash builtin running; the
    // materialized BASH_CMDS cell must still track the owner table.
    let (stdout, stderr, code, _) = run_script(
        "lookup",
        "declare -p BASH_CMDS\n\
         ls >/dev/null\n\
         declare -p BASH_CMDS\n\
         printf '%s\\n' \"${BASH_CMDS[ls]}\"\n",
    );
    assert_success(&stdout, &stderr, code);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "stdout={stdout:?}");
    assert_eq!(lines[0], "declare -A BASH_CMDS=()");
    let Some(path) = lines[1]
        .strip_prefix("declare -A BASH_CMDS=([ls]=\"")
        .and_then(|rest| rest.strip_suffix("\" )"))
    else {
        panic!("stdout={stdout:?}");
    };
    assert!(path.ends_with("/ls"), "stdout={stdout:?}");
    assert_eq!(lines[2], path, "stdout={stdout:?}");
}

#[test]
fn hash_writes_resync_materialized_bash_cmds_cell() {
    let (stdout, stderr, code, _) = run_script(
        "hash-writes",
        "declare -p BASH_CMDS\n\
         hash -p /bin/ls lscmd\n\
         declare -p BASH_CMDS\n\
         printf '%s\\n' \"${BASH_CMDS[lscmd]}\"\n\
         hash -d lscmd\n\
         declare -p BASH_CMDS\n\
         hash -p /bin/ls lscmd\n\
         hash -r\n\
         declare -p BASH_CMDS\n",
    );
    assert_success(&stdout, &stderr, code);
    assert_eq!(
        stdout,
        "declare -A BASH_CMDS=()\n\
         declare -A BASH_CMDS=([lscmd]=\"/bin/ls\" )\n\
         /bin/ls\n\
         declare -A BASH_CMDS=()\n\
         declare -A BASH_CMDS=()\n"
    );
}
