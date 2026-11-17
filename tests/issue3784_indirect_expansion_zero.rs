//! rubash#3784 — `${!...}` indirect expansion mishandles positional
//! parameter 0 in two faces.
//!
//! Face 1 — a literal digit-0 name after `!` indirects through the VALUE
//! of $0 (the shell path), which is not a valid variable name, so GNU
//! aborts with `<$0>: invalid variable name` (rc 1, no stdout).
//! rubash silently expanded to empty, rc 0.
//!
//! Face 2 — `${!#}` expands to the last positional parameter, and when
//! $# is 0 that is `$0` itself. rubash expanded to empty.
//!
//! GNU anchors (bash 5.3 subst.c, vendored tree):
//! - `parameter_brace_expand_indir` (subst.c:7883): `t` comes from
//!   `parameter_brace_find_indir` (subst.c:7839), which expands the
//!   SOURCE parameter itself — for name `0`/`00` that is $0's value,
//!   for `#` that is $#'s value (`"0"` with no args, i.e. $0).
//! - The re-expanded target is validated by
//!   `valid_brace_expansion_word` (subst.c:7583) and an invalid `t`
//!   reports `<t>: invalid variable name` (subst.c:7947); the error is
//!   fatal to a non-interactive shell (subst.c:4296
//!   exp_jump_to_top_level), so the script stops with status 1.
//! - A valid `t` expands the variable it names: with $0 a bare valid
//!   name (`BASH_ARGV0=target`) `${!0}` reads `$target`
//!   (variables.c:1547-1555 set_argv0 ranks an imported BASH_ARGV0
//!   above argv[0]).
//!
//! Oracle: GNU bash 5.3.20 probes (2026-11-16) + system bash 5.2.21
//! confirmation probes, byte-compared. Harness: `rubash -c` via
//! CARGO_BIN_EXE_rubash, LC_ALL=C.UTF-8 pinned. `$0` under the
//! harness is the absolute binary path (an invalid variable name),
//! so the error faces fire without any override; the positive faces
//! set BASH_ARGV0 to a valid name.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    run_env(script, &[])
}

