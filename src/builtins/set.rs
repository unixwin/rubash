//! `set` and `unset` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/set.def (`unset_builtin`)

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct UnsetOptions {
    functions: bool,
    variables: bool,
    nameref: bool,
}

/// Execute `unset` with arguments after the command name.
pub fn unset(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stderr = io::stderr().lock();
    unset_with_stderr(args.iter().map(String::as_str), env_vars, &mut stderr)
}

fn unset_with_stderr<'a, I, W>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let (options, first_name) = match parse_unset_options(&args, stderr)? {
        Ok(parsed) => parsed,
        Err(status) => return Ok(status),
    };

    if options.functions && options.variables {
        writeln!(
            stderr,
            "rubash: unset: cannot simultaneously unset a function and a variable"
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    let mut status = EXECUTION_SUCCESS;
    for name in &args[first_name..] {
        if unset_name(name, options, env_vars, stderr)? != EXECUTION_SUCCESS {
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn parse_unset_options<W>(
    args: &[&str],
    stderr: &mut W,
) -> io::Result<Result<(UnsetOptions, usize), i32>>
where
    W: Write,
{
    let mut options = UnsetOptions::default();
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            return Ok(Ok((options, index + 1)));
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'f' => options.functions = true,
                'v' => options.variables = true,
                'n' => options.nameref = true,
                other => {
                    writeln!(stderr, "rubash: unset: -{}: invalid option", other)?;
                    writeln!(stderr, "unset: usage: unset [-f] [-v] [-n] [name ...]")?;
                    return Ok(Err(EX_USAGE));
                }
            }
        }

        index += 1;
    }

    Ok(Ok((options, index)))
}

fn unset_name<W>(
    name: &str,
    options: UnsetOptions,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    if options.functions {
        return Ok(EXECUTION_SUCCESS);
    }

    if !valid_identifier(name) {
        writeln!(stderr, "rubash: unset: `{}`: not a valid identifier", name)?;
        return Ok(EXECUTION_FAILURE);
    }

    env_vars.remove(name);
    env::remove_var(name);
    Ok(EXECUTION_SUCCESS)
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], env_vars: &mut HashMap<String, String>) -> (i32, String) {
        let mut stderr = Vec::new();
        let status = unset_with_stderr(args.iter().copied(), env_vars, &mut stderr).unwrap();
        (status, String::from_utf8(stderr).unwrap())
    }

    #[test]
    fn unsets_variable() {
        let mut env_vars = HashMap::from([("NAME".to_string(), "value".to_string())]);

        assert_eq!(run(&["NAME"], &mut env_vars).0, EXECUTION_SUCCESS);
        assert!(!env_vars.contains_key("NAME"));
    }

    #[test]
    fn rejects_invalid_identifier_for_variable_unset() {
        let mut env_vars = HashMap::new();
        let (status, stderr) = run(&["1BAD"], &mut env_vars);

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stderr.contains("not a valid identifier"));
    }

    #[test]
    fn rejects_function_and_variable_modes_together() {
        let mut env_vars = HashMap::new();
        let (status, stderr) = run(&["-fv", "NAME"], &mut env_vars);

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stderr.contains("cannot simultaneously"));
    }
}
