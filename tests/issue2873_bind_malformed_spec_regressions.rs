//! rubash#2873 — `bind` silently swallowed malformed binding specs
//! instead of printing readline's diagnostics. GNU ground:
//! rl_parse_and_bind (lib/readline/bind.c:1594) scans the spec for a
//! `:`, space or tab separator; when the scan reaches the end of the
//! string (foundsep == 0) the spec names no value and readline
//! reports `%s: no key sequence terminator` (bind.c:1754) — the
//! builtin still succeeds (builtins/bind.def:308-312 ignores
//! rl_parse_and_bind's return). A spec that STARTS with the separator
//! has no key sequence at all: `` `%s': invalid key binding: missing
//! key sequence `` (bind.c:1637). The same parse loop consumes the
//! extra arguments after `bind -q NAME`, so they diagnose too.
//! Shapes GNU absorbs silently stay silent: empty specs, `#`
//! comments, and separator-carrying old-style specs (`bind 'ls -l'`).

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
fn issue2873_bare_word_reports_no_key_sequence_terminator() {
    let r = run("bind 'backward-char'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(
        r.stderr,
        "readline: backward-char: no key sequence terminator\n"
    );
    let r = run("bind 'garbage'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: garbage: no key sequence terminator\n");
}

#[test]
fn issue2873_leading_whitespace_is_stripped_before_the_diagnostic() {
    // rl_parse_and_bind advances past leading whitespace first; the
    // diagnostic names the stripped spec.
    let r = run("bind ' backward-char'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(
        r.stderr,
        "readline: backward-char: no key sequence terminator\n"
    );
}

#[test]
fn issue2873_separator_first_reports_missing_key_sequence() {
    let r = run("bind ': foo'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(
        r.stderr,
        "readline: `: foo': invalid key binding: missing key sequence\n"
    );
    let r = run("bind ':'; echo rc=$?");
    assert_eq!(
        r.stderr,
        "readline: `:': invalid key binding: missing key sequence\n"
    );
}

#[test]
fn issue2873_extra_arguments_after_query_are_parsed_as_specs() {
    // bind.def:308-312: arguments after the -q name go through
    // rl_parse_and_bind one by one.
    let r = run("bind -q forward-char backward-char");
    assert_eq!(
        r.stdout,
        "forward-char can be invoked via \"\\C-f\", \"\\eOC\", \"\\e[C\".\n"
    );
    assert_eq!(
        r.stderr,
        "readline: backward-char: no key sequence terminator\n"
    );
}

#[test]
fn issue2873_extra_arguments_after_query_still_apply() {
    // Positive control: a well-formed extra argument takes effect.
    let r = run("bind -q forward-char '\"\\C-a\": backward-char'; bind -q backward-char");
    assert!(r.stdout.contains("\"\\C-a\""), "stdout: {}", r.stdout);
    assert_eq!(r.stderr, "");
    let r = run("bind -q forward-char 'set bell-style none'; bind -v | grep bell-style");
    assert_eq!(r.stdout.lines().last(), Some("set bell-style none"));
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2873_absorbed_shapes_stay_silent() {
    let r = run("bind 'ls -l'; bind ''; bind '# comment'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2873_bare_set_reports_unknown_variable_name() {
    // The first token IS `set`, so rl_parse_and_bind enters the
    // variable path with an empty name (bind.c:1709).
    let r = run("bind 'set'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: : unknown variable name\n");
}

#[test]
fn issue2873_missing_closing_quote_for_macro_matches_gnu_form() {
    // bind.c:1731: `` `%s': missing closing quote for macro `` with
    // the function text (including its opening quote) as %s.
    let r = run(r#"bind '"\C-t": "macro'"#);
    assert_eq!(
        r.stderr,
        "readline: `\"macro': missing closing quote for macro\n"
    );
}
