use std::collections::{HashMap, HashSet};

use super::value::valid_identifier;
use super::{ARRAY_VARS, ASSOC_VARS, EXPORTED_VARS, NAMEREF_VARS, READONLY_VARS};
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};

pub(super) fn mark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = marked_vars(env_vars, EXPORTED_VARS);
    exported.insert(name.to_string());
    let value = exported
        .into_iter()
        .collect::<Vec<_>>()
        .join(DATA_DOLLAR_STR);
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

pub(super) fn unmark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = marked_vars(env_vars, EXPORTED_VARS);
    exported.remove(name);
    let value = exported
        .into_iter()
        .collect::<Vec<_>>()
        .join(DATA_DOLLAR_STR);
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

pub(super) fn mark_readonly(env_vars: &mut HashMap<String, String>, name: &str) {
    // TODO(variables.c/variables.h): Bash stores readonly as att_readonly on
    // SHELL_VAR. Keep a side table until variables are real objects.
    let mut readonly = marked_vars(env_vars, READONLY_VARS);
    readonly.insert(name.to_string());
    env_vars.insert(
        READONLY_VARS.to_string(),
        readonly
            .into_iter()
            .collect::<Vec<_>>()
            .join(DATA_DOLLAR_STR),
    );
}

pub(super) fn mark_array(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut arrays = marked_vars(env_vars, ARRAY_VARS);
    arrays.insert(name.to_string());
    env_vars.insert(
        ARRAY_VARS.to_string(),
        arrays.into_iter().collect::<Vec<_>>().join(DATA_DOLLAR_STR),
    );
}

/// GNU setattr.def:240-258 rewrites `readonly -A name=value` /
/// `export -A name=value` into `declare -g{r,x}A name=value`, so the
/// assoc attribute follows declare.def's table creation: an existing
/// variable converts through convert_var_to_assoc (assoc_create(0),
/// DEFAULT_HASH_BUCKETS=128, arrayfunc.c:114-117/hashlib.h:72), while a
/// fresh name gets ASSOC_HASH_BUCKETS=1024 (variables.c:2857, assoc.h:28).
pub(super) fn mark_assoc(env_vars: &mut HashMap<String, String>, name: &str, converted: bool) {
    let mut assoc = marked_vars(env_vars, ASSOC_VARS);
    assoc.insert(name.to_string());
    env_vars.insert(
        ASSOC_VARS.to_string(),
        assoc.into_iter().collect::<Vec<_>>().join(DATA_DOLLAR_STR),
    );
    let mut arrays = marked_vars(env_vars, ARRAY_VARS);
    arrays.remove(name);
    env_vars.insert(
        ARRAY_VARS.to_string(),
        arrays.into_iter().collect::<Vec<_>>().join(DATA_DOLLAR_STR),
    );
    let mut assoc128 = marked_vars(env_vars, crate::executor::types::ASSOC_128_VARS);
    if converted {
        assoc128.insert(name.to_string());
    } else {
        assoc128.remove(name);
    }
    env_vars.insert(
        crate::executor::types::ASSOC_128_VARS.to_string(),
        assoc128
            .into_iter()
            .collect::<Vec<_>>()
            .join(DATA_DOLLAR_STR),
    );
}

pub(super) fn marked_vars(env_vars: &HashMap<String, String>, key: &str) -> HashSet<String> {
    env_vars
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

pub(super) fn nameref_target_name(
    env_vars: &HashMap<String, String>,
    name: &str,
) -> Option<String> {
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

/// GNU builtins/setattr.def:651 + variables.c:2188-2205
/// find_variable_nameref_for_create: attribute builtins resolve a nameref
/// chain to its FINAL cell verbatim — including cells that are not valid
/// identifiers (`ref` -> `var[0]`), which the caller then rejects with
/// sh_invalidid. Unlike nameref_target_name this does not pre-validate
/// the target.
pub(super) fn nameref_resolved_cell(
    env_vars: &HashMap<String, String>,
    name: &str,
) -> Option<String> {
    let mut current = name;
    for _ in 0..8 {
        if !marked_vars(env_vars, NAMEREF_VARS).contains(current) {
            return None;
        }
        let target = env_vars.get(current)?;
        if !marked_vars(env_vars, NAMEREF_VARS).contains(target) {
            return Some(target.clone());
        }
        current = target;
    }
    None
}
