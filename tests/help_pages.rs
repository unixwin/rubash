//! Regression tests for the per-builtin `help` pages (GNU help.def).
//!
//! The full 77-topic table is byte-verified against WSL GNU bash 5.3.0 by
//! target/gc/helptext-gen/ (compare-topics.sh / compare-edges.sh); these
//! tests pin the load-bearing invariants so regressions surface in
//! `cargo test` without the external oracle.

use std::process::Command;

fn run(args: &[&str]) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn help(args: &[&str]) -> (String, String, i32) {
    // Single-quote every argument so special topic names (`(( ... ))`, `[`,
    // glob patterns) reach `help` as single words.
    let mut script = String::from("help");
    for arg in args {
        script.push_str(" '");
        script.push_str(arg);
        script.push('\'');
    }
    run(&["-c", &script])
}

#[test]
fn help_set_prints_the_full_gnu_page() {
    let (stdout, stderr, rc) = help(&["set"]);
    assert_eq!(rc, 0);
    assert!(stderr.is_empty(), "stderr: {stderr:?}");
    assert!(
        stdout.starts_with("set: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]\n"),
        "first line: {:?}",
        stdout.lines().next(),
    );
    // The -o option table must be there verbatim (owner-verified fidelity bar).
    assert!(stdout.contains("pipefail"));
    assert!(stdout.contains("Exit Status:"));
    // Every long-doc line carries GNU's four-space BASE_INDENT.
    assert!(
        stdout
            .lines()
            .skip(1)
            .all(|line| line == "    " || line.starts_with("    ")),
        "unindented line in help set"
    );
}

#[test]
fn help_short_mode_prints_only_the_synopsis() {
    let (stdout, stderr, rc) = help(&["-s", "read"]);
    assert_eq!(rc, 0);
    assert!(stderr.is_empty());
    assert_eq!(
        stdout,
        "read: read [-Eers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] \
[-p prompt] [-t timeout] [-u fd] [name ...]\n"
    );
}

#[test]
fn help_desc_mode_prints_the_first_doc_line() {
    let (stdout, stderr, rc) = help(&["-d", "shift"]);
    assert_eq!(rc, 0);
    assert!(stderr.is_empty());
    assert_eq!(stdout, "shift - Shift positional parameters.\n");
}

#[test]
fn help_compopt_and_special_names_have_pages() {
    for name in ["compopt", ":", ".", "[", "!", "%", "(( ... ))", "[[ ... ]]"] {
        let (stdout, stderr, rc) = help(&[name]);
        assert_eq!(rc, 0, "help {name:?}");
        assert!(stderr.is_empty(), "help {name:?} stderr: {stderr:?}");
        // `[[ ... ]]' is itself a glob pattern (glob_pattern_p sees the
        // bracket pair), so GNU prints the matching header before the page.
        if name == "[[ ... ]]" {
            assert!(
                stdout.starts_with(
                    "Shell commands matching keyword `[[ ... ]]'\n\n[[ ... ]]: [[ expression ]]\n"
                ),
                "help {name:?}: {stdout:?}"
            );
            continue;
        }
        let first = stdout.lines().next().unwrap_or_default();
        assert!(
            first.starts_with(&format!("{name}: ")) || first == ": :",
            "help {name:?} first line: {first:?}"
        );
    }
}

#[test]
fn help_variables_keeps_history_lines_and_trailing_empty_line() {
    // reserved.def guards the history variables with #if HISTORY; the runtime
    // doc keeps the guarded lines (mkbuiltins drops only the `#` guards) and
    // ends with the dangling-separator empty line.
    let (stdout, stderr, rc) = help(&["variables"]);
    assert_eq!(rc, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("HISTFILE\tThe name of the file where your command history is stored."));
    assert!(stdout.ends_with("history list.\n\n"));
}

#[test]
fn help_pattern_matching_follows_gnu_passes() {
    // `read*' is a glob: header line, blank line, read/readarray/readonly.
    let (stdout, _, rc) = help(&["read*"]);
    assert_eq!(rc, 0);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "Shell commands matching keyword `read*'");
    assert_eq!(lines[1], "");
    assert!(lines[2].starts_with("read: "));
    assert!(stdout.contains("readarray: "));
    assert!(stdout.contains("readonly: "));

    // `sh' is not a glob: pass-2 prefix match, no header.
    let (stdout, _, rc) = help(&["sh"]);
    assert_eq!(rc, 0);
    assert!(stdout.starts_with("shift: shift [n]\n"));
    assert!(stdout.contains("shopt: "));
    assert!(!stdout.contains("Shell commands matching"));
}

#[test]
fn help_unknown_topic_fails_like_gnu() {
    let (stdout, stderr, rc) = help(&["zzz"]);
    assert_eq!(rc, 1);
    assert!(stdout.is_empty());
    assert!(
        stderr.ends_with(
            "help: no help topics match `zzz'.  Try `help help' or `man -k zzz' or `info zzz'.\n"
        ),
        "stderr: {stderr:?}"
    );
}

#[test]
fn help_invalid_option_reports_the_offending_letter() {
    let (stdout, stderr, rc) = help(&["-q"]);
    assert_eq!(rc, 2);
    assert!(stdout.is_empty());
    assert!(
        stderr.ends_with("help: -q: invalid option\nhelp: usage: help [-dms] [pattern ...]\n"),
        "stderr: {stderr:?}"
    );

    let (_, stderr, rc) = help(&["-qz"]);
    assert_eq!(rc, 2);
    assert!(
        stderr.contains("help: -q: invalid option"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn help_dash_and_dash_dash_are_patterns_not_options() {
    // bashgetopt NOTOPT: a bare `-` is an argument; `--` ends options.
    let (stdout, stderr, rc) = help(&["-"]);
    assert_eq!(rc, 1);
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("no help topics match `-'"),
        "stderr: {stderr:?}"
    );

    let (stdout, _, rc) = help(&["--", "set"]);
    assert_eq!(rc, 0);
    assert!(stdout.starts_with("set: set [-"));
}

#[test]
fn help_self_help_flag_prints_own_page_with_usage_rc() {
    let (stdout, stderr, rc) = help(&["--help"]);
    assert_eq!(rc, 2);
    assert!(stderr.is_empty());
    assert!(stdout.starts_with("help: help [-dms] [pattern ...]\n"));
    assert!(stdout.contains("Display information about builtin commands."));
}

#[test]
fn help_manpage_mode_renders_the_gnu_sections() {
    let (stdout, stderr, rc) = help(&["-m", ":"]);
    assert_eq!(rc, 0);
    assert!(stderr.is_empty());
    assert!(stdout.starts_with("NAME\n    : - Null command.\n\nSYNOPSIS\n    :\n\nDESCRIPTION\n"));
    assert!(stdout.contains("\nSEE ALSO\n    bash(1)\n\nIMPLEMENTATION\n"));
    assert!(stdout.contains("Copyright (C) 2025 Free Software Foundation, Inc."));
    assert!(stdout.ends_with(
        "License GPLv3+: GNU GPL version 3 or later <http://gnu.org/licenses/gpl.html>\n\n"
    ));
}

#[test]
fn bare_help_overview_is_unchanged() {
    let (stdout, _, rc) = run(&["-c", "help"]);
    assert_eq!(rc, 0);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with("GNU bash, version 5.3.0(1)-release ("));
    assert_eq!(
        lines[1],
        "These shell commands are defined internally.  Type `help' to see this list."
    );
}
