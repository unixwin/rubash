//! rubash#2972: no-operand `declare`/`typeset` listings ignored the
//! attribute filter flags — `declare -c` and `declare -t` printed the
//! full listing, and combined flags used AND semantics.
//!
//! GNU mechanism: builtins/setattr.def set_or_show_attributes (the
//! listing branch declare_builtin reaches via declare.def) gates on
//! the array attribute FIRST (an `else if` assoc gate when -a is
//! absent), removes the gate bit from the attribute set unless it was
//! the only flag, and then shows a variable iff ANY of the remaining
//! attributes matches: `(var->attributes & attribute) != 0` after the
//! gate. So `declare -ix` lists integer-OR-exported variables,
//! `declare -ac` lists capcase arrays (none in the fixture: empty),
//! and `declare -aA` lists nothing (array gate, assoc required).
//!
//! Pre-fix rubash: trace/capcase were not filter parameters at all
//! (full listing), and the remaining filters were conjoined.
//!
//! Expectations measured against GNU bash 5.3.20 (oracle binary),
//! script probes 2026-11-07. Dynamic-variable VALUE rendering in
//! listings is rubash#2973's subject; these pins assert name sets and
//! the fixture variables' exact lines only.

use std::collections::BTreeSet;
use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

const FIXTURE: &str = "\
declare -i iv=3
declare -a arr=(1 2)
declare -A asc=([k]=v)
declare -r rv=7
declare -x xv=8
declare -t tv=9
declare -c cv=abc
declare -l lv=ABC
declare pv=5
declare -n nv=pv
";

fn run_script(body: &str) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2972-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, format!("{FIXTURE}{body}")).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = wait_bounded(&mut child, Duration::from_secs(20));
    assert!(!output.timed_out, "probe timed out");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.code.unwrap_or(-1),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The variable name of each `declare ... NAME...` listing line:
/// the first token after the flag tokens (array values may contain
/// spaces, so the name is never the last token).
fn listing_names(stdout: &str) -> BTreeSet<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("declare ")?;
            let token = rest
                .split_whitespace()
                .find(|token| !token.starts_with('-') && !token.starts_with('+'))?;
            token.split(['=', '(']).next().map(str::to_string)
        })
        .collect()
}

fn names(body: &str) -> BTreeSet<String> {
    let (stdout, stderr, status) = run_script(body);
    assert_eq!(status, 0, "{body}: {stderr}");
    listing_names(&stdout)
}

#[test]
fn declare_c_lists_only_capcase() {
    let (stdout, stderr, status) = run_script("declare -c\n");
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "declare -c cv=\"Abc\"\n");
}

#[test]
fn declare_t_lists_only_traced() {
    let (stdout, stderr, status) = run_script("declare -t\n");
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "declare -t tv=\"9\"\n");
}

#[test]
fn typeset_t_lists_only_traced() {
    let (stdout, stderr, status) = run_script("typeset -t\n");
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "declare -t tv=\"9\"\n");
}

#[test]
fn declare_ac_is_empty() {
    // Array gate + capcase required: arr has no capcase, cv is no array.
    let (stdout, stderr, status) = run_script("declare -ac\n");
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "");
}

#[test]
fn declare_aa_listing_is_empty() {
    // Array gate + assoc required: arr lacks assoc, asc lacks array.
    let (stdout, stderr, status) = run_script("declare -aA\n");
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "");
}

#[test]
fn declare_i_lists_integers() {
    let names = names("declare -i\n");
    for expected in ["iv", "EUID", "UID", "PPID", "OPTIND"] {
        assert!(names.contains(expected), "missing {expected}: {names:?}");
    }
    for excluded in ["arr", "asc", "rv", "xv", "tv", "cv", "lv", "pv", "nv"] {
        assert!(
            !names.contains(excluded),
            "unexpected {excluded}: {names:?}"
        );
    }
}

#[test]
fn declare_a_lists_arrays() {
    let names = names("declare -a\n");
    assert!(names.contains("arr"), "{names:?}");
    assert!(names.contains("GROUPS"), "{names:?}");
    assert!(names.contains("BASH_VERSINFO"), "{names:?}");
    for excluded in ["asc", "iv", "rv", "xv", "tv", "cv", "lv", "pv", "nv"] {
        assert!(
            !names.contains(excluded),
            "unexpected {excluded}: {names:?}"
        );
    }
}

