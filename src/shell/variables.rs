//! variables module.
//!
//! GNU Bash source ownership:
// - variables.c
// - variables.h

use std::collections::HashMap;

pub const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";

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
        .map(|error| (current, error.message().to_string()))
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
