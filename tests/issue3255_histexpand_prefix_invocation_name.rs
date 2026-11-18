//! rubash#3255 (G2 of the #3193 census) — two argv[0]-derivation
//! residuals of the diagnostic-name funnel:
//!
//! 1. `ShellInvocation::apply_to_executor` seeds `__RUBASH_SCRIPT_NAME`
//!    with the literal `"bash"` when a host drives `-c` without an
//!    explicit `$0` operand. main.rs's twin site derives the seed from
//!    BASH_ARGV0 since #2714 (main.rs:949-963, with the #3355 empty-$0
//!    rule: a present-but-empty $0 leaves the carrier unset, only a
//!    wholly absent carrier falls back to PROGRAM `"bash"`). A host
//!    (niu-class) driving `-c` under a renamed argv[0] therefore gets
//!    its diagnostics named `"bash"`.
//! 2. `script_driver::run_history_group` reports a failed history
//!    expansion with a hand-rolled `(script, line)` prefix that falls
//!    back to the literal `"bash: "` when the script name is absent —
//!    the interactive read-line shape (the line is stamped by
//!    `set_current_line_value`, the script name is never seeded on
//!    that route). GNU reports the failure through `internal_error`
//!    (bashhist.c:594) -> `error_prolog` (error.c:75-86) ->
//!    `get_name_for_error` (error.c:90-120): an interactive shell
//!    prints the shell name alone, derived from argv[0], with no line
//!    segment and no `eval`/`-c` segment (those belong to parser_error
//!    only). The executor's `error_prefix()` family is that chain.
//!
//! GNU oracle (bash 5.3.20, setsid, piped stdin, this lane):
//!   `exec -a myname bash -i` + `!nosuch` -> `myname: !nosuch: event not found`
//!   plain argv0                         -> `bash: !nosuch: event not found`
//! The renamed form is the fail-first cell: on base the interactive
//! failure prints the literal `bash:` prefix under any argv[0].
//!
//! Fail-first account (base 27e1d0d2, expected): the two
//! `invocation_c_without_explicit_name_*` derivation cells and the
//! renamed interactive cell are RED on base; the no-carrier PROGRAM
//! fallback cell and the explicit-operand guard cell are GREEN on
//! both sides (they lock the GNU chain ends).

fn parse_c_no_name() -> rubash::invocation::ShellInvocation {
    let args: Vec<String> = ["-c", "true"].iter().map(|s| s.to_string()).collect();
    rubash::invocation::ShellInvocation::parse(&args).expect("parse -c")
}

fn apply_with_bash_argv0(seeded: Option<&str>) -> rubash::executor::Executor {
    // Mirror the #591 harness: clear any ambient BASH_ARGV0 import so
    // the executor's carriers come only from this test's seeding.
    std::env::remove_var("BASH_ARGV0");
    let mut executor = rubash::executor::Executor::new_process_exit();
    if let Some(value) = seeded {
        executor.set_env("BASH_ARGV0", value);
    } else {
        executor.remove_env("BASH_ARGV0");
    }
    parse_c_no_name()
        .apply_to_executor(&mut executor)
        .expect("apply -c invocation");
    executor
}

/// #2714 twin: a host that pre-seeds BASH_ARGV0 with its own (renamed)
/// name must see that name as the `-c` script name, not `"bash"`.
#[test]
fn invocation_c_without_explicit_name_derives_script_name_from_bash_argv0() {
    let executor = apply_with_bash_argv0(Some("myname"));
    assert_eq!(
        executor.get_env("__RUBASH_SCRIPT_NAME"),
        Some("myname"),
        "host-seeded BASH_ARGV0 must seed the -c script name verbatim"
    );
}

/// #3355 rule, Invocation twin: a present-but-empty $0 is the verdict
/// (error.c:90-120 stops at dollar_vars[0] itself) — the carrier must
/// stay unset so the $0 arm owns the prolog, never `"bash"`.
#[test]
fn invocation_c_without_explicit_name_empty_bash_argv0_leaves_script_name_unset() {
    let executor = apply_with_bash_argv0(Some(""));
    assert_eq!(
        executor.get_env("__RUBASH_SCRIPT_NAME"),
        None,
        "empty BASH_ARGV0 must not plant a script name"
    );
}

/// Only a wholly absent carrier falls back to PROGRAM (error.c:110-120).
#[test]
fn invocation_c_without_explicit_name_no_carrier_falls_back_to_program() {
    let executor = apply_with_bash_argv0(None);
    assert_eq!(
        executor.get_env("__RUBASH_SCRIPT_NAME"),
        Some("bash"),
        "no BASH_ARGV0 at all keeps the PROGRAM fallback"
    );
}

