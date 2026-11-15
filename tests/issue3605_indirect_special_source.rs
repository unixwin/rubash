//! rubash#3605 — `${!<special|positional><op>tail}` indirection: an
//! indirect SOURCE may be a positional index or a VALID_INDIR_PARAM
//! special (`#`, `?`, `@`, `*`), and the resolved text feeds the
//! operator word like any other `${name<op>word}`.
//!
//! GNU anchors (bash 5.3 subst.c):
//! - `want_indir` (subst.c:9920-9922): `!` + legal_variable_starter |
//!   DIGIT | VALID_INDIR_PARAM. VALID_INDIR_PARAM (subst.c:122) admits
//!   `@`/`*` always and `#`/`?` outside posix mode; `$`/`-`/`!` heads
//!   are never sources.
//! - `parameter_brace_expand_indir` (subst.c:7883-7952): the source
//!   resolves through `parameter_brace_find_indir` for EVERY parameter
//!   kind — positional text (`$9`), the `$#` count, the `$?` status,
//!   joined positionals for `@`/`*`, the script name for `0`. With an
//!   operator tail the same resolution feeds the operator word: only a
//!   valid-identifier source with NO tail earns
//!   `invalid indirect expansion`; an unresolvable tail-less source
//!   proceeds silently; a resolved-but-invalid target reports
//!   `<t>: invalid variable name` (subst.c:7947).
//!
//! Oracle: GNU bash 5.3.20 (`/opt/homebrew/bin/bash`), byte-compared
//! 2026-11-16 across script and `-c` modes. Harness: `rubash -c` via
//! CARGO_BIN_EXE_rubash, LC_ALL=C.UTF-8 pinned.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
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

fn stderr_text(out: &Outcome) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `${!#:-d}` resolves t = $# and applies `:-` to the parameter t
/// names: with two positionals that is `$2` (`b`), not the default.
#[test]
fn hash_default_resolves_through_count() {
    let out = run("set a b; echo \"x${!#:-d}y\"");
    assert_eq!(out.stdout, b"xby\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

/// Bare `${!#-d}` takes the same indirection; the unset tail is dead.
#[test]
fn hash_dash_default_resolves_through_count() {
    let out = run("set a b; echo \"x${!#-d}y\"");
    assert_eq!(out.stdout, b"xby\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

/// With zero positionals `$#` is `0`, so `${!#}` names the script/shell
/// name — GNU prints the argv0 path under `-c`. Assert the shape, not
/// the absolute path (the test binary lives under target/).
#[test]
fn hash_empty_resolves_to_script_name() {
    let out = run("echo \"x${!#}y\"");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.starts_with('x') && stdout.trim_end().ends_with('y') && stdout.contains("rubash"),
        "stdout must carry the shell name; got {stdout:?} stderr {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0);
}

/// `${!?}` indirects through the exit status: after `true`, t = `0`
/// names the script/shell name (posixexp2 semantics, subst.c:7917).
#[test]
fn question_resolves_status_to_script_name() {
    let out = run("true; echo \"x${!?}y\"");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("rubash"),
        "stdout must carry the shell name; got {stdout:?} stderr {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0);
}

/// `${!?-d}` applies `-` to the parameter t names: after `true` that
/// is `0` (the script name), not the default word.
#[test]
fn question_dash_default_resolves_through_status() {
    let out = run("true; echo \"x${!?-d}y\"");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("rubash"),
        "stdout must carry the shell name; got {stdout:?} stderr {:?}",
        stderr_text(&out)
    );
}

/// After `false`, t = `1` names positional `$1` — unset, so the `-`
/// default fires with the script name shape GNU produces.
#[test]
fn question_dash_default_falls_back_when_target_unset() {
    let out = run("false; echo \"x${!?-d}y\"");
    assert_eq!(out.stdout, b"xdy\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

/// `${!@:-d}`: `@` joins the positionals into the indirect name; the
/// joined text `a b` fails valid_brace_expansion_word (subst.c:7947)
/// and reports `<t>: invalid variable name`.
#[test]
fn at_default_reports_joined_name() {
    let out = run("a=hello b=world; set -- a b; echo ${!@:-d}");
    let err = stderr_text(&out);
    assert!(
        err.contains("a b: invalid variable name"),
        "stderr must name the joined name; got {err:?}"
    );
    assert!(!err.contains("bad substitution"), "got {err:?}");
    assert_eq!(out.code, 1, "stderr {err:?}");
}

/// Same for `${!*:-d}` — `*` joins with IFS[0].
#[test]
fn star_default_reports_joined_name() {
    let out = run("a=hello b=world; set -- a b; echo ${!*:-d}");
    let err = stderr_text(&out);
    assert!(
        err.contains("a b: invalid variable name"),
        "stderr must name the joined name; got {err:?}"
    );
    assert_eq!(out.code, 1, "stderr {err:?}");
}

/// `${!9:-d}`: positional 9 is unset — the `-` default applies to the
/// resolved name silently (no `invalid indirect expansion`: only
/// valid identifiers with v==0 earn that report, subst.c:7907-7918).
#[test]
fn positional_default_silent_when_unset() {
    let out = run("set a b; echo \"x${!9:-d}y\"");
    assert_eq!(out.stdout, b"xdy\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

/// `${!0}` names the script name, which is not a valid variable name —
/// `<t>: invalid variable name`, never a bare `$1` read.
#[test]
fn zero_reports_script_name_as_target() {
    let out = run("echo ${!0}");
    let err = stderr_text(&out);
    assert!(
        err.contains("invalid variable name") && !err.contains("bad substitution"),
        "stderr must be invalid-name, not bad-substitution; got {err:?}"
    );
    assert_eq!(out.code, 1, "stderr {err:?}");
}

/// `$`/`-` heads are never indirect sources (VALID_INDIR_PARAM
/// excludes them, subst.c:122): `${!$-d}` stays a bad substitution.
#[test]
fn dollar_head_stays_bad_substitution() {
    let out = run("echo ${!$-d}");
    let err = stderr_text(&out);
    assert!(err.contains("bad substitution"), "got {err:?}");
    assert_eq!(out.code, 1, "stderr {err:?}");
}

/// `${!#x}` — `x` is not an operator tail and `#x` is not a parameter —
/// bad substitution in GNU.
#[test]
fn hash_tail_garbage_is_bad_substitution() {
    let out = run("echo ${!#x}");
    let err = stderr_text(&out);
    assert!(err.contains("bad substitution"), "got {err:?}");
    assert_eq!(out.code, 1, "stderr {err:?}");
}

/// Posix mode: `?`/`#` leave VALID_INDIR_PARAM, so `${!?}` is the `!`
/// parameter itself — unset, it reports `!: parameter not set`.
#[test]
fn posix_question_is_bang_parameter() {
    let out = run("set -o posix; echo \"${!?}\"");
    let err = stderr_text(&out);
    assert!(
        err.contains("!: parameter not set"),
        "stderr must name the ! parameter; got {err:?}"
    );
    assert_eq!(out.code, 127, "stderr {err:?}");
}
