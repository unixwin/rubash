//! Issue rubash#1370: `command coproc C { ...; }` — a keyword-valued word
//! read in ARGUMENT position must not act as a reserved word. GNU produces
//! a reserved-word token only when the word is read where a reserved word
//! is acceptable (CHECK_FOR_RESERVED_WORD, parse.y:3168-3175, gated on
//! reserved_word_acceptable(last_read_token), parse.y:5899); anywhere else
//! the reader returns a plain WORD. Rubash's reader history recorded the
//! value-based TokenKind::Keyword for an argument-position `coproc`, so
//! `reserved_word_position()` treated the `{` after the coproc NAME as a
//! group opener and folded `{ echo hi; }` into one word token — the parser
//! never saw the stray `}`, and the line ran as `command coproc ...`
//! (127, execution continues) where GNU reports a syntax error and
//! abandons the line (rc=2).
//!
//! The companion guard is the 830774bd regression: an earlier fix demoted
//! EVERY keyword-kind token in the history, including the dedicated-branch
//! `(`/`)` tokens after a function NAME. That broke the reserved chain for
//! the body `{`, stopped the group fold, and the parser's brace-end scan
//! fell back to the last `}` of the stream — a script defining a function
//! and then another function inside a subshell died with a spurious
//! `syntax error near unexpected token `}'`. The demotion now applies only
//! to word-path keywords (classification::is_keyword), the exact set GNU
//! produces through read_token_word + CHECK_FOR_RESERVED_WORD.
//!
//! Baselines verified against GNU bash 5.3.20 (self-compiled oracle).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1370-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1370 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The issue's reproducer: GNU reports `syntax error near unexpected
/// token `}'` (rc=2) and abandons the whole line — the marker after it
/// never prints.
#[test]
fn issue1370_command_coproc_group_is_syntax_error() {
    let (rc, out, err) = run_script("command coproc C { echo hi; }\necho SHOULD_NOT_PRINT\n");
    assert_eq!(rc, Some(2), "stdout={out:?} stderr={err:?}");
    assert!(out.is_empty(), "stdout={out:?}");
    assert!(
        err.contains("syntax error near unexpected token `}'"),
        "stderr={err:?}"
    );
}

/// Same family through `function`: the keyword after `command` is an
/// argument word, so the following group braces are word text and the
/// stray `}` is a syntax error (GNU rc=2).
#[test]
fn issue1370_command_function_group_is_syntax_error() {
    let (rc, out, err) = run_script("command function f { echo hi; }\necho SHOULD_NOT_PRINT\n");
    assert_eq!(rc, Some(2), "stdout={out:?} stderr={err:?}");
    assert!(out.is_empty(), "stdout={out:?}");
    assert!(
        err.contains("syntax error near unexpected token `}'"),
        "stderr={err:?}"
    );
}

/// Regression guard for the 830774bd history-demotion regression: a
/// top-level function definition followed by a function definition inside
/// a subshell is an ordinary scripting pattern and must parse and run.
#[test]
fn issue1370_function_def_then_subshell_def_parses() {
    let (rc, out, err) = run_script(
        "show() { echo \"== $1\"; }\n(fq() { echo o1; }; fq) 2>&1 | cat\necho \"MARKER rc=$?\"\n",
    );
    assert_eq!(rc, Some(0), "stdout={out:?} stderr={err:?}");
    assert_eq!(out, "o1\nMARKER rc=0\n", "stderr={err:?}");
}

/// A real coproc in command position is unaffected: the named coproc runs
/// and its output is readable through the NAME fd array. The coproc body
/// lingers (`sleep 10`) so the NAME variables are still bound at the
/// read — an instantly-exiting coproc is reaped and unbound at the next
/// command boundary (the rubash#1023 semantics), which would make the
/// probe a reap race rather than a behavior check.
#[test]
fn issue1370_named_coproc_still_works() {
    let (rc, out, err) = run_script(
        "coproc C { echo hello; sleep 10; }\nread -u ${C[0]} line\necho \"got:$line\"\nkill $C_PID 2>/dev/null\n",
    );
    assert_eq!(rc, Some(0), "stdout={out:?} stderr={err:?}");
    assert_eq!(out, "got:hello\n", "stderr={err:?}");
}
