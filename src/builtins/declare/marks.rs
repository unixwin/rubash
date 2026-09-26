use std::collections::{HashMap, HashSet};

use super::{ARRAY_VARS, ASSOC_128_VARS, ASSOC_VARS, EXPORTED_VARS};
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};

pub(super) fn mark_exported(variables: &mut HashMap<String, String>, name: &str) {
    // TODO(variables.c/variables.h): Bash stores export as a variable
    // attribute. Keep a side table until Rubash has a real SHELL_VAR model.
    let mut exported = exported_vars(variables);
    exported.insert(name.to_string());
    let value = exported
        .into_iter()
        .collect::<Vec<_>>()
        .join(DATA_DOLLAR_STR);
    variables.insert(EXPORTED_VARS.to_string(), value);
}

pub(super) fn mark_array(variables: &mut HashMap<String, String>, name: &str) {
    mark_typed(variables, ARRAY_VARS, name);
    unmark_typed(variables, ASSOC_VARS, name);
    // The assoc hash table is discarded with the attribute; its bucket count
    // must not survive into a later re-declaration.
    unmark_typed(variables, ASSOC_128_VARS, name);
}

pub(super) fn mark_assoc(variables: &mut HashMap<String, String>, name: &str) {
    mark_typed(variables, ASSOC_VARS, name);
    unmark_typed(variables, ARRAY_VARS, name);
}

pub(super) fn mark_typed(variables: &mut HashMap<String, String>, key: &str, name: &str) {
    let mut marked = marked_vars(variables, key);
    marked.insert(name.to_string());
    variables.insert(
        key.to_string(),
        marked.into_iter().collect::<Vec<_>>().join(DATA_DOLLAR_STR),
    );
}

pub(super) fn unmark_typed(variables: &mut HashMap<String, String>, key: &str, name: &str) {
    let mut marked = marked_vars(variables, key);
    marked.remove(name);
    variables.insert(
        key.to_string(),
        marked.into_iter().collect::<Vec<_>>().join(DATA_DOLLAR_STR),
    );
}

pub(super) fn marked_vars(variables: &HashMap<String, String>, key: &str) -> HashSet<String> {
    variables
        .get(key)
        .map(|value| {
            value
                .split(DATA_DOLLAR)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn exported_vars(variables: &HashMap<String, String>) -> HashSet<String> {
    variables
        .get(EXPORTED_VARS)
        .map(|value| {
            value
                .split(DATA_DOLLAR)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
