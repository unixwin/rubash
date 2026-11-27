//! Issue rubash#757: the prompt-expansion path was incomplete — bare
//! special parameters stayed literal and `shopt -u promptvars` had no
//! effect. Both facets are reachable without a PTY through `${PS1@P}`.
//!
//! GNU spec (third_party/bash parse.y, vendored 5.3):
//! - parse.y:6683-6704 decode_prompt_string runs a SECOND pass over the
//!   backslash-decoded string: `promptvars || posixly_correct` expands it
//!   as a double-quoted word (parse.y:6689 expand_prompt_string with
//!   Q_DOUBLE_QUOTES); with `shopt -u promptvars` (and no posix mode)
//!   there is no expansion pass at all — `$var` text stays literal.
//! - parse.y:6586-6592: with the second pass enabled a non-root `\$` is
//!   emitted backslash-quoted so the pass keeps the dollar literal (root
//!   renders `#`), and syntax.h:26 `slashify_in_quotes` (`\` $ " `) is
//!   the escape set sh_backslash_quote_for_double_quotes applies to the
//!   `\w`/`\u`/`\h`/`\D{}` decode outputs (parse.y:6434/6452/6533/6549/
//!   6563) so their data cannot be misread as expansion syntax.
//!
//! GNU differential evidence (WSL GNU bash 5.3.0, stdin probes 2026-10-18,
//! LF scripts, `cat probe | wsl.exe bash -s | tr -d '\0'`):
//!   false; PS1='x$?y'          -> [x0y]        (was [x$?y])
//!   PS1='A$MYVAR B' vars on    -> [Axyz B]
//!   shopt -u promptvars        -> [A$MYVAR B]  (was [Axyz B])
//!   set -o posix + vars off    -> [Pxyz S]     (posix keeps expanding)
//!   PS1='c`echo bt`d'          -> [cbtd]       (was literal backticks)
//!   set -- one two; '$1 a$2'   -> [none atwo z]
//!   '$$ q$! r$- s$#'           -> pid/empty/flags/2 (bare specials expand)
//!   '$((3+4))'                 -> [m7n]        (was `3+4: command not found`)
//!   PS1='q"quote$dollar'       -> [q"quote]    (dq word: `"` is data)
//!   mkdir d$x; PS1='\w'        -> .../d$x      (`\$` in \w data stays literal)

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run(exec: &mut Executor, src: &str) {
    let ast = parse(&tokenize(src));
    exec.execute_ast(&ast).ok();
}

fn prompt_transform(exec: &mut Executor, value: &str) -> String {
    // The script-level shape of the `${PS1@P}` transform: stage the value
    // in PS1 (single-quoted, so backslashes and `"` ride through raw and
    // only `'` needs the `'\''` dance), expand through the transform into
    // a variable, read it back.
    let escaped = value.replace('\'', "'\\''");
    run(exec, &format!("PS1='{escaped}'; p757_out=${{PS1@P}}"));
    exec.get_env("p757_out").unwrap_or_default().to_string()
}

/// Facet 1: bare special parameters expand in the prompt word (GNU
/// expands the decoded prompt as a double-quoted word, parse.y:6689).
#[test]
fn bare_special_parameters_expand_in_prompt() {
    let mut exec = Executor::new();

    // $? (the ticket's repro: `false; PS1='x$?y'` -> [x0y]).
    run(&mut exec, "false");
    assert_eq!(prompt_transform(&mut exec, "x$?y"), "x0y");

    // Positional parameters and their count.
    run(&mut exec, "set -- one two");
    assert_eq!(prompt_transform(&mut exec, "n$1 a$2 z"), "none atwo z");
    assert_eq!(prompt_transform(&mut exec, "c$#"), "c2");

    // Arithmetic expansion, which GNU also reaches (probe: [m7n]).
    assert_eq!(prompt_transform(&mut exec, "m$((3+4))n"), "m7n");

    // Backtick command substitution (probe: [cbtd]).
    assert_eq!(prompt_transform(&mut exec, "c`echo bt`d"), "cbtd");

    // $$ renders digits (the PID), $- the option flags letter set.
    let pid = prompt_transform(&mut exec, "p$$");
    assert!(
        pid.len() > 1 && pid[1..].chars().all(|c| c.is_ascii_digit()),
        "{pid}"
    );
    let flags = prompt_transform(&mut exec, "f$-");
    assert!(flags.len() > 1, "flags render: {flags}");
}

