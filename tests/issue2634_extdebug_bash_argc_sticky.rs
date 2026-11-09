//! rubash#2634 — after `shopt -s extdebug` is turned back off, GNU still
//! creates BASH_ARGC with the bottom frame (`([0]="0")`) at function call
//! time; rubash renders an empty array.
//!
//! GNU anchor: the BASH_ARGV/BASH_ARGC bottom frame is latched exactly
//! once by `init_bash_argv` (variables.c:5640-5650, static
//! `bash_argv_initialized`), which pushes the current positional
//! parameters via `save_bash_argv`/`push_args` (variables.c:5652-5690).
//! Three triggers call it: (1) `shopt -s extdebug` —
//! builtins/shopt.def:618-626 `shopt_set_debug_mode` calls
//! `init_bash_argv()` when `debugging_mode` is set; (2) function entry
//! when `debugging_mode || shell_compatibility_level <= 44`
//! (execute_cmd.c:5337-5338; `push_args` itself runs only in
//! debugging_mode, execute_cmd.c:5342-5348); (3) a top-level reference
//! via variables.c:1656-1670 `get_bashargcv`. `shopt -u extdebug` clears
//! `debugging_mode` only — it never unbinds the arrays or resets the
//! latch, so the bottom frame stays observable inside functions after
//! the option is turned back off.
//!
//! rubash models the arrays as composed views
//! (executor/dynamic_arrays.rs `bash_argc_view`/`bash_arg_bottom_frame`)
//! and implements triggers (2) (executor/function_calls.rs:616-653) and
//! (3) (dynamic_arrays.rs:1399-1420, top-level reads only), but the
//! shopt extdebug arm (builtins/shopt.rs:671-675) only mirrors the
//! functrace/errtrace flags — trigger (1) is missing, so after
//! on->off an in-function read finds no bottom frame and renders `()`.
//!
//! Expectations are byte-level from the GNU bash 5.3.20 oracle
//! (probes in the lane REPORT). Cells that pass on the base tree are
//! regression locks for the neighbouring materialization paths and are
//! marked as such.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions plus exit code and empty-stderr assertions.
//! `LC_ALL=C.UTF-8` is set on the child explicitly so the pin is
//! hermetic under any ambient locale.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    run_args(script, &[])
}

fn run_args(script: &str, extra_args: &[&str]) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .args(extra_args)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn expect(script: &str, expected_stdout: &[u8], ctx: &str) {
    expect_args(script, &[], expected_stdout, ctx);
}

