//! Issue rubash#3454 — under `set -u`, a compound command whose own
//! redirect target is an unbound variable must fail the expansion with
//! `name: unbound variable` and abort the script (GNU execute_cmd.c:854
//! do_redirections, shared by every compound kind), not expand the word
//! to the empty string, print `: Is a directory`, and continue.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    #[cfg(unix)]
    std::os::unix::process::CommandExt::arg0(&mut command, "bash");
    let output = command
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_aborts(script: &str, name: &str) {
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stdout, "", "stdout={stdout:?}");
    assert!(
        stderr.contains(&format!("{name}: unbound variable")),
        "stderr={stderr:?}"
    );
    assert!(!stderr.contains(": Is a directory"), "stderr={stderr:?}");
    assert_eq!(code, Some(1), "rc={code:?}");
}

#[test]
fn if_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\nif true; then echo yes; fi > $undefined_target_xyz\necho after=$?\n",
        "undefined_target_xyz",
    );
}

#[test]
fn while_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\nwhile false; do echo x; done > $undefined2\necho after=$?\n",
        "undefined2",
    );
}

#[test]
fn for_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\nfor i in 1 2; do echo $i; done > $undefined3\necho after=$?\n",
        "undefined3",
    );
}

#[test]
fn arith_for_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\nfor ((i=0;i<1;i++)); do :; done > $undefined_af\necho after=$?\n",
        "undefined_af",
    );
}

#[test]
fn until_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\nuntil true; do :; done > $undefined_u\necho after=$?\n",
        "undefined_u",
    );
}

#[test]
fn case_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\ncase x in x) echo yes;; esac > $undefined4\necho after=$?\n",
        "undefined4",
    );
}

#[test]
fn cond_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\n[[ -n x ]] > $undefined5\necho after=$?\n",
        "undefined5",
    );
}

#[test]
fn select_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\nselect s in a b; do echo $s; done > $undefined6\necho after=$?\n",
        "undefined6",
    );
}

#[test]
fn if_input_redirect_unbound_aborts() {
    assert_aborts(
        "set -u\nif true; then echo yes; fi < $nope_in\necho after=$?\n",
        "nope_in",
    );
}

/// The `time`-prefixed word-sequence path reparses the compound and
/// carries the redirects on the synthetic `done`/`fi` node — GNU still
/// aborts, with the diagnostic preceding the timing report.
#[test]
fn time_while_redirect_unbound_aborts() {
    let (stdout, stderr, code) =
        rubash("set -u\ntime while false; do :; done > $undefined_t\necho after=$?\n");
    assert_eq!(stdout, "", "stdout={stdout:?}");
    assert!(
        stderr.contains("undefined_t: unbound variable"),
        "stderr={stderr:?}"
    );
    // GNU prints the timing report after the diagnostic on the way out.
    assert!(stderr.contains("real"), "stderr={stderr:?}");
    assert_eq!(code, Some(1), "rc={code:?}");
}

/// Controls: a bound target still runs the compound and continues.
#[test]
fn bound_target_still_runs() {
    let (stdout, stderr, code) = rubash(
        "set -u\nt=/tmp/rubash3454out\nif true; then echo yes; fi > $t\necho after=$?\ncat $t\n",
    );
    assert_eq!(stdout, "after=0\nyes\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
    assert_eq!(code, Some(0), "rc={code:?}");
}

/// Without `set -u` the unbound name expands to the empty target — the
/// pre-existing non-nounset behavior is untouched.
#[test]
fn no_nounset_keeps_old_behavior() {
    let (stdout, _, code) = rubash("if true; then echo yes; fi > $undefined_xyz\necho after=$?\n");
    assert_eq!(stdout, "after=1\n", "stdout={stdout:?}");
    assert_eq!(code, Some(0), "rc={code:?}");
}
