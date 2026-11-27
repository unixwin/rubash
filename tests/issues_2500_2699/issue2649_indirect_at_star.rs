//! rubash#2649 — `${!*}` / `${!@}` were rejected as "bad substitution"
//! instead of performing indirect expansion over the positional names.
//!
//! GNU anchors (bash 5.3 subst.c):
//! - `parameter_brace_expand` (subst.c:10062) dispatches a `!`-led body
//!   whose remainder is a special parameter to
//!   `parameter_brace_expand_indir` (subst.c:7883) — `*`/`@` are
//!   VALID_INDIR_PARAM (subst.c:122) in every mode.
//! - `parameter_brace_find_indir` (subst.c:7839) expands the special
//!   parameter itself with PF_ASSIGNRHS + expand_no_split_dollar_star:
//!   `*` joins the positionals with IFS[0], `@` joins with a space.
//! - The joined text becomes the indirect NAME: when it is not a valid
//!   brace-expansion word, subst.c:7947 reports
//!   `<joined>: invalid variable name`; the expansion error is fatal
//!   to a non-interactive shell (subst.c:4296 exp_jump_to_top_level),
//!   so the script stops with status 1. A single positional that is
//!   a valid name expands that variable; no positionals expand empty.
//!
//! Oracle: GNU bash 5.3.20 probe scripts (2026-11-08), byte-compared.
//! Harness: `rubash -c` via CARGO_BIN_EXE_rubash, LC_ALL=C.UTF-8 pinned.

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

#[test]
fn star_two_positionals_reports_joined_name() {
    let out = run("a=hello b=world; set -- a b; echo ${!*}");
    assert_eq!(out.stdout, b"", "stdout: {:?}", stderr_text(&out));
    let err = stderr_text(&out);
    assert!(
        err.contains("a b: invalid variable name"),
        "stderr must name the joined name; got {err:?}"
    );
    assert!(
        !err.contains("bad substitution"),
        "must not be a bad substitution; got {err:?}"
    );
    assert_eq!(out.code, 1, "abandoned command status; stderr {err:?}");
}

#[test]
fn at_two_positionals_reports_joined_name() {
    let out = run("a=hello b=world; set -- a b; echo ${!@}");
    assert_eq!(out.stdout, b"", "stdout: {:?}", stderr_text(&out));
    let err = stderr_text(&out);
    assert!(
        err.contains("a b: invalid variable name"),
        "stderr must name the joined name; got {err:?}"
    );
    assert!(!err.contains("bad substitution"), "got {err:?}");
    assert_eq!(out.code, 1, "stderr {err:?}");
}

#[test]
fn star_error_abandons_only_the_command() {
    // Corrected 2026-11-09 against the GNU bash 5.3.20 oracle: on the
    // indirection path the invalid-name report (subst.c:7947) is an
    // expansion error, which unwinds a non-interactive shell via
    // subst.c:4296 exp_jump_to_top_level — the failure is script-fatal,
    // not command-local. Oracle, byte-for-byte: stdout empty (`echo
    // DONE` never runs), stderr names `a b: invalid variable name`,
    // status 1. The original expectation (script continues, status 0)
    // contradicted the oracle; the product already matches it.
    let out = run("a=hello b=world; set -- a b; echo ${!*}; echo DONE");
    assert_eq!(out.stdout, b"", "stderr: {:?}", stderr_text(&out));
    let err = stderr_text(&out);
    assert!(
        err.contains("a b: invalid variable name"),
        "stderr must name the joined name; got {err:?}"
    );
    assert_eq!(out.code, 1, "stderr: {:?}", stderr_text(&out));
}

#[test]
fn star_single_positional_expands_named_variable() {
    let out = run("a=hello; set -- a; echo \"[${!*}]\"");
    assert_eq!(out.stdout, b"[hello]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn at_single_positional_expands_named_variable() {
    let out = run("a=hello; set -- a; echo \"[${!@}]\"");
    assert_eq!(out.stdout, b"[hello]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn star_no_positionals_expands_empty() {
    let out = run("set --; echo \"[${!*}]\"");
    assert_eq!(out.stdout, b"[]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn at_no_positionals_expands_empty() {
    let out = run("set --; echo \"[${!@}]\"");
    assert_eq!(out.stdout, b"[]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn star_joined_name_uses_ifs_first_char() {
    // string_list_dollar_star joins with IFS[0] even on the no-split
    // find_indir path, so the diagnostic names `a:b` under IFS=':'.
    let out = run("IFS=':'; a=hello b=world; set -- a b; echo ${!*}");
    let err = stderr_text(&out);
    assert!(
        err.contains("a:b: invalid variable name"),
        "stderr must show the IFS[0] join; got {err:?}"
    );
    assert_eq!(out.code, 1);
}

#[test]
fn at_joined_name_uses_space_even_under_ifs() {
    // string_list_dollar_at under PF_ASSIGNRHS joins with a space
    // regardless of IFS, so the diagnostic still names `a b`.
    let out = run("IFS=':'; a=hello b=world; set -- a b; echo ${!@}");
    let err = stderr_text(&out);
    assert!(
        err.contains("a b: invalid variable name"),
        "stderr must show the space join; got {err:?}"
    );
    assert_eq!(out.code, 1);
}

#[test]
fn control_plain_indirect_still_works() {
    let out = run("x=a; a=indirect-ok; echo \"[${!x}]\"");
    assert_eq!(
        out.stdout,
        b"[indirect-ok]\n",
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0);
}

#[test]
fn control_prefix_listing_still_works() {
    let out = run("pre_one=1 pre_two=2; echo \"[${!pre_*}]\"");
    assert_eq!(
        out.stdout,
        b"[pre_one pre_two]\n",
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0);
}

#[test]
fn control_bang_operator_form_still_works() {
    let out = run("echo \"[${!:-fallback}]\"");
    assert_eq!(
        out.stdout,
        b"[fallback]\n",
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0);
}
