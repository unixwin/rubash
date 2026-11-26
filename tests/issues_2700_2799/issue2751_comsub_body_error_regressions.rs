//! rubash#2751 — command-substitution body parse errors, reported
//! the way GNU's reader reports them.
//!
//! GNU parses a `$(` body inside the enclosing reader with
//! shell_eof_token = `)' (parse.y:4519-4521 parse_comsub), so the
//! report is the enclosing command's own yyerror:
//! - a near error gains ` while looking for matching `)'`;
//! - a body that runs into its end names the `)' itself;
//! - a conditional's SHORT near form walks over the closing paren
//!   when it is glued to the conditional's end;
//! - exactly ONE diagnostic prints, however deep the nesting — there
//!   is no per-level `syntax error in command substitution' line.
//! A backtick body instead runs in the parse_and_execute child
//! (subst.c:7413), whose reports carry the `command substitution:'
//! prefix, let the enclosing command continue, and exit 0.
//!
//! The `-c` exit status for the `$(` shapes (GNU 127 vs rubash 2) is
//! an OWNER-DECISION item and deliberately NOT pinned here; every
//! `$(` case below runs in file mode, where both shells exit 2.
//! The offending-line ECHO for `$(` bodies is the enclosing line
//! since rubash#2783 (the driver parks the reader unit's text on the
//! shell state for the unit's extent) —
//! see issue2751_dollar_echo_is_the_enclosing_line.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-04).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2751-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_c(script: &str) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    // rubash#2714 (decision D1): the diagnostic prolog follows
    // argv[0], so the expectations below — measured against GNU
    // invoked as `bash` — pin the invocation name to `bash` where
    // the platform allows it (std exposes no argv[0] override on
    // Windows; there the harness's own exe-path token is normalized
    // to `bash` in the captured stderr instead, the same
    // normalization the GNU suites apply to the baseline's path).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0("bash");
    }
    let output = command.output().expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    #[cfg(windows)]
    let stderr = {
        let exe = env!("CARGO_BIN_EXE_rubash");
        let slashed = exe.replace('\\', "/");
        let mut normalized = stderr.replace(exe, "bash").replace(&slashed, "bash");
        if let Some(drive) = slashed.chars().next().filter(|c| c.is_ascii_alphabetic()) {
            if slashed.as_bytes().get(1) == Some(&b':') {
                let posix = format!("/{}/{}", drive.to_ascii_lowercase(), &slashed[3..]);
                normalized = normalized.replace(&posix, "bash");
            }
        }
        normalized
    };
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        code: output.status.code(),
    }
}

/// The diagnostic lines (everything except the trailing offending-
/// line echo) are GNU's, byte for byte; nothing executes; the file
/// exits 2; and no second `syntax error` line appears at any depth.
fn assert_diagnostic_lines(tag: &str, run: &Run, expected_lines: &[&str]) {
    assert_eq!(run.stdout, "", "{tag}: stdout");
    assert_eq!(run.code, Some(2), "{tag}: rc");
    let expected = expected_lines.join("\n") + "\n";
    assert!(
        run.stderr.starts_with(&expected),
        "{tag}: stderr must start with the GNU diagnostic lines\nwant prefix: {expected:?}\ngot: {:?}",
        run.stderr
    );
    let syntax_lines = run
        .stderr
        .lines()
        .filter(|line| line.contains("syntax error"))
        .count();
    let expected_syntax = expected_lines
        .iter()
        .filter(|line| line.contains("syntax error"))
        .count();
    assert_eq!(
        syntax_lines, expected_syntax,
        "{tag}: exactly one report — no per-level escalation line: {:?}",
        run.stderr
    );
}

#[test]
fn issue2751_suffix_for_if_body() {
    let run = run_file("a", "echo $(if then)\n");
    assert_diagnostic_lines(
        "a",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `then' while looking for matching `)'"],
    );
}

#[test]
fn issue2751_suffix_for_case_body() {
    let run = run_file("b", "echo $(case in esac)\n");
    assert_diagnostic_lines(
        "b",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `esac' while looking for matching `)'"],
    );
}

#[test]
fn issue2751_suffix_for_operator_body() {
    let run = run_file("c", "echo $(&&)\n");
    assert_diagnostic_lines(
        "c",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `&&' while looking for matching `)'"],
    );
}

#[test]
fn issue2751_suffix_for_list_body_and_nothing_runs() {
    // The body's `echo hi` must not print: GNU's failure is at read
    // time, before any of the body executes.
    let run = run_file("d", "echo $(echo hi; &&)\n");
    assert_diagnostic_lines(
        "d",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `&&' while looking for matching `)'"],
    );
}

#[test]
fn issue2751_single_report_at_nesting_depth_two() {
    let run = run_file("e", "echo $(echo $(if then))\n");
    assert_diagnostic_lines(
        "e",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `then' while looking for matching `)'"],
    );
}

