//! rubash#636: `a+=( [@]=q )` must report `cannot assign to non-numeric
//! index`, not take the arithmetic-error branch.
//!
//! GNU rejects ALL_ELEMENT_SUB before arithmetic eval, naming the whole
//! element word:
//!
//! ```c
//! /* arrayfunc.c:786-792 assign_compound_array_list() */
//! if (ALL_ELEMENT_SUB (w[1]) && len == 2 && array_p (var))
//!   {
//!     set_exit_status (EXECUTION_FAILURE);
//!     report_error (_("%s: cannot assign to non-numeric index"), w);
//!     any_failed++;
//!     break;
//!   }
//! ```
//!
//! rubash fell through to `eval_indexed_subscript_expression`, reporting
//! `@: arithmetic syntax error`. The guard below matches the resolved
//! subscript exactly (`[@]`, `['@']`, `[$k]`, both `=`/`+=` tails) on the
//! indexed compound arms only — assoc `@` stays a valid key and `[ @ ]`
//! stays arithmetic.
//!
//! Scope note: on the `declare` route the message is fixed here but the
//! abandon itself arrives with #635's builtin fatal channel
//! (`declare -a d=( [@]=q )` still continues today, exactly like the
//! pre-existing `[]=` gap on that route). The declare test below pins the
//! message bytes only, deliberately not the status.

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

/// Whole-word rejection: exact message, nothing on stdout, status 1,
// no `AFTER` (GNU breaks the list — `any_failed++; break`).
fn assert_atstar_rejected(script: &str, word: &str) {
    let out = run(script);
    assert_eq!(out.stdout, "", "{script} -> stdout {:?}", out.stdout);
    assert_eq!(
        out.stderr,
        format!("bash: line 1: {word}: cannot assign to non-numeric index\n"),
        "{script} -> stderr {:?}",
        out.stderr
    );
    assert_eq!(out.status, 1, "{script} -> status {:?}", out.status);
}

#[test]
fn append_at_rejected() {
    assert_atstar_rejected("a=(p); a+=( [@]=q ); echo AFTER", "[@]=q");
}

#[test]
fn append_star_pluseq_rejected() {
    assert_atstar_rejected("a=(p); a+=( [*]+=q ); echo AFTER", "[*]+=q");
}

#[test]
fn reset_form_rejected() {
    assert_atstar_rejected("a=( [@]=q ); echo AFTER", "[@]=q");
}

#[test]
fn quoted_and_indirect_spellings_rejected() {
    assert_atstar_rejected("a+=( ['@']=q ); echo AFTER", "[@]=q");
    assert_atstar_rejected("k=@; a+=( [$k]=q ); echo AFTER", "[@]=q");
}

#[test]
fn partial_store_kept() {
    // Elements before the failure stay stored (GNU scans and breaks). Via
    // eval so the contained failure lets the outer list observe the state
    // (same-line abandon would skip the probe on both shells).
    let out = run("a=(p); eval \"a+=( [0]=x [@]=q [1]=y )\"; echo \"rc=$?\"; declare -p a");
    assert_eq!(out.stdout, "rc=1\ndeclare -a a=([0]=\"x\")\n");
    assert_eq!(out.status, 0);
    assert_eq!(out.status, 0);
}

#[test]
fn declare_route_message_fixed_status_with_635() {
    // Message bytes are #636; the abandon itself is #635's channel.
    let out = run("declare -a d=( [@]=q ); echo AFTER");
    assert_eq!(
        out.stderr,
        "bash: line 1: [@]=q: cannot assign to non-numeric index\n"
    );
}

#[test]
fn single_element_route_untouched() {
    // `a[@]=q` is a different GNU owner (arrayfunc.c:355-360) with its own
    // message — must not change.
    let out = run("a[@]=q; echo AFTER");
    assert_eq!(out.stderr, "bash: line 1: a[@]: bad array subscript\n");
    assert_eq!(out.status, 1);
}

#[test]
fn spaced_subscript_stays_arithmetic() {
    // `[ @ ]` is not ALL_ELEMENT_SUB (exact match, no trim).
    let out = run("a+=( [ @ ]=q ); echo AFTER");
    assert!(
        out.stderr.contains("arithmetic syntax error"),
        "{:?}",
        out.stderr
    );
    assert_eq!(out.status, 1);
}

#[test]
fn assoc_at_key_untouched() {
    let out = run("declare -A A; A+=( [@]=q ); echo \"${A[@]}\"");
    assert_eq!(out.stdout, "q\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, 0);
}
