//! rubash#2915 — `bind 'set: foo'` was silently swallowed. GNU
//! ground: rl_parse_and_bind's separator scan stops the first
//! token at `:`, space or tab (lib/readline/bind.c:1637), so for
//! `set: foo` the token IS `set` and the variable path runs with
//! var `foo`: `%s: unknown variable name` (bind.c:1705). rubash's
//! strip_set_keyword required whitespace after `set`, so the
//! colon form never reached the variable arm.

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
}

fn run(script: &str) -> Run {
    #[cfg(unix)]
    const EMPTY_INPUTRC: &str = "/dev/null";
    #[cfg(windows)]
    const EMPTY_INPUTRC: &str = "NUL";
    let out = Command::new(RUBASH)
        .args(["-c", script])
        .env("INPUTRC", EMPTY_INPUTRC)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    let stderr = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| !line.contains("line editing not enabled"))
        .map(|line| format!("{line}\n"))
        .collect();
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr,
    }
}

#[test]
fn issue2915_set_colon_unknown_variable_is_reported() {
    let r = run("bind 'set: foo'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: foo: unknown variable name\n");
    let r = run("bind 'set:foo'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: foo: unknown variable name\n");
}

#[test]
fn issue2915_set_colon_alone_reports_the_empty_name() {
    // var is the empty string after the colon: the same empty-name
    // diagnostic as a bare `bind 'set'` (rubash#2873).
    let r = run("bind 'set:'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: : unknown variable name\n");
}

#[test]
fn issue2915_set_colon_valid_assignment_applies() {
    let r = run("bind 'set: completion-ignore-case on'; bind -v | grep completion-ignore-case");
    assert_eq!(r.stdout, "set completion-ignore-case on\n");
    assert_eq!(r.stderr, "");
    let r = run("bind 'set: bell-style visible'; bind -v | grep bell-style");
    assert_eq!(r.stdout, "set bell-style visible\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2915_set_keyword_match_is_case_insensitive() {
    // The token comparison is _rl_stricmp (bind.c:1658).
    let r = run("bind 'SET: foo'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: foo: unknown variable name\n");
}

#[test]
fn issue2915_colon_inside_the_variable_word_stays_in_the_word() {
    // The scan stops at the SPACE: var is the whole next word,
    // colon included — GNU reports `bell-style:` as the name.
    let r = run("bind 'set bell-style: on'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: bell-style:: unknown variable name\n");
}
