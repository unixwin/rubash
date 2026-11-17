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
/// `<t>: invalid variable name`, never a bare `$1` read. Script-file
/// form: under `-c` GNU's $0 is the shell name (`bash`), a valid
/// identifier, so the script path is the form that exercises the
/// invalid-name verdict.
#[test]
fn zero_reports_script_name_as_target() {
    let dir = std::env::temp_dir().join(format!("rubash-i3605-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let script = dir.join("zero_indir.sh");
    std::fs::write(&script, "echo ${!0}\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let out = Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    };
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

/// `^`/`,` are NOT in the scanned tail class (subst.c:9844-9860 scans
/// `#%:-=?+/@` only): `${!#^}` is a bad substitution, not a case-mod on
/// the count or a silent empty.
#[test]
fn hash_caret_tail_is_bad_substitution() {
    for src in ["${!#^}", "${!#,}", "${!#^2}"] {
        let out = run(&format!("set a b; echo {src}"));
        let err = stderr_text(&out);
        assert!(
            err.contains("bad substitution"),
            "{src} must be bad substitution; got {err:?}"
        );
        assert_eq!(out.code, 1, "{src} stderr {err:?}");
    }
}

/// `:off:len` tails slice the resolved-t parameter: `${!#:0:1}` with
/// $#=2 is `${2:0:1}` = `b` (parameter_brace_find_indir feeding the
/// operator, subst.c:7839-7881, 9978-10017).
#[test]
fn hash_substring_tail_slices_target() {
    let out = run("set a b; echo \"x${!#:0:1}y\"");
    assert_eq!(out.stdout, b"xby\n", "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

/// `?` resolves t to `0` — the `0` parameter — so `${!?/a/Q}` applies
/// the patsub to $0's value, identically to `${0/a/Q}` (find_indir
/// feeding parameter ops, subst.c:7839-7881, 9978-10017).
#[test]
fn question_patsub_applies_to_resolved_target() {
    // `true` pins $? to 0 first: rubash's startup $? is a tracked
    // pre-existing divergence (1 vs GNU 0), so the pin must not depend
    // on the entry status.
    let out = run("true; echo \"x${!?/a/Q}y\"; echo \"x${0/a/Q}y\"");
    let mut lines = out.stdout.split(|b| *b == b'\n');
    let (indirect, direct) = (lines.next().unwrap(), lines.next().unwrap());
    assert_eq!(indirect, direct, "stderr: {:?}", stderr_text(&out));
    assert_eq!(out.code, 0);
}

/// Patsub/removal/substring tails apply to the resolved-t parameter:
/// with `set a b`, `${!#%2}`/`${!#/@(2)/Q}`/`${!#/2/zz}` all operate on
/// `$2` = `b` (parameter_brace_find_indir feeding the operator,
/// subst.c:7839-7881, 9978-10017); `${!2/a/Q}` resolves t=`b` — the
/// parameter `b`, which is unset and renders empty.
#[test]
fn f2_tails_apply_to_resolved_t_parameter() {
    for (src, want) in [
        ("${!#%2}", "b"),
        ("${!#/@(2)/Q}", "b"),
        ("${!#/2/Q}", "b"),
        ("${!#/2/zz}", "b"),
        ("${!#:0:1}", "b"),
        ("${!#:1:1}", ""),
        ("${!2/a/Q}", ""),
        // t is NULL for unset `$9`: the `:-` operator adjudicates an
        // unset parameter and renders the default word.
        ("${!9:-d}", "d"),
    ] {
        let out = run(&format!("set a b; echo \"x{src}y\""));
        assert_eq!(
            out.stdout,
            format!("x{want}y\n").as_bytes(),
            "{src} stderr: {:?}",
            stderr_text(&out)
        );
        assert_eq!(out.code, 0, "{src}");
    }
}

/// A normal-name indirect source resolves t from the source's VALUE:
/// `x=zzz` (unset target) applies the tail operator to the unset
/// parameter `zzz` — `${!x@Q}` is empty (subst.c:8927 NULL word), not
/// the quoted-empty `''` a set-empty cell earns; `^`/`,` are legal
/// casemod tails on a NAME head (`${!x^}` == `${zzz^}` -> empty), the
/// `#%:-=?+/@` tail-class scan (subst.c:9844-9860) restricting
/// `^`/`,` applies to `!`+special/digit heads only.
#[test]
fn named_source_tails_follow_the_resolved_target() {
    for src in ["${!x@Q}", "${!x^}", "${!x,Q}"] {
        let out = run(&format!("x=zzz; echo \"x{src}y\""));
        assert_eq!(out.stdout, b"xy\n", "{src} stderr: {:?}", stderr_text(&out));
        assert_eq!(out.code, 0, "{src}");
    }
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

/// GNU subst.c:10060-10076: a `?`/`:?` tail on a `!`-indirect source is
/// judged on the RESOLVED target, not on the `!src` spelling —
/// `parameter_brace_expand_indir` -> `parameter_brace_find_indir`
/// (subst.c:7839-7881) produces t, and `parameter_brace_expand_word(t)`
/// (subst.c:7907) produces the value the operator tests. Named, digit and
/// special sources all take this path when the target is set / non-null.
#[test]
fn question_tail_judges_the_resolved_target() {
    for (script, want) in [
        ("u=x; x=hello; echo \"${!u?v}\"", "hello\n"),
        ("u=x; x=hello; echo \"${!u:?v}\"", "hello\n"),
        ("set -- a b c; b=ok; echo \"${!2?v}\"", "ok\n"),
        ("set -- a b c; echo \"${!#?v}\"", "c\n"),
        ("set -- a b c; echo \"${!#:?v}\"", "c\n"),
        ("u=x; x=; echo \"a${!u?v}b\"", "ab\n"),
    ] {
        let out = run(script);
        assert_eq!(
            out.stdout,
            want.as_bytes(),
            "{script} stderr: {:?}",
            stderr_text(&out)
        );
        assert_eq!(out.code, 0, "{script} stderr: {:?}", stderr_text(&out));
    }
}

/// The same resolution decides the fatal arms: an unset target (or a null
/// target under `:?`) reports the ORIGINAL expression name (`!u: v`,
/// subst.c:10412 parameter_brace_expand_error) and aborts the
/// non-interactive shell before the echo runs.
#[test]
fn question_tail_fatals_on_unset_target_naming_the_source() {
    for (script, want_stdout) in [
        ("u=x; unset x; echo before; echo \"${!u?v}\"", "before\n"),
        ("u=x; unset x; echo \"${!u:?msg}\"", ""),
        ("u=x; x=; echo \"${!u:?v}\"", ""),
    ] {
        let out = run(script);
        assert_eq!(out.stdout, want_stdout.as_bytes(), "{script}");
        assert_eq!(out.code, 127, "{script} stderr: {:?}", stderr_text(&out));
        let err = stderr_text(&out);
        assert!(
            err.contains("!u:"),
            "{script} wanted a !u diagnostic; got {err:?}"
        );
    }
}

/// `${!?<op>}` — the first `?` is the indirect SOURCE ($?), so the error
/// operator is the character after it (subst.c:9920 want_indir with
/// subst.c:122 VALID_INDIR_PARAM). `$?` is `0`, which names `$0`: a
/// set parameter, so both the bare and the colon form expand to the
/// shell/script name. The operator family's `?` split used to take the
/// SOURCE `?` for the operator and return an empty value.
#[test]
fn question_source_bang_question_expands_through_status() {
    // `$0` under `-c` is the shell path, so compare the three forms
    // against `$0` itself rather than against a literal name.
    let out = run("true; echo \"$0|${!??v}|${!?:?v}|${!?#0}\"");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let fields: Vec<&str> = stdout.trim_end().split('|').collect();
    assert_eq!(
        fields.len(),
        4,
        "got {stdout:?} stderr {:?}",
        stderr_text(&out)
    );
    assert!(
        fields
            .iter()
            .all(|field| *field == fields[0] && !field.is_empty()),
        "every form must equal $0; got {fields:?} stderr {:?}",
        stderr_text(&out)
    );
    assert_eq!(out.code, 0, "stderr {:?}", stderr_text(&out));
}
