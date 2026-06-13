//! `test` and `[` builtins.
//!
//! GNU Bash source ownership:
//! - builtins/test.def (`test_builtin`)
//! - test.c
//! - test.h

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_BADUSAGE: i32 = 2;
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";

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
        [left, op, right] if is_binary_operator(op) => eval_binary(left, op, right),
        _ => Err("syntax error".to_string()),
    }
}

fn find_logical_operator(args: &[&str], op: &str) -> Option<usize> {
    args.iter().rposition(|arg| *arg == op)
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
        "-v" => Ok(variable_is_set(operand, env_vars)),
        "-R" => Ok(false),
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

fn variable_is_set(operand: &str, env_vars: &HashMap<String, String>) -> bool {
    // TODO(test.c/variables.c/array.c): Bash `test -v name[subscript]`
    // consults shell variable attributes and array elements. This handles the
    // array forms used by upstream builtins5.sub.
    if let Some(name) = operand
        .strip_suffix("[@]")
        .or_else(|| operand.strip_suffix("[*]"))
    {
        let arrays = marked_vars(env_vars, ARRAY_VARS);
        let assocs = marked_vars(env_vars, ASSOC_VARS);
        if assocs.iter().any(|marked| marked == name) {
            return false;
        }
        if arrays.iter().any(|marked| marked == name) {
            return env_vars
                .get(name)
                .map(|value| value.starts_with('(') && value.ends_with(')') && value.len() > 2)
                .unwrap_or(false);
        }
        return env_vars.contains_key(name) || env::var_os(name).is_some();
    }

    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let assocs = marked_vars(env_vars, ASSOC_VARS);
    if arrays.iter().any(|marked| marked == operand)
        || assocs.iter().any(|marked| marked == operand)
    {
        return env_vars
            .get(operand)
            .map(|value| !value.starts_with('(') && !value.is_empty())
            .unwrap_or(false);
    }

    env_vars.contains_key(operand) || env::var_os(operand).is_some()
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

fn eval_binary(left: &str, op: &str, right: &str) -> Result<bool, String> {
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
        "-nt" => Ok(modified(left) > modified(right)),
        "-ot" => Ok(modified(left) < modified(right)),
        "-ef" => Ok(same_file(left, right)),
        _ => Err(format!("{}: binary operator expected", op)),
    }
}

fn parse_int(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|_| format!("{}: integer expression expected", value))
}

fn modified(path: &str) -> Option<std::time::SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn same_file(left: &str, right: &str) -> bool {
    let Ok(left) = fs::canonicalize(left) else {
        return false;
    };
    let Ok(right) = fs::canonicalize(right) else {
        return false;
    };
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], bracket: bool) -> (i32, String) {
        let env_vars = HashMap::new();
        let mut stderr = Vec::new();
        let status =
            execute_with_stderr(args.iter().copied(), bracket, &env_vars, &mut stderr).unwrap();
        (status, String::from_utf8(stderr).unwrap())
    }

    #[test]
    fn empty_expression_is_false() {
        assert_eq!(run(&[], false).0, EXECUTION_FAILURE);
    }

    #[test]
    fn single_non_empty_string_is_true() {
        assert_eq!(run(&["hello"], false).0, EXECUTION_SUCCESS);
        assert_eq!(run(&[""], false).0, EXECUTION_FAILURE);
    }

    #[test]
    fn supports_string_and_numeric_binary_operators() {
        assert_eq!(run(&["a", "=", "a"], false).0, EXECUTION_SUCCESS);
        assert_eq!(run(&["2", "-lt", "3"], false).0, EXECUTION_SUCCESS);
    }

    #[test]
    fn supports_not_and_logical_operators() {
        assert_eq!(run(&["!", ""], false).0, EXECUTION_SUCCESS);
        assert_eq!(run(&["x", "-a", ""], false).0, EXECUTION_FAILURE);
        assert_eq!(run(&["x", "-o", ""], false).0, EXECUTION_SUCCESS);
    }

    #[test]
    fn bracket_requires_closing_bracket() {
        let (status, stderr) = run(&["x"], true);

        assert_eq!(status, EX_BADUSAGE);
        assert!(stderr.contains("missing `]'"));
    }
}
