//! `eval` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/eval.def (`eval_builtin`)

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalAction {
    Complete(i32),
    Execute(String),
}

/// Execute `eval` option processing with arguments after the command name.
pub fn execute(args: &[String]) -> io::Result<EvalAction> {
    let mut stderr = io::stderr().lock();
    execute_with_io(args.iter().map(String::as_str), &mut stderr)
}

pub(crate) fn execute_with_io<'a, I, E>(args: I, stderr: &mut E) -> io::Result<EvalAction>
where
    I: IntoIterator<Item = &'a str>,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut index = 0;

    if let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
        } else if arg.starts_with('-') && *arg != "-" {
            let option = arg.chars().nth(1).unwrap_or('-');
            writeln!(stderr, "rubash: eval: -{}: invalid option", option)?;
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
        let mut stderr = Vec::new();
        let action = execute_with_io(args.iter().copied(), &mut stderr).unwrap();

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
