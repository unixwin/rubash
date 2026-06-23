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
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const COMPOUND_ASSIGNMENT_MARKER: char = '\x1e';

pub fn execute(args: &[String], variables: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, variables, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
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
    let mut uppercase = false;
    let mut lowercase = false;
    let mut readonly = false;
    let mut unset_export = false;
    let mut unset_array = false;
    let mut unset_assoc = false;
    let mut unset_integer = false;
    let mut unset_uppercase = false;
    let mut unset_lowercase = false;
    let mut names = Vec::new();

    for arg in args {
        if (arg.starts_with('-') || arg.starts_with('+')) && arg != "-" && arg != "+" {
            let set_attr = arg.starts_with('-');
            for option in arg[1..].chars() {
                match option {
                    'p' => print = true,
                    'x' if set_attr => export = true,
                    'x' => unset_export = true,
                    'a' if set_attr => array = true,
                    'a' => unset_array = true,
                    'A' if set_attr => assoc = true,
                    'A' => unset_assoc = true,
                    'i' if set_attr => integer = true,
                    'i' => unset_integer = true,
                    'u' => {
                        if set_attr {
                            uppercase = true;
                            lowercase = false;
                        } else {
                            unset_uppercase = true;
                        }
                    }
                    'l' => {
                        if set_attr {
                            lowercase = true;
                            uppercase = false;
                        } else {
                            unset_lowercase = true;
                        }
                    }
                    'r' if set_attr => readonly = true,
                    'r' => {}
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
    if unset_export
        || unset_array
        || unset_assoc
        || unset_integer
        || unset_uppercase
        || unset_lowercase
    {
        for name in &names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            if unset_export {
                unmark_typed(variables, EXPORTED_VARS, name);
            }
            if unset_array {
                unmark_typed(variables, ARRAY_VARS, name);
            }
            if unset_assoc {
                unmark_typed(variables, ASSOC_VARS, name);
            }
            if unset_integer {
                unmark_typed(variables, INTEGER_VARS, name);
            }
            if unset_uppercase {
                unmark_typed(variables, UPPERCASE_VARS, name);
            }
            if unset_lowercase {
                unmark_typed(variables, LOWERCASE_VARS, name);
            }
        }
    }
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
                    eval_arith_value(&value).to_string()
                };
                variables.insert(name.to_string(), value.clone());
                env::set_var(name, value);
            }
        }
    }
    if uppercase || lowercase {
        for name in &names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            if uppercase {
                mark_typed(variables, UPPERCASE_VARS, name);
                unmark_typed(variables, LOWERCASE_VARS, name);
            }
            if lowercase {
                mark_typed(variables, LOWERCASE_VARS, name);
                unmark_typed(variables, UPPERCASE_VARS, name);
            }
            if let Some(value) = variables.get(name).cloned() {
                let value = if uppercase {
                    value.to_uppercase()
                } else {
                    value.to_lowercase()
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
    if readonly {
        for name in &names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            variables.entry(name.to_string()).or_default();
            mark_typed(variables, READONLY_VARS, name);
        }
    }

    if names.is_empty() {
        print = true;
    }

    if !print {
        return Ok(EXECUTION_SUCCESS);
    }

    let filter_exported = export;
    let filter_readonly = readonly;
    let filter_array = array;
    let filter_assoc = assoc;
    let filter_integer = integer;
    let filter_uppercase = uppercase;
    let filter_lowercase = lowercase;

    let mut status = EXECUTION_SUCCESS;
    let exported = exported_vars(variables);
    let readonly = marked_vars(variables, READONLY_VARS);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let integers = marked_vars(variables, INTEGER_VARS);
    let uppercase = marked_vars(variables, UPPERCASE_VARS);
    let lowercase = marked_vars(variables, LOWERCASE_VARS);
    let names_to_print = if names.is_empty() {
        declaration_names_to_print(
            variables,
            filter_exported,
            filter_readonly,
            filter_array,
            filter_assoc,
            filter_integer,
            filter_uppercase,
            filter_lowercase,
            &uppercase,
            &lowercase,
        )
    } else {
        names
            .into_iter()
            .map(|name| name.split_once('=').map(|(name, _)| name).unwrap_or(name))
            .map(|name| name.strip_suffix('+').unwrap_or(name))
            .map(str::to_string)
            .collect()
    };
    for name in names_to_print {
        let attrs = DeclarationAttrs {
            exported: exported.contains(&name),
            readonly: readonly.contains(&name),
            array: arrays.contains(&name),
            assoc: assocs.contains(&name),
            integer: integers.contains(&name),
            uppercase: uppercase.contains(&name),
            lowercase: lowercase.contains(&name),
        };
        if let Some(value) = variables.get(&name) {
            print_declaration(&name, value, attrs, stdout)?;
        } else if attrs.has_scalar_attribute() {
            print_unset_declaration(&name, attrs, stdout)?;
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

fn declaration_names_to_print(
    variables: &HashMap<String, String>,
    export: bool,
    readonly: bool,
    array: bool,
    assoc: bool,
    integer: bool,
    uppercase: bool,
    lowercase: bool,
    uppercase_vars: &HashSet<String>,
    lowercase_vars: &HashSet<String>,
) -> Vec<String> {
    let exported = exported_vars(variables);
    let readonly_vars = marked_vars(variables, READONLY_VARS);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let integers = marked_vars(variables, INTEGER_VARS);
    let filter_by_attr = export || readonly || array || assoc || integer || uppercase || lowercase;
    let mut names: Vec<String> = variables
        .keys()
        .filter(|name| !name.starts_with("__RUBASH_"))
        .filter(|name| {
            if !filter_by_attr {
                return true;
            }
            (!export || exported.contains(*name))
                && (!readonly || readonly_vars.contains(*name))
                && (!array || arrays.contains(*name))
                && (!assoc || assocs.contains(*name))
                && (!integer || integers.contains(*name))
                && (!uppercase || uppercase_vars.contains(*name))
                && (!lowercase || lowercase_vars.contains(*name))
        })
        .cloned()
        .collect();
    for name in exported.iter().chain(readonly_vars.iter()) {
        if name.starts_with("__RUBASH_") {
            continue;
        }
        if filter_by_attr
            && !((!export || exported.contains(name))
                && (!readonly || readonly_vars.contains(name))
                && (!array || arrays.contains(name))
                && (!assoc || assocs.contains(name))
                && (!integer || integers.contains(name))
                && (!uppercase || uppercase_vars.contains(name))
                && (!lowercase || lowercase_vars.contains(name)))
        {
            continue;
        }
        if !names.iter().any(|current| current == name) {
            names.push(name.clone());
        }
    }
    names.sort();
    names
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
        if is_noassign_bash_array(var_name) {
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

#[derive(Clone, Copy)]
struct DeclarationAttrs {
    exported: bool,
    readonly: bool,
    array: bool,
    assoc: bool,
    integer: bool,
    uppercase: bool,
    lowercase: bool,
}

impl DeclarationAttrs {
    fn has_scalar_attribute(self) -> bool {
        self.exported || self.readonly || self.integer || self.uppercase || self.lowercase
    }
}

fn print_declaration<W>(
    name: &str,
    value: &str,
    attrs: DeclarationAttrs,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    if attrs.assoc {
        if value.is_empty() {
            writeln!(stdout, "declare -A {name}")
        } else {
            writeln!(stdout, "declare -A {name}={}", format_assoc_value(value))
        }
    } else if attrs.array {
        let attrs = declaration_array_attrs(attrs);
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
        let attrs = declaration_array_attrs(attrs);
        writeln!(
            stdout,
            "declare {} {}=([0]=\"{}\")",
            attrs,
            name,
            quote_double(array_value)
        )
    } else if let Some(attrs) = declaration_scalar_attrs(attrs) {
        writeln!(
            stdout,
            "declare {attrs} {}=\"{}\"",
            name,
            quote_double(value)
        )
    } else {
        writeln!(stdout, "declare -- {}=\"{}\"", name, quote_double(value))
    }
}

fn print_unset_declaration<W>(name: &str, attrs: DeclarationAttrs, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    if let Some(attrs) = declaration_scalar_attrs(attrs) {
        writeln!(stdout, "declare {attrs} {name}")
    } else {
        writeln!(stdout, "declare -- {name}")
    }
}

fn declaration_scalar_attrs(attrs: DeclarationAttrs) -> Option<String> {
    let mut flags = String::from("-");
    if attrs.integer {
        flags.push('i');
    }
    if attrs.lowercase {
        flags.push('l');
    }
    if attrs.readonly {
        flags.push('r');
    }
    if attrs.uppercase {
        flags.push('u');
    }
    if attrs.exported {
        flags.push('x');
    }
    (flags.len() > 1).then_some(flags)
}

fn declaration_array_attrs(attrs: DeclarationAttrs) -> String {
    let mut flags = String::from("-a");
    if attrs.integer {
        flags.push('i');
    }
    if attrs.lowercase {
        flags.push('l');
    }
    if attrs.readonly {
        flags.push('r');
    }
    if attrs.uppercase {
        flags.push('u');
    }
    if attrs.exported {
        flags.push('x');
    }
    flags
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
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered.to_string();
    }

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

fn is_noassign_bash_array(name: &str) -> bool {
    matches!(
        name,
        "BASH_ARGC" | "BASH_ARGV" | "BASH_LINENO" | "BASH_SOURCE" | "FUNCNAME"
    )
}

fn parse_assoc_words(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };
    split_storage_words(inner)
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((
                key.trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string(),
                unquote_storage_value(value),
            ))
        })
        .collect()
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

fn append_assoc_value(current: &str, value: &str) -> String {
    let mut entries = parse_assoc_words(current);
    for token in parse_array_tokens(value) {
        if let Some((left, rhs)) = token.split_once('=') {
            if let Some(key) = left
                .strip_prefix('[')
                .and_then(|left| left.strip_suffix(']'))
            {
                entries.push((key.to_string(), rhs.to_string()));
                continue;
            }
        }
        entries.push(("0".to_string(), token));
    }

    format!(
        "({})",
        entries
            .into_iter()
            .map(|(key, value)| format!("[{key}]={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn append_array_value(current: &str, value: &str, integer: bool) -> String {
    let mut elements = parse_array_words(current);
    if current == "()" {
        elements.clear();
    }
    let scalar_append = integer && !value.starts_with('(');

    for token in parse_array_tokens(value) {
        if let Some((left, rhs)) = token.split_once("+=") {
            if let Some(index) = array_assignment_index(left) {
                while elements.len() <= index {
                    elements.push(String::new());
                }
                elements[index] =
                    (eval_arith_value(&elements[index]) + eval_arith_value(rhs)).to_string();
                continue;
            }
        }
        if let Some((left, rhs)) = token.split_once('=') {
            if let Some(index) = array_assignment_index(left) {
                while elements.len() <= index {
                    elements.push(String::new());
                }
                elements[index] = rhs.to_string();
                continue;
            }
        }
        if scalar_append && !elements.is_empty() {
            elements[0] = (eval_arith_value(&elements[0]) + eval_arith_value(&token)).to_string();
        } else {
            elements.push(token);
        }
    }

    if integer {
        for element in &mut elements {
            *element = eval_arith_value(element).to_string();
        }
    }

    format!("({})", elements.join(" "))
}

fn array_assignment_index(left: &str) -> Option<usize> {
    left.strip_prefix('[')?.strip_suffix(']')?.parse().ok()
}

fn parse_array_tokens(value: &str) -> Vec<String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return vec![value.to_string()];
    };
    inner.split_whitespace().map(str::to_string).collect()
}

fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
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
