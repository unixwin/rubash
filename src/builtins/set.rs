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
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";

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

pub(crate) fn print_shell_options<W>(
    env_vars: &HashMap<String, String>,
    recreate: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    for option in SHELL_OPTIONS.iter().map(|option| option.name) {
        let enabled = shell_option_enabled(env_vars, option);
        if recreate {
            writeln!(
                stdout,
                "set {}o {}",
                if enabled { "-" } else { "+" },
                option
            )?;
        } else {
            writeln!(
                stdout,
                "{:<15}\t{}",
                option,
                if enabled { "on" } else { "off" }
            )?;
        }
    }

    Ok(())
}

pub(crate) fn print_shell_options_by_state<W>(
    env_vars: &HashMap<String, String>,
    enabled_state: bool,
    recreate: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    for option in SHELL_OPTIONS.iter().map(|option| option.name) {
        if shell_option_enabled(env_vars, option) != enabled_state {
            continue;
        }
        if recreate {
            writeln!(
                stdout,
                "set {}o {}",
                if enabled_state { "-" } else { "+" },
                option
            )?;
        } else {
            writeln!(
                stdout,
                "{:<15}\t{}",
                option,
                if enabled_state { "on" } else { "off" }
            )?;
        }
    }
    Ok(())
}

pub(crate) fn print_shell_option<W>(
    env_vars: &HashMap<String, String>,
    name: &str,
    recreate: bool,
    stdout: &mut W,
) -> io::Result<Option<()>>
where
    W: Write,
{
    if !is_shell_option(name) {
        return Ok(None);
    }
    let enabled = shell_option_enabled(env_vars, name);
    if recreate {
        writeln!(stdout, "set {}o {}", if enabled { "-" } else { "+" }, name)?;
    } else {
        writeln!(
            stdout,
            "{:<15}\t{}",
            name,
            if enabled { "on" } else { "off" }
        )?;
    }
    Ok(Some(()))
}

#[derive(Clone, Copy)]
struct ShellOption {
    name: &'static str,
    default_enabled: bool,
}

const SHELL_OPTIONS: &[ShellOption] = &[
    ShellOption {
        name: "allexport",
        default_enabled: false,
    },
    ShellOption {
        name: "braceexpand",
        default_enabled: true,
    },
    ShellOption {
        name: "emacs",
        default_enabled: true,
    },
    ShellOption {
        name: "errexit",
        default_enabled: false,
    },
    ShellOption {
        name: "errtrace",
        default_enabled: false,
    },
    ShellOption {
        name: "functrace",
        default_enabled: false,
    },
    ShellOption {
        name: "hashall",
        default_enabled: true,
    },
    ShellOption {
        name: "histexpand",
        default_enabled: false,
    },
    ShellOption {
        name: "history",
        default_enabled: true,
    },
    ShellOption {
        name: "ignoreeof",
        default_enabled: false,
    },
    ShellOption {
        name: "interactive-comments",
        default_enabled: true,
    },
    ShellOption {
        name: "keyword",
        default_enabled: false,
    },
    ShellOption {
        name: "monitor",
        default_enabled: false,
    },
    ShellOption {
        name: "noclobber",
        default_enabled: false,
    },
    ShellOption {
        name: "noexec",
        default_enabled: false,
    },
    ShellOption {
        name: "noglob",
        default_enabled: false,
    },
    ShellOption {
        name: "nolog",
        default_enabled: false,
    },
    ShellOption {
        name: "notify",
        default_enabled: false,
    },
    ShellOption {
        name: "nounset",
        default_enabled: false,
    },
    ShellOption {
        name: "onecmd",
        default_enabled: false,
    },
    ShellOption {
        name: "physical",
        default_enabled: false,
    },
    ShellOption {
        name: "pipefail",
        default_enabled: false,
    },
    ShellOption {
        name: "posix",
        default_enabled: false,
    },
    ShellOption {
        name: "privileged",
        default_enabled: false,
    },
    ShellOption {
        name: "verbose",
        default_enabled: false,
    },
    ShellOption {
        name: "vi",
        default_enabled: false,
    },
    ShellOption {
        name: "xtrace",
        default_enabled: false,
    },
];

