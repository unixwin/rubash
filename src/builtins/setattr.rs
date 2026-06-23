//! `export` and `readonly` attribute builtins.
//!
//! GNU Bash source ownership:
//! - builtins/setattr.def (`export_builtin`, `readonly_builtin`)

use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const COMPOUND_ASSIGNMENT_MARKER: char = '\x1e';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportMode {
    Set,
    Unset,
}

/// Execute `export` with arguments after the command name.
pub fn export(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    export_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

/// Execute `readonly` with arguments after the command name.
pub fn readonly(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    readonly_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn export_with_io<'a, I, W, E>(
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
    let mut mode = ExportMode::Set;
    let mut print = false;
    let mut array = false;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'n' => mode = ExportMode::Unset,
                'p' => print = true,
                'a' => array = true,
                'f' => {
                    writeln!(
                        stderr,
                        "rubash: export: -f: shell functions are not supported yet"
                    )?;
                    return Ok(EXECUTION_FAILURE);
                }
                other => {
                    writeln!(stderr, "rubash: export: -{}: invalid option", other)?;
                    writeln!(
                        stderr,
                        "export: usage: export [-fn] [name[=value] ...] or export -p"
                    )?;
                    return Ok(EX_USAGE);
                }
            }
        }

        index += 1;
    }

    if index >= args.len() || print {
        print_exported(env_vars, stdout)?;
        if index >= args.len() {
            return Ok(EXECUTION_SUCCESS);
        }
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in &args[index..] {
        if apply_export_arg(arg, mode, array, env_vars, stderr)? != EXECUTION_SUCCESS {
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn apply_export_arg<W>(
    arg: &str,
    mode: ExportMode,
    array: bool,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    if !valid_identifier(name) {
        writeln!(stderr, "rubash: export: `{}`: not a valid identifier", arg)?;
        return Ok(EXECUTION_FAILURE);
    }
    let resolved_name = nameref_target_name(env_vars, name).unwrap_or_else(|| name.to_string());
    let name = resolved_name.as_str();

    match mode {
        ExportMode::Set => {
            if value.is_some() && marked_vars(env_vars, READONLY_VARS).contains(name) {
                writeln!(stderr, "{}{}: readonly variable", diagnostic_prefix(), name)?;
                return Ok(EXECUTION_FAILURE);
            }

            if value.is_none() && !env_vars.contains_key(name) && env::var(name).is_err() {
                mark_exported(env_vars, name);
                return Ok(EXECUTION_SUCCESS);
            }

            let value = value
                .map(|value| array_attribute_assignment_value(value, array, env_vars, name))
                .or_else(|| env_vars.get(name).cloned())
                .or_else(|| env::var(name).ok())
                .unwrap_or_default();
            let value = if append {
                let mut current = env_vars.get(name).cloned().unwrap_or_default();
                if marked_vars(env_vars, INTEGER_VARS).contains(name) {
                    (eval_arith_value(&current) + eval_arith_value(&value)).to_string()
                } else {
                    current.push_str(&value);
                    current
                }
            } else {
                value
            };
            env_vars.insert(name.to_string(), value.clone());
            env::set_var(name, value);
            mark_exported(env_vars, name);
            if array || is_array_value(env_vars.get(name).map(String::as_str).unwrap_or("")) {
                mark_array(env_vars, name);
            }
        }
        ExportMode::Unset => {
            env::remove_var(name);
            unmark_exported(env_vars, name);
        }
    }

    Ok(EXECUTION_SUCCESS)
}

pub(crate) fn readonly_with_io<'a, I, W, E>(
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
    let mut print = false;
    let mut array = false;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || *arg == "-" {
            break;
        }
        for option in arg[1..].chars() {
            match option {
                'p' => print = true,
                'a' => array = true,
                'f' => {}
                other => {
                    writeln!(
                        stderr,
                        "{}readonly: -{}: invalid option",
                        diagnostic_prefix(),
                        other
                    )?;
                    writeln!(
                        stderr,
                        "readonly: usage: readonly [-aAf] [name[=value] ...] or readonly -p"
                    )?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    if index >= args.len() || print {
        print_readonly(env_vars, stdout)?;
        if index >= args.len() {
            return Ok(EXECUTION_SUCCESS);
        }
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in &args[index..] {
        if apply_readonly_arg(arg, array, env_vars, stderr)? != EXECUTION_SUCCESS {
            status = EXECUTION_FAILURE;
        }
    }
    Ok(status)
}

fn apply_readonly_arg<W>(
    arg: &str,
    array: bool,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    if !valid_identifier(name) {
        writeln!(
            stderr,
            "{}readonly: `{}`: not a valid identifier",
            diagnostic_prefix(),
            arg
        )?;
        return Ok(EXECUTION_FAILURE);
    }
    let resolved_name = nameref_target_name(env_vars, name).unwrap_or_else(|| name.to_string());
    let name = resolved_name.as_str();

    let readonly = marked_vars(env_vars, READONLY_VARS);
    if readonly.contains(name) && value.is_some() {
        if let Some(subject) = readonly_error_subject(value.unwrap_or_default(), array) {
            writeln!(
                stderr,
                "{}{}: {}: readonly variable",
                diagnostic_prefix(),
                subject,
                name
            )?;
        } else {
            writeln!(stderr, "{}{}: readonly variable", diagnostic_prefix(), name)?;
        }
        return Ok(EXECUTION_FAILURE);
    }

    if value.is_none() && !env_vars.contains_key(name) && env::var(name).is_err() {
        mark_readonly(env_vars, name);
        return Ok(EXECUTION_SUCCESS);
    }

    let value = value
        .map(|value| array_attribute_assignment_value(value, array, env_vars, name))
        .or_else(|| env_vars.get(name).cloned())
        .or_else(|| env::var(name).ok())
        .unwrap_or_default();
    let value = if append {
        let mut current = env_vars.get(name).cloned().unwrap_or_default();
        if marked_vars(env_vars, INTEGER_VARS).contains(name) {
            (eval_arith_value(&current) + eval_arith_value(&value)).to_string()
        } else {
            current.push_str(&value);
            current
        }
    } else {
        value
    };
    env_vars.insert(name.to_string(), value.clone());
    env::set_var(name, value);
    mark_readonly(env_vars, name);
    if array || is_array_value(env_vars.get(name).map(String::as_str).unwrap_or("")) {
        mark_array(env_vars, name);
    }
    Ok(EXECUTION_SUCCESS)
}

fn print_readonly<W>(env_vars: &HashMap<String, String>, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let readonly = marked_vars(env_vars, READONLY_VARS);
    let exported = marked_vars(env_vars, EXPORTED_VARS);
    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let integers = marked_vars(env_vars, INTEGER_VARS);
    let uppercase = marked_vars(env_vars, UPPERCASE_VARS);
    let lowercase = marked_vars(env_vars, LOWERCASE_VARS);
    let mut names: Vec<_> = readonly.into_iter().collect();
    names.sort();
    for name in names {
        if let Some(value) = env_vars.get(&name) {
            if arrays.contains(&name) || is_array_value(value) {
                let attrs = setattr_array_attrs(
                    &name,
                    true,
                    exported.contains(&name),
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {name}={}",
                    format_array_value(value)
                )?;
            } else {
                let attrs = setattr_scalar_attrs(
                    &name,
                    true,
                    exported.contains(&name),
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {name}=\"{}\"",
                    quote_export_value(value)
                )?;
            }
        } else {
            let attrs = setattr_scalar_attrs(
                &name,
                true,
                exported.contains(&name),
                &integers,
                &uppercase,
                &lowercase,
            );
            writeln!(stdout, "declare {attrs} {name}")?;
        }
    }
    Ok(())
}

fn print_exported<W>(env_vars: &HashMap<String, String>, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let readonly = marked_vars(env_vars, READONLY_VARS);
    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let integers = marked_vars(env_vars, INTEGER_VARS);
    let uppercase = marked_vars(env_vars, UPPERCASE_VARS);
    let lowercase = marked_vars(env_vars, LOWERCASE_VARS);
    let mut names: Vec<_> = marked_vars(env_vars, EXPORTED_VARS).into_iter().collect();
    names.sort();

    for name in names {
        if name.starts_with("__RUBASH_") {
            continue;
        }
        if let Some(value) = env_vars.get(&name) {
            if arrays.contains(&name) || is_array_value(value) {
                let attrs = setattr_array_attrs(
                    &name,
                    readonly.contains(&name),
                    true,
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {name}={}",
                    format_array_value(value)
                )?;
            } else {
                let attrs = setattr_scalar_attrs(
                    &name,
                    readonly.contains(&name),
                    true,
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {}=\"{}\"",
                    name,
                    quote_export_value(value)
                )?;
            }
        } else {
            let attrs = setattr_scalar_attrs(
                &name,
                readonly.contains(&name),
                true,
                &integers,
                &uppercase,
                &lowercase,
            );
            writeln!(stdout, "declare {attrs} {name}")?;
        }
    }

    Ok(())
}

fn setattr_scalar_attrs(
    name: &str,
    readonly: bool,
    exported: bool,
    integers: &HashSet<String>,
    uppercase: &HashSet<String>,
    lowercase: &HashSet<String>,
) -> String {
    let mut attrs = String::from("-");
    if integers.contains(name) {
        attrs.push('i');
    }
    if readonly {
        attrs.push('r');
    }
    if exported {
        attrs.push('x');
    }
    if lowercase.contains(name) {
        attrs.push('l');
    }
    if uppercase.contains(name) {
        attrs.push('u');
    }
    attrs
}

fn setattr_array_attrs(
    name: &str,
    readonly: bool,
    exported: bool,
    integers: &HashSet<String>,
    uppercase: &HashSet<String>,
    lowercase: &HashSet<String>,
) -> String {
    let mut attrs = String::from("-a");
    attrs.push_str(
        setattr_scalar_attrs(name, readonly, exported, integers, uppercase, lowercase)
            .trim_start_matches('-'),
    );
    attrs
}

fn mark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = marked_vars(env_vars, EXPORTED_VARS);
    exported.insert(name.to_string());
    let value = exported.into_iter().collect::<Vec<_>>().join("\x1f");
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

fn unmark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = marked_vars(env_vars, EXPORTED_VARS);
    exported.remove(name);
    let value = exported.into_iter().collect::<Vec<_>>().join("\x1f");
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

fn mark_readonly(env_vars: &mut HashMap<String, String>, name: &str) {
    // TODO(variables.c/variables.h): Bash stores readonly as att_readonly on
    // SHELL_VAR. Keep a side table until variables are real objects.
    let mut readonly = marked_vars(env_vars, READONLY_VARS);
    readonly.insert(name.to_string());
    env_vars.insert(
        READONLY_VARS.to_string(),
        readonly.into_iter().collect::<Vec<_>>().join("\x1f"),
    );
}

fn mark_array(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut arrays = marked_vars(env_vars, ARRAY_VARS);
    arrays.insert(name.to_string());
    env_vars.insert(
        ARRAY_VARS.to_string(),
        arrays.into_iter().collect::<Vec<_>>().join("\x1f"),
    );
}

fn marked_vars(env_vars: &HashMap<String, String>, key: &str) -> HashSet<String> {
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

fn nameref_target_name(env_vars: &HashMap<String, String>, name: &str) -> Option<String> {
    let mut current = name;
    let mut seen = HashSet::new();
    for _ in 0..16 {
        if !seen.insert(current.to_string())
            || !marked_vars(env_vars, NAMEREF_VARS).contains(current)
        {
            return None;
        }
        let target = env_vars.get(current)?;
        if !valid_identifier(target) {
            return None;
        }
        if !marked_vars(env_vars, NAMEREF_VARS).contains(target) {
            return Some(target.clone());
        }
        current = target;
    }
    None
}

fn is_array_value(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')')
}

fn array_attribute_assignment_value(
    value: &str,
    explicit_array: bool,
    env_vars: &HashMap<String, String>,
    name: &str,
) -> String {
    if let Some(compound) = value.strip_prefix(COMPOUND_ASSIGNMENT_MARKER) {
        return compound.to_string();
    }
    // TODO(array.c/variables.c): Bash distinguishes compound array syntax
    // from a quoted scalar assigned to an existing array. The lexer has removed
    // quote state by this point, so preserve attr.tests' existing-array shape.
    if !explicit_array
        && is_array_value(value)
        && env_vars
            .get(name)
            .is_some_and(|current| is_array_value(current))
    {
        return format!("({value})");
    }
    value.to_string()
}

fn readonly_error_subject(value: &str, explicit_array: bool) -> Option<String> {
    // TODO(builtins/setattr.def/variables.c/execute_cmd.c): Bash diagnostics
    // depend on whether assignment processing or the builtin detects the
    // readonly attribute. Preserve attr.tests' split until assignment words
    // carry full parse metadata.
    if explicit_array && value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
        return env::var("__RUBASH_CURRENT_FUNCTION").ok();
    }
    if explicit_array {
        return Some("readonly".to_string());
    }
    None
}

fn format_array_value(value: &str) -> String {
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered.to_string();
    }
    if value == "()" {
        return "()".to_string();
    }
    let inner = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value);
    format!("([0]=\"{}\")", quote_export_value(inner))
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

fn split_assignment(arg: &str) -> (&str, bool, Option<&str>) {
    match arg.find('=') {
        Some(index) => {
            let name = &arg[..index];
            let Some(base_name) = name.strip_suffix('+') else {
                return (name, false, Some(&arg[index + 1..]));
            };
            (base_name, true, Some(&arg[index + 1..]))
        }
        None => (arg, false, None),
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

fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}

fn quote_export_value(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn env_map() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn exports_assignment() {
        let mut vars = env_map();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let status = export_with_io(["NAME=value"], &mut vars, &mut stdout, &mut stderr).unwrap();

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(vars.get("NAME"), Some(&"value".to_string()));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn rejects_invalid_identifier() {
        let mut vars = env_map();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let status = export_with_io(["1BAD=value"], &mut vars, &mut stdout, &mut stderr).unwrap();

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(String::from_utf8(stderr)
            .unwrap()
            .contains("not a valid identifier"));
    }

    #[test]
    fn prints_exported_variables() {
        let mut vars = env_map();
        vars.insert("NAME".to_string(), "value".to_string());
        vars.insert(EXPORTED_VARS.to_string(), "NAME".to_string());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let status = export_with_io(["-p"], &mut vars, &mut stdout, &mut stderr).unwrap();

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "declare -x NAME=\"value\"\n"
        );
        assert!(stderr.is_empty());
    }
}
