//! rubash#2581 — `env` appeared in every builtin face (BUILTIN_NAMES,
//! the command registry row dispatching to execute_env_command,
//! `enable -n env`, `compgen -b`) but `execute_builtin_direct_command`
//! had no `env` arm, so `builtin env` failed with "not a shell
//! builtin" rc=1 — a self-contradicting mirrored table.
//!
//! GNU evidence: GNU bash has NO env builtin (there is no
//! builtins/env.def in bash 5.3; env is a coreutils program), so GNU's
//! `builtin env` fails. rubash deliberately diverges: its internal env
//! implementation is the only env on platforms without /usr/bin/env,
//! and every other face already commits to env-as-builtin. Removing
//! env from the faces would break `enable -n env`, `compgen -b`, and
//! `type env`; the consistent fix is the missing dispatch arm.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2581-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_2581_builtin_env_prints_environment() {
    let script = "export MARKER2581=present\nout=$(builtin env)\ncase \"$out\" in *MARKER2581=present*) echo found;; *) echo missing;; esac\n";
    let (stdout, stderr, _code) = run_script(script);
    assert_eq!(stdout, "found\n", "stderr: {stderr}");
}

#[test]
fn issue_2581_builtin_env_assignment_form() {
    // `builtin env NAME=value <prog>` must run the program with the
    // assignment applied — here the program is env itself via the
    // shell's own internal dispatch path (echo of the composed env).
    let script = "out=$(builtin env ADDED2581=zzz env)\ncase \"$out\" in *ADDED2581=zzz*) echo applied;; *) echo absent;; esac\n";
    let (stdout, stderr, _code) = run_script(script);
    assert_eq!(stdout, "applied\n", "stderr: {stderr}");
}

#[test]
fn issue_2581_builtin_env_respects_enable_disable() {
    let script = "enable -n env\nbuiltin env\necho \"rc=$?\"\n";
    let (stdout, stderr, _code) = run_script(script);
    assert!(
        stdout.contains("rc=1"),
        "stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stderr.contains("not a shell builtin"),
        "disabled env must report like any disabled builtin, stderr: {stderr}"
    );
}

#[test]
fn issue_2581_builtin_echo_control() {
    let (stdout, stderr, _code) = run_script("builtin echo hi\n");
    assert_eq!(stdout, "hi\n", "stderr: {stderr}");
}
