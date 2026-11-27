//! Issue rubash#543: two missing parser guards, each verified against WSL
//! GNU bash 5.3.0 from a script file before pinning.
//!
//! Format 1 (huck#618): an unterminated `$[` is a P_ARITH body — GNU
//! read_token_word hands it to parse_matched_pair (parse.y:4126-4136), so
//! EOF reports `unexpected EOF while looking for matching `]'` and exits 2;
//! the legacy form must not fall through as literal text (rc 0). The
//! closed form `echo $[1+2]` still evaluates to 3. Rust owner:
//! lexer/continuation.rs close_char_residuals_advance `$[` push (branch
//! fix/wave2c-parse-expand-env, P_ARITH delimiter `arith` mark); fresh
//! master RED.
//!
//! Format 2 (huck#340): GNU caps the number of here-documents pending in
//! one parse level at HEREDOC_MAX (shell.h:172, = 16). parse.y:3105
//! push_heredoc raises the fatal `maximum here-document count exceeded`
//! when the 17th redirection pushes (report_syntax_error -> reset_parser ->
//! exit_shell(EX_BADUSAGE)), so the 17-heredoc command never runs and later
//! script lines never execute. Counting facts pinned here, each probed:
//! the counter is per parse level — parse_command_substitution resets it
//! (parse.y:4524) and reset_parser clears it (parse.y:3529), so a command
//! substitution counts independently of its surrounding line; gather
//! (parse.y:3120) drains it per command line, so semicolon-joined
//! redirections on one line accumulate toward the cap; `<<<` herestrings
//! never push (parse.y grammar pushes only `<<`/`<<-` forms). The issue's
//! 70-line `cat <<E` construction keeps a single pending heredoc — the
//! first body swallows the remaining lines — and GNU itself answers rc 0
//! with the EOF warning; rubash matches. Rust owner: the lexer parks the
//! overflow in HEREDOC_OVERFLOW_LINE (lexer/mod.rs) and every driver turns
//! it into the EX_BADUSAGE exit (script_driver.rs, external_finish.rs,
//! builtins/source/execution.rs) — landed on master in 8b09b2f4, before
//! this issue's audit baseline, so format 2 here is a confirmation suite
//! pinning the real GNU semantics.

#![cfg(windows)]

use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_script(lines: &str) -> RunOutcome {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SCRATCH_SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue543-{}-{}",
        std::process::id(),
        SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue543 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, lines).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run_stdin(script: &str) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(script.as_bytes())?;
            child.wait_with_output()
        })
        .expect("run rubash on stdin");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn n_heredoc_line(n: usize) -> String {
    let mut line = String::from("cat");
    for _ in 0..n {
        line.push_str(" <<E");
    }
    line
}

// ---- Format 1: unterminated `$[` is a parse-time EOF naming `]` ----

#[test]
fn f1_unterminated_dollar_bracket_is_fatal_eof_naming_bracket() {
    let out = run_script("echo $[1+2\necho AFTER\n");
    assert_eq!(out.code, Some(2), "GNU exits 2 (parse.y EOF check)");
    let first = out.stderr.lines().next().unwrap_or_default();
    assert!(
        first.ends_with("unexpected EOF while looking for matching `]'"),
        "GNU message names `]', got: {first:?}"
    );
    assert!(
        !out.stdout.contains("AFTER"),
        "fatal: later lines never run"
    );
}

