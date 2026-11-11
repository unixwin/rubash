//! rubash#2677 — a `$((` construct that fails the arithmetic
//! classification is a command substitution in GNU, and its body
//! parse errors are contained in the substitution child.
//!
//! GNU reads `$((` with parse_matched_pair(P_ARITH) only
//! (parse.y:4470) — no read-time yyparse. At expansion time the
//! LPAREN arm (subst.c:10841-10866) treats the extracted text as
//! arithmetic only when, past the leading `(`, it ends in `)` and
//! survives chk_arithsub (subst.c:9727); on either failure it goes
//! to the `comsub:` label (subst.c:10895) and runs the WHOLE
//! extracted text as a command substitution
//! (subst.c:10901 command_substitute). A body parse error is
//! therefore reported in the child shape
//! (`<name>: command substitution: line N: ...`), the substitution
//! yields nothing with status 2, and the enclosing command and
//! script run on — the same containment class as a backtick body
//! (#515). rubash classifies identically
//! (embedded_mutations.rs force_comsub) but reports the fallback
//! body's parse error in the READ-TIME shape and latches the
//! whole-script abort that models plain `$(` bodies (#2751),
//! which GNU never read-parsed here.
//!
//! The plain-`$(` controls pin the boundary: those bodies ARE
//! yyparsed at read time (parse.y:4549), stay fatal (rc 2,
//! nothing runs), and must NOT gain the `command substitution:`
//! segment. Their trailing offending-line echo is asserted only
//! by prefix: rubash echoes the stored body fragment there, GNU
//! the enclosing line — the #2751 documented remaining face, not
//! this issue's. The fallback cells assert stderr byte-for-byte,
//! including the echo, which for a fallback body is the extracted
//! body text itself in both shells.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-09).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2677-{tag}-{}", std::process::id()));
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
    // The diagnostic prolog follows argv[0] (rubash#2714 decision
    // D1); pin the invocation name to `bash` where the platform
    // allows, and normalize the harness exe path to `bash` in the
    // captured stderr on Windows (same harness shape as #2751).
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

