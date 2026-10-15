//! rubash#631: a compound element header whose negative subscript resolves
//! before the array start is a fatal diagnostic, not a silent skip.
//!
//! GNU assign_compound_array_list computes `ind` and, when it is still below
//! zero after the count-back `array_max_index + 1 + ind` (arrayfunc.c:802-803),
//! calls `err_badarraysub (w)` — the whole element word, via report_error
//! (error.c:440; message arrayfunc.c:63) — counts `any_failed` and `break`s
//! the list (arrayfunc.c:804-809). The elements it already inserted stay
//! stored (it array_inserts as it scans), and assign_array_var_from_string
//! (arrayfunc.c:913-932) then returns NULL so the assignment fails with
//! status 1.
//!
//! rubash's `array_assignment_index` returned `Option<usize>`, conflating
//! "this word is not a `[subscript]` header" with "this subscript was
//! rejected", so the caller `continue`d and the element vanished.
//!
//! Expectations come from WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`) script
//! probes.

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

const BAD_SUB: &str = "[-2]=q: bad array subscript";

#[test]
fn out_of_range_negative_append_is_reported_and_aborts() {
    // GNU: `line 1: [-2]=q: bad array subscript`, no AFTER, status 1.
    let out = run("a=(p); a+=([-2]=q); echo AFTER");
    assert_eq!(
        out.stdout, "",
        "the list after the failing list is discarded"
    );
    assert_eq!(out.status, 1);
    assert!(
        out.stderr.contains(BAD_SUB),
        "stderr should name the element word: {:?}",
        out.stderr
    );
}

#[test]
fn out_of_range_negative_reset_compound_is_reported() {
    // The non-append `name=(...)` form walks the same list (GNU
    // assign_array_var_from_string only decodes the compound text).
    let out = run("a=(p); a=( [-2]=q ); echo AFTER");
    assert_eq!(out.stdout, "");
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains(BAD_SUB), "{:?}", out.stderr);
}

#[test]
fn out_of_range_negative_append_on_empty_array_is_reported() {
    // `array_max_index` on an empty array has no element to count back from,
    // so `[-1]` is out of range too.
    let out = run("a=(); a+=([-1]=z); echo AFTER");
    assert_eq!(out.stdout, "");
    assert_eq!(out.status, 1);
    assert!(
        out.stderr.contains("[-1]=z: bad array subscript"),
        "{:?}",
        out.stderr
    );
}

#[test]
fn elements_written_before_the_bad_subscript_stay_stored() {
    // GNU inserts as it scans, so `[0]=x` lands before `[-2]` fails. Observe
    // through an EXIT trap (the failing assignment otherwise ends the shell).
    let out = run("trap 'echo \"T:${a[*]}\"' EXIT; a=(p); a+=([0]=x [-2]=q)");
    assert!(
        out.stdout.contains("T:x"),
        "the element before the failure must be stored: {:?}",
        out.stdout
    );
}

#[test]
fn the_scan_stops_at_the_bad_subscript() {
    // A later element is never processed: GNU breaks out of the list.
    let out = run("trap 'echo \"T:${a[*]}\"' EXIT; a=(p); a+=([-2]=q [0]=x)");
    assert!(
        out.stdout.contains("T:p"),
        "elements after the failure must not be stored: {:?}",
        out.stdout
    );
}

#[test]
fn bad_subscript_unwinds_the_enclosing_function() {
    // GNU discards the command list: the function body and the caller both
    // stop.
    let out = run("f(){ a=(p); a+=([-2]=q); echo IN; }; f; echo AFTER");
    assert_eq!(out.stdout, "");
    assert_eq!(out.status, 1);
}

#[test]
fn bad_subscript_ends_only_the_subshell() {
    // Inside `( )` the jump ends the subshell and the parent carries on.
    let out = run("( a=(p); a+=([-2]=q); echo SUB ); echo AFTER");
    assert_eq!(out.stdout, "AFTER\n");
    assert_eq!(out.status, 0);
    assert!(out.stderr.contains(BAD_SUB), "{:?}", out.stderr);
}

#[test]
fn subscripts_above_the_array_start_are_untouched() {
    // Controls: in-range negatives keep counting from the end, positives are
    // sparse inserts, and an assoc key is a literal string.
    let out = run("a=(p q r); a+=([-1]=z); declare -p a");
    assert_eq!(out.stdout, "declare -a a=([0]=\"p\" [1]=\"q\" [2]=\"z\")\n");

    let out = run("a=(p); a+=([2]=q); declare -p a");
    assert_eq!(out.stdout, "declare -a a=([0]=\"p\" [2]=\"q\")\n");

    let out = run("declare -A m; m+=([-2]=v); declare -p m");
    assert_eq!(out.stdout, "declare -A m=([-2]=\"v\" )\n");
}

#[test]
fn in_range_negative_increment_still_appends() {
    let out = run("a=(p q); a+=([-1]+=z); declare -p a");
    assert_eq!(out.stdout, "declare -a a=([0]=\"p\" [1]=\"qz\")\n");
}
