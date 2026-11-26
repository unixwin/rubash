// rubash#3909: extdebug armed at startup (-O extdebug / --debug) must
// latch the BASH_ARGC/BASH_ARGV bottom frame at the bind, exactly like
// GNU's bind_args (shell.c:697-704 -> bind_args shell.c:1504-1529:
// `if (args) { remember_args (...); if (debugging_mode)
// { push_args (...); bash_argv_initialized = 1; } }`).
//
// The true divergence is the WITH-ARGS shapes: with startup extdebug on,
// `-c` / script with args must show ([0]="2") / (y x) inside functions,
// rubash showed (). The NO-ARGS shape A is the opposite: GNU shows ()
// everywhere (variables.c:5640-5650 init_bash_argv sets the
// bash_argv_initialized flag even when the push is a no-op, and
// variables.c:1656-1670 get_bashargcv never re-initializes once the flag
// is set), so the issue's original ([0]="0") expectation for shape A was
// GNU-wrong and is pinned as () here instead.
//
// Every expectation below is byte-level from the GNU bash 5.3.20 oracle
// (~/workspace/rubash-audit/build-gnu/bash-5.3/bash), except the --debug
// cell, which has no GNU counterpart (GNU has no --debug long option;
// rubash --debug is a niu extension that arms extdebug) and is pinned
// as rubash-internal parity with -O extdebug. Shapes that leave extdebug
// ON at function entry are deliberately NOT pinned: the local oracle
// lacks bashdb, so start_debugger clears debugging_mode at startup and
// those shapes read differently on the polluted oracle than on clean
// GNU; the entry-push mechanism itself is already pinned by
// tests/issue2634_extdebug_bash_argc_sticky.rs via runtime enable (which
// runs after start_debugger, so it is pollution-free).
//
// Oracle table (all with `shopt -u extdebug` first unless noted):
//   A  no args, in function:  BASH_ARGC=()  BASH_ARGV=()
//   A  no args, top level:    BASH_ARGC=()  BASH_ARGV=()
//   A  no args, `set -- p q` then top level: BASH_ARGC=()
//   A  no args, `( declare -p BASH_ARGC )`:  BASH_ARGC=()
//   A  no args, re-enable then `f a`: BASH_ARGC=([0]="1") BASH_ARGV=([0]="a")
//   A  no args, re-enable then top level: BASH_ARGC=()
//   A  no args, compat44 then `f a`: BASH_ARGC=()
//   B  `-c` with args, in function: BASH_ARGC=([0]="2") BASH_ARGV=([0]="y" [1]="x")
//   B  `-c` with args, top level: same (synthesis on the old code, latch after the fix)
//   B  `-c` with args, `( f ...; f a )`: same as in-function
//   C  script file with args: same as B, in function and at top level
//   -s with args: same as C
//   D  no extdebug, `-c` with args, in function: BASH_ARGC=() BASH_ARGV=()
//   no extdebug, `set -- p q` then top level: BASH_ARGC=([0]="2")
//   compat44, no startup arming, `f a`: BASH_ARGC=([0]="0")

use std::io::Write;
use std::process::{Command, Stdio};

fn run_args(script: &str, extra_args: &[&str]) -> (String, String, i32) {
    run_leading(&[], script, extra_args)
}

fn run_leading(leading: &[&str], script: &str, extra_args: &[&str]) -> (String, String, i32) {
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let mut cmd = Command::new(rubash);
    for flag in leading {
        cmd.arg(flag);
    }
    cmd.arg("-c").arg(script);
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn run_stdin_script(script: &str, extra_args: &[&str]) -> (String, String, i32) {
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let mut child = Command::new(rubash)
        .arg("-O")
        .arg("extdebug")
        .arg("-s")
        .args(extra_args)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -s");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write stdin script");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("run rubash -s");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn run_script_file(script_text: &str, extra_args: &[&str]) -> (String, String, i32) {
    let path =
        std::env::temp_dir().join(format!("rubash-issue3909-{}-script.sh", std::process::id()));
    std::fs::write(&path, script_text).expect("write temp script");
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(rubash)
        .arg("-O")
        .arg("extdebug")
        .arg(&path)
        .args(extra_args)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash script file");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn assert_clean(stdout: &str, stderr: &str, rc: i32, expected_stdout: &str) {
    assert_eq!(rc, 0, "rc");
    assert_eq!(stderr, "", "stderr must be empty");
    assert_eq!(stdout, expected_stdout, "stdout");
}

const FN_READ: &str = "f(){ declare -p BASH_ARGC; declare -p BASH_ARGV; }; f a";
const TOP_READ: &str = "declare -p BASH_ARGC; declare -p BASH_ARGV";
const EMPTY_BOTH: &str = "declare -a BASH_ARGC=()\ndeclare -a BASH_ARGV=()\n";
const TWO_BOTH: &str =
    "declare -a BASH_ARGC=([0]=\"2\")\ndeclare -a BASH_ARGV=([0]=\"y\" [1]=\"x\")\n";

#[test]
fn startup_no_args_function_lock() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        &format!("shopt -u extdebug; {FN_READ}"),
        &[],
    );
    assert_clean(&out, &err, rc, EMPTY_BOTH);
}

