use std::collections::{HashMap, HashSet};

use super::value::valid_identifier;
use super::{ARRAY_VARS, EXPORTED_VARS, NAMEREF_VARS, READONLY_VARS};

pub(super) fn mark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = marked_vars(env_vars, EXPORTED_VARS);
    exported.insert(name.to_string());
    let value = exported.into_iter().collect::<Vec<_>>().join("\x1f");
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

pub(super) fn unmark_exported(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut exported = marked_vars(env_vars, EXPORTED_VARS);
    exported.remove(name);
    let value = exported.into_iter().collect::<Vec<_>>().join("\x1f");
    env_vars.insert(EXPORTED_VARS.to_string(), value);
}

pub(super) fn mark_readonly(env_vars: &mut HashMap<String, String>, name: &str) {
    // TODO(variables.c/variables.h): Bash stores readonly as att_readonly on
    // SHELL_VAR. Keep a side table until variables are real objects.
    let mut readonly = marked_vars(env_vars, READONLY_VARS);
    readonly.insert(name.to_string());
    env_vars.insert(
        READONLY_VARS.to_string(),
        readonly.into_iter().collect::<Vec<_>>().join("\x1f"),
    );
}

pub(super) fn mark_array(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut arrays = marked_vars(env_vars, ARRAY_VARS);
    arrays.insert(name.to_string());
    env_vars.insert(
        ARRAY_VARS.to_string(),
        arrays.into_iter().collect::<Vec<_>>().join("\x1f"),
    );
}

pub(super) fn marked_vars(env_vars: &HashMap<String, String>, key: &str) -> HashSet<String> {
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