/// Guard: an explicit `-c` name operand still wins over BASH_ARGV0
/// (shell.c:523-528; the operand becomes dollar_vars[0]).
#[test]
fn invocation_c_with_explicit_name_keeps_operand() {
    std::env::remove_var("BASH_ARGV0");
    let mut executor = rubash::executor::Executor::new_process_exit();
    executor.set_env("BASH_ARGV0", "hostname");
    let args: Vec<String> = ["-c", "true", "customzero"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let invocation = rubash::invocation::ShellInvocation::parse(&args).expect("parse -c with name");
    invocation
        .apply_to_executor(&mut executor)
        .expect("apply -c invocation");
    assert_eq!(
        executor.get_env("__RUBASH_SCRIPT_NAME"),
        Some("customzero"),
        "explicit -c name operand must win over BASH_ARGV0"
    );
}

#[cfg(unix)]
mod interactive {
    use std::io::Write as _;
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};

    fn scratch_home(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rubash-issue3255-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("scratch home");
        dir
    }

    /// An interactive session fed `input` on a pipe, optionally under a
    /// renamed argv[0]. Same setsid/no-ctty + cleared-env + scratch-HOME
    /// shape as issue2733; the job-control warnings and the prompt share
    /// stderr, so callers assert on the diagnostic line itself.
    fn interactive_session(arg0: Option<&str>, input: &str) -> (String, String) {
        let home = scratch_home(arg0.unwrap_or("plain"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
        if let Some(name) = arg0 {
            cmd.arg0(name);
        }
        cmd.arg("-i")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("TERM", "xterm")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn().expect("spawn rubash -i");
        child
            .stdin
            .take()
            .expect("stdin pipe")
            .write_all(input.as_bytes())
            .expect("feed interactive input");
        let output = child.wait_with_output().expect("interactive session ends");
        let _ = std::fs::remove_dir_all(&home);
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// A literal `bash:` diagnostic prefix must never appear. Match on
    /// line boundaries: `bash:` is a substring of `rubash:`, so a plain
    /// `stderr.contains("bash: ...")` fires on the correct output too
    /// (rubash#3800).
    fn assert_no_literal_bash_prefix(stderr: &str) {
        assert!(
            !stderr.lines().any(|line| line.starts_with("bash: ")),
            "literal bash: prefix must not appear, got stderr: {stderr:?}"
        );
    }

    const FAILED_INPUT: &str = "echo FIRST\n!nosuchcmd123\necho AFTER\n";

    /// GNU (5.3.20, `exec -a myname bash -i`): the failed expansion
    /// reports under the invocation name —
    /// `myname: !nosuchcmd123: event not found` — and the session
    /// continues with the following line.
    #[test]
    fn interactive_histexpand_failure_prefix_follows_argv0() {
        let (stdout, stderr) = interactive_session(Some("zimsh"), FAILED_INPUT);
        assert!(
            stderr.contains("zimsh: !nosuchcmd123: event not found"),
            "renamed interactive failure must carry argv[0], got stderr: {stderr:?}"
        );
        assert_no_literal_bash_prefix(&stderr);
        assert!(
            stdout.contains("FIRST") && stdout.contains("AFTER"),
            "failed line is discarded but the session continues, got stdout: {stdout:?}"
        );
    }

    /// Control: under the plain test-binary argv[0] the failure still
    /// reports (under the binary's basename, never the literal `bash:`).
    #[test]
    fn interactive_histexpand_failure_plain_argv0_never_prints_literal_bash() {
        let (stdout, stderr) = interactive_session(None, FAILED_INPUT);
        assert!(
            stderr.contains("!nosuchcmd123: event not found"),
            "failure diagnostic must be reported, got stderr: {stderr:?}"
        );
        assert_no_literal_bash_prefix(&stderr);
        // The diagnostic line's prefix must be the binary's own
        // basename (GNU error.c:90-120 `get_name_for_error`: an
        // interactive shell prints the shell name alone, from argv[0]).
        let basename = std::path::Path::new(env!("CARGO_BIN_EXE_rubash"))
            .file_stem()
            .expect("test binary has a file stem")
            .to_string_lossy()
            .into_owned();
        assert!(
            stderr.lines().any(
                |line| line.starts_with(&format!("{basename}: !nosuchcmd123: event not found"))
            ),
            "failure diagnostic must carry the binary basename, got stderr: {stderr:?}"
        );
        assert!(
            stdout.contains("FIRST") && stdout.contains("AFTER"),
            "failed line is discarded but the session continues, got stdout: {stdout:?}"
        );
    }
}
