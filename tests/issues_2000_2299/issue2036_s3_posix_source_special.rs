//! Behavior pins for rubash#2036 S3 / KD-2: in POSIX mode `source`
//! is a special builtin exactly like its synonym `.`.
//!
//! GNU anchor: `special_builtins[]` (bash 5.3 builtins/mkbuiltins.c:
//! 148-154) lists 16 names, `source` included, and the SPECIAL flag
//! is applied by name (mkbuiltins.c:819-820). The measured GNU
//! 5.3.20 consequences pinned here:
//! * defining a function named `source` in POSIX mode is a fatal
//!   error ("`source': is a special builtin", exit 2) —
//!   execute_cmd.c's intern-function gate;
//! * a function named `source` defined before POSIX mode was
//!   enabled never shadows the builtin in POSIX mode (special
//!   builtins are found before functions, execute_cmd.c:4657-4676),
//!   while in default mode the function wins;
//! * assignments preceding `source` persist in POSIX mode
//!   (execute_cmd.c:4893 merge_temporary_env);
//! * a failed readonly assignment before `source` is fatal in a
//!   non-interactive POSIX shell (subst.c tempenv arm via
//!   execute_cmd.c:4693), while before a non-special builtin the
//!   list is merely discarded;
//! * a missing file makes a non-interactive POSIX shell exit
//!   (builtins/source.def:182-187), in default mode it is a plain
//!   rc=1, and under `command` the POSIX exit is suppressed
//!   (executing_command_builtin == 0 condition, same citation).
//!
//! Every shape runs for BOTH spellings (`source` and `.`): before
//! S3 the `source` spelling diverged on the definition, invocation
//! and readonly shapes (the execution-side predicate
//! `is_posix_special_builtin` lacked the name; the persistence
//! shape was held up by a one-off `|| command == "source"`
//! workaround in variable_state, removed with the alignment).
//!
//! Unix-only: the shapes are script files on a POSIX host, like the
//! S1 battery.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

struct Output {
    stdout: String,
    stderr: String,
    status: Option<i32>,
}

/// Run one script FILE in `dir` (script mode: non-interactive, the
/// mode the POSIX fatality rules govern).
fn run_script(dir: &Path, name: &str, script: &str) -> Output {
    let file = dir.join(name);
    fs::write(&file, script).expect("write script");
    let output = Command::new(shell_bin())
        .arg(&file)
        .current_dir(dir)
        .env("PATH", "/usr/bin:/bin")
        .env_remove("BASH_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
        status: output.status.code(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2036-s3-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn issue2036_s3_posix_source_definition_rejected() {
    let dir = scratch("defn");
    for spelling in ["source", "."] {
        let out = run_script(
            &dir,
            "defn.sh",
            &format!("set -o posix\n{spelling}() {{ echo DEFN; }}\necho \"defined rc=$?\"\n"),
        );
        assert_eq!(
            out.status,
            Some(2),
            "{spelling} stdout={} stderr={}",
            out.stdout,
            out.stderr
        );
        assert!(
            !out.stdout.contains("defined"),
            "{spelling} definition must abort the script: {}",
            out.stdout
        );
        assert!(
            out.stderr
                .contains(&format!("`{spelling}': is a special builtin")),
            "{spelling} diagnostic: {}",
            out.stderr
        );
    }
}

#[test]
fn issue2036_s3_posix_source_invocation_not_shadowed() {
    let dir = scratch("invoke");
    let body = "source() { echo FUNC-source; }\n.() { echo FUNC-dot; }\n";
    let calls =
        "source /dev/null; echo \"call-source rc=$?\"\n. /dev/null; echo \"call-dot rc=$?\"\n";
    // POSIX mode: the special builtin wins over the pre-defined
    // functions for both spellings.
    let out = run_script(&dir, "invoke.sh", &format!("{body}set -o posix\n{calls}"));
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout, "call-source rc=0\ncall-dot rc=0\n");
    // Default mode control: the functions win (the historical
    // function-first lookup) — the alignment must not move this.
    let out = run_script(&dir, "invoke-default.sh", &format!("{body}{calls}"));
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout,
        "FUNC-source\ncall-source rc=0\nFUNC-dot\ncall-dot rc=0\n"
    );
}

#[test]
fn issue2036_s3_posix_source_assignments_persist() {
    let dir = scratch("persist");
    fs::write(dir.join("inc.sh"), "echo \"in=$FOO\"\n").expect("inc.sh");
    let out = run_script(
        &dir,
        "persist.sh",
        "set -o posix\nFOO=bar source ./inc.sh\necho \"after-source=${FOO-unset}\"\nFOO=qux . ./inc.sh\necho \"after-dot=${FOO-unset}\"\n",
    );
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout,
        "in=bar\nafter-source=bar\nin=qux\nafter-dot=qux\n"
    );
    // Default mode control: the assignments are temporary.
    let out = run_script(
        &dir,
        "persist-default.sh",
        "FOO=bar source ./inc.sh\necho \"after-source=${FOO-unset}\"\n",
    );
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout, "in=bar\nafter-source=unset\n");
}

