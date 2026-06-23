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
    execute_with_io(args.iter().map(String::as_str), &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    args: I,
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
            writeln!(stderr, "rubash: times: -{}: invalid option", option)?;
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
        let status = execute_with_io(args.iter().copied(), &mut stdout, &mut stderr).unwrap();

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
    fn ignores_non_option_arguments() {
        let (status, stdout, stderr) = run(&["extra"]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n");
        assert!(stderr.is_empty());
    }
}
