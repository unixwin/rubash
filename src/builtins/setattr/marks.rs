use std::collections::HashSet;

use super::value::valid_identifier;
use super::{ARRAY_VARS, ASSOC_VARS, EXPORTED_VARS, NAMEREF_VARS, READONLY_VARS};
use crate::executor::markers::DATA_DOLLAR;

/// GNU variables.h:124-133: attributes are att_* bits on the SHELL_VAR.
/// The writers below delegate to `VarTable`, which keeps the structured
/// attribute map authoritative and the `__RUBASH_*_VARS` marker strings
/// (the serialization child shells and list consumers read) synchronized
/// bit-for-bit.
pub(super) fn mark_exported(env_vars: &mut crate::shell::var_table::VarTable, name: &str) {
    env_vars.mark_name(EXPORTED_VARS, name);
}

pub(super) fn unmark_exported(env_vars: &mut crate::shell::var_table::VarTable, name: &str) {
    env_vars.unmark_name(EXPORTED_VARS, name);
}

pub(super) fn mark_readonly(env_vars: &mut crate::shell::var_table::VarTable, name: &str) {
    env_vars.mark_name(READONLY_VARS, name);
}

pub(super) fn mark_array(env_vars: &mut crate::shell::var_table::VarTable, name: &str) {
    env_vars.mark_name(ARRAY_VARS, name);
}

/// GNU setattr.def:240-258 rewrites `readonly -A name=value` /
/// `export -A name=value` into `declare -g{r,x}A name=value`, so the
/// assoc attribute follows declare.def's table creation: an existing
/// variable converts through convert_var_to_assoc (assoc_create(0),
/// DEFAULT_HASH_BUCKETS=128, arrayfunc.c:114-117/hashlib.h:72), while a
/// fresh name gets ASSOC_HASH_BUCKETS=1024 (variables.c:2857, assoc.h:28).
pub(super) fn mark_assoc(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    converted: bool,
) {
    env_vars.mark_name(ASSOC_VARS, name);
    env_vars.unmark_name(ARRAY_VARS, name);
    // ASSOC_128_VARS is list-shaped bookkeeping (the assoc hash table's
    // bucket count), not a SHELL_VAR attribute — it stays in the marker
    // string layer only.
    if converted {
        env_vars.mark_name(crate::executor::types::ASSOC_128_VARS, name);
    } else {
        env_vars.unmark_name(crate::executor::types::ASSOC_128_VARS, name);
    }
}

pub(super) fn marked_vars(
    env_vars: &crate::shell::var_table::VarTable,
    key: &str,
) -> HashSet<String> {
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
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
) -> Option<String> {
    let mut current = name;
    let mut seen = HashSet::new();
    for _ in 0..16 {
        if !seen.insert(current.to_string()) || !env_vars.is_marked(NAMEREF_VARS, current) {
            return None;
        }
        let target = env_vars.get(current)?;
        if !valid_identifier(target) {
            return None;
        }
        if !env_vars.is_marked(NAMEREF_VARS, target) {
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
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
) -> Option<String> {
    let mut current = name;
    for _ in 0..8 {
        if !env_vars.is_marked(NAMEREF_VARS, current) {
            return None;
        }
        let target = env_vars.get(current)?;
        if !env_vars.is_marked(NAMEREF_VARS, target) {
            return Some(target.clone());
        }
        current = target;
    }
    None
}
