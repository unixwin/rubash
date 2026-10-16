//! rubash#639: arithmetic array subscripts skip `set -u` on the write path.
//!
//! GNU evaluates every subscript through `evalexp` → `expr_streval`, whose
//! unbound-variable block aborts on both read and write:
//!
//! ```c
//! /* expr.c:1190-1216 expr_streval() */
//! if ((v == 0 || invisible_p (v)) && unbound_vars_is_error)
//!   {
//!     value = (e == ']') ? array_variable_name (...) : tok;
//!     set_exit_status (EXECUTION_FAILURE);
//!     err_unboundvar (value);          /* error.c:447 */
//!     ...
//!     else
//!       jump_to_top_level (FORCE_EOF); /* noninteractive: abandon list */
//!   }
//! ```
//!
//! rubash's write-path subscript evaluator
//! (`eval_indexed_subscript_expression`) was missing the nounset prologue
//! its arithmetic command/expansion siblings have, so `set -u; a[k]=v`
//! silently stored with status 0. The trap is FORCE_EOF, not a plain
//! bad-subscript DISCARD: the list is abandoned with status 127 at the
//! `-c` top level (shell.c:1471) and 1 in script mode.
//!
//! Expectations come from the C source above; spot-checked against GNU Bash
//! 5.3.20 locally (the repo oracle is WSL 5.3.0 — no version-specific
//! behavior is asserted here).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

/// A nounset subscript trap: exactly one `k: unbound variable` line on
/// stderr, nothing on stdout, fatal status (127 under `-c`).
fn assert_subscript_unbound(script: &str, name: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout, "",
        "a trapped subscript must print nothing: {script} -> {:?}",
        out.stdout
    );
    assert_eq!(
        out.stderr,
        format!("bash: line 1: {name}: unbound variable\n"),
        "{script} -> stderr {:?}",
        out.stderr
    );
    assert_eq!(out.status, 127, "{script} -> status {:?}", out.status);
}

#[test]
fn element_write_traps() {
    assert_subscript_unbound("set -u; a[k]=1", "k");
}

#[test]
fn element_write_arithmetic_subscript_traps() {
    assert_subscript_unbound("set -u; a[k+1]=v", "k");
}

#[test]
fn element_append_traps() {
    assert_subscript_unbound("set -u; a[k]+=v", "k");
}

#[test]
fn declared_array_write_traps() {
    assert_subscript_unbound("set -u; declare -a a; a[k]=v", "k");
}

#[test]
fn compound_writes_trap() {
    assert_subscript_unbound("set -u; m=([k]=v)", "k");
    assert_subscript_unbound("set -u; a=([k]=v [j]=w)", "k");
    assert_subscript_unbound("set -u; declare -a m=([k]=v)", "k");
}

#[test]
fn existing_array_unset_traps() {
    // GNU evaluates the subscript when the array exists (and skips the
    // trap for a missing array, where there is nothing to unbind).
    assert_subscript_unbound("set -u; a=(p q); unset \"a[k]\"", "k");
    assert_subscript_unbound("set -u; a=(p q); unset \"a[k]\"; echo after", "k");
    let out = run("set -u; unset \"a[k]\"; echo after");
    assert_eq!(out.stdout, "after\n");
    assert_eq!(out.status, 0);
}

#[test]
fn nested_lists_abandon_with_fatal_status() {
    // FORCE_EOF unwinds through function and if bodies alike (a trapped
    // tail with no following commands still converts at the reader top).
    assert_subscript_unbound("set -u; f(){ a[k]=v; echo in; }; f; echo after", "k");
    assert_subscript_unbound("set -u; if true; then a[k]=v; fi; echo after", "k");
    assert_subscript_unbound("set -u; f(){ a[k]=v; }; f", "k");
}

#[test]
fn dollar_subscript_already_trapped() {
    // `$`-prefixed subscripts trapped before this fix; they keep working.
    assert_subscript_unbound("set -u; a[$unset]=v", "unset");
}

#[test]
fn bound_names_pass_silently() {
    let out = run("set -u; k=5; a[k]=v; echo \"${a[k]}\"");
    assert_eq!(out.stdout, "v\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, 0);
}

#[test]
fn assoc_literal_keys_stay_silent() {
    // Assoc keys are data, not arithmetic: no trap by construction.
    let out = run("set -u; declare -A A; A[k]=v; echo \"${A[k]}\"");
    assert_eq!(out.stdout, "v\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, 0);
}

#[test]
fn toggle_off_then_on() {
    // A store without nounset must not memoize away a later trap, and a
    // trap must not poison later stores: the memo frames are per
    // word-expansion, so each statement re-evaluates.
    assert_subscript_unbound("a[k]=v; set -u; a[k]=w", "k");
    let out = run("set -u; k=5; a[k]=v; set +u; unset k; a[j]=w; echo \"${a[j]}\"");
    assert_eq!(out.stdout, "w\n");
    assert_eq!(out.status, 0);
}
