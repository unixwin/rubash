//! `exit` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/exit.def (`exit_builtin`)

use std::io::{self, Write};

const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAction {
    Exit(i32),
    Continue(i32),
}

/// Execute `exit` with arguments after the command name.
pub fn execute(args: &[String], last_status: i32) -> io::Result<ExitAction> {
    let mut stderr = io::stderr().lock();
    execute_with_stderr(args.iter().map(String::as_str), last_status, &mut stderr)
}

fn execute_with_stderr<'a, I, W>(
    args: I,
    last_status: i32,
    stderr: &mut W,
) -> io::Result<ExitAction>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let args: Vec<&str> = args.into_iter().collect();

    if args.is_empty() {
        return Ok(ExitAction::Exit(normalize_status(last_status)));
    }

    let status = match parse_exit_status(args[0]) {
        Some(status) => status,
        None => {
            writeln!(
                stderr,
                "rubash: exit: {}: numeric argument required",
                args[0]
            )?;
            return Ok(ExitAction::Exit(EX_USAGE));
        }
    };

    if args.len() > 1 {
        writeln!(stderr, "rubash: exit: too many arguments")?;
        return Ok(ExitAction::Continue(EXECUTION_FAILURE));
    }

    Ok(ExitAction::Exit(status))
}

fn parse_exit_status(arg: &str) -> Option<i32> {
    let value = arg.parse::<i128>().ok()?;
    Some(normalize_status(value))
}

fn normalize_status<T>(status: T) -> i32
where
    T: Into<i128>,
{
    status.into().rem_euclid(256) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], last_status: i32) -> (ExitAction, String) {
        let mut stderr = Vec::new();
        let action = execute_with_stderr(args.iter().copied(), last_status, &mut stderr).unwrap();
        (action, String::from_utf8(stderr).unwrap())
    }

    #[test]
    fn exits_with_last_status_without_arguments() {
        assert_eq!(run(&[], 42).0, ExitAction::Exit(42));
    }

    #[test]
    fn normalizes_numeric_status_to_eight_bits() {
        assert_eq!(run(&["258"], 0).0, ExitAction::Exit(2));
        assert_eq!(run(&["-1"], 0).0, ExitAction::Exit(255));
    }

    #[test]
    fn invalid_numeric_argument_exits_with_usage() {
        let (action, stderr) = run(&["abc"], 0);

        assert_eq!(action, ExitAction::Exit(EX_USAGE));
        assert!(stderr.contains("numeric argument required"));
    }

    #[test]
    fn too_many_arguments_does_not_exit() {
        let (action, stderr) = run(&["1", "2"], 0);

        assert_eq!(action, ExitAction::Continue(EXECUTION_FAILURE));
        assert!(stderr.contains("too many arguments"));
    }
}