#[test]
fn f1_closed_dollar_bracket_still_evaluates() {
    let out = run_script("echo $[1+2]\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout.trim_end(), "3");
}

// ---- Format 2: HEREDOC_MAX pending-heredoc cap (parse.y:3105) ----

#[test]
fn f2_seventeenth_pending_heredoc_is_fatal() {
    let mut script = n_heredoc_line(17);
    script.push_str("\nE\necho AFTER\n");
    let out = run_script(&script);
    assert_eq!(out.code, Some(2), "GNU exits 2 (exit_shell(EX_BADUSAGE))");
    assert!(
        out.stderr.contains("maximum here-document count exceeded"),
        "GNU message, got: {:?}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("AFTER"),
        "fatal: later lines never run"
    );
}

#[test]
fn f2_sixteen_pending_heredocs_still_allowed() {
    let mut script = n_heredoc_line(16);
    script.push_str("\nE\necho OK16\n");
    let out = run_script(&script);
    assert_eq!(out.code, Some(0), "boundary: 16 pending is legal");
    assert!(
        out.stdout.contains("OK16"),
        "script continues, got {:?}",
        out.stdout
    );
}

#[test]
fn f2_semicolon_joined_redirections_accumulate_to_the_cap() {
    // 10 + 10 pending on one command line: gather runs at the newline
    // (parse.y:3120), so the 17th push fires before any body is read.
    let mut script = n_heredoc_line(10);
    script.push_str(" cat");
    script.push_str(&n_heredoc_line(10)[3..]);
    script.push_str("\nE\necho SEMI\n");
    let out = run_script(&script);
    assert_eq!(out.code, Some(2));
    assert!(out.stderr.contains("maximum here-document count exceeded"));
    assert!(!out.stdout.contains("SEMI"));
}

#[test]
fn f2_herestrings_do_not_count_toward_the_cap() {
    let mut script = String::from("cat");
    for _ in 0..17 {
        script.push_str(" <<<E");
    }
    script.push_str("\n echo HSTR\n");
    let out = run_script(&script);
    assert_eq!(out.code, Some(0), "17 herestrings are legal");
    assert!(out.stdout.contains("HSTR"));
}

#[test]
fn f2_command_substitution_counts_independently() {
    // parse.y:4524 parse_command_substitution resets need_here_doc: 10
    // pending on the outer line plus 10 inside `$( )` never reach 17.
    let mut script = n_heredoc_line(10);
    script.push_str(" $(cat");
    script.push_str(&n_heredoc_line(10)[3..]);
    script.push_str(")\nE\nF\necho CS10\n");
    let out = run_script(&script);
    assert_eq!(out.code, Some(0), "10 + 10 across levels is legal");
    assert!(out.stdout.contains("CS10"));
}

#[test]
fn f2_seventeen_pending_inside_command_substitution_is_fatal() {
    let mut script = String::from("echo $(cat");
    script.push_str(&n_heredoc_line(17)[3..]);
    script.push_str(")\n");
    let out = run_script(&script);
    assert_eq!(out.code, Some(2));
    assert!(out.stderr.contains("maximum here-document count exceeded"));
}

#[test]
fn f2_fatal_reports_the_line_of_the_offending_command() {
    let mut script = String::from("echo one\necho two\n");
    script.push_str(&n_heredoc_line(17));
    script.push('\n');
    let out = run_script(&script);
    assert_eq!(out.code, Some(2));
    assert!(
        out.stderr
            .contains("line 3: maximum here-document count exceeded"),
        "GNU reports the current input line, got: {:?}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim_end(),
        "one\ntwo",
        "earlier lines ran; parse died at line 3"
    );
}

#[test]
fn f2_cap_fires_inside_a_function_definition_before_it_runs() {
    let mut script = String::from("f() {\n");
    script.push_str(&n_heredoc_line(20));
    script.push_str("\nE\n}\necho FUNCDEF_DONE\n");
    let out = run_script(&script);
    assert_eq!(out.code, Some(2));
    assert!(
        !out.stdout.contains("FUNCDEF_DONE"),
        "definition itself fails to parse; nothing after runs"
    );
}

#[test]
fn f2_seventy_unterminated_heredoc_lines_stay_accepted_like_gnu() {
    // The issue's literal construction: 70 `cat <<E` lines. Only one
    // heredoc is ever pending (its body swallows the remaining lines), so
    // GNU answers rc 0 with the EOF warning and the first cat echoes the
    // 69 swallowed lines. rubash must keep matching, not "fix" toward the
    // issue's misread oracle cell.
    let mut script = String::new();
    for _ in 0..70 {
        script.push_str("cat <<E\n");
    }
    let out = run_script(&script);
    assert_eq!(out.code, Some(0));
    let body_lines = out.stdout.lines().filter(|l| *l == "cat <<E").count();
    assert_eq!(body_lines, 69, "first heredoc swallows lines 2..70");
    assert!(out
        .stderr
        .contains("here-document at line 1 delimited by end-of-file"));
}

#[test]
fn f2_stdin_driver_enforces_the_same_cap() {
    let mut script = n_heredoc_line(17);
    script.push_str("\nE\necho AFTER\n");
    let out = run_stdin(&script);
    assert_eq!(out.code, Some(2));
    assert!(out.stderr.contains("maximum here-document count exceeded"));
    assert!(!out.stdout.contains("AFTER"));
}
