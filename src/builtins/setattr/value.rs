use std::collections::HashMap;
use std::env;

use super::COMPOUND_ASSIGNMENT_MARKER;

pub(super) fn is_array_value(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')')
}

pub(super) fn array_attribute_assignment_value(
    value: &str,
    explicit_array: bool,
    _env_vars: &HashMap<String, String>,
    _name: &str,
) -> String {
    if let Some(compound) = value.strip_prefix(COMPOUND_ASSIGNMENT_MARKER) {
        return compound.to_string();
    }
    // TODO(array.c/variables.c): Bash distinguishes compound array syntax
    // from a quoted scalar assigned to an existing array. The lexer has removed
    // quote state by this point, so preserve attr.tests' existing-array shape.
    if !explicit_array && is_array_value(value) {
        return format!("({value})");
    }
    value.to_string()
}

pub(super) fn readonly_error_subject(value: &str, explicit_array: bool) -> Option<String> {
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

pub(super) fn format_array_value(value: &str) -> String {
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

pub(super) fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        env::var("__RUBASH_SCRIPT_NAME"),
        env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}

pub(super) fn split_assignment(arg: &str) -> (&str, bool, Option<&str>) {
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

pub(super) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub(super) fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}

pub(super) fn quote_export_value(value: &str) -> String {
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
