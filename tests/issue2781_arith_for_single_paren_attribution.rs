//! rubash#2781 — arith-for headers closed by a SINGLE `)` are not
//! the matched-pair EOF form: GNU parse_arith_cmd (parse.y:4957)
//! gets its matched pair, finds the next char is not `)`, and the
//! header read fails; report_syntax_error then names a token by
//! error_token_from_text (parse.y:6772) — the SHORT form
//! `syntax error near `<tok>'` plus the offending-line echo. The
//! token is the text scanned back from just after the pair-close
//! paren over non-delimiter chars (delimiters " \n\t;|&", the
//! delimiter itself included): `;` for a header whose sections
//! completed, `;i<3)` for `for ((i=0;i<3) x`, `;)` for
//! `for ((i=0;)`. Expectations measured against GNU bash 5.3.20
//! (2026-11-08).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    run_file_env(tag, script.as_bytes(), &[])
}

fn run_file_env(tag: &str, script: &[u8], extra_env: &[(&str, &str)]) -> Run {
    let dir: PathBuf = std::env::temp_dir().join(format!("rubashP1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
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

fn assert_run(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

#[test]
fn issue2781_single_paren_header_with_body_names_semicolon() {
    let run = run_c("for ((i=0;i<3;i++); do echo hi; done");
    assert_run(
        "a",
        &run,
        "",
        "bash: -c: line 1: syntax error near `;'\n\
         bash: -c: line 1: `for ((i=0;i<3;i++); do echo hi; done'\n",
        2,
    );
}

#[test]
fn issue2781_single_paren_header_with_body_file_mode() {
    let run = run_file("a-file", "for ((i=0;i<3;i++); do echo hi; done\n");
    assert_run(
        "a-file",
        &run,
        "",
        "case.sh: line 1: syntax error near `;'\n\
         case.sh: line 1: `for ((i=0;i<3;i++); do echo hi; done'\n",
        2,
    );
}

#[test]
fn issue2781_single_paren_header_junk_word_names_from_text() {
    let run = run_c("for ((i=0;i<3) x");
    assert_run(
        "b",
        &run,
        "",
        "bash: -c: line 1: syntax error near `;i<3)'\n\
         bash: -c: line 1: `for ((i=0;i<3) x'\n",
        2,
    );
}

#[test]
fn issue2781_single_paren_incomplete_sections_names_semi_paren() {
    let run = run_c("for ((i=0;)");
    assert_run(
        "c",
        &run,
        "",
        "bash: -c: line 1: syntax error near `;)'\n\
         bash: -c: line 1: `for ((i=0;)'\n",
        2,
    );
}

#[test]
fn issue2781_spaced_single_paren_header_names_semicolon() {
    // The spaced form used to route to header validation and print
    // the `syntax error: `;' unexpected` production; GNU gives the
    // same short from-text form as the compact shapes.
    let run = run_c("for (( i=0; i<1; i++ ); do :; done");
    assert_run(
        "d",
        &run,
        "",
        "bash: -c: line 1: syntax error near `;'\n\
         bash: -c: line 1: `for (( i=0; i<1; i++ ); do :; done'\n",
        2,
    );
}

#[test]
fn issue2781_wellformed_arith_for_control() {
    let run = run_c("for ((i=0;i<3;i++)); do echo $i; done");
    assert_run("ctrl", &run, "0\n1\n2\n", "", 0);
}

#[test]
fn issue2781_unclosed_header_keeps_eof_form_control() {
    // No closing paren at all: the matched-pair EOF form stands
    // (rubash#2749), NOT the from-text short form.
    let run = run_c("for ((i=0;i<3;i++");
    assert_run(
        "eof-ctrl",
        &run,
        "",
        "bash: -c: line 1: unexpected EOF while looking for matching `)'\n",
        2,
    );
}
