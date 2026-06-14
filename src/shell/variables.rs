//! variables module.
//!
//! GNU Bash source ownership:
// - variables.c
// - variables.h

use std::collections::HashMap;

pub const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
pub const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
pub const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
pub const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
pub const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";

pub fn is_internal_attribute_table(name: &str) -> bool {
    matches!(
        name,
        INTEGER_VARS | EXPORTED_VARS | ARRAY_VARS | ASSOC_VARS | READONLY_VARS
    )
}

pub fn integer_append_assignment_error(
    name: &str,
    variables: &HashMap<String, String>,
) -> Option<(String, String)> {
    // TODO(variables.c/expr.c): Bash evaluates the existing integer value
    // before append assignment. If that expression is invalid, the whole
    // assignment command fails and later assignment words are not applied.
    let base_name = name.strip_suffix('+')?;
    if !marked_vars(variables, INTEGER_VARS)
        .iter()
        .any(|marked| marked == base_name)
    {
        return None;
    }
    let current = variables.get(base_name).cloned().unwrap_or_default();
    if current.trim().is_empty() || crate::shell::arrays::indexed::is_storage(&current) {
        return None;
    }
    let mut vars = variables.clone();
    crate::expand::arithmetic::eval(&current, &mut vars)
        .err()
        .map(|error| {
            (
                current.clone(),
                integer_append_error_message(&current, error.message()),
            )
        })
}

fn integer_append_error_message(expr: &str, message: &str) -> String {
    if expr.trim_end().ends_with('+') && message == "operand expected" {
        return "arithmetic syntax error: operand expected (error token is \"+\")".to_string();
    }
    message.to_string()
}

fn marked_vars(variables: &HashMap<String, String>, key: &str) -> Vec<String> {
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
