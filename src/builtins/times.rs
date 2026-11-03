//! `times` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/times.def (`times_builtin`)

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EX_USAGE: i32 = 2;

/// Execute `times` with arguments after the command name.
pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        args.iter().map(String::as_str),
        &diagnostic_prefix(),
        &mut stdout,
        &mut stderr,
    )
}

/// The fallback diagnostic prefix for the stdio entry point, following the
/// same pattern as builtins/alias.rs: the executor-dispatched path passes
/// `Executor::diagnostic_prefix()` explicitly instead.
fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    args: I,
    diagnostic_prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    if let Some(arg) = args.into_iter().next() {
        if arg != "--" && arg.starts_with('-') {
            let option = arg.chars().nth(1).unwrap_or('-');
            // GNU builtins/common.c builtin_error: the diagnostic carries
            // the shell's diagnostic prefix (`bash: line 1: times: ...`),
            // never a hardcoded program name (deep-audit B6-F2).
            writeln!(
                stderr,
                "{diagnostic_prefix}times: -{}: invalid option",
                option
            )?;
            writeln!(stderr, "times: usage: times")?;
            return Ok(EX_USAGE);
        }
    }

    writeln!(stdout, "0m0.000s 0m0.000s")?;
    writeln!(stdout, "0m0.000s 0m0.000s")?;
    Ok(EXECUTION_SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (i32, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = execute_with_io(
            args.iter().copied(),
            "bash: line 1: ",
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn prints_two_time_lines() {
        assert_eq!(
            run(&[]),
            (
                EXECUTION_SUCCESS,
                "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n".to_string(),
                String::new()
            )
        );
    }

    #[test]
    fn rejects_options() {
        let (status, stdout, stderr) = run(&["-x"]);

        assert_eq!(status, EX_USAGE);
        assert!(stdout.is_empty());
        assert!(stderr.contains("invalid option"));
    }

    #[test]
    fn b6_f2_invalid_option_uses_callers_diagnostic_prefix() {
        // Deep-audit B6-F2: GNU builtins/common.c builtin_error carries the
        // shell diagnostic prefix (`bash: line 1: times: -z: invalid
        // option`); the builtin must not hardcode a program name.
        let (status, _, stderr) = run(&["-z"]);

        assert_eq!(status, EX_USAGE);
        assert_eq!(
            stderr,
            "bash: line 1: times: -z: invalid option\ntimes: usage: times\n"
        );
    }

    #[test]
    fn ignores_non_option_arguments() {
        let (status, stdout, stderr) = run(&["extra"]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n");
        assert!(stderr.is_empty());
    }
}