#[test]
fn issue2036_s3_posix_source_readonly_assignment_failure_is_fatal() {
    let dir = scratch("ro");
    for spelling in ["source", "."] {
        let out = run_script(
            &dir,
            "ro.sh",
            &format!(
                "set -o posix\nreadonly R=1\nR=2 {spelling} /dev/null\necho \"AFTER-{spelling}\"\n"
            ),
        );
        assert_eq!(out.status, Some(1), "{spelling}");
        assert!(
            !out.stdout.contains("AFTER"),
            "{spelling}: the shell must exit, got {}",
            out.stdout
        );
        assert!(
            out.stderr.contains("R: readonly variable"),
            "{spelling} diagnostic: {}",
            out.stderr
        );
    }
    // Non-special control: the command is discarded, the shell
    // continues with the next list.
    let out = run_script(
        &dir,
        "ro-echo.sh",
        "set -o posix\nreadonly R=1\nR=2 echo hi\necho \"AFTER-echo\"\n",
    );
    assert_eq!(out.status, Some(0));
    assert!(out.stdout.contains("AFTER-echo"));
}

#[test]
fn issue2036_s3_posix_source_missing_file_fatality_matrix() {
    let dir = scratch("missing");
    // POSIX mode, non-interactive: a special builtin that cannot
    // find its file exits the shell (both spellings).
    for spelling in ["source", "."] {
        let out = run_script(
            &dir,
            "missing.sh",
            &format!("set -o posix\n{spelling} /nonexistent-xyz-2036\necho \"AFTER rc=$?\"\n"),
        );
        assert_eq!(out.status, Some(1), "{spelling}");
        assert!(!out.stdout.contains("AFTER"), "{spelling}: {}", out.stdout);
        assert!(
            out.stderr.contains("No such file or directory"),
            "{spelling} diagnostic: {}",
            out.stderr
        );
    }
    // Default mode: plain rc=1, the shell continues.
    let out = run_script(
        &dir,
        "missing-default.sh",
        "source /nonexistent-xyz-2036\necho \"AFTER-source rc=$?\"\n. /nonexistent-xyz-2036\necho \"AFTER-dot rc=$?\"\n",
    );
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout, "AFTER-source rc=1\nAFTER-dot rc=1\n");
    // Under `command`, the POSIX exit is suppressed
    // (executing_command_builtin): rc=1 and the shell continues.
    let out = run_script(
        &dir,
        "missing-command.sh",
        "set -o posix\ncommand source /nonexistent-xyz-2036\necho \"AFTER-cmd-source rc=$?\"\ncommand . /nonexistent-xyz-2036\necho \"AFTER-cmd-dot rc=$?\"\n",
    );
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout, "AFTER-cmd-source rc=1\nAFTER-cmd-dot rc=1\n");
}
