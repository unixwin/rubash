//! rubash#3031 — the token named by `syntax error near unexpected
//! token ...` must be the token GNU names: for a case clause the
//! `;|` terminator splits and the offending token is `|`
//! (t0382); for `[[ a == (#i)bar ]]` the conditional parser
//! first reports the `(` as an unexpected binary-operator
//! argument, then names `(#` via error_token_from_text
//! (parse.y:6772) — not the bare `(`. Fixtures are the eco-2906
//! mvdan/sh snippets t0382/t0416. Expectations measured against
//! GNU bash 5.3.20 (2026-11-08).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf = std::env::temp_dir().join(format!("rubashP1-{tag}-{}", std::process::id()));
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
fn issue3031_case_fallthrough_terminator_names_pipe() {
    // t0382
    let run = run_file("t0382", "case i in 1) a ;| 2) b ;; esac\n");
    assert_run(
        "t0382",
        &run,
        "",
        "case.sh: line 1: syntax error near unexpected token `|'\n\
         case.sh: line 1: `case i in 1) a ;| 2) b ;; esac'\n",
        2,
    );
}

#[test]
fn issue3031_conditional_extglob_arg_names_paren_hash() {
    // t0416
    let run = run_file("t0416", "[[ a == (#i)bar ]]\n");
    assert_run(
        "t0416",
        &run,
        "",
        "case.sh: line 1: unexpected argument `(' to conditional binary operator\n\
         case.sh: line 1: syntax error near `(#'\n\
         case.sh: line 1: `[[ a == (#i)bar ]]'\n",
        2,
    );
}

#[test]
fn issue3031_bare_close_paren_control() {
    let run = run_c("echo )");
    assert_run(
        "ctrl-rparen",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `)'\n\
         bash: -c: line 1: `echo )'\n",
        2,
    );
}

#[test]
fn issue3031_wellformed_case_control() {
    let run = run_c("case i in 1) a ;; esac");
    assert_run("ctrl-case", &run, "", "", 0);
}
