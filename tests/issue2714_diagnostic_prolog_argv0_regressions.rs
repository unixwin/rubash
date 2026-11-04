//! rubash#2714 (owner decision D1, 2026-11-04) — the diagnostic
//! prolog follows argv[0], not the hardcoded literal `bash`.
//!
//! GNU error.c:88-120 get_name_for_error: the prolog name is
//! BASH_SOURCE[0] when the innermost frame names one, else
//! dollar_vars[0] ($0) VERBATIM, else base_pathname(shell_name),
//! else "bash". A shell invoked as `./sh` therefore reports
//! `./sh: line 1: ...`, and only a shell whose $0 is `bash`
//! reports `bash:`. Rubash used to seed the `-c` reader name with
//! the literal "bash" (main.rs) and hardcode `bash` in every
//! no-script arm of the executor's prefix family
//! (public_accessors.rs), so a renamed binary — and every host
//! embedding the engine — misreported itself.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-04),
//! invoked under the same argv[0] spellings via CommandExt::arg0.

#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_arg0(arg0: &str, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg0(arg0)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_c(arg0: &str, script: &str) -> Run {
    run_arg0(arg0, &["-c", script])
}

/// Runtime-error prolog (error.c error_prolog): a failglob failure
/// under argv[0] `./sh` reports `./sh: line 1:`, exit 1.
#[test]
fn issue2714_failglob_prolog_follows_argv0() {
    let run = run_c("./sh", "shopt -s failglob; echo zzz*");
    assert_eq!(run.stderr, "./sh: line 1: no match: zzz*\n");
    assert_eq!(run.stdout, "");
    assert_eq!(run.code, Some(1));
}

/// Parameter-error prolog (also error_prolog): `${v:?}` under
/// argv[0] `./sh` reports `./sh: line 1:`; the `-c` string driver
/// exits 127 (error.c expansion-fatal arm).
#[test]
fn issue2714_param_error_prolog_follows_argv0() {
    let run = run_c("./sh", "echo ${v:?custom msg}");
    assert_eq!(run.stderr, "./sh: line 1: v: custom msg\n");
    assert_eq!(run.code, Some(127));
}

/// Command-not-found prolog: the full argv[0] spelling is used
/// verbatim, path included (GNU does not basename a non-interactive
/// $0).
#[test]
fn issue2714_command_not_found_prolog_uses_argv0_verbatim() {
    let run = run_c("/some/dir/sh", "nosuchcmd");
    assert_eq!(
        run.stderr,
        "/some/dir/sh: line 1: nosuchcmd: command not found\n"
    );
    assert_eq!(run.code, Some(127));
}

/// Parser-error prolog (error.c:300-316 parser_error) appends the
/// `-c` stream name after the SAME argv[0]-derived name, and echoes
/// the offending line under it; exit 2.
#[test]
fn issue2714_parser_error_prolog_follows_argv0() {
    let run = run_c("./sh", "if then; fi");
    assert_eq!(
        run.stderr,
        "./sh: -c: line 1: syntax error near unexpected token `then'\n\
         ./sh: -c: line 1: `if then; fi'\n"
    );
    assert_eq!(run.code, Some(2));
}

/// An explicit $0 argument to `-c` wins over argv[0] (GNU: the
/// first word after the string binds dollar_vars[0]).
#[test]
fn issue2714_explicit_dollar_zero_still_wins() {
    let run = run_arg0("./sh", &["-c", "echo ${v:?x}", "myname"]);
    assert_eq!(run.stderr, "myname: line 1: v: x\n");
    assert_eq!(run.code, Some(127));
}

/// The DEFAULT invocation (no arg0 override: argv[0] is the test
/// binary's own path, exactly as cargo spawns it) reports that
/// path verbatim — the headline D1 face. GNU invoked by absolute
/// path does the same.
#[test]
fn issue2714_default_invocation_reports_its_own_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "echo ${v:?x}"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        stderr,
        format!("{}: line 1: v: x\n", env!("CARGO_BIN_EXE_rubash"))
    );
    assert_eq!(output.status.code(), Some(127));
}

/// The script-file route is unchanged: the prolog is the script's
/// own name (BASH_SOURCE[0] outranks $0 in get_name_for_error), and
/// a file-route expansion failure exits 1.
#[test]
fn issue2714_script_file_prolog_stays_the_script_name() {
    let dir = std::env::temp_dir().join(format!("rubash2714-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("s1.sh"), "echo ${v:?boom}\n").expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg0("./sh")
        .current_dir(&dir)
        .arg("s1.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stderr, "s1.sh: line 1: v: boom\n");
    assert_eq!(output.status.code(), Some(1));
}
