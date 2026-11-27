use std::collections::HashSet;

use super::{ARRAY_VARS, ASSOC_VARS, EXPORTED_VARS, NAMEREF_VARS, READONLY_VARS};

/// GNU variables.h:124-133: attributes are att_* bits on the SHELL_VAR.
/// The writers below delegate to `VarTable`, whose structured attribute
/// map is authoritative; serialized marker strings are rendered on demand
/// (`VarTable::marked_names`) instead of rewritten per mark (#916).
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
    // Rendered from the structured flags (VarTable::marked_names), not
    // the serialized string: writers no longer maintain it per mark
    // (#916 follow-up). Non-attribute keys still read their string.
    env_vars.marked_names(key).into_iter().collect()
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
