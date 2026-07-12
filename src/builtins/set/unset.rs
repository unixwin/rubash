use super::{
    ARRAY_VARS, ASSOC_VARS, DECLARED_UNSET_VARS, EXECUTION_FAILURE, EXECUTION_SUCCESS,
    EXPORTED_VARS, EX_USAGE, INTEGER_VARS, LOWERCASE_VARS, NAMEREF_VARS, READONLY_VARS,
    UPPERCASE_VARS,
};
use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

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
    unmark_variable(env_vars, DECLARED_UNSET_VARS, &unset_name);
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
