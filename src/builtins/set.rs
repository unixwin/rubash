//! `set` and `unset` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/set.def (`set_builtin`, `unset_builtin`)

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

const SET_FLAGS: &str = "abefhkmnptuvxBCEHPT";

/// Execute `set` with arguments after the command name.
pub fn set(args: &[String], env_vars: &HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    set_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

fn set_with_io<'a, I, W, E>(
    args: I,
    env_vars: &HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();

    if args.is_empty() {
        print_shell_variables(env_vars, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if *arg == "--" || *arg == "-" {
            return Ok(EXECUTION_SUCCESS);
        }

        let Some(prefix) = arg.chars().next().filter(|ch| *ch == '-' || *ch == '+') else {
            return Ok(EXECUTION_SUCCESS);
        };

        let options = &arg[1..];
        if options.is_empty() {
            return Ok(EXECUTION_SUCCESS);
        }

        let mut chars = options.chars().peekable();
        while let Some(option) = chars.next() {
            if option == 'o' {
                if chars.peek().is_some() {
                    writeln!(stderr, "rubash: set: {}: invalid option", arg)?;
                    writeln!(
                        stderr,
                        "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [arg ...]"
                    )?;
                    return Ok(EX_USAGE);
                }

                match args.get(index + 1) {
                    Some(name)
                        if !name.is_empty() && !name.starts_with('-') && !name.starts_with('+') =>
                    {
                        if !is_shell_option(name) {
                            writeln!(stderr, "rubash: set: {}: invalid option name", name)?;
                            return Ok(EXECUTION_FAILURE);
                        }
                        index += 1;
                    }
                    _ => print_shell_options(prefix == '+', stdout)?,
                }
                break;
            }

            if !SET_FLAGS.contains(option) {
                writeln!(stderr, "rubash: set: {}{}: invalid option", prefix, option)?;
                writeln!(
                    stderr,
                    "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [arg ...]"
                )?;
                return Ok(EXECUTION_FAILURE);
            }
        }

        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

fn print_shell_variables<W>(env_vars: &HashMap<String, String>, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let mut vars: Vec<(&String, &String)> = env_vars.iter().collect();
    vars.sort_by(|left, right| left.0.cmp(right.0));

    for (name, value) in vars {
        writeln!(stdout, "{}={}", name, shell_quote(value))?;
    }

    Ok(())
}

fn print_shell_options<W>(recreate: bool, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    for option in SHELL_OPTIONS {
        if recreate {
            writeln!(stdout, "set +o {}", option)?;
        } else {
            writeln!(stdout, "{:<16}\toff", option)?;
        }
    }

    Ok(())
}

const SHELL_OPTIONS: &[&str] = &[
    "allexport",
    "braceexpand",
    "errexit",
    "errtrace",
    "functrace",
    "hashall",
    "ignoreeof",
    "interactive-comments",
    "keyword",
    "noclobber",
    "noexec",
    "noglob",
    "nolog",
    "nounset",
    "onecmd",
    "physical",
    "pipefail",
    "posix",
    "privileged",
    "verbose",
    "xtrace",
];

fn is_shell_option(name: &str) -> bool {
    SHELL_OPTIONS.contains(&name)
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '/' | '.' | '-' | ':'))
    {
        value.to_string()
    } else {
        let escaped = value.replace('\'', "'\\''");
        format!("'{}'", escaped)
    }
}

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

    fn run_set(args: &[&str], env_vars: &HashMap<String, String>) -> (i32, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = set_with_io(args.iter().copied(), env_vars, &mut stdout, &mut stderr).unwrap();
        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn set_without_arguments_prints_variables() {
        let env_vars = HashMap::from([("NAME".to_string(), "value".to_string())]);
        let (status, stdout, stderr) = run_set(&[], &env_vars);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "NAME=value\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn set_rejects_unknown_flag() {
        let env_vars = HashMap::new();
        let (status, _stdout, stderr) = run_set(&["-Z"], &env_vars);

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stderr.contains("invalid option"));
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
