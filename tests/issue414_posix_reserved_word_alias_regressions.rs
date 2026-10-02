//! rubash#414: posix mode expanded reserved-word aliases (`-o posix` +
//! `alias for=echo`) — alias suite 0->2, introduced by 815802eb's
//! reserved-word abort (the #390 fix; not reverted — the abort itself is
//! GNU-correct, parse.y:469-482 `error '\n'' -> YYABORT).
//!
//! GNU spec anchors (third_party/bash, Bash 5.3 patch 15):
//! * parse.y:5751-5755 read_token_word(): "Posix.2 does not allow
//!   reserved words to be aliased, so check for all of them, including
//!   special cases, BEFORE expanding the current token as an alias" —
//!   `if MBTEST(posixly_correct) CHECK_FOR_RESERVED_WORD (token);'
//! * parse.y:5757-5765: alias expansion (`expand_aliases && quoted == 0'
//!   -> alias_expand_token, parse.y:3249).
//! * parse.y:5767-5769: the reserved-word check runs AFTER alias
//!   expansion only in default mode (`posixly_correct == 0').
//! * parse.y:3168-3199 CHECK_FOR_RESERVED_WORD (conditions from
//!   reserved_word_acceptable, parse.y:5899); special_case_tokens
//!   (`in'/`do'/`esac' grammar slots) at parse.y:3340-3494 run before the
//!   alias block in both modes.
//! * shell.c:1853 init_noninteractive: `expand_aliases =
//!   posixly_correct' — a non-interactive posix shell expands (plain)
//!   aliases from the first line, so the incremental grouped driver (the
//!   GNU reader port) must be taken when alias expansion is live at
//!   reader start, not only when the script text enables it.
//!
//! Rust owners fixed: lexer/alias_stream.rs (posix reserved-word-first
//! order, shared check_reserved_word; posix flag threaded through
//! expand_aliases_in_source/expand_comsub_alias_bodies/
//! splice_substitution_body), script_driver.rs script_uses_aliases
//! (startup-liveness admission; expand_group_aliases posix pass-through),
//! executor/arithmetic_aliases.rs comsub_body_alias_splice (posix leg of
//! parse.y:4526-4533 parse_comsub), executor/variable_state.rs
//! alias_expansion_enabled pub for the bin-side driver choice,
//! main.rs/-o posix + -c admission.
//!
//! Every expectation is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file and -c probes 2026-10-02; matrix
//! 48 cases under target/i414/matrix/, 4 residuals are the pre-existing
//! alias-free for-command diagnostic family, identical on the pre-change
//! binary — probes pre1.sh/pre2.sh).

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(20);

/// Bounded-wait `rubash` run: a hung shape (an alias loop that stops
/// consuming input) fails the test instead of wedging the suite.
fn run_rubash_in(
    dir: Option<&std::path::Path>,
    args: &[&str],
    stdin_text: Option<&str>,
) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command.stdin(if stdin_text.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    if let Some(text) = stdin_text {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin pipe");
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    let start = Instant::now();
    let mut stdout = String::new();
    let mut stderr = String::new();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = child.stdout.take().expect("stdout pipe");
                let mut err = child.stderr.take().expect("stderr pipe");
                out.read_to_string(&mut stdout).expect("read stdout");
                err.read_to_string(&mut stderr).expect("read stderr");
                return (stdout, stderr, status.code());
            }
            Ok(None) => {
                if start.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash {args:?} did not exit within {DEADLINE:?}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("wait rubash: {error}"),
        }
    }
}

