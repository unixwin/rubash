//! Issues rubash#346/#347/#348 — compat31 regex quoting, execfail, set -v.
//!
//! #346: under `shopt -s compat31` a QUOTED =~ right-hand side keeps REGEX
//! semantics. GNU execute_cmd.c:4048-4057: `rmatch && shell_compatibility_
//! level > 31` selects cond_expand_word mode 2 (QGLOB_REGEXP: quoted
//! characters literalize metacharacters); compat31 pins the level at 31,
//! so mode stays 0 — plain dequote_list — and sh_regmatch still compiles
//! the result as a regex (`[[ xa+b =~ "$re" ]]` with re='a+b' does NOT
//! match: a+ is one-or-more-a).
//!
//! #347: `shopt -s execfail; exec /no/such/cmd` printed the diagnostic and
//! killed the script. GNU exec.def failed_exec: `if (subshell_environment
//! || (interactive == 0 && no_exit_on_failed_exec == 0)) exit_shell
//! (exit_value)` — the execfail shopt (no_exit_on_failed_exec, shopt.def)
//! keeps a noninteractive shell alive with $? = 127.
//!
//! #348: `set -v` produced no stderr trace. GNU flags.c:284
//! (echo_input_at_read = verbose_flag) + the reader's line echo
//! (y.tab.c:5071): every input line is written to stderr as it is read —
//! the flag applies to lines read after the `set -v` command itself.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

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

/// #346: compat31 on — the quoted variable RHS is a REGEX (a+b = one-or-
/// more a then b), which does not match the text `xa+b`.
#[test]
fn compat31_quoted_regex_rhs_is_regex() {
    let (stdout, stderr, code) =
        rubash("shopt -s compat31\nre='a+b'\nv='xa+b'\n[[ $v =~ \"$re\" ]] && echo m || echo nm");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "nm\n");
}

/// #346 guard: default (compat31 off) — the quoted RHS is a LITERAL string
/// (`a+b` occurs in `xa+b`), so it matches.
#[test]
fn default_quoted_regex_rhs_is_literal() {
    let (stdout, stderr, code) =
        rubash("re='a+b'\nv='xa+b'\n[[ $v =~ \"$re\" ]] && echo m || echo nm");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "m\n");
}

/// #346: an UNQUOTED regex RHS keeps working in both states.
#[test]
fn unquoted_regex_rhs_unchanged_by_compat31() {
    for state in ["shopt -s compat31\n", ""] {
        let (stdout, stderr, code) = rubash(&format!(
            "{state}v='xa+b'\n[[ $v =~ a+b ]] && echo m || echo nm"
        ));
        assert_eq!(stderr, "");
        assert_eq!(code, Some(0));
        assert_eq!(stdout, "nm\n");
    }
}

/// #347: execfail keeps the script alive after a failed exec.
#[test]
fn execfail_keeps_script_running() {
    let (stdout, stderr, code) =
        rubash("shopt -s execfail\nexec /no/such/cmd\necho \"survived rc=$?\"");
    assert!(
        stderr.contains("/no/such/cmd: No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "survived rc=127\n");
    assert_eq!(code, Some(0));
}

/// #347 guard: without execfail the noninteractive shell dies at the failed
/// exec (GNU exit_shell(EX_NOTFOUND)).
#[test]
fn failed_exec_still_exits_without_execfail() {
    let (stdout, _, code) = rubash("exec /no/such/cmd\necho unreachable");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(127));
}

/// #348: set -v echoes the input lines read after the flag turns on.
#[test]
fn set_v_echoes_input_lines_to_stderr() {
    let (stdout, stderr, code) = rubash("set -v\necho a\necho b");
    assert_eq!(stdout, "a\nb\n");
    assert_eq!(stderr, "echo a\necho b\n");
    assert_eq!(code, Some(0));
}

/// #348 guard: set -v's own line is read before the flag applies, and
/// turning it off stops the echo.
#[test]
fn set_v_scope_and_off() {
    let (stdout, stderr, code) = rubash("echo first\nset -v\nset +v\necho last");
    assert_eq!(stdout, "first\nlast\n");
    // GNU reads `set +v` while verbose is still on, so that one line
    // echoes; `echo last` is read after it turned off.
    assert!(stderr.contains("set +v"), "stderr: {stderr}");
    assert!(!stderr.contains("echo last"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
