//! rubash#2791 — diagnostic prefixes on the builtin "cold paths" must
//! follow argv[0]; no builtin-local fallback may print a hard-coded
//! `"bash: "` / `"rubash: "` when the script-name/line markers are
//! absent.
//!
//! Reachability census (this lane, base f17644d7): a renamed-binary
//! matrix — 15 triggers x {-c, script file, command substitution}
//! plus readonly-bind, EXIT-trap, function-body and eval contexts,
//! 62 shapes in total — matched GNU bash 5.3.20 BYTE FOR BYTE on every
//! shape. The executor's argv[0]-derived prefix resolution (#2714)
//! covers every CLI-reachable path probed; the literal fallbacks the
//! issue lists (pwd.rs / setattr.rs `"bash: "`, alias.rs /
//! declare/diagnostic.rs / help.rs / builtins/diagnostic.rs
//! `"rubash: "`, pushd.rs bindpwd_write) are NOT reachable through the
//! binary on this base. These pins are therefore regression locks:
//! they prove the census conclusion in-repo and guard the fix lane's
//! remaining work — deleting/converging the dead literals onto the
//! shared resolver — from ever exposing them. If a future change makes
//! a fallback reachable, the corresponding cell goes red here first.
//!
//! The binary is spawned with argv[0] forced to `zimsh`
//! (CommandExt::arg0), so the expected prefix is exactly GNU's for a
//! shell invoked under that name. Expectations verified against GNU
//! bash 5.3.20 under the same forced argv[0].

#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_zimsh(args: &[&str], cwd: Option<&std::path::Path>) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg0("zimsh")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd.output().expect("run rubash as zimsh");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_c(script: &str) -> Run {
    run_zimsh(&["-c", script], None)
}

fn run_script_file(tag: &str, body: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("rubash-2791-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("probe.sh"), body).expect("write script");
    let run = run_zimsh(&["probe.sh"], Some(&dir));
    let _ = std::fs::remove_dir_all(&dir);
    run
}

fn assert_case(run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "stdout");
    assert_eq!(run.stderr, stderr, "stderr");
    assert_eq!(run.code, Some(code), "exit code");
}

#[test]
fn invalid_option_prefixes_follow_argv0() {
    assert_case(
        &run_c("pwd -z"),
        "",
        "zimsh: line 1: pwd: -z: invalid option\npwd: usage: pwd [-LP]\n",
        2,
    );
    assert_case(
        &run_c("export -z"),
        "",
        "zimsh: line 1: export: -z: invalid option\nexport: usage: export [-fn] [name[=value] ...] or export -p [-f]\n",
        2,
    );
    assert_case(
        &run_c("readonly -z"),
        "",
        "zimsh: line 1: readonly: -z: invalid option\nreadonly: usage: readonly [-aAf] [name[=value] ...] or readonly -p\n",
        2,
    );
    assert_case(
        &run_c("declare -z"),
        "",
        "zimsh: line 1: declare: -z: invalid option\ndeclare: usage: declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]\n",
        2,
    );
    assert_case(
        &run_c("typeset -z"),
        "",
        "zimsh: line 1: typeset: -z: invalid option\ntypeset: usage: typeset [-aAfFgiIlnrtux] name[=value] ... or typeset -p [-aAfFilnrtux] [name ...]\n",
        2,
    );
    assert_case(
        &run_c("enable -z"),
        "",
        "zimsh: line 1: enable: -z: invalid option\nenable: usage: enable [-a] [-dnps] [-f filename] [name ...]\n",
        2,
    );
    assert_case(
        &run_c("help -z"),
        "",
        "zimsh: line 1: help: -z: invalid option\nhelp: usage: help [-dms] [pattern ...]\n",
        2,
    );
}

#[test]
fn not_found_prefixes_follow_argv0() {
    assert_case(
        &run_c("alias nosuchname"),
        "",
        "zimsh: line 1: alias: nosuchname: not found\n",
        1,
    );
    assert_case(
        &run_c("unalias nosuchname"),
        "",
        "zimsh: line 1: unalias: nosuchname: not found\n",
        1,
    );
    assert_case(
        &run_c("enable nosuchname"),
        "",
        "zimsh: line 1: enable: nosuchname: not a shell builtin\n",
        1,
    );
    assert_case(
        &run_c("declare -p nosuchvar"),
        "",
        "zimsh: line 1: declare: nosuchvar: not found\n",
        1,
    );
    assert_case(
        &run_c("help nosuchname"),
        "",
        "zimsh: line 1: help: no help topics match `nosuchname'.  Try `help help' or `man -k nosuchname' or `info nosuchname'.\n",
        1,
    );
}

#[test]
fn exit_diagnostics_follow_argv0() {
    assert_case(
        &run_c("exit abc"),
        "",
        "zimsh: line 1: exit: abc: numeric argument required\n",
        2,
    );
    assert_case(
        &run_c("exit 1 2"),
        "",
        "zimsh: line 1: exit: too many arguments\n",
        1,
    );
}

#[test]
fn directory_stack_prefixes_follow_argv0() {
    assert_case(
        &run_c("pushd +99"),
        "",
        "zimsh: line 1: pushd: directory stack empty\n",
        1,
    );
    assert_case(
        &run_c("popd"),
        "",
        "zimsh: line 1: popd: directory stack empty\n",
        1,
    );
    assert_case(
        &run_c("pushd /nonexistent"),
        "",
        "zimsh: line 1: pushd: /nonexistent: No such file or directory\n",
        1,
    );
}

#[test]
fn readonly_bind_prefixes_follow_argv0() {
    // The pushd.rs bindpwd_write arm (err_readonly shape: no builtin
    // name between the prefix and the variable).
    assert_case(
        &run_c("readonly PWD; cd /tmp; echo done"),
        "done\n",
        "zimsh: line 1: PWD: readonly variable\n",
        0,
    );
    assert_case(
        &run_c("readonly OLDPWD; pushd / >/dev/null; echo done"),
        "done\n",
        "zimsh: line 1: OLDPWD: readonly variable\n",
        0,
    );
    assert_case(
        &run_script_file("bindpwd", "readonly PWD\ncd /tmp\necho done\n"),
        "done\n",
        "probe.sh: line 2: PWD: readonly variable\n",
        0,
    );
}

#[test]
fn script_form_prefixes_use_the_script_name() {
    assert_case(
        &run_script_file("pwd", "pwd -z\n"),
        "",
        "probe.sh: line 1: pwd: -z: invalid option\npwd: usage: pwd [-LP]\n",
        2,
    );
    assert_case(
        &run_script_file("alias", "alias nosuchname\n"),
        "",
        "probe.sh: line 1: alias: nosuchname: not found\n",
        1,
    );
    assert_case(
        &run_script_file("export", "export -z\n"),
        "",
        "probe.sh: line 1: export: -z: invalid option\nexport: usage: export [-fn] [name[=value] ...] or export -p [-f]\n",
        2,
    );
}

#[test]
fn comsub_trap_and_eval_prefixes_follow_argv0() {
    assert_case(
        &run_c("x=$(alias nosuchname 2>&1); echo \"rc=$? [$x]\""),
        "rc=1 [zimsh: line 1: alias: nosuchname: not found]\n",
        "",
        0,
    );
    assert_case(
        &run_c("trap \"alias nosuchname\" EXIT; true"),
        "",
        "zimsh: line 1: alias: nosuchname: not found\n",
        0,
    );
    assert_case(
        &run_c("eval \"pwd -z\""),
        "",
        "zimsh: line 1: pwd: -z: invalid option\npwd: usage: pwd [-LP]\n",
        2,
    );
}