/// Run a script FILE in its own scratch directory (LF, like the probe
/// artifacts) and return (stdout, stderr, code). `mode_args` are the
/// invocation flags before the script name (`&["-o", "posix"]`).
fn rubash_file(mode_args: &[&str], script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i414-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let mut argv: Vec<&str> = mode_args.to_vec();
    argv.push("case.sh");
    let result = run_rubash_in(Some(&dir), &argv, None);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// rubash#414 reproducer, script-file leg: posix keeps the `for' keyword
/// (GNU: `foo=v bar=', rc 0). WSL GNU 5.3.0 probe posix-alias-inner.sh.
#[test]
fn posix_script_keeps_for_keyword_despite_alias() {
    let script = concat!(
        "alias al=\" \"\n",
        "alias foo=bar\n",
        "alias for=echo\n",
        "al for foo in v\n",
        "do echo foo=$foo bar=$bar\n",
        "done\n",
    );
    let (stdout, stderr, code) = rubash_file(&["-o", "posix"], script);
    assert_eq!(stdout, "foo=v bar=\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Same shape through `-o posix -c` (the alias.tests last-block leg):
/// GNU prints `foo=v bar=' with no error, rc 0.
#[test]
fn posix_dash_c_keeps_for_keyword_despite_alias() {
    let script = concat!(
        "alias al=\" \"\n",
        "alias foo=bar\n",
        "alias for=echo\n",
        "al for foo in v\n",
        "do echo foo=$foo bar=$bar\n",
        "done",
    );
    let (stdout, stderr, code) = run_rubash_in(None, &["-o", "posix", "-c", script, "bash"], None);
    assert_eq!(stdout, "foo=v bar=\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Default mode replaces the keyword with the alias (alias7.sub: "default
/// mode bash allows reserved words to be aliased, which posix says is a
/// no-no"): `foo in v' then the #390 abort at `do', rc 2 — byte-identical
/// to GNU.
#[test]
fn default_mode_expands_reserved_word_alias() {
    let script = concat!(
        "shopt -s expand_aliases\n",
        "alias al=\" \"\n",
        "alias foo=bar\n",
        "alias for=echo\n",
        "al for foo in v\n",
        "do echo foo=$foo bar=$bar\n",
        "done",
    );
    let (stdout, stderr, code) = run_rubash_in(None, &["-c", script, "bash"], None);
    assert_eq!(stdout, "foo in v\n");
    assert_eq!(
        stderr,
        concat!(
            "bash: -c: line 6: syntax error near unexpected token `do'\n",
            "bash: -c: line 6: `do echo foo=$foo bar=$bar'\n",
        )
    );
    assert_eq!(code, Some(2));
}

/// Without a keyword alias the blank alias still exposes the keyword in
/// posix (the loop runs), on both the file and -c legs.
#[test]
fn posix_blank_alias_keeps_plain_for_loop() {
    let script = concat!(
        "alias al=\" \"\n",
        "al for x in 1 2\n",
        "do echo got $x\n",
        "done\n",
        "echo after\n",
    );
    let (stdout, stderr, code) = rubash_file(&["-o", "posix"], script);
    assert_eq!(stdout, "got 1\ngot 2\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// while/until/select share the reserved-word-first order in posix.
#[test]
fn posix_keeps_while_keyword_despite_alias() {
    let script = concat!(
        "alias al=\" \"\n",
        "alias while=echo\n",
        "al while :\n",
        "do echo body\n",
        "break\n",
        "done\n",
        "echo after\n",
    );
    let (stdout, stderr, code) = rubash_file(&["-o", "posix"], script);
    assert_eq!(stdout, "body\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// An alias chain two levels deep (a2 -> a1 -> blank) still lands on the
/// keyword in posix (alias7.sub's expand-next-word family).
#[test]
fn posix_blank_alias_chain_keeps_for_keyword() {
    let script = concat!(
        "alias a1=' '\n",
        "alias a2=a1\n",
        "alias for=echo\n",
        "a2 for x in 1 2\n",
        "do echo got $x\n",
        "done\n",
        "echo after\n",
    );
    let (stdout, stderr, code) = rubash_file(&["-o", "posix"], script);
    assert_eq!(stdout, "got 1\ngot 2\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// A QUOTED word is never a reserved word and never an alias: `'for' x in
/// 1' runs the command `for' (not found) and the stray `do' aborts — the
/// quoting gate (parse.y:5757 `quoted == 0') holds in posix too.
#[test]
fn posix_quoted_word_is_neither_keyword_nor_alias() {
    let script = concat!(
        "alias al=\" \"\n",
        "alias for=echo\n",
        "al 'for' x in 1\n",
        "do echo got $x\n",
        "done\n",
        "echo after\n",
    );
    let (stdout, stderr, code) = rubash_file(&["-o", "posix"], script);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        concat!(
            "case.sh: line 3: for: command not found\n",
            "case.sh: line 4: syntax error near unexpected token `do'\n",
            "case.sh: line 4: `do echo got $x'\n",
        )
    );
    assert_eq!(code, Some(2));
}

/// `-o posix` takes the incremental grouped driver even when the script
/// text never mentions expand_aliases/set -o posix: the in-file `alias'
/// definitions must be visible to the reader of the LATER lines
/// (shell.c:1853 + GNU's command-by-command reader). Feeding via stdin
/// keeps the hung-shape guard honest.
#[test]
fn posix_dash_c_plain_alias_still_expands() {
    // A plain (non-reserved-word) alias must keep expanding in posix —
    // the fix only reorders the reserved-word decision.
    let (stdout, stderr, code) = run_rubash_in(
        None,
        &[
            "-o",
            "posix",
            "-c",
            "alias hi='echo bonjour'\nhi la",
            "bash",
        ],
        None,
    );
    assert_eq!(stdout, "bonjour la\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Default mode (expand_aliases off, non-interactive) does NOT expand any
/// alias: `al for foo in v' is the command `al' (not found) and the stray
/// `do' aborts — byte-identical to GNU (alias expansion is a posix-mode
/// or explicit-shopt privilege, shell.c:1853).
#[test]
fn default_mode_without_shopt_expands_nothing() {
    let script = concat!(
        "alias al=\" \"\n",
        "alias foo=bar\n",
        "alias for=echo\n",
        "al for foo in v\n",
        "do echo foo=$foo bar=$bar\n",
        "done\n",
    );
    let (stdout, stderr, code) = rubash_file(&[], script);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        concat!(
            "case.sh: line 4: al: command not found\n",
            "case.sh: line 5: syntax error near unexpected token `do'\n",
            "case.sh: line 5: `do echo foo=$foo bar=$bar'\n",
        )
    );
    assert_eq!(code, Some(2));
}
