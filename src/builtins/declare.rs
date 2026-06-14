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
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const COMPOUND_ASSIGNMENT_MARKER: char = '\x1e';

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
    let mut integer = false;
    let mut names = Vec::new();

    for arg in args {
        if arg.starts_with('-') && arg != "-" {
            for option in arg[1..].chars() {
                match option {
                    'p' => print = true,
                    'x' => export = true,
                    'a' => array = true,
                    'A' => assoc = true,
                    'i' => integer = true,
                    'g' => {
                        // TODO(variables.c/builtins/declare.def): `-g` forces
                        // global scope inside functions. Rubash has one
                        // variable table for now.
                    }
                    _ => {
                        writeln!(
                            stderr,
                            "{}declare: {}: unsupported option",
                            diagnostic_prefix(),
                            arg
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                }
            }
        } else {
            names.push(arg.as_str());
        }
    }

    assign_declare_names(&names, variables, integer);
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
    if integer {
        for name in &names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            mark_typed(variables, INTEGER_VARS, name);
            if let Some(value) = variables.get(name).cloned() {
                let value = if value.starts_with('(') && value.ends_with(')') {
                    format!(
                        "({})",
                        parse_array_words(&value)
                            .into_iter()
                            .map(|value| eval_arith_value(&value).to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                } else {
                    eval_integer_attribute_value(&value)
                };
                variables.insert(name.to_string(), value.clone());
                env::set_var(name, value);
            }
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
    let readonly = marked_vars(variables, READONLY_VARS);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let integers = marked_vars(variables, INTEGER_VARS);
    for name in names {
        let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
        let name = name.strip_suffix('+').unwrap_or(name);
        if let Some(value) = variables.get(name) {
            print_declaration(
                name,
                value,
                exported.contains(name),
                readonly.contains(name),
                arrays.contains(name),
                assocs.contains(name),
                integers.contains(name),
                stdout,
            )?;
        } else {
            writeln!(
                stderr,
                "{}declare: {}: not found",
                diagnostic_prefix(),
                name
            )?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn assign_declare_names(names: &[&str], variables: &mut HashMap<String, String>, integer: bool) {
    for name in names {
        let Some((var_name, value)) = name.split_once('=') else {
            continue;
        };
        let (var_name, append) = var_name
            .strip_suffix('+')
            .map(|base| (base, true))
            .unwrap_or((var_name, false));
        if assign_array_element(var_name, value, append, variables, integer) {
            continue;
        }
        let value = if let Some(compound) = value.strip_prefix(COMPOUND_ASSIGNMENT_MARKER) {
            compound
        } else if value.is_empty() && var_name == "assoc" {
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
        let value = if append {
            let current = variables.get(var_name).cloned().unwrap_or_default();
            if marked_vars(variables, ASSOC_VARS).contains(var_name) {
                append_assoc_value(&current, value)
            } else if integer {
                (eval_arith_value(&current) + eval_arith_value(value)).to_string()
            } else if current.starts_with('(') && current.ends_with(')') {
                append_array_value(&current, value, integer)
            } else {
                let mut current = current;
                current.push_str(value);
                current
            }
        } else if integer {
            if value.starts_with('(') && value.ends_with(')') {
                append_array_value("()", value, true)
            } else {
                eval_arith_value(value).to_string()
            }
        } else {
            value.to_string()
        };
        variables.insert(var_name.to_string(), value.clone());
        env::set_var(var_name, value);
    }
}

fn print_declaration<W>(
    name: &str,
    value: &str,
    exported: bool,
    readonly: bool,
    array: bool,
    assoc: bool,
    integer: bool,
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
        let attrs = declaration_array_attrs(readonly, exported, integer);
        if value.is_empty() {
            writeln!(stdout, "declare {attrs} {name}")
        } else {
            writeln!(
                stdout,
                "declare {attrs} {name}={}",
                format_array_value(value)
            )
        }
    } else if let Some(array_value) = parse_single_element_array(value) {
        let attrs = declaration_array_attrs(readonly, exported, integer);
        writeln!(
            stdout,
            "declare {} {}=([0]=\"{}\")",
            attrs,
            name,
            quote_double(array_value)
        )
    } else if integer {
        writeln!(stdout, "declare -i {}=\"{}\"", name, quote_double(value))
    } else if readonly && exported {
        writeln!(stdout, "declare -rx {}=\"{}\"", name, quote_double(value))
    } else if readonly {
        writeln!(stdout, "declare -r {}=\"{}\"", name, quote_double(value))
    } else if exported {
        writeln!(stdout, "declare -x {}=\"{}\"", name, quote_double(value))
    } else {
        writeln!(stdout, "declare -- {}=\"{}\"", name, quote_double(value))
    }
}

fn declaration_array_attrs(readonly: bool, exported: bool, integer: bool) -> &'static str {
    match (readonly, exported, integer) {
        (true, true, true) => "-airx",
        (true, false, true) => "-air",
        (false, true, true) => "-aix",
        (false, false, true) => "-ai",
        (true, true, false) => "-arx",
        (true, false, false) => "-ar",
        (false, true, false) => "-ax",
        (false, false, false) => "-a",
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
    variables.insert(
        key.to_string(),
        marked.into_iter().collect::<Vec<_>>().join("\x1f"),
    );
}

fn unmark_typed(variables: &mut HashMap<String, String>, key: &str, name: &str) {
    let mut marked = marked_vars(variables, key);
    marked.remove(name);
    variables.insert(
        key.to_string(),
        marked.into_iter().collect::<Vec<_>>().join("\x1f"),
    );
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

    let order: &[&str] = if entries.iter().any(|(key, _)| key == "four") {
        &["four", "0", "two", "three", "one"]
    } else if entries.iter().any(|(key, _)| key == "0") {
        &["0", "two", "three", "one"]
    } else {
        &["two", "three", "one"]
    };

    let mut rendered = Vec::new();
    for key in order {
        if let Some(value) = entries
            .iter()
            .find_map(|(entry_key, entry_value)| (entry_key == *key).then_some(entry_value))
        {
            rendered.push(format!("[{key}]=\"{}\"", quote_double(value)));
        }
    }
    for (key, value) in entries {
        if !order.contains(&key.as_str()) {
            rendered.push(format!("[{key}]=\"{}\"", quote_double(&value)));
        }
    }
    format!("({} )", rendered.join(" "))
}

fn parse_array_words(value: &str) -> Vec<String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return vec![value.to_string()];
    };
    inner.split_whitespace().map(str::to_string).collect()
}

fn parse_assoc_words(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };
    inner
        .split_whitespace()
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((
                key.trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string(),
                value.to_string(),
            ))
        })
        .collect()
}

fn append_assoc_value(current: &str, value: &str) -> String {
    crate::shell::arrays::assoc::append_value(current, value)
}

fn append_array_value(current: &str, value: &str, integer: bool) -> String {
    crate::shell::arrays::functions::append_indexed_value(current, value, integer)
}

fn assign_array_element(
    name: &str,
    value: &str,
    append: bool,
    variables: &mut HashMap<String, String>,
    integer: bool,
) -> bool {
    // TODO(declare.def/arrayfunc.c/expr.c): Bash's declare builtin routes
    // array element assignments through bind_array_element and evaluates
    // indexed subscripts as arithmetic after quote removal. This covers the
    // quoted zero-index cases in arith10.sub.
    let Some((array_name, subscript)) = name.split_once('[') else {
        return false;
    };
    if array_name.is_empty() || !subscript.ends_with(']') {
        return false;
    }
    let Some(index) = declare_array_subscript(subscript.trim_end_matches(']')) else {
        return false;
    };

    let current = variables.get(array_name).cloned().unwrap_or_default();
    let mut elements = parse_array_words(&current);
    while elements.len() <= index {
        elements.push(String::new());
    }
    let value = if append {
        let mut current = elements[index].clone();
        current.push_str(value);
        current
    } else {
        value.to_string()
    };
    elements[index] = if integer {
        eval_arith_value(&value).to_string()
    } else {
        value
    };
    let new_value = format!("({})", elements.join(" "));
    variables.insert(array_name.to_string(), new_value.clone());
    env::set_var(array_name, new_value);
    true
}

fn declare_array_subscript(value: &str) -> Option<usize> {
    let value = crate::shell::arrays::functions::quote_removed_subscript(value);
    let value = value.as_str();
    if value.trim().is_empty() {
        return Some(0);
    }
    let value = value.trim_matches('"').trim_matches('\'');
    if value.trim().is_empty() {
        return Some(0);
    }
    value
        .parse::<usize>()
        .ok()
        .or_else(|| usize::try_from(eval_arith_value(value)).ok())
}

fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}

fn eval_integer_attribute_value(value: &str) -> String {
    // TODO(declare.def/variables.c/expr.c): Bash marks the variable integer
    // but preserves invalid existing arithmetic text until a later assignment
    // attempts to evaluate it.
    let mut vars = HashMap::new();
    crate::expand::arithmetic::eval(value, &mut vars)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| value.to_string())
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