fn run_env(script: &str, extra_env: &[(&str, &str)]) -> Outcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn stderr_text(out: &Outcome) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// $0 as the harness binary sees it: the absolute CARGO_BIN_EXE path.
fn dollar_zero() -> String {
    let out = run("printf '%s' \"$0\"");
    assert_eq!(out.code, 0, "probe $0: {:?}", stderr_text(&out));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn assert_invalid_variable_name(out: &Outcome, zero: &str) {
    assert_eq!(
        out.stdout,
        b"",
        "stdout must be empty; stderr {0:?}",
        stderr_text(out)
    );
    let err = stderr_text(out);
    assert!(
        err.contains("invalid variable name"),
        "stderr must report invalid variable name; got {err:?}"
    );
    assert!(
        err.contains(zero),
        "diagnostic must name $0 [{zero}]; got {err:?}"
    );
    assert!(
        !err.contains("bad substitution"),
        "must not be a bad substitution; got {err:?}"
    );
    assert_eq!(out.code, 1, "abandoned command status; stderr {err:?}");
}

#[test]
fn bang_zero_bare_errors() {
    let zero = dollar_zero();
    let out = run("echo \"x=${!0}\"");
    assert_invalid_variable_name(&out, &zero);
}

#[test]
fn bang_zero_zero_bare_errors() {
    let zero = dollar_zero();
    let out = run("echo \"x=${!00}\"");
    assert_invalid_variable_name(&out, &zero);
}

#[test]
fn bang_zero_zero_default_errors() {
    // Seed check 3: `${!00-unset}` aborts with rc 1 — the operator
    // applies to the parameter t names, and t ($0's value) is invalid
    // before any default is considered (subst.c:7955-7961).
    let zero = dollar_zero();
    let out = run("echo \"x=${!00-unset}\"");
    assert_invalid_variable_name(&out, &zero);
}

#[test]
fn bang_zero_default_errors_inside_subshell() {
    // Seed check 9: the error stays inside its subshell (inner stderr
    // discarded, only the rc line is compared).
    let out = run("f() { (echo \"z=${!0-unset}\") 2>/dev/null; echo \"inner-rc=$?\"; }; f p q");
    assert_eq!(
        out.stdout,
        b"inner-rc=1\n",
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_valid_name_expands_target() {
    // Positive branch of face 1: with $0 a valid identifier, `${!0}`
    // reads the variable $0 names (GNU probe: BASH_ARGV0=target).
    let out = run_env(
        "target=hello; echo \"[${!0}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[hello]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_zero_valid_name_expands_target() {
    let out = run_env(
        "target=hello; echo \"[${!00}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[hello]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_default_valid_name_applies_operator() {
    // `${!0-unset}` with $0 a valid name applies `-` to $target.
    let out = run_env(
        "target=hello; echo \"[${!0-unset}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[hello]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_hash_no_args_is_dollar_zero() {
    // Seed check 4: `${!#}` with $#=0 equals $0.
    let out = run("[ \"${!#}\" = \"$0\" ]; echo $?");
    assert_eq!(out.stdout, b"0\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_hash_with_args_is_last() {
    // Seed check 5 (control): `${!#}` with 3 args is the last arg.
    let out = run("set -- a b c; echo \"${!#}\"");
    assert_eq!(out.stdout, b"c\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_hash_single_arg_is_that_arg() {
    let out = run("set -- only; echo \"${!#}\"");
    assert_eq!(out.stdout, b"only\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_hash_pattern_no_args_is_dollar_zero() {
    // Same $0 fallback through the indirect pattern-removal holder
    // (subst.c:7883 + parameter_brace_expand pattern path): with no
    // args `${!#%*}` strips nothing from $0 itself.
    let out = run("[ \"${!#%*}\" = \"$0\" ]; echo $?");
    assert_eq!(out.stdout, b"0\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_hash_pattern_with_args_is_last_arg() {
    let out = run("set -- a b c; [ \"${!#%*}\" = \"c\" ]; echo $?");
    assert_eq!(out.stdout, b"0\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn control_bang_n_through_variable() {
    // Seed check 6 (control): `${!n}` with n=1 reads $1.
    let out = run("set -- x y; n=1; echo \"${!n}\"");
    assert_eq!(out.stdout, b"x\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn control_bang_01_default_parses_to_one() {
    // Seed check 7 (control): `${!01-unset}` parses numerically to $1.
    let out = run("echo \"${!01-unset}\"");
    assert_eq!(out.stdout, b"unset\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn control_bang_question_is_dollar_zero() {
    // Seed check 8 (control): `${!?}` with $?==0 equals $0.
    let out = run("[ \"${!?}\" = \"$0\" ]; echo $?");
    assert_eq!(out.stdout, b"0\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn control_bang_one_bare_is_empty_on_gnu_too() {
    // Boundary from the issue: bare `${!1}` is the variable named by
    // $1, not $1 itself — empty on GNU too when $1 holds no set name.
    let out = run("set -- x y; echo \"[${!1}]\"");
    assert_eq!(out.stdout, b"[]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn control_bang_one_unset_positional_is_empty() {
    let out = run("set -- x; echo \"[${!5}]\"");
    assert_eq!(out.stdout, b"[]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

// Operator faces on a numeric-0 source. GNU resolves the source to $0's
// VALUE (find_indir, subst.c:7839), then applies the operator to the
// parameter that value names (get_var_and_type, subst.c:8529-8545) — the
// resolved name here is `target`, so every operator acts on $target.

#[test]
fn bang_zero_prefix_removal_operates_on_target() {
    let out = run_env(
        "target=hello; echo \"[${!0#h}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[ello]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_suffix_removal_operates_on_target() {
    let out = run_env(
        "target=hello; echo \"[${!0%lo}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[hel]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_zero_case_mod_operates_on_target() {
    let out = run_env(
        "target=hello; echo \"[${!00^}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[Hello]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_zero_substring_operates_on_target() {
    let out = run_env(
        "target=hello; echo \"[${!000:1:2}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[el]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_replacement_operates_on_target() {
    let out = run_env(
        "target=hello; echo \"[${!0/hello/world}]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[world]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_assignment_binds_target() {
    // subst.c:8151: `:=` binds the parameter the source value names.
    let out = run_env(
        "echo \"[${!0:=d}]\"; echo \"[$target]\"",
        &[("BASH_ARGV0", "target")],
    );
    assert_eq!(out.stdout, b"[d]\n[d]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_zero_zero_dash_keeps_set_empty_target() {
    // `-` substitutes only for an UNSET parameter: target="" stays empty.
    let out = run_env("target=; echo \"[${!00-d}]\"", &[("BASH_ARGV0", "target")]);
    assert_eq!(out.stdout, b"[]\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_hash_pattern_whole_word_argument_is_last_arg() {
    // The whole word is a quoted `${!#%*}`: GNU's list-form gate
    // (subst.c:9974 legal_variable_starter) rejects `!#`, so this is the
    // `%` operator on `${!#}`, and `%*` removes the empty suffix.
    let out = run("set -- a b c; printf '<%s>\\n' \"${!#%*}\"");
    assert_eq!(out.stdout, b"<c>\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

#[test]
fn bang_hash_pattern_whole_word_no_args_is_dollar_zero() {
    let out = run("printf '%s' \"${!#%*}\"");
    assert_eq!(
        out.stdout,
        dollar_zero().into_bytes(),
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0);
}

// Nounset reports the whole `!source` name when the parameter that source
// names is unbound (subst.c:10164-10175 check_unbound_variable over the
// find_indir target). GNU 5.3.20: `${!0}` with $0=target and target unset
// is `!0: unbound variable`, status 127.

#[test]
fn bang_zero_unset_target_reports_unbound_under_nounset() {
    let out = run_env("set -u; echo \"${!0}\"", &[("BASH_ARGV0", "target")]);
    assert_eq!(out.stdout, b"", "stderr: {:?}", stderr_text(&out));
    assert!(
        stderr_text(&out).contains("!0: unbound variable"),
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 127);
}

#[test]
fn bang_zero_zero_unset_target_reports_unbound_under_nounset() {
    let out = run_env("set -u; echo \"${!00}\"", &[("BASH_ARGV0", "target")]);
    assert_eq!(out.stdout, b"", "stderr: {:?}", stderr_text(&out));
    assert!(
        stderr_text(&out).contains("!00: unbound variable"),
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 127);
}

#[test]
fn bang_one_source_unset_target_reports_unbound_under_nounset() {
    // A positional SOURCE resolves its target the same way: $1 holds
    // `target`, and target is unbound.
    let out = run("set -u; set -- target; echo \"${!1}\"");
    assert_eq!(out.stdout, b"", "stderr: {:?}", stderr_text(&out));
    assert!(
        stderr_text(&out).contains("!1: unbound variable"),
        "stderr: {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 127);
}
