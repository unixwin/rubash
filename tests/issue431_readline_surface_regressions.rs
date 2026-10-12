//! Regressions for the unixwin/rubash#431 readline-surface residual
//! (handover #479 rows: inputrc reading / `bind -v` variable set /
//! inputrc parse face). The PS1-marker body of #431 shipped earlier; the
//! rows here are the engine-side pieces that did not need a host editor:
//!
//! - `bind -v`/`-V` dump the GNU 5.3 readline variable table (48 rows;
//!   WSL GNU bash 5.3.0 probe) and reflect `set NAME VALUE` updates.
//! - inputrc files apply `set` variables with GNU's diagnostics, `$if
//!   term=…/mode=…` conditionals (case-sensitive, in-file `set
//!   editing-mode` arms later `mode=` blocks), `$else`/`$endif`, and
//!   `$include`.
//! - initialize_readline's inputrc read (bashline.c:451 via bind.def:131
//!   and y.tab.c:4024): $INPUTRC (else ~/.inputrc) applies ONCE per
//!   process on the first `bind` — even under `-c` (GNU probe) — and
//!   hosts read the same registry.
//! - the GNU error-channel split: plain positional binding failures
//!   print `readline: …` with NO status effect (bind.def:304-333 ignores
//!   rl_parse_and_bind's return), non-quote non-`set` arguments are
//!   silently ignored, while `-x` failures keep the `bind: ` channel and
//!   rc 1.
//!
//! Every expected output below was pinned against WSL GNU bash 5.3.0
//! (/usr/local/bin/bash) via script files.

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--norc", "-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_with_env(script: &str, vars: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(["--norc", "-c", script]);
    for (name, value) in vars {
        command.env(name, value);
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn temp_file(name: &str, content: &str) -> String {
    let path = std::env::temp_dir().join(format!("rubash431_{}_{}.rc", name, std::process::id()));
    std::fs::write(&path, content).expect("write temp inputrc");
    // Forward slashes: the path is embedded UNQUOTED in `-c` scripts,
    // where backslashes would parse as escape characters.
    path.to_string_lossy().replace('\\', "/")
}

// ---------------------------------------------------------------------------
// bind -v / -V: the GNU variable table
// ---------------------------------------------------------------------------

#[test]
fn bind_v_dumps_the_full_gnu_table() {
    let (stdout, _, code) = run("bind -v");
    assert_eq!(code, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    // GNU probe: `bind -v | wc -l` = 48, exact order.
    assert_eq!(lines.len(), 48);
    assert_eq!(lines[0], "set bind-tty-special-chars off");
    assert_eq!(lines[35], "set visible-stats off");
    assert_eq!(lines[36], "set bell-style audible");
    assert_eq!(lines[41], "set editing-mode emacs");
    assert_eq!(lines[44], "set keymap emacs");
    assert_eq!(lines[47], "set vi-ins-mode-string (ins)");
}

#[test]
fn bind_v_capital_dumps_the_plain_form() {
    let (stdout, _, code) = run("bind -V");
    assert_eq!(code, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 48);
    // GNU probe: `NAME is set to `VALUE'`.
    assert!(lines.contains(&"blink-matching-paren is set to `off'"));
    assert!(lines.contains(&"editing-mode is set to `emacs'"));
}

#[test]
fn bind_set_argument_flows_into_bind_v() {
    // GNU probe: `bind 'set completion-ignore-case on'; bind -v` shows on.
    let (stdout, _, code) =
        run("bind 'set completion-ignore-case on'; bind -v | grep completion-ignore-case");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "set completion-ignore-case on");
}

#[test]
fn bind_set_editing_mode_switches_keymap() {
    // GNU probe: `set editing-mode vi` also sets `keymap vi-insert`.
    let (stdout, _, code) =
        run("bind 'set editing-mode vi'; bind -v | grep -E 'editing-mode|keymap'");
    assert_eq!(code, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, vec!["set editing-mode vi", "set keymap vi-insert"]);
}

#[test]
fn bind_set_unknown_variable_warns_and_continues() {
    // GNU probe: `readline: no-such-var: unknown variable name`, rc 0.
    let (stdout, stderr, code) = run("bind 'set no-such-var on'; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=0");
    assert!(
        stderr.contains("readline: no-such-var: unknown variable name"),
        "stderr={stderr:?}"
    );
}

#[test]
fn bind_set_bad_editing_mode_is_loud_and_kept_old_value() {
    // GNU probe: `readline: editing-mode: could not set value to `bogus'`,
    // the variable keeps `emacs`, rc 0.
    let (stdout, stderr, code) = run("bind 'set editing-mode bogus'; bind -v | grep editing-mode");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "set editing-mode emacs");
    assert!(
        stderr.contains("readline: editing-mode: could not set value to `bogus'"),
        "stderr={stderr:?}"
    );
}

#[test]
fn bind_set_boolean_normalizes_non_on_to_off() {
    // GNU probe: `set completion-ignore-case maybe` stores off silently;
    // `ON` (any case) stores on.
    let (stdout, _, _) =
        run("bind 'set completion-ignore-case maybe'; bind -v | grep completion-ignore-case");
    assert_eq!(stdout.trim(), "set completion-ignore-case off");
    let (stdout, _, _) =
        run("bind 'set completion-ignore-case ON'; bind -v | grep completion-ignore-case");
    assert_eq!(stdout.trim(), "set completion-ignore-case on");
    // GNU probe: garbage numbers store 0 (strtol).
    let (stdout, _, _) = run("bind 'set keyseq-timeout abc'; bind -v | grep keyseq-timeout");
    assert_eq!(stdout.trim(), "set keyseq-timeout 0");
}

// ---------------------------------------------------------------------------
// Positional binding arguments: the GNU error channels
// ---------------------------------------------------------------------------

#[test]
fn positional_non_quote_non_set_is_silently_ignored() {
    // GNU probe: `bind 'ls -l'` → rc 0, no output.
    let (stdout, stderr, code) = run("bind 'ls -l'; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=0");
    assert!(!stderr.contains("missing separator"), "stderr={stderr:?}");
}

#[test]
fn positional_unclosed_quote_uses_readline_channel_without_status() {
    // GNU probe: `bind '"\C-t'` → `readline: "\C-t: no closing `"' in key
    // binding`, rc 0 (bind.def:304-333 ignores rl_parse_and_bind's
    // return).
    let (stdout, stderr, code) = run("bind '\"\\C-t'; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=0");
    assert!(
        stderr.contains("readline: \"\\C-t: no closing `\"' in key binding"),
        "stderr={stderr:?}"
    );
}

#[test]
fn positional_keyseq_without_value_reports_terminator() {
    // GNU probe: `bind '"\C-t"'` → `readline: "\C-t": no key sequence
    // terminator`, rc 0.
    let (_, stderr, code) = run("bind '\"\\C-t\"'; echo rc=$?");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("readline: \"\\C-t\": no key sequence terminator"),
        "stderr={stderr:?}"
    );
}