#[test]
fn issue2751_cond_body_walk_extends_over_glued_paren() {
    let run = run_file("cond", "echo $([[ a ==]])\n");
    assert_diagnostic_lines(
        "cond",
        &run,
        &[
            "case.sh: line 1: unexpected token `==]]', conditional binary operator expected",
            "case.sh: line 1: syntax error near `==]])'",
        ],
    );
    let run = run_file("cond2", "echo $([[ a == ]])\n");
    assert_diagnostic_lines(
        "cond2",
        &run,
        &[
            "case.sh: line 1: unexpected argument `]]' to conditional binary operator",
            "case.sh: line 1: syntax error near `]])'",
        ],
    );
}

#[test]
fn issue2751_cond_body_not_at_end_keeps_body_near() {
    let run = run_file("cond3", "echo $([[ a == ]] && echo inner)\n");
    assert_diagnostic_lines(
        "cond3",
        &run,
        &[
            "case.sh: line 1: unexpected argument `]]' to conditional binary operator",
            "case.sh: line 1: syntax error near `]]'",
        ],
    );
}

#[test]
fn issue2751_body_running_into_its_end_names_the_paren() {
    let run = run_file("eof-if", "echo $(if true; then echo)\n");
    assert_diagnostic_lines(
        "eof-if",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `)'"],
    );
    let run = run_file("eof-while", "echo $(while true; do echo)\n");
    assert_diagnostic_lines(
        "eof-while",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `)'"],
    );
}

#[test]
fn issue2751_following_lines_never_run() {
    let run = run_file("file", "echo $(if then)\necho AFTER\n");
    assert_diagnostic_lines(
        "file",
        &run,
        &["case.sh: line 1: syntax error near unexpected token `then' while looking for matching `)'"],
    );
}

#[test]
fn issue2751_dollar_echo_is_the_enclosing_line() {
    // rubash#2783: GNU echoes the enclosing line (`echo $(if then)',
    // print_offending_line over shell_input_line, parse.y:6814). The
    // driver now parks the reader unit's source text on the shell
    // state for the unit's extent, and the body reporter reads the
    // physical line back out of it — the echo is byte-exact GNU.
    let run = run_file("echo-doc", "echo $(if then)\n");
    assert_eq!(
        run.stderr,
        "case.sh: line 1: syntax error near unexpected token `then' while looking for matching `)'\n\
         case.sh: line 1: `echo $(if then)'\n",
        "echo-doc: stderr"
    );
}

#[test]
fn issue2751_backtick_if_body_child_report() {
    let run = run_c("echo `if then`");
    assert_eq!(run.stdout, "\n", "bt-if: stdout");
    assert_eq!(
        run.stderr,
        "bash: command substitution: line 1: syntax error near unexpected token `then'\n\
         bash: command substitution: line 1: `if then'\n",
        "bt-if: stderr"
    );
    assert_eq!(run.code, Some(0), "bt-if: rc");
}

#[test]
fn issue2751_backtick_loop_body_child_report() {
    let run = run_c("echo `while do done`");
    assert_eq!(run.stdout, "\n", "bt-while: stdout");
    assert_eq!(
        run.stderr,
        "bash: command substitution: line 1: syntax error near unexpected token `do'\n\
         bash: command substitution: line 1: `while do done'\n",
        "bt-while: stderr"
    );
    assert_eq!(run.code, Some(0), "bt-while: rc");
}

#[test]
fn issue2751_backtick_dangling_pipe_child_report() {
    let run = run_c("echo `echo hi |`");
    assert_eq!(run.stdout, "\n", "bt-pipe: stdout");
    assert_eq!(
        run.stderr, "bash: command substitution: line 2: syntax error: unexpected end of file\n",
        "bt-pipe: stderr"
    );
    assert_eq!(run.code, Some(0), "bt-pipe: rc");
}

#[test]
fn issue2751_backtick_cond_child_report() {
    let run = run_c("echo `[[ a == ]]`");
    assert_eq!(run.stdout, "\n", "bt-cond: stdout");
    assert_eq!(
        run.stderr,
        "bash: command substitution: line 1: unexpected argument `]]' to conditional binary operator\n\
         bash: command substitution: line 1: syntax error near `]]'\n\
         bash: command substitution: line 1: `[[ a == ]]'\n",
        "bt-cond: stderr"
    );
    assert_eq!(run.code, Some(0), "bt-cond: rc");
}

#[test]
fn issue2751_backtick_cond_absorbed_child_report() {
    let run = run_c("echo `[[ a ==]]`");
    assert_eq!(run.stdout, "\n", "bt-cond-abs: stdout");
    assert_eq!(
        run.stderr,
        "bash: command substitution: line 1: unexpected token `==]]', conditional binary operator expected\n\
         bash: command substitution: line 1: syntax error near `==]]'\n\
         bash: command substitution: line 1: `[[ a ==]]'\n",
        "bt-cond-abs: stderr"
    );
    assert_eq!(run.code, Some(0), "bt-cond-abs: rc");
}

#[test]
fn issue2751_valid_substitution_control() {
    let run = run_c("echo $(echo ok)");
    assert_eq!(run.stdout, "ok\n", "ctrl: stdout");
    assert_eq!(run.stderr, "", "ctrl: stderr");
    assert_eq!(run.code, Some(0), "ctrl: rc");
}
