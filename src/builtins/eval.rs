//! `eval` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/eval.def (`eval_builtin`)

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
pub(crate) const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalAction {
    Complete(i32),
    Execute(String),
}

/// Execute `eval` option processing with arguments after the command name.
pub fn execute(args: &[String], prefix: &str) -> io::Result<EvalAction> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        args.iter().map(String::as_str),
        prefix,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    args: I,
    prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<EvalAction>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut index = 0;

    if let Some(arg) = args.get(index) {
        // GNU eval.def's option table maps --help to CASE_HELPOPT: long
        // help to stdout, EX_USAGE status (rubash#1229). Only as the
        // first word — eval has no other options, and after `--` the
        // word is data to evaluate.
        if *arg == "--help" {
            crate::builtins::help::print_builtin_help("eval", stdout)?;
            return Ok(EvalAction::Complete(EX_USAGE));
        }
        if *arg == "--" {
            index += 1;
        } else if arg.starts_with('-') && *arg != "-" {
            let option = arg.chars().nth(1).unwrap_or('-');
            writeln!(stderr, "{prefix}eval: -{}: invalid option", option)?;
            writeln!(stderr, "eval: usage: eval [arg ...]")?;
            return Ok(EvalAction::Complete(EX_USAGE));
        }
    }

    if index >= args.len() {
        return Ok(EvalAction::Complete(EXECUTION_SUCCESS));
    }

    Ok(EvalAction::Execute(args[index..].join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (EvalAction, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action =
            execute_with_io(args.iter().copied(), "rubash: ", &mut stdout, &mut stderr).unwrap();

        (action, String::from_utf8(stderr).unwrap())
    }

    #[test]
    fn empty_eval_succeeds() {
        assert_eq!(
            run(&[]),
            (EvalAction::Complete(EXECUTION_SUCCESS), String::new())
        );
    }

    #[test]
    fn skips_double_dash() {
        assert_eq!(
            run(&["--", "echo", "hi"]),
            (EvalAction::Execute("echo hi".to_string()), String::new())
        );
    }

    #[test]
    fn rejects_options() {
        let (action, stderr) = run(&["-x"]);

        assert_eq!(action, EvalAction::Complete(EX_USAGE));
        assert!(stderr.contains("invalid option"));
    }
}