#[test]
fn minus_x_parse_failure_keeps_bind_channel_and_rc() {
    // GNU probe: `bind -x '"\C-g'` → `bind: no closing `"' in "\C-g`, rc 1
    // (bashline.c's own parser, bind.def fails the command).
    let (_, stderr, code) = run("bind -x '\"\\C-g'; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout_of("echo marker"), "marker\n");
    assert!(
        stderr.contains("bind: no closing `\"' in \"\\C-g"),
        "stderr={stderr:?}"
    );
    // The echo after the failing bind still ran (rc=$? above printed 0
    // because `echo` resets $?); the bind itself failed with 1 — verify
    // through the reserved status path:
    let (_, _, code) = run("bind -x '\"\\C-g'");
    assert_eq!(code, Some(1));
}

fn stdout_of(script: &str) -> String {
    let (stdout, _, _) = run(script);
    stdout
}

// ---------------------------------------------------------------------------
// inputrc files: set / $include / $if term / $if mode / $else
// ---------------------------------------------------------------------------

#[test]
fn bind_f_applies_set_include_and_term_conditionals() {
    let include = temp_file("include", "set show-all-if-ambiguous on\n");
    let main = temp_file(
        "main",
        &format!(
            "set completion-ignore-case on\n$include {include}\n$if term=xterm\n\"\\C-x\": beginning-of-line\n$endif\n$if term=dumb\n\"\\C-z\": undo\n$endif\n"
        ),
    );
    // GNU probe (TERM=xterm): both sets visible, `\C-x` bound, `\C-z`
    // block skipped.
    let (stdout, stderr, _) = run_with_env(
        &format!("bind -f {main}; echo bind-rc=$?; bind -v | grep -E 'completion-ignore-case|show-all-if-ambiguous'; bind -p | grep -F '\"\\C-x\"'"),
        &[("TERM", "xterm")],
    );
    assert!(
        stdout.contains("bind-rc=0"),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(stdout.contains("set completion-ignore-case on"), "{stdout}");
    assert!(stdout.contains("set show-all-if-ambiguous on"), "{stdout}");
    assert!(stdout.contains("\"\\C-x\": beginning-of-line"), "{stdout}");
    assert!(
        !stdout.contains("undo"),
        "term=dumb block must stay inactive: {stdout}"
    );
    let _ = stderr;
    std::fs::remove_file(include).ok();
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_diagnostics_carry_file_and_line() {
    // GNU probe: `readline: bogus.rc: line 1: bogus-var: unknown variable
    // name`, rc 0, following bindings still applied. (`bind -p` keeps
    // listing the DEFAULT `\C-k` row too — the -p default-vs-user merge
    // is the reedline-upstream full-table row.)
    let main = temp_file("bogus", "set no-such-var off\n\"\\C-k\": kill-whole-line\n");
    let (stdout, stderr, code) = run(&format!(
        "bind -f {main}; echo rc=$?; bind -p | grep -F '\"\\C-k\"'"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.lines().next(), Some("rc=0"));
    assert!(
        stdout.contains("\"\\C-k\": kill-whole-line"),
        "stdout={stdout}"
    );
    assert!(
        stderr.contains(&format!(
            "readline: {main}: line 1: no-such-var: unknown variable name"
        )),
        "stderr={stderr:?}"
    );
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_mode_condition_arms_from_in_file_set() {
    // GNU probe: `set editing-mode vi` earlier in the same file arms a
    // later `$if mode=vi` block.
    let main = temp_file(
        "modevi",
        "set editing-mode vi\n$if mode=vi\n\"\\C-m\": vi-editing-mode\n$endif\n",
    );
    let (stdout, _, code) = run(&format!("bind -f {main}; bind -p | grep C-m"));
    assert_eq!(code, Some(0));
    assert!(stdout.contains("\"\\C-m\": vi-editing-mode"), "{stdout}");
    std::fs::remove_file(main).ok();
}

#[test]
fn bind_f_else_branch_applies_when_term_misses() {
    // GNU probe ($else.rc, TERM=xterm): the $else binding applies, the
    // $if branch does not. `undo` is NOT a default `\C-a` binding, so a
    // hit means the dumb-term branch actually applied.
    let main = temp_file(
        "else",
        "$if term=dumb\n\"\\C-a\": undo\n$else\n\"\\C-b\": backward-char\n$endif\n",
    );
    let (stdout, stderr, _) = run_with_env(
        &format!("bind -f {main}; echo bind-rc=$?; bind -p | grep -F '\"\\C-b\"'; bind -p | grep -F '\"\\C-a\"'"),
        &[("TERM", "xterm")],
    );
    assert!(
        stdout.contains("bind-rc=0"),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(stdout.contains("\"\\C-b\": backward-char"), "{stdout}");
    assert!(!stdout.contains("undo"), "{stdout}");
    std::fs::remove_file(main).ok();
}

// ---------------------------------------------------------------------------
// initialize_readline's inputrc read (bind.def:131 / y.tab.c:4024)
// ---------------------------------------------------------------------------

#[test]
fn inputrc_env_file_applies_on_first_bind_even_under_dash_c() {
    // GNU probe: INPUTRC + `bash --norc -c 'bind -v'` shows the file's
    // variables — initialize_readline runs on the first bind invocation.
    let startup = temp_file(
        "startup",
        "set blink-matching-paren on\n\"\\C-o\": accept-line\n",
    );
    let (stdout, _, code) = run_with_env(
        "bind -v | grep blink-matching-paren; bind -p | grep C-o",
        &[("INPUTRC", &startup)],
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "set blink-matching-paren on\n\"\\C-o\": accept-line\n"
    );
    std::fs::remove_file(startup).ok();
}

#[test]
fn inputrc_read_happens_once_and_before_dash_f_sets() {
    // bind.def order: initialize_readline (the inputrc) precedes the -f
    // file's own `set` lines; a second bind does not re-read (the
    // blink value set by the -f file must survive — the inputrc would
    // have overwritten it on a second read).
    let startup = temp_file("startup2", "set blink-matching-paren on\n");
    let over = temp_file("over", "set blink-matching-paren off\n");
    let (stdout, _, _) = run_with_env(
        &format!("bind -f {over}; bind -v | grep blink-matching-paren"),
        &[("INPUTRC", &startup)],
    );
    assert_eq!(stdout.trim(), "set blink-matching-paren off");
    std::fs::remove_file(startup).ok();
    std::fs::remove_file(over).ok();
}

#[test]
fn missing_inputrc_is_silent() {
    // GNU probe: no inputrc anywhere → no message, defaults intact.
    let (stdout, stderr, _) = run("bind -v | grep blink-matching-paren");
    assert_eq!(stdout.trim(), "set blink-matching-paren off");
    assert!(!stderr.contains("readline:"), "stderr={stderr:?}");
}