pub(crate) fn is_shell_option(name: &str) -> bool {
    SHELL_OPTIONS.iter().any(|option| option.name == name)
}

pub(crate) fn shell_option_enabled(env_vars: &HashMap<String, String>, name: &str) -> bool {
    let key = shell_option_key(name);
    env_vars
        .get(&key)
        .map(|value| value == "1")
        .unwrap_or_else(|| {
            SHELL_OPTIONS
                .iter()
                .find(|option| option.name == name)
                .map(|option| option.default_enabled)
                .unwrap_or(false)
        })
}

pub(crate) fn shellopts_value(env_vars: &HashMap<String, String>) -> String {
    SHELL_OPTIONS
        .iter()
        .map(|option| option.name)
        .filter(|name| shellopts_includes_option(name))
        .filter(|name| shell_option_enabled(env_vars, name))
        .collect::<Vec<_>>()
        .join(":")
}

pub(crate) fn set_shell_option(env_vars: &mut HashMap<String, String>, name: &str, enabled: bool) {
    env_vars.insert(
        shell_option_key(name),
        if enabled { "1" } else { "0" }.to_string(),
    );
    env_vars.insert("SHELLOPTS".to_string(), shellopts_value(env_vars));
}

fn shell_option_key(name: &str) -> String {
    format!("__RUBASH_SETOPT_{}", name.replace('-', "_"))
}

fn shellopts_includes_option(name: &str) -> bool {
    !matches!(name, "emacs" | "history" | "vi")
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

pub(crate) fn unset_with_stderr<'a, I, W>(
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

    let unset_name = if !options.nameref && is_marked_variable(env_vars, NAMEREF_VARS, name) {
        env_vars
            .get(name)
            .filter(|target| valid_identifier(target))
            .map(String::as_str)
            .unwrap_or(name)
    } else {
        name
    };

    if is_unsettable_bash_variable(unset_name) {
        writeln!(
            stderr,
            "{}unset: {unset_name}: cannot unset",
            diagnostic_prefix(env_vars)
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    if is_marked_variable(env_vars, READONLY_VARS, unset_name) {
        writeln!(
            stderr,
            "{}unset: {unset_name}: cannot unset: readonly variable",
            diagnostic_prefix(env_vars)
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    let unset_name = unset_name.to_string();
    env_vars.remove(&unset_name);
    env::remove_var(&unset_name);
    unmark_variable(env_vars, EXPORTED_VARS, &unset_name);
    unmark_variable(env_vars, READONLY_VARS, &unset_name);
    unmark_variable(env_vars, ARRAY_VARS, &unset_name);
    unmark_variable(env_vars, ASSOC_VARS, &unset_name);
    unmark_variable(env_vars, INTEGER_VARS, &unset_name);
    unmark_variable(env_vars, UPPERCASE_VARS, &unset_name);
    unmark_variable(env_vars, LOWERCASE_VARS, &unset_name);
    unmark_variable(env_vars, NAMEREF_VARS, &unset_name);
    Ok(EXECUTION_SUCCESS)
}

fn is_marked_variable(env_vars: &HashMap<String, String>, key: &str, name: &str) -> bool {
    env_vars
        .get(key)
        .map(|value| value.split('\x1f').any(|marked| marked == name))
        .unwrap_or(false)
}

fn unmark_variable(env_vars: &mut HashMap<String, String>, key: &str, name: &str) {
    let Some(value) = env_vars.get(key).cloned() else {
        return;
    };
    let marked = value
        .split('\x1f')
        .filter(|marked| !marked.is_empty() && *marked != name)
        .collect::<Vec<_>>()
        .join("\x1f");
    if marked.is_empty() {
        env_vars.remove(key);
    } else {
        env_vars.insert(key.to_string(), marked);
    }
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_unsettable_bash_variable(name: &str) -> bool {
    matches!(name, "BASH_LINENO" | "BASH_SOURCE")
}

fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
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