#[test]
fn startup_no_args_top_level() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        &format!("shopt -u extdebug; {TOP_READ}"),
        &[],
    );
    assert_clean(&out, &err, rc, EMPTY_BOTH);
}

#[test]
fn startup_no_args_set_then_top_level() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        "shopt -u extdebug; set -- p q; declare -p BASH_ARGC",
        &[],
    );
    assert_clean(&out, &err, rc, "declare -a BASH_ARGC=()\n");
}

#[test]
fn startup_no_args_subshell_top_level() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        "shopt -u extdebug; ( declare -p BASH_ARGC )",
        &[],
    );
    assert_clean(&out, &err, rc, "declare -a BASH_ARGC=()\n");
}

#[test]
fn startup_no_args_reenable_function() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        &format!("shopt -u extdebug; shopt -s extdebug; {FN_READ}"),
        &[],
    );
    assert_clean(
        &out,
        &err,
        rc,
        "declare -a BASH_ARGC=([0]=\"1\")\ndeclare -a BASH_ARGV=([0]=\"a\")\n",
    );
}

#[test]
fn startup_no_args_reenable_top_level() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        "shopt -u extdebug; shopt -s extdebug; declare -p BASH_ARGC",
        &[],
    );
    assert_clean(&out, &err, rc, "declare -a BASH_ARGC=()\n");
}

#[test]
fn startup_no_args_compat44_function() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        "shopt -u extdebug; shopt -s compat44; f(){ declare -p BASH_ARGC; }; f a",
        &[],
    );
    assert_clean(&out, &err, rc, "declare -a BASH_ARGC=()\n");
}

#[test]
fn startup_no_args_extdebug_still_on_top_level() {
    let (out, err, rc) = run_leading(&["-O", "extdebug"], TOP_READ, &[]);
    assert_clean(&out, &err, rc, EMPTY_BOTH);
}

#[test]
fn startup_with_args_function() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        &format!("shopt -u extdebug; {FN_READ}"),
        &["name", "x", "y"],
    );
    assert_clean(&out, &err, rc, TWO_BOTH);
}

#[test]
fn startup_with_args_top_level_lock() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        &format!("shopt -u extdebug; {TOP_READ}"),
        &["name", "x", "y"],
    );
    assert_clean(&out, &err, rc, TWO_BOTH);
}

#[test]
fn startup_with_args_subshell_function() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        &format!("shopt -u extdebug; ( {FN_READ} )"),
        &["name", "x", "y"],
    );
    assert_clean(&out, &err, rc, TWO_BOTH);
}

#[test]
fn script_file_with_args() {
    let script = format!("shopt -u extdebug\n{FN_READ}\n{TOP_READ}\n");
    let (out, err, rc) = run_script_file(&script, &["x", "y"]);
    assert_clean(&out, &err, rc, &format!("{TWO_BOTH}{TWO_BOTH}"));
}

#[test]
fn stdin_with_args() {
    let script = format!("shopt -u extdebug\n{FN_READ}\n{TOP_READ}\n");
    let (out, err, rc) = run_stdin_script(&script, &["x", "y"]);
    assert_clean(&out, &err, rc, &format!("{TWO_BOTH}{TWO_BOTH}"));
}

#[test]
fn debug_flag_with_args_function() {
    let (out, err, rc) = run_leading(
        &["--debug"],
        &format!("shopt -u extdebug; {FN_READ}"),
        &["name", "x", "y"],
    );
    assert_clean(&out, &err, rc, TWO_BOTH);
}

#[test]
fn no_extdebug_with_args_function_lock() {
    let (out, err, rc) = run_args(FN_READ, &["name", "x", "y"]);
    assert_clean(&out, &err, rc, EMPTY_BOTH);
}

#[test]
fn no_extdebug_set_then_top_level_lock() {
    let (out, err, rc) = run_args("set -- p q; declare -p BASH_ARGC", &[]);
    assert_clean(&out, &err, rc, "declare -a BASH_ARGC=([0]=\"2\")\n");
}

#[test]
fn startup_no_args_function_after_set_lock() {
    let (out, err, rc) = run_leading(
        &["-O", "extdebug"],
        "shopt -u extdebug; set -- p q; f(){ declare -p BASH_ARGC; }; f a",
        &[],
    );
    assert_clean(&out, &err, rc, "declare -a BASH_ARGC=()\n");
}

#[test]
fn no_extdebug_top_level_with_args_lock() {
    let (out, err, rc) = run_args(TOP_READ, &["name", "x", "y"]);
    assert_clean(&out, &err, rc, TWO_BOTH);
}

#[test]
fn compat44_function_no_startup_lock() {
    let (out, err, rc) = run_args(
        "shopt -s compat44; f(){ declare -p BASH_ARGC; }; f a",
        &["name", "x", "y"],
    );
    assert_clean(&out, &err, rc, "declare -a BASH_ARGC=([0]=\"0\")\n");
}
