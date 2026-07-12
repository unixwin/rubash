//! `set` and `unset` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/set.def (`set_builtin`, `unset_builtin`)

mod options;
mod unset;

pub(crate) use options::{
    is_shell_option, print_shell_option, print_shell_options, print_shell_options_by_state,
    set_shell_option, shell_option_enabled, shellopts_value,
};
pub use unset::unset;
pub(crate) use unset::unset_with_stderr;

use std::collections::HashMap;
use std::io::{self, Write};

pub(super) const EXECUTION_SUCCESS: i32 = 0;
pub(super) const EXECUTION_FAILURE: i32 = 1;
pub(super) const EX_USAGE: i32 = 2;

const SET_FLAGS: &str = "abefhkmnptuvxBCEHPT";
pub(super) const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
pub(super) const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
pub(super) const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
pub(super) const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
pub(super) const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
pub(super) const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
pub(super) const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
pub(super) const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
pub(super) const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";

/// Execute `set` with arguments after the command name.
pub fn set(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    set_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn set_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
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
                        set_shell_option(env_vars, name, prefix == '-');
                        index += 1;
                    }
                    _ => print_shell_options(env_vars, prefix == '+', stdout)?,
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
        let mut env_vars = env_vars.clone();
        let status = set_with_io(
            args.iter().copied(),
            &mut env_vars,
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