/// A contained fallback report in file mode: the two GNU lines,
/// byte for byte — diagnostic, then the extracted body echoed.
fn assert_fallback(
    tag: &str,
    run: &Run,
    expected_stdout: &str,
    line: usize,
    token: &str,
    body: &str,
) {
    assert_eq!(run.stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(run.code, Some(0), "{tag}: rc");
    let expected_stderr = format!(
        "case.sh: command substitution: line {line}: syntax error near unexpected token `{token}'\n\
         case.sh: command substitution: line {line}: `{body}'\n"
    );
    assert_eq!(run.stderr, expected_stderr, "{tag}: stderr");
}

#[test]
fn issue2677_fallback_bare_argument() {
    let run = run_file("d1", "echo $(( 1 + 2)x)\necho continued\n");
    assert_fallback("d1", &run, "\ncontinued\n", 1, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_inside_double_quotes() {
    let run = run_file("d2", "echo \"[$(( 1 + 2)x)]\"\necho continued\n");
    assert_fallback("d2", &run, "[]\ncontinued\n", 1, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_assignment_value_and_use() {
    // Chain: the failed substitution still performs the
    // assignment (empty value), the next line reads the variable,
    // the line after proves the script ran on.
    let run = run_file("d3", "x=$(( 1 + 2)x)\necho \"x=[$x]\"\necho continued\n");
    assert_fallback("d3", &run, "x=[]\ncontinued\n", 1, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_assignment_status_is_two() {
    // Chain: an assignment-only command takes the substitution's
    // exit status — 2 for the contained parse failure — while the
    // script itself still exits 0 via the following lines.
    let run = run_file(
        "d8",
        "x=$(( 1 + 2)x)\necho \"rc=$? x=[$x]\"\necho continued\n",
    );
    assert_fallback("d8", &run, "rc=2 x=[]\ncontinued\n", 1, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_glued_word_text() {
    let run = run_file("d4", "echo pre$(( 1 + 2)x)post\necho continued\n");
    assert_fallback("d4", &run, "prepost\ncontinued\n", 1, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_sibling_word_still_expands() {
    let run = run_file("d5", "echo $(( 1 + 2)x) after\necho continued\n");
    assert_fallback("d5", &run, "after\ncontinued\n", 1, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_chk_arithsub_arm() {
    // The subst.c:10864 arm: the text ends in `)` but chk_arithsub
    // balance fails, so the whole span is a command substitution
    // whose body fails at the bare `+`.
    let run = run_file("d6", "echo $(( 1) + (2 ))\necho continued\n");
    assert_fallback("d6", &run, "\ncontinued\n", 1, "+", "( 1) + (2 )");
}

#[test]
fn issue2677_fallback_subshell_led_body() {
    // The body opens with a complete subshell command; the glued
    // bare word after it is the body's syntax error.
    let run = run_file("dval", "echo [$((echo hi) x)]\necho continued\n");
    assert_fallback("dval", &run, "[]\ncontinued\n", 1, "x", "(echo hi) x");
}

#[test]
fn issue2677_fallback_nested_in_plain_comsub() {
    // The fallback sits inside a plain `$(` body: one contained
    // report, the inner echo prints the (empty) substitution, the
    // outer substitution captures that line.
    let run = run_file("d7", "echo $(echo $(( 1 + 2)x))\necho continued\n");
    assert_fallback("d7", &run, "\ncontinued\n", 1, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_diagnostic_line_is_enclosing_line() {
    let run = run_file(
        "dline3",
        "echo a\necho b\necho $(( 1 + 2)x)\necho continued\n",
    );
    assert_fallback("dline3", &run, "a\nb\n\ncontinued\n", 3, "x", "( 1 + 2)x");
}

#[test]
fn issue2677_fallback_body_executes_when_valid() {
    // Mechanism proof: the fallback is a real command
    // substitution, not an error swallow — a valid body runs and
    // its output is captured (unquoted: field-split on the
    // newline).
    let run = run_file("drun", "echo [$((echo hi); echo yo)]\necho continued\n");
    assert_eq!(run.stdout, "[hi yo]\ncontinued\n", "drun: stdout");
    assert_eq!(run.stderr, "", "drun: stderr");
    assert_eq!(run.code, Some(0), "drun: rc");
}

#[test]
fn issue2677_fallback_body_output_quoted_verbatim() {
    let run = run_file(
        "drunq",
        "echo \"[$((echo hi); echo yo)]\"\necho continued\n",
    );
    assert_eq!(run.stdout, "[hi\nyo]\ncontinued\n", "drunq: stdout");
    assert_eq!(run.stderr, "", "drunq: stderr");
    assert_eq!(run.code, Some(0), "drunq: rc");
}

#[test]
fn issue2677_fallback_dash_c() {
    let run = run_c("echo $(( 1 + 2)x); echo continued");
    assert_eq!(run.stdout, "\ncontinued\n", "dc: stdout");
    assert_eq!(run.code, Some(0), "dc: rc");
    assert_eq!(
        run.stderr,
        "bash: command substitution: line 1: syntax error near unexpected token `x'\n\
         bash: command substitution: line 1: `( 1 + 2)x'\n",
        "dc: stderr"
    );
}

#[test]
fn issue2677_plain_comsub_stays_read_time_fatal() {
    // Boundary control: a plain `$(` body IS read-parsed
    // (parse.y:4549), so the same juxtaposition error stays a
    // whole-script fatal — no `command substitution:` segment, no
    // line runs. The diagnostic line is GNU's byte for byte; the
    // trailing echo is asserted by prefix only (see file header).
    let run = run_file("c1", "echo $( (1) x)\necho AFTER\n");
    assert_eq!(run.stdout, "", "c1: stdout");
    assert_eq!(run.code, Some(2), "c1: rc");
    assert!(
        run.stderr.starts_with(
            "case.sh: line 1: syntax error near unexpected token `x' \
             while looking for matching `)'\n"
        ),
        "c1: stderr diagnostic line\n got: {:?}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("command substitution:"),
        "c1: plain comsub must not gain the contained segment: {:?}",
        run.stderr
    );
}

#[test]
fn issue2677_plain_comsub_assignment_stays_fatal() {
    let run = run_file("c3", "x=$( (1) x)\necho AFTER\n");
    assert_eq!(run.stdout, "", "c3: stdout");
    assert_eq!(run.code, Some(2), "c3: rc");
    assert!(
        run.stderr.starts_with(
            "case.sh: line 1: syntax error near unexpected token `x' \
             while looking for matching `)'\n"
        ),
        "c3: stderr diagnostic line\n got: {:?}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("command substitution:"),
        "c3: plain comsub must not gain the contained segment: {:?}",
        run.stderr
    );
}

#[test]
fn issue2677_valid_arithmetic_controls() {
    // Regression locks on the classification itself: genuine
    // arithmetic — including nested parens and an embedded command
    // substitution — keeps evaluating as arithmetic.
    let run = run_file("c4", "echo $(( 1 + 2 ))\n");
    assert_eq!(run.stdout, "3\n", "c4: stdout");
    assert_eq!(run.stderr, "", "c4: stderr");
    assert_eq!(run.code, Some(0), "c4: rc");

    let run = run_file("c5", "echo $(( (1+2) * 3 ))\n");
    assert_eq!(run.stdout, "9\n", "c5: stdout");
    assert_eq!(run.stderr, "", "c5: stderr");
    assert_eq!(run.code, Some(0), "c5: rc");

    let run = run_file("c6", "echo $(( $(echo 5) + 1 ))\n");
    assert_eq!(run.stdout, "6\n", "c6: stdout");
    assert_eq!(run.stderr, "", "c6: stderr");
    assert_eq!(run.code, Some(0), "c6: rc");
}
