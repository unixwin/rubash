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
        "-R" => Ok(marked_vars(env_vars, READONLY_VARS)
            .iter()
            .any(|name| name == operand)),
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

pub(crate) fn variable_is_set(operand: &str, env_vars: &HashMap<String, String>) -> bool {
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

    if let Some((name, subscript)) = parse_array_subscript(operand) {
        let arrays = marked_vars(env_vars, ARRAY_VARS);
        let assocs = marked_vars(env_vars, ASSOC_VARS);
        let Some(value) = env_vars.get(name) else {
            return false;
        };

        if assocs.iter().any(|marked| marked == name) {
            return assoc_key_is_set(value, subscript);
        }

        if arrays.iter().any(|marked| marked == name) || is_array_storage(value) {
            return subscript
                .parse::<usize>()
                .ok()
                .map(|index| array_index_is_set(value, index))
                .unwrap_or(false);
        }

        return subscript == "0" && (!value.is_empty() || env_vars.contains_key(name));
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

fn parse_array_subscript(value: &str) -> Option<(&str, &str)> {
    let (name, subscript) = value.split_once('[')?;
    Some((name, subscript.strip_suffix(']')?))
}

fn is_array_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')') || value.starts_with('\x1d')
}

fn array_index_is_set(value: &str, index: usize) -> bool {
    array_entries(value)
        .into_iter()
        .any(|(entry_index, _)| entry_index == index)
}

fn array_entries(value: &str) -> Vec<(usize, String)> {
    let value = value.strip_prefix('\x1d').unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };

    inner
        .split_whitespace()
        .enumerate()
        .filter_map(|(default_index, part)| {
            if let Some((left, right)) = part.split_once('=') {
                let index = left
                    .strip_prefix('[')
                    .and_then(|left| left.strip_suffix(']'))
                    .and_then(|index| index.parse::<usize>().ok())?;
                return Some((index, strip_array_value_quotes(right).to_string()));
            }
            Some((default_index, strip_array_value_quotes(part).to_string()))
        })
        .collect()
}

fn assoc_key_is_set(value: &str, key: &str) -> bool {
    let value = value.strip_prefix('\x1d').unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };

    split_storage_words(inner).any(|part| {
        let Some((left, _)) = part.split_once('=') else {
            return false;
        };
        left.strip_prefix('[')
            .and_then(|left| left.strip_suffix(']'))
            .map(unquote_storage_value)
            .as_deref()
            == Some(key)
    })
}

fn split_storage_words(value: &str) -> impl Iterator<Item = String> + '_ {
    StorageWordIter {
        input: value,
        offset: 0,
    }
}

struct StorageWordIter<'a> {
    input: &'a str,
    offset: usize,
}

impl Iterator for StorageWordIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ch) = self.input.get(self.offset..)?.chars().next() {
            if !ch.is_ascii_whitespace() {
                break;
            }
            self.offset += ch.len_utf8();
        }

        let mut word = String::new();
        let mut in_double = false;
        let mut escaped = false;
        for (relative, ch) in self.input[self.offset..].char_indices() {
            if escaped {
                word.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' && in_double {
                word.push(ch);
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_double = !in_double;
                word.push(ch);
                continue;
            }
            if ch.is_ascii_whitespace() && !in_double {
                self.offset += relative + ch.len_utf8();
                return Some(word);
            }
            word.push(ch);
        }
        self.offset = self.input.len();
        (!word.is_empty()).then_some(word)
    }
}

fn unquote_storage_value(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value.to_string();
    };

    let mut unquoted = String::new();
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            unquoted.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            unquoted.push(ch);
        }
    }
    if escaped {
        unquoted.push('\\');
    }
    unquoted
}

fn strip_array_value_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], bracket: bool) -> (i32, String) {
        let env_vars = HashMap::new();
        run_with_env(args, bracket, &env_vars)
    }

    fn run_with_env(
        args: &[&str],
        bracket: bool,
        env_vars: &HashMap<String, String>,
    ) -> (i32, String) {
        let mut stderr = Vec::new();
        let status =
            execute_with_stderr(args.iter().copied(), bracket, env_vars, &mut stderr).unwrap();
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
    fn supports_shell_option_unary_operator() {
        let mut env_vars = HashMap::new();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        crate::builtins::set::set_with_io(
            ["-o", "errexit"],
            &mut env_vars,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(
            run_with_env(&["-o", "errexit"], false, &env_vars).0,
            EXECUTION_SUCCESS
        );
        crate::builtins::set::set_with_io(
            ["+o", "errexit"],
            &mut env_vars,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(
            run_with_env(&["-o", "errexit"], false, &env_vars).0,
            EXECUTION_FAILURE
        );
        assert_eq!(
            run_with_env(&["-o", "no_such_option"], false, &env_vars).0,
            EXECUTION_FAILURE
        );
    }

    #[test]
    fn leading_file_operator_is_unary_not_logical_and() {
        assert_eq!(run(&["-a", "Cargo.toml"], false).0, EXECUTION_SUCCESS);
    }

    #[test]
    fn supports_parenthesized_logical_expressions() {
        assert_eq!(
            run(&["(", "", "-o", "x", ")", "-a", ""], false).0,
            EXECUTION_FAILURE
        );
        assert_eq!(
            run(&["\\(", "", "-o", "x", "\\)", "-a", "x"], false).0,
            EXECUTION_SUCCESS
        );
    }

    #[test]
    fn bracket_requires_closing_bracket() {
        let (status, stderr) = run(&["x"], true);

        assert_eq!(status, EX_BADUSAGE);
        assert!(stderr.contains("missing `]'"));
    }
}
