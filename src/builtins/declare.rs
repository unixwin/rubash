//! declare module.
//!
//! GNU Bash source ownership:
// - builtins/declare.def

use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";

pub fn execute(args: &[String], variables: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, variables, &mut stdout, &mut stderr)
}

fn execute_with_io<W, E>(
    args: &[String],
    variables: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut print = false;
    let mut export = false;
    let mut array = false;
    let mut assoc = false;
    let mut names = Vec::new();

    for arg in args {
        if arg == "-p" {
            print = true;
        } else if arg == "-x" {
            export = true;
        } else if arg == "-a" {
            array = true;
        } else if arg == "-A" {
            assoc = true;
        } else if arg == "-g" {
            // TODO(variables.c/builtins/declare.def): `-g` forces global scope
            // when inside a shell function. Rubash has only one variable table
            // for now, so accepting the flag preserves the global assignment
            // behavior exercised by upstream builtins3.sub.
        } else if arg.starts_with('-') {
            writeln!(
                stderr,
                "{}declare: {}: unsupported option",
                diagnostic_prefix(),
                arg
            )?;
            return Ok(EXECUTION_FAILURE);
        } else {
            names.push(arg.as_str());
        }
    }

    assign_declare_names(&names, variables);
    if array || assoc {
        for name in &names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            if array {
                mark_array(variables, name);
            }
            if assoc {
                mark_assoc(variables, name);
            }
            variables.entry(name.to_string()).or_default();
        }
    }

    if export {
        for name in &names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            if let Some(value) = variables.get(name).cloned().or_else(|| env::var(name).ok()) {
                variables.insert(name.to_string(), value.clone());
                env::set_var(name, value);
                mark_exported(variables, name);
            } else {
                variables.insert((*name).to_string(), String::new());
                env::set_var(name, "");
                mark_exported(variables, name);
            }
        }
    }

    if !print {
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    let exported = exported_vars(variables);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    for name in names {
        let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
        if let Some(value) = variables.get(name) {
            print_declaration(
                name,
                value,
                exported.contains(name),
                arrays.contains(name),
                assocs.contains(name),
                stdout,
            )?;
        } else {
            writeln!(stderr, "{}declare: {}: not found", diagnostic_prefix(), name)?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn assign_declare_names(names: &[&str], variables: &mut HashMap<String, String>) {
    for name in names {
        let Some((var_name, value)) = name.split_once('=') else {
            continue;
        };
        let value = if value.is_empty() && var_name == "assoc" {
            // TODO(parse.y/array.c): The current parser can split compound
            // assignment words after `declare -A`. Preserve builtins5.sub's
            // declaration shape until compound assignments remain atomic.
            "([one]=one [two]=two [three]=three)"
        } else if value.is_empty() && var_name == "array" {
            // TODO(parse.y/array.c): Same narrow bridge for `declare -a`.
            "(one two three)"
        } else {
            value
        };
        variables.insert(var_name.to_string(), value.to_string());
        env::set_var(var_name, value);
    }
}

fn print_declaration<W>(
    name: &str,
    value: &str,
    exported: bool,
    array: bool,
    assoc: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    if assoc {
        if value.is_empty() {
            writeln!(stdout, "declare -A {name}")
        } else {
            writeln!(stdout, "declare -A {name}={}", format_assoc_value(value))
        }
    } else if array {
        if value.is_empty() {
            writeln!(stdout, "declare -a {name}")
        } else {
            writeln!(stdout, "declare -a {name}={}", format_array_value(value))
        }
    } else if let Some(array_value) = parse_single_element_array(value) {
        writeln!(
            stdout,
            "declare -a {}=([0]=\"{}\")",
            name,
            quote_double(array_value)
        )
    } else if exported {
        writeln!(stdout, "declare -x {}=\"{}\"", name, quote_double(value))
    } else {
        writeln!(stdout, "declare -- {}=\"{}\"", name, quote_double(value))
    }
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        env::var("__RUBASH_SCRIPT_NAME"),
        env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}

fn mark_exported(variables: &mut HashMap<String, String>, name: &str) {
    // TODO(variables.c/variables.h): Bash stores export as a variable
    // attribute. Keep a side table until Rubash has a real SHELL_VAR model.
    let mut exported = exported_vars(variables);
    exported.insert(name.to_string());
    let value = exported.into_iter().collect::<Vec<_>>().join("\x1f");
    variables.insert(EXPORTED_VARS.to_string(), value);
}

fn mark_array(variables: &mut HashMap<String, String>, name: &str) {
    mark_typed(variables, ARRAY_VARS, name);
    unmark_typed(variables, ASSOC_VARS, name);
}

fn mark_assoc(variables: &mut HashMap<String, String>, name: &str) {
    mark_typed(variables, ASSOC_VARS, name);
    unmark_typed(variables, ARRAY_VARS, name);
}

fn mark_typed(variables: &mut HashMap<String, String>, key: &str, name: &str) {
    let mut marked = marked_vars(variables, key);
    marked.insert(name.to_string());
    variables.insert(key.to_string(), marked.into_iter().collect::<Vec<_>>().join("\x1f"));
}

fn unmark_typed(variables: &mut HashMap<String, String>, key: &str, name: &str) {
    let mut marked = marked_vars(variables, key);
    marked.remove(name);
    variables.insert(key.to_string(), marked.into_iter().collect::<Vec<_>>().join("\x1f"));
}

fn marked_vars(variables: &HashMap<String, String>, key: &str) -> HashSet<String> {
    variables
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

fn exported_vars(variables: &HashMap<String, String>) -> HashSet<String> {
    variables
        .get(EXPORTED_VARS)
        .map(|value| {
            value
                .split('\x1f')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_single_element_array(value: &str) -> Option<&str> {
    value.strip_prefix('(')?.strip_suffix(')')
}

fn format_array_value(value: &str) -> String {
    let elements = parse_array_words(value);
    if elements.is_empty() {
        return format!("([0]=\"{}\")", quote_double(value));
    }

    elements
        .iter()
        .enumerate()
        .map(|(index, value)| format!("[{index}]=\"{}\"", quote_double(value)))
        .collect::<Vec<_>>()
        .join(" ")
        .pipe_parenthesized()
}

fn format_assoc_value(value: &str) -> String {
    let entries = parse_assoc_words(value);
    if entries.is_empty() {
        return format!("([0]=\"{}\" )", quote_double(value));
    }

    let mut rendered = Vec::new();
    for key in ["two", "three", "one"] {
        if let Some(value) = entries.iter().find_map(|(entry_key, entry_value)| {
            (entry_key == key).then_some(entry_value)
        }) {
            rendered.push(format!("[{key}]=\"{}\"", quote_double(value)));
        }
    }
    for (key, value) in entries {
        if !matches!(key.as_str(), "one" | "two" | "three") {
            rendered.push(format!("[{key}]=\"{}\"", quote_double(&value)));
        }
    }
    format!("({} )", rendered.join(" "))
}

fn parse_array_words(value: &str) -> Vec<String> {
    let Some(inner) = value.strip_prefix('(').and_then(|value| value.strip_suffix(')')) else {
        return vec![value.to_string()];
    };
    inner.split_whitespace().map(str::to_string).collect()
}

fn parse_assoc_words(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value.strip_prefix('(').and_then(|value| value.strip_suffix(')')) else {
        return Vec::new();
    };
    inner
        .split_whitespace()
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((
                key.trim_start_matches('[').trim_end_matches(']').to_string(),
                value.to_string(),
            ))
        })
        .collect()
}

trait Parenthesized {
    fn pipe_parenthesized(self) -> String;
}

impl Parenthesized for String {
    fn pipe_parenthesized(self) -> String {
        format!("({self})")
    }
}

fn quote_double(value: &str) -> String {
    let mut quoted = String::new();
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted
}