fn expect_args(script: &str, extra_args: &[&str], expected_stdout: &[u8], ctx: &str) {
    let out = run_args(script, extra_args);
    assert_eq!(
        out.stdout,
        expected_stdout,
        "{ctx}: stdout mismatch; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        out.code,
        0,
        "{ctx}: rc must be 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------
// Defect cells: the bottom frame latched by `shopt -s extdebug` must
// stay visible inside functions after the option is turned off again.
// ---------------------------------------------------------------------

#[test]
fn seed_on_off_in_function() {
    // The issue's regression seed.
    expect(
        "shopt -s extdebug; shopt -u extdebug; f(){ declare -p BASH_ARGC; }; f a",
        b"declare -a BASH_ARGC=([0]=\"0\")\n",
        "seed: extdebug on->off, declare BASH_ARGC inside f",
    );
}

#[test]
fn on_off_positional_in_function() {
    // The latched bottom frame carries the positional parameters that
    // were current when extdebug was enabled (stack order: last first).
    expect_args(
        "shopt -s extdebug; shopt -u extdebug; f(){ declare -p BASH_ARGC; declare -p BASH_ARGV; }; f a",
        &["_", "p", "q"],
        b"declare -a BASH_ARGC=([0]=\"2\")\ndeclare -a BASH_ARGV=([0]=\"q\" [1]=\"p\")\n",
        "on->off with positional params, read inside f",
    );
}

#[test]
fn on_off_nested_calls() {
    // No frames are pushed while extdebug is off, however deep the
    // call chain goes; only the latched bottom frame shows.
    expect(
        "shopt -s extdebug; shopt -u extdebug; g(){ declare -p BASH_ARGC; }; f(){ g x; }; f a",
        b"declare -a BASH_ARGC=([0]=\"0\")\n",
        "on->off, nested f -> g, declare inside g",
    );
}

#[test]
fn on_off_repeated_calls_persist() {
    // The latched frame is not consumed by a call: a second call with
    // a different arity observes the same bottom frame.
    expect(
        "shopt -s extdebug; shopt -u extdebug; f(){ declare -p BASH_ARGC; }; f a; f a b c",
        b"declare -a BASH_ARGC=([0]=\"0\")\ndeclare -a BASH_ARGC=([0]=\"0\")\n",
        "on->off, two calls with different arities",
    );
}

#[test]
fn on_off_element_and_length_reads() {
    // Element/length reads inside a function see the same latched
    // frame the declare rendering shows.
    expect(
        "shopt -s extdebug; shopt -u extdebug; f(){ echo \"e0=${BASH_ARGC[0]} n=${#BASH_ARGC[@]} argvn=${#BASH_ARGV[@]}\"; }; f a",
        b"e0=0 n=1 argvn=0\n",
        "on->off, element and length reads inside f",
    );
}

#[test]
fn enable_after_first_call_chain() {
    // Chain: a call before extdebug was ever enabled renders (); the
    // on->off latch then feeds the next call, which must render the
    // bottom frame. Intermediate state is asserted at each step.
    expect(
        "f(){ declare -p BASH_ARGC; }; f a; shopt -s extdebug; shopt -u extdebug; f a",
        b"declare -a BASH_ARGC=()\ndeclare -a BASH_ARGC=([0]=\"0\")\n",
        "chain: call, then extdebug on->off, then call again",
    );
}

// ---------------------------------------------------------------------
// Regression locks: neighbouring materialization paths that already
// agree with GNU on the base tree and must not move.
// ---------------------------------------------------------------------

#[test]
fn never_enabled_lock() {
    expect(
        "f(){ declare -p BASH_ARGC; declare -p BASH_ARGV; }; f a",
        b"declare -a BASH_ARGC=()\ndeclare -a BASH_ARGV=()\n",
        "lock: extdebug never enabled",
    );
}

#[test]
fn only_disabled_lock() {
    expect(
        "shopt -u extdebug; f(){ declare -p BASH_ARGC; }; f a",
        b"declare -a BASH_ARGC=()\n",
        "lock: only `shopt -u extdebug`, never enabled",
    );
}

#[test]
fn on_off_top_level_read_lock() {
    // A top-level read materializes the bottom frame through
    // get_bashargcv (trigger 3) even with extdebug off.
    expect(
        "shopt -s extdebug; shopt -u extdebug; declare -p BASH_ARGC; declare -p BASH_ARGV",
        b"declare -a BASH_ARGC=([0]=\"0\")\ndeclare -a BASH_ARGV=()\n",
        "lock: on->off, top-level declare",
    );
}

#[test]
fn on_off_positional_top_level_lock() {
    expect_args(
        "shopt -s extdebug; shopt -u extdebug; declare -p BASH_ARGC; declare -p BASH_ARGV",
        &["_", "p", "q"],
        b"declare -a BASH_ARGC=([0]=\"2\")\ndeclare -a BASH_ARGV=([0]=\"q\" [1]=\"p\")\n",
        "lock: on->off with positional params, top-level declare",
    );
}

#[test]
fn on_top_level_read_lock() {
    expect(
        "shopt -s extdebug; declare -p BASH_ARGC; declare -p BASH_ARGV",
        b"declare -a BASH_ARGC=([0]=\"0\")\ndeclare -a BASH_ARGV=()\n",
        "lock: extdebug on, top-level declare",
    );
}

#[test]
fn enabled_call_counts_lock() {
    // With extdebug on, the current frame's count/args sit at index 0
    // and the bottom frame below them.
    expect(
        "shopt -s extdebug; f(){ declare -p BASH_ARGC; declare -p BASH_ARGV; }; f a b",
        b"declare -a BASH_ARGC=([0]=\"2\" [1]=\"0\")\ndeclare -a BASH_ARGV=([0]=\"b\" [1]=\"a\")\n",
        "lock: extdebug on, f a b",
    );
}

#[test]
fn reenable_restores_counts_lock() {
    // On -> off -> on: function entry under debugging_mode pushes the
    // live frame above the latched bottom frame again.
    expect(
        "shopt -s extdebug; shopt -u extdebug; shopt -s extdebug; f(){ declare -p BASH_ARGC; declare -p BASH_ARGV; }; f a",
        b"declare -a BASH_ARGC=([0]=\"1\" [1]=\"0\")\ndeclare -a BASH_ARGV=([0]=\"a\")\n",
        "lock: on->off->on, f a",
    );
}
