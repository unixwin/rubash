//! rubash#3227 — a `#` comment after a conditional binary/unary
//! operator swallows the rest of the line before tokenization, so the
//! collected arguments cannot see it — but GNU's error_token_from_text
//! (parse.y:6772-6808) walks the RAW line back from the newline and
//! names what is actually there: `[[ a == #x ]]` names `]]` (not
//! `==`), `[[ a == #x` names `#x`, `[[ -n #x ]]` names `]]`.
//! A plain newline with no swallowed text still names the last
//! collected argument (`[[ a ==` names `==`). Same root-cause family
//! as #3031/t0416's one-character source_extra borrow, different
//! landing point: here the offender is the virtual newline itself and
//! the whole near token comes from the raw line tail.
//! Expectations measured against GNU bash 5.3.20 (2026-11-16).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash3227-{tag}-{}", std::process::id()));
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

fn assert_run(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

#[test]
fn issue3227_rhs_hash_names_close_bracket() {
    // GNU t1: the comment swallows `#x ]]`, the newline trips as the
    // binary-operator argument, and the near token is the `]]` the raw
    // line still carries — not the last collected argument `==`.
    let run = run_file("rhs-hash", "[[ a == #x ]]\necho rc=$?\n");
    assert_run(
        "rhs-hash",
        &run,
        "",
        "case.sh: line 1: unexpected argument `newline' to conditional binary operator\n\
         case.sh: line 1: syntax error near `]]'\n\
         case.sh: line 1: `[[ a == #x ]]'\n",
        2,
    );
}

#[test]
fn issue3227_rhs_hash_eol_names_hash_word() {
    // GNU t4: `]]` on the next line never becomes a token (the comment
    // ate line 1 to EOL), so the raw-line walk ends on the `#x` word.
    let run = run_file("rhs-hash-eol", "[[ a == #x\n]]\necho rc=$?\n");
    assert_run(
        "rhs-hash-eol",
        &run,
        "",
        "case.sh: line 1: unexpected argument `newline' to conditional binary operator\n\
         case.sh: line 1: syntax error near `#x'\n\
         case.sh: line 1: `[[ a == #x'\n",
        2,
    );
}

#[test]
fn issue3227_unary_rhs_hash_names_close_bracket() {
    // GNU t12: same shape through the unary-operator slot.
    let run = run_file("unary-rhs-hash", "[[ -n #x ]]\necho rc=$?\n");
    assert_run(
        "unary-rhs-hash",
        &run,
        "",
        "case.sh: line 1: unexpected argument `newline' to conditional unary operator\n\
         case.sh: line 1: syntax error near `]]'\n\
         case.sh: line 1: `[[ -n #x ]]'\n",
        2,
    );
}

#[test]
fn issue3227_rhs_hash_spaced_names_close_bracket() {
    // GNU t2: a bare `#` still comments out `]]`, same near token.
    let run = run_file("rhs-hash-spaced", "[[ a == # ]]\necho rc=$?\n");
    assert_run(
        "rhs-hash-spaced",
        &run,
        "",
        "case.sh: line 1: unexpected argument `newline' to conditional binary operator\n\
         case.sh: line 1: syntax error near `]]'\n\
         case.sh: line 1: `[[ a == # ]]'\n",
        2,
    );
}

#[test]
fn issue3227_rhs_hash_trailing_space_names_close_bracket() {
    // GNU t16: trailing blanks after `]]` are backed over first, so
    // the near token is still `]]` and the echo keeps the blanks.
    let run = run_file("rhs-hash-trail", "[[ a == #x ]]   \necho rc=$?\n");
    assert_run(
        "rhs-hash-trail",
        &run,
        "",
        "case.sh: line 1: unexpected argument `newline' to conditional binary operator\n\
         case.sh: line 1: syntax error near `]]'\n\
         case.sh: line 1: `[[ a == #x ]]   '\n",
        2,
    );
}

#[test]
fn issue3227_plain_newline_rhs_still_names_operator() {
    // GNU t8 (no-change guard): with no comment-swallowed text the
    // raw-line walk lands on the same `==` the args walk names.
    let run = run_file("plain-newline", "[[ a ==\necho hi\n");
    assert_run(
        "plain-newline",
        &run,
        "",
        "case.sh: line 1: unexpected argument `newline' to conditional binary operator\n\
         case.sh: line 1: syntax error near `=='\n\
         case.sh: line 1: `[[ a =='\n",
        2,
    );
}

#[test]
fn issue3227_newline_before_closer_still_names_operator() {
    // GNU t10 (no-change guard): the newline before a real `]]` on
    // the next line still names `==`.
    let run = run_file("newline-closer", "[[ a ==\n]]\necho rc=$?\n");
    assert_run(
        "newline-closer",
        &run,
        "",
        "case.sh: line 1: unexpected argument `newline' to conditional binary operator\n\
         case.sh: line 1: syntax error near `=='\n\
         case.sh: line 1: `[[ a =='\n",
        2,
    );
}

#[test]
fn issue3227_c_rhs_hash_names_close_bracket() {
    // GNU -c form: no trailing newline in the input string, the
    // virtual end-of-input newline still walks the raw line to `]]`.
    let run = run_c("[[ a == #x ]]");
    assert_run(
        "c-rhs-hash",
        &run,
        "",
        "bash: -c: line 1: unexpected argument `newline' to conditional binary operator\n\
         bash: -c: line 1: syntax error near `]]'\n\
         bash: -c: line 1: `[[ a == #x ]]'\n",
        2,
    );
}

#[test]
fn issue3227_c_rhs_hash_eol_names_hash_word() {
    let run = run_c("[[ a == #x");
    assert_run(
        "c-rhs-hash-eol",
        &run,
        "",
        "bash: -c: line 1: unexpected argument `newline' to conditional binary operator\n\
         bash: -c: line 1: syntax error near `#x'\n\
         bash: -c: line 1: `[[ a == #x'\n",
        2,
    );
}

#[test]
fn issue3227_c_plain_rhs_still_names_operator() {
    // GNU -c control: no comment, the near stays `==`.
    let run = run_c("[[ a ==");
    assert_run(
        "c-plain",
        &run,
        "",
        "bash: -c: line 1: unexpected argument `newline' to conditional binary operator\n\
         bash: -c: line 1: syntax error near `=='\n\
         bash: -c: line 1: `[[ a =='\n",
        2,
    );
}