/// Facet 2: `shopt -u promptvars` stops the expansion pass entirely
/// (parse.y:6683); posix mode keeps it on (parse.y:6683
/// `promptvars || posixly_correct`).
#[test]
fn promptvars_off_keeps_parameter_text_literal() {
    let mut exec = Executor::new();
    run(&mut exec, "MYVAR=xyz");

    // Default (promptvars on): expansion happens.
    assert_eq!(prompt_transform(&mut exec, "A$MYVAR B"), "Axyz B");

    // promptvars off: the value stays literal — the option whose entire
    // purpose is to stop prompt expansion must have that effect.
    run(&mut exec, "shopt -u promptvars");
    assert_eq!(prompt_transform(&mut exec, "A$MYVAR B"), "A$MYVAR B");
    assert_eq!(
        prompt_transform(&mut exec, "c`echo bt`d"),
        "c`echo bt`d",
        "command substitution is part of the same gated pass"
    );

    // ...and back on: expansion resumes.
    run(&mut exec, "shopt -s promptvars");
    assert_eq!(prompt_transform(&mut exec, "A$MYVAR B"), "Axyz B");

    // Posix mode forces the pass even with promptvars off (GNU probe:
    // set -o posix + shopt -u promptvars -> [Pxyz S]).
    run(&mut exec, "set -o posix; shopt -u promptvars");
    assert_eq!(prompt_transform(&mut exec, "P$MYVAR S"), "Pxyz S");
    run(&mut exec, "set +o posix");
}

/// The render channel obeys the same gate: a promptvars-off render leaves
/// `$var` literal (the interactive channel shares parse.y:6683).
#[test]
fn render_channel_respects_promptvars_gate() {
    let mut exec = Executor::new();
    run(&mut exec, "MYVAR=xyz");

    assert_eq!(exec.expand_prompt_string("A$MYVAR B"), "Axyz B");
    run(&mut exec, "shopt -u promptvars");
    assert_eq!(exec.expand_prompt_string("A$MYVAR B"), "A$MYVAR B");
    assert_eq!(
        exec.expand_prompt_string_mut("A$MYVAR B"),
        "A$MYVAR B",
        "the interactive driver channel obeys the same shopt"
    );
}

/// The `\$` decode contract (parse.y:6586-6592): the dollar that came from
/// `\$` survives the second pass literally, so `\$NAME` never expands NAME.
/// Root renders `#` (rubash on Windows), non-root renders the
/// backslash-quoted `$` (unix CI runners); both branches must keep the
/// trailing name un-expanded. The exact per-branch byte contract is pinned
/// by the in-crate unit test `prompt_dollar_escape_uses_effective_uid`.
#[test]
fn escaped_dollar_stays_literal_through_the_pass() {
    let mut exec = Executor::new();
    run(&mut exec, "USER=rootish");

    let root = exec.get_env("EUID") == Some("0");
    let expected = if root { "#USER tail" } else { "$USER tail" };
    assert_eq!(prompt_transform(&mut exec, "\\$USER tail"), expected);
}

/// A `"` inside the prompt value is DATA in the double-quoted pass word
/// (GNU probe: PS1='q"quote$dollar' with `dollar` unset -> [q"quote]).
#[test]
fn data_double_quotes_stay_in_the_prompt() {
    let mut exec = Executor::new();
    assert_eq!(prompt_transform(&mut exec, "q\"quote$dollar"), "q\"quote");
}

/// Decode outputs that carry filesystem text (`\w`) are protected from the
/// pass (parse.y:6533): a `$` inside the directory name stays literal
/// (GNU probe: mkdir 'd$x'; PS1='\w' -> .../d$x).
#[test]
fn working_directory_dollar_stays_literal() {
    let mut exec = Executor::new();
    run(
        &mut exec,
        r#"mkdir -p 'p757_d$x' 2>/dev/null; cd 'p757_d$x' 2>/dev/null"#,
    );
    let rendered = exec.expand_prompt_string("W:\\w");
    assert!(rendered.starts_with("W:"), "prompt prefix: {rendered}");
    // The trailing component is the literal directory name — no expansion
    // of `$x`, no stray backslash from the protection quoting.
    assert!(
        rendered.ends_with("p757_d$x"),
        "trailing component intact: {rendered}"
    );
}

/// CR follow-up (F1): the subst.c:4392-4418 containment — a malformed
/// `${...}` inside the prompt value REPORTS and falls back to the literal
/// decoded string without failing the enclosing command (GNU probe:
/// `PS1='a${1x}b'; p=${PS1@P}` keeps rc 0 with p literal).
#[test]
fn malformed_braced_parameter_contained_in_prompt_transform() {
    let mut exec = Executor::new();

    run(&mut exec, "PS1='a${1x}b'; p757_out=${PS1@P}");
    assert_eq!(exec.last_exit_code(), 0, "assignment rc stays 0");
    assert_eq!(
        exec.get_env("p757_out"),
        Some("a${1x}b"),
        "value falls back to the literal decoded prompt"
    );

    // The `?` operator verdict reports instead of substituting the message
    // (GNU probe: `PS1='er:${nose?bad} t'` -> stderr `nose: bad`, literal).
    run(
        &mut exec,
        "unset p757_nose; PS1='er:${p757_nose?bad} t'; p757_out=${PS1@P}",
    );
    assert_eq!(exec.last_exit_code(), 0);
    assert_eq!(
        exec.get_env("p757_out"),
        Some("er:${p757_nose?bad} t"),
        "`?` verdict never substitutes the message into the value"
    );
}