#[test]
fn declare_big_a_lists_assocs() {
    let names = names("declare -A\n");
    for expected in ["asc", "BASH_ALIASES", "BASH_CMDS"] {
        assert!(names.contains(expected), "missing {expected}: {names:?}");
    }
    for excluded in ["arr", "iv", "rv", "xv", "tv", "cv", "lv", "pv", "nv"] {
        assert!(
            !names.contains(excluded),
            "unexpected {excluded}: {names:?}"
        );
    }
}

#[test]
fn declare_ix_is_any_of_integer_or_exported() {
    let names = names("declare -ix\n");
    assert!(names.contains("iv"), "{names:?}");
    assert!(names.contains("xv"), "{names:?}");
    for excluded in ["arr", "asc", "rv", "tv", "cv", "pv", "nv"] {
        assert!(
            !names.contains(excluded),
            "unexpected {excluded}: {names:?}"
        );
    }
}

#[test]
fn declare_n_lists_namerefs() {
    let names = names("declare -n\n");
    assert!(names.contains("nv"), "{names:?}");
    for excluded in ["arr", "asc", "iv", "rv", "xv", "tv", "cv", "pv"] {
        assert!(
            !names.contains(excluded),
            "unexpected {excluded}: {names:?}"
        );
    }
}

#[test]
fn declare_x_lists_exported() {
    let names = names("declare -x\n");
    assert!(names.contains("xv"), "{names:?}");
    for excluded in ["arr", "asc", "iv", "rv", "tv", "cv", "lv", "pv", "nv"] {
        assert!(
            !names.contains(excluded),
            "unexpected {excluded}: {names:?}"
        );
    }
}

#[test]
fn declare_r_lists_readonly() {
    let names = names("declare -r\n");
    for expected in ["rv", "BASHOPTS", "SHELLOPTS", "EUID", "UID", "PPID"] {
        assert!(names.contains(expected), "missing {expected}: {names:?}");
    }
    for excluded in ["arr", "asc", "iv", "xv", "tv", "cv", "lv", "pv", "nv"] {
        assert!(
            !names.contains(excluded),
            "unexpected {excluded}: {names:?}"
        );
    }
}

#[test]
fn declared_unset_names_are_listing_candidates() {
    // A name declared without a value lives only in the attribute
    // mark sets; GNU's enumeration walks every variable cell, so the
    // name appears in the filtered listings its attributes select
    // and in the full listing.
    let (stdout, stderr, status) = run_script("declare -i iv2\ndeclare -t tv2\ndeclare -i\n");
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("declare -i iv2\n"), "{stdout}");
    let (stdout, stderr, status) = run_script("declare -i iv2\ndeclare -t tv2\ndeclare -t\n");
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("declare -t tv2\n"), "{stdout}");
    let (stdout, stderr, status) =
        run_script("declare plain_unset\ndeclare -a arr_unset\ndeclare -p\n");
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("declare -- plain_unset\n"), "{stdout}");
    assert!(stdout.contains("declare -a arr_unset\n"), "{stdout}");
}

#[test]
fn declare_plus_x_is_the_set_form() {
    // Control: clearing-form flags alone take the set-form listing,
    // byte-identical to a bare `declare`. Two separate processes:
    // back-to-back in one shell the listings legitimately differ
    // (`_` mutates and PIPESTATUS materializes after the first
    // command — GNU shows the same instability). Each process runs
    // in its own temp dir, so the three lines embedding that path
    // (HOME/PWD/BASH_SOURCE) are dropped before comparing.
    let volatile = |listing: &str| -> String {
        listing
            .lines()
            .filter(|line| {
                !line.starts_with("HOME=")
                    && !line.starts_with("PWD=")
                    && !line.starts_with("BASH_SOURCE=")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (plain, stderr, status) = run_script("declare\n");
    assert_eq!(status, 0, "{stderr}");
    let (plus, stderr, status) = run_script("declare +x\n");
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(volatile(&plain), volatile(&plus));
}
