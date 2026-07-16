//! `test` and `[` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/test.def (`test_builtin`)
//! - test.c
//! - test.h

#[cfg(test)]
#[path = "test_tests.rs"]
mod tests;
mod variable;

pub(crate) use variable::variable_is_set;

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_BADUSAGE: i32 = 2;
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";

/// Execute `test` or `[` with arguments after the command name.
pub fn execute(
    args: &[String],
    bracket: bool,
    env_vars: &HashMap<String, String>,
) -> io::Result<i32> {
    let mut stderr = io::stderr().lock();
    execute_with_stderr(
        args.iter().map(String::as_str),
        bracket,
        env_vars,
        &mut stderr,
    )
}

fn execute_with_stderr<'a, I, W>(
    args: I,
    bracket: bool,
    env_vars: &HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let mut args: Vec<&str> = args.into_iter().collect();

    if bracket {
        match args.last() {
            Some(&"]") => {
                args.pop();
            }
            _ => {
                writeln!(stderr, "rubash: [: missing `]'")?;
                return Ok(EX_BADUSAGE);
            }
        }
    }

    if args.is_empty() {
        return Ok(EXECUTION_FAILURE);
    }

    match eval_expr(&args, env_vars) {
        Ok(true) => Ok(EXECUTION_SUCCESS),
        Ok(false) => Ok(EXECUTION_FAILURE),
        Err(message) => {
            writeln!(stderr, "rubash: test: {}", message)?;
            Ok(EX_BADUSAGE)
        }
    }
}

fn eval_expr(args: &[&str], env_vars: &HashMap<String, String>) -> Result<bool, String> {
    if let Some(inner) = outer_parenthesized_expr(args) {
        return eval_expr(inner, env_vars);
    }

    if let Some(index) = find_logical_operator(args, "-o") {
        return Ok(eval_expr(&args[..index], env_vars)? || eval_expr(&args[index + 1..], env_vars)?);
    }

    if let Some(index) = find_logical_operator(args, "-a") {
        return Ok(eval_expr(&args[..index], env_vars)? && eval_expr(&args[index + 1..], env_vars)?);
    }

    match args {
        [] => Ok(false),
        ["!", rest @ ..] => Ok(!eval_expr(rest, env_vars)?),
        [single] => Ok(!single.is_empty()),
        [op, operand] if is_unary_operator(op) => eval_unary(op, operand, env_vars),
        [left, op, right] if is_binary_operator(op) => eval_binary(left, op, right, env_vars),
        _ => Err("syntax error".to_string()),
    }
}

fn find_logical_operator(args: &[&str], op: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate().rev() {
        if is_close_paren(arg) {
            depth += 1;
            continue;
        }
        if is_open_paren(arg) {
            depth = depth.saturating_sub(1);
            continue;
        }
        if depth == 0 && *arg == op && index > 0 && index + 1 < args.len() {
            return Some(index);
        }
    }
    None
}

fn outer_parenthesized_expr<'a>(args: &'a [&str]) -> Option<&'a [&'a str]> {
    if args.len() < 2 || !is_open_paren(args[0]) || !is_close_paren(args[args.len() - 1]) {
        return None;
    }

    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate() {
        if is_open_paren(arg) {
            depth += 1;
        } else if is_close_paren(arg) {
            depth = depth.checked_sub(1)?;
            if depth == 0 && index != args.len() - 1 {
                return None;
            }
        }
    }

    (depth == 0).then_some(&args[1..args.len() - 1])
}

fn is_open_paren(value: &str) -> bool {
    matches!(value, "(" | "\\(")
}

fn is_close_paren(value: &str) -> bool {
    matches!(value, ")" | "\\)")
}

fn is_unary_operator(op: &str) -> bool {
    matches!(
        op,
        "-a" | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-h"
            | "-L"
            | "-k"
            | "-p"
            | "-r"
            | "-s"
            | "-S"
            | "-t"
            | "-u"
            | "-w"
            | "-x"
            | "-z"
            | "-n"
            | "-o"
            | "-v"
            | "-R"
            | "-O"
            | "-G"
            | "-N"
    )
}

fn eval_unary(op: &str, operand: &str, env_vars: &HashMap<String, String>) -> Result<bool, String> {
    match op {
        "-z" => Ok(operand.is_empty()),
        "-n" => Ok(!operand.is_empty()),
        "-o" => Ok(crate::builtins::set::is_shell_option(operand)
            && crate::builtins::set::shell_option_enabled(env_vars, operand)),
        "-v" => Ok(variable_is_set(operand, env_vars)),
        "-R" => {
            let namerefs = marked_vars(env_vars, NAMEREF_VARS);
            let readonly = marked_vars(env_vars, READONLY_VARS);
            Ok(namerefs
                .iter()
                .chain(readonly.iter())
                .any(|name| name == operand))
        }
        "-a" | "-e" => Ok(test_path(operand, env_vars).exists()),
        "-d" => Ok(test_path(operand, env_vars).is_dir()),
        "-f" => Ok(test_path(operand, env_vars).is_file()),
        "-h" | "-L" => Ok(fs::symlink_metadata(test_path(operand, env_vars))
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)),
        "-s" => Ok(fs::metadata(test_path(operand, env_vars))
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)),
        "-r" | "-w" | "-x" => Ok(test_path(operand, env_vars).exists()),
        "-b" | "-c" | "-g" | "-k" | "-p" | "-S" | "-t" | "-u" | "-O" | "-G" | "-N" => Ok(false),
        _ => Err(format!("{}: unary operator expected", op)),
    }
}

fn test_path(operand: &str, env_vars: &HashMap<String, String>) -> std::path::PathBuf {
    crate::executor::path::shell_path_to_windows(operand, env_vars)
}

fn marked_vars(env_vars: &HashMap<String, String>, key: &str) -> Vec<String> {
    env_vars
        .get(key)
        .map(|value| {
            value
                .split('\x1f')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn is_binary_operator(op: &str) -> bool {
    matches!(
        op,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-nt"
            | "-ot"
            | "-ef"
    )
}

fn eval_binary(
    left: &str,
    op: &str,
    right: &str,
    env_vars: &HashMap<String, String>,
) -> Result<bool, String> {
    match op {
        "=" | "==" => Ok(left == right),
        "!=" => Ok(left != right),
        "<" => Ok(left < right),
        ">" => Ok(left > right),
        "-eq" => Ok(parse_int(left)? == parse_int(right)?),
        "-ne" => Ok(parse_int(left)? != parse_int(right)?),
        "-lt" => Ok(parse_int(left)? < parse_int(right)?),
        "-le" => Ok(parse_int(left)? <= parse_int(right)?),
        "-gt" => Ok(parse_int(left)? > parse_int(right)?),
        "-ge" => Ok(parse_int(left)? >= parse_int(right)?),
        "-nt" => Ok(modified(left, env_vars) > modified(right, env_vars)),
        "-ot" => Ok(modified(left, env_vars) < modified(right, env_vars)),
        "-ef" => Ok(same_file(left, right, env_vars)),
        _ => Err(format!("{}: binary operator expected", op)),
    }
}

fn parse_int(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|_| format!("{}: integer expression expected", value))
}

fn modified(path: &str, env_vars: &HashMap<String, String>) -> Option<std::time::SystemTime> {
    fs::metadata(test_path(path, env_vars))
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn same_file(left: &str, right: &str, env_vars: &HashMap<String, String>) -> bool {
    let Ok(left) = fs::canonicalize(test_path(left, env_vars)) else {
        return false;
    };
    let Ok(right) = fs::canonicalize(test_path(right, env_vars)) else {
        return false;
    };
    left == right
}
