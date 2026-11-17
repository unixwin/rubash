//! Issue rubash#3605 class B regression (run-exportfunc): an EOF-verdict
//! matched-pair inside an eval string must report the same line GNU's
//! yyerror reports — `line_number` at EOF (parse.y:6883-6891), i.e. one
//! past the eval string's last input line, mapped through the eval line
//! convention `caller_line + i - 1` (evalstring.c:345 `line_number--`).
//!
//! `unclosed_input_close_char_posix`'s `eof_line` is a line index (last
//! content line + 1), so the eval path maps it as `caller + eof_line - 1`;
//! the pre-fix code added `caller + eof_line` and reported `eval: line 84`
//! where GNU reports `eval: line 83` (exportfunc.tests:82
//! `eval 'foo() { _; } >_[${ $() }] ;{ echo eval ok; }'`).

use std::io::Write;
use std::process::Command;

/// Run `script` as a script FILE and return (stdout, stderr, rc).
fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| {
            // Strip the invocation-dependent `<path>: ` prolog.
            match line.find(": eval: ") {
                Some(idx) => line[idx + 2..].to_string(),
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, output.status.code())
}

/// The exportfunc.tests shape: `${ ` (post-bash43 dollar-brace) inside a
/// redirect target leaves an unterminated `}' — GNU reports EOF at
/// caller+1, not caller+2.
#[test]
fn eval_dollar_brace_eof_reports_eof_line() {
    let (stdout, stderr, _rc) = run_script("eval 'foo() { _; } >_[${ $() }] ;{ echo eval ok; }'\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        stderr, "eval: line 2: unexpected EOF while looking for matching `}'",
        "stderr"
    );
}

/// Same verdict at a deeper caller line and inside a multi-line eval
/// string: EOF line = caller + (eval lines) — GNU `eval: line 5`.
#[test]
fn eval_dollar_brace_eof_line_scales_with_caller() {
    let (stdout, stderr, _rc) =
        run_script("true\ntrue\neval 'echo a\nfoo() { _; } >_[${ $() }] ;{ echo eval ok; }'\n");
    assert_eq!(stdout, "a\n", "stdout");
    assert_eq!(
        stderr, "eval: line 5: unexpected EOF while looking for matching `}'",
        "stderr"
    );
}

/// An unterminated `${` enclosing an array subscript takes the same EOF
/// mapping — the named close is the parameter `}' (GNU `eval: line 2`
/// for `eval 'x=${a[1'`).
#[test]
fn eval_unclosed_subscript_reports_eof_line() {
    let (_stdout, stderr, _rc) = run_script("true\neval 'x=${a[1'\n");
    assert_eq!(
        stderr, "eval: line 2: unexpected EOF while looking for matching `}'",
        "stderr"
    );
}
