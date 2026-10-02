//! rubash#385: `case ... esac | cmd` — a case command as the first stage of
//! a pipeline — was rejected with `syntax error near unexpected token `|'`
//! (rc 2) where GNU 5.3.0 runs the pipeline.
//!
//! GNU spec:
//! - parse.y:855-856 `shell_command: ... | case_command` — the case command
//!   is a shell_command, which composes `command` and therefore the
//!   pipeline production; a `|` after a finished `esac` is a pipeline
//!   connector, never a case-level stray token.
//! - A malformed pipeline tail (`case x in esac|y) echo hi;;`) is rejected
//!   by the OUTER grammar's stray-`)` rule (yyerror, parse.y report_
//!   syntax_error), which is what names the `)`.
//!
//! Rust owners fixed:
//! - `case_stray_delimiter_index` (parser/case_command.rs) dropped its
//!   `Some("|")` arm, which blamed the `|` itself (`.or(Some(esac_index+1))`)
//!   for every `esac | ...` shape, empty case or not.
//! - `push_unexpected_token_error_named` (parser/parse_loop.rs) now drops
//!   the trailing command's PIPE/AND-OR LINKED stages when the failed
//!   command spans lines (GNU yyerror discards the whole in-progress
//!   command; the same-line heuristic alone let `case x in\nesac|y) ...`
//!   execute the pipeline stage `y` before silently losing the error).
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes
//! (target/issue-suites/results/wt37-gapfix1/, 2026-10-02).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(20);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue385-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue385 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");

    let stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stdout_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None => {
                if started.elapsed() >= RUN_LIMIT {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = stdout_reader.join().expect("join stdout reader");
    let stderr = stderr_reader.join().expect("join stderr reader");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

fn assert_script(script: &str, expected_stdout: &[u8]) {
    let outcome = run_rubash_script(script);
    assert!(!outcome.timed_out, "rubash hung on {script:?}");
    assert_eq!(outcome.code, Some(0), "rc for {script:?}");
    assert_eq!(outcome.stdout, expected_stdout, "stdout for {script:?}");
    assert!(
        outcome.stderr.is_empty(),
        "stderr for {script:?}: {:?}",
        String::from_utf8_lossy(&outcome.stderr)
    );
}

fn assert_syntax_error(script: &str, expected_stdout: &[u8], expected_line_text: &str) {
    let outcome = run_rubash_script(script);
    assert!(!outcome.timed_out, "rubash hung on {script:?}");
    assert_eq!(outcome.code, Some(2), "rc for {script:?}");
    assert_eq!(outcome.stdout, expected_stdout, "stdout for {script:?}");
    let stderr = String::from_utf8_lossy(&outcome.stderr).to_string();
    // Every stderr line carries the probe script's absolute path prefix;
    // assert the diagnostic text on each line instead.
    let mut lines = stderr.lines();
    let first = lines.next().unwrap_or_default();
    let second = lines.next().unwrap_or_default();
    assert!(
        first.ends_with("line 2: syntax error near unexpected token `)'"),
        "first stderr line for {script:?} was {first:?}"
    );
    assert!(
        second.ends_with(&format!("line 2: `{expected_line_text}'")),
        "second stderr line for {script:?} was {second:?}"
    );
}

#[test]
fn case_command_as_pipeline_stage_runs() {
    // The #385 reproducer (GNU: rc=1 from grep, empty stderr).
    assert_script(
        "case $w in yes) echo yes ;; *) echo no ;; esac | grep yes\necho rc=$?\n",
        b"rc=1\n",
    );
}

#[test]
fn empty_case_as_pipeline_stage_runs() {
    assert_script("case x in esac | cat\necho a=$?\n", b"a=0\n");
    assert_script("case x in esac | grep q\necho c=$?\n", b"c=1\n");
}

#[test]
fn case_pipeline_stage_with_output_and_chaining() {
    assert_script(
        "case x in a) :;; esac | echo tail\necho d=$?\n",
        b"tail\nd=0\n",
    );
    assert_script(
        "w2=zz\ncase $w2 in zz) echo z1 ;; esac | grep z1\necho e=$?\n",
        b"z1\ne=0\n",
    );
    assert_script(
        "case $w in *) echo multi ;; esac | cat | cat\necho f=$?\n",
        b"multi\nf=0\n",
    );
}

#[test]
fn other_compound_stages_still_work() {
    assert_script("{ echo b1; } | cat\n", b"b1\n");
    assert_script("if true; then echo i1; fi | cat\n", b"i1\n");
    assert_script("for i in 1; do echo f$i; done | cat\n", b"f1\n");
    assert_script("(echo s1) | cat\n", b"s1\n");
}

#[test]
fn malformed_pipeline_tail_still_errors_at_the_paren() {
    // `|` after esac is legal; the stray `)` in command position is the
    // yacc error, and nothing from the failed command runs.
    assert_syntax_error(
        "case x in\nesac|y) echo hi;;\necho after\n",
        b"",
        "esac|y) echo hi;;",
    );
    assert_syntax_error(
        "case x in\nesac | cat) echo hi;;\necho after\n",
        b"",
        "esac | cat) echo hi;;",
    );
}

#[test]
fn spanning_pipeline_error_discards_the_whole_command() {
    // The pipeline's first stage opened on line 2; the stray `)` on line 4
    // must abort BOTH stages (GNU yyerror discards the in-progress
    // command; `run2` never prints on either shell, `run1` already ran).
    let outcome = run_rubash_script("echo run1\ntrue |\nfalse\n) echo bad\necho run2\n");
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(2));
    assert_eq!(outcome.stdout, b"run1\n");
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    assert!(
        stderr.contains("line 4: syntax error near unexpected token `)'"),
        "{stderr}"
    );
    assert!(stderr.contains("line 4: `) echo bad'"), "{stderr}");
}

#[test]
fn stray_paren_directly_after_esac_still_errors() {
    // rubash#381 shapes keep their rejection (kept `)`/`(` arms).
    let outcome = run_rubash_script("case x in\nesac) echo hi;;\nesac\n");
    assert_eq!(outcome.code, Some(2));
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "{stderr}"
    );
}
