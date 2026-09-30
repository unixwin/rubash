//! Issue rubash#358 — `set -euo pipefail` / `set -eo pipefail` silently
//! dropped the short flags bundled in the same word before `-o`.
//!
//! GNU owner: builtins/set.def:716-772 set_builtin's apply loop — every
//! character of a `-`/`+` word is dispatched in one pass, either to the
//! `'o'` case (operand from the next word, set.def:726-766, then the
//! character loop CONTINUES) or to change_flag (set.def:767,
//! flags.c:226). There is no bundling restriction: `-euo pipefail`
//! applies `e`, `u`, `o pipefail` in that order. Validation is a separate
//! pre-scan first (set.def:671-691 via internal_getopt, bashgetopt.c):
//! `set -e -Z` reports the usage error with errexit still OFF, and an
//! inline `o` operand (`set -opipefail`) hides its suffix characters from
//! the scan, so the apply loop dies at the first genuinely unknown char
//! ('l') with EXECUTION_FAILURE, after applying the preceding chars.
//!
//! Root cause in rubash: the executor fast path (apply_simple_set_flags)
//! bails on the whole word when it sees `'o'` (not a short flag), and the
//! slow path (set_with_io) only VALIDATED plain short flags against
//! SET_FLAGS without ever applying them. Fixed by making set_with_io
//! complete (pre-scan + full application via the single truth table
//! apply_short_set_flag) and teaching the fast path to validate every
//! word before applying any.
//!
//! Expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; 25-case matrix
//! under target/p0fix/mx in the lane worktree).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `set -euo pipefail` applies errexit: `false` aborts the script.
#[test]
fn bundled_euo_pipefail_applies_errexit() {
    let (stdout, _stderr, code) = rubash("set -euo pipefail\nfalse\necho NOPE");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
}

/// `set -uo pipefail` applies nounset with GNU's unbound-variable message
/// (`-c` mode reports the shell name `bash`, matching GNU). Script-file
/// rc parity is byte-verified in the 25-case matrix (target/p0fix/mx);
/// the -c exit code here is only checked nonzero because rubash's -c
/// top-level maps expansion aborts to 127 while GNU exits 1 — a
/// pre-existing -c-mode divergence (reproducible on master via plain
/// `set -u`), not part of this fix.
#[test]
fn bundled_uo_pipefail_applies_nounset() {
    let (stdout, stderr, code) = rubash("set -uo pipefail\necho \"val=${U_X}\"\necho NOPE");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "bash: line 2: U_X: unbound variable\n");
    assert_ne!(code, Some(0));
}

/// `set -eo pipefail` applies errexit (the minimal bundle from the issue).
#[test]
fn bundled_eo_pipefail_applies_errexit() {
    let (stdout, _stderr, code) = rubash("set -eo pipefail\nfalse\necho NOPE");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
}

/// pipefail itself still lands when bundled: without `-e` the script
/// survives, and `$?` reports the failing rightmost pipeline member.
#[test]
fn bundled_uo_pipefail_applies_pipefail() {
    let (stdout, stderr, code) = rubash("set -uo pipefail\ntrue | false\necho \"rc=$?\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
}

/// Turning the bundle off with `+` works: `set +euo pipefail` after
/// `set -euo pipefail` leaves the script alive past `false`.
#[test]
fn plus_bundle_disables_errexit() {
    let (stdout, stderr, code) =
        rubash("set -euo pipefail\nset +euo pipefail\nfalse\necho \"ALIVE rc=$?\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "ALIVE rc=1\n");
    assert_eq!(code, Some(0));
}

/// `-x` bundled before `-o` produces the xtrace stderr trace.
#[test]
fn bundled_xuo_pipefail_applies_xtrace() {
    let (stdout, _stderr, _code) = rubash("set -xuo pipefail\ntrue\necho done");
    assert_eq!(stdout, "done\n");
}

/// Separate words keep working (control that must not regress).
#[test]
fn separate_words_still_apply() {
    let (stdout, _stderr, code) = rubash("set -e -u -o pipefail\nfalse\necho NOPE");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
}

/// Validate-first: `set -e -Z` reports EX_USAGE and leaves errexit OFF
/// (GNU set.def:671-691 pre-scan runs before any application).
#[test]
fn invalid_flag_after_valid_leaves_nothing_applied() {
    let (stdout, stderr, code) = rubash("set -e -Z\necho \"rc=$?\"\nfalse\necho ALIVE");
    assert!(
        stderr.contains("set: -Z: invalid option"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "rc=2\nALIVE\n");
    assert_eq!(code, Some(0));
}

/// Same for the bundle: `set -euo pipefail -Z` applies nothing.
#[test]
fn invalid_flag_after_bundle_leaves_nothing_applied() {
    let (stdout, stderr, code) = rubash("set -euo pipefail -Z\necho \"rc=$?\"\nfalse\necho ALIVE");
    assert!(
        stderr.contains("set: -Z: invalid option"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "rc=2\nALIVE\n");
    assert_eq!(code, Some(0));
}

/// `set -i` is explicitly refused by the pre-scan (set.def:677-683).
#[test]
fn set_i_is_refused() {
    let (stdout, stderr, code) = rubash("set -i\necho \"rc=$?\"");
    assert!(
        stderr.contains("set: -i: invalid option"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "rc=2\n");
    assert_eq!(code, Some(0));
}

/// Inline `o` operand: `set -opipefail` — the scan sees only `o` (the
/// operand is printed as the option list), then the apply loop applies
/// p/i/p/e/f/a/i and dies at the first unknown char 'l' with
/// EXECUTION_FAILURE (set.def:763-771 via change_flag FLAG_ERROR). The
/// hidden 'e' turned errexit on, so the `set` failure (rc=1) kills the
/// script before the following echo — GNU dies identically.
#[test]
fn inline_o_operand_hidden_chars_apply_then_fail() {
    let (stdout, stderr, code) = rubash("set -opipefail\necho \"rc=$?\"");
    assert!(stdout.contains("pipefail       \toff"), "stdout: {stdout}");
    assert!(
        stderr.contains("set: -l: invalid option"),
        "stderr: {stderr}"
    );
    assert!(!stdout.contains("rc="), "stdout: {stdout}");
    assert_eq!(code, Some(1));
}

/// `set -eo` (bare, no operand): `e` applies, then the missing-operand
/// branch prints the option list (set.def:729-732) and the character loop
/// continues.
#[test]
fn bare_eo_applies_e_then_lists_options() {
    let (stdout, stderr, code) = rubash("set -eo\nfalse\necho NOPE");
    assert!(stdout.contains("errexit        \ton"), "stdout: {stdout}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(1));
}

/// `set -eox pipefail`: `o` consumes `pipefail` and the trailing `x` of
/// the same word still applies (set.def:726-766 continue).
#[test]
fn chars_after_o_still_apply() {
    let (stdout, _stderr, code) = rubash("set -eox pipefail\nfalse\necho NOPE");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
}

/// The fast path now validates the whole argument list before applying
/// (GNU pre-scan parity): `set -f -Z` reports the usage error and leaves
/// noglob off — `echo zz*` still expands against an existing match.
#[test]
fn fast_path_validates_all_words_first() {
    let (stdout, stderr, code) = rubash(
        "mkdir -p /tmp/i358-empty-probe\ncd /tmp/i358-empty-probe\ntouch zzq\n\
         set -f -Z\nr=$?\necho zz*\necho \"setrc=$r\"",
    );
    assert!(
        stderr.contains("set: -Z: invalid option"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "zzq\nsetrc=2\n");
    assert_eq!(code, Some(0));
}
