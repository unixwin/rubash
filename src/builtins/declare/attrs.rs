use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

use super::diagnostic::diagnostic_prefix;
use super::marks::{mark_array, mark_assoc, mark_exported, mark_typed, marked_vars, unmark_typed};
use super::storage::{
    eval_arith_value, format_indexed_array_storage, indexed_array_entries, parse_array_words,
};
use super::{
    ARRAY_VARS, ASSOC_VARS, EXECUTION_FAILURE, EXPORTED_VARS, INTEGER_VARS, LOWERCASE_VARS,
    NAMEREF_VARS, READONLY_VARS, UPPERCASE_VARS,
};

#[derive(Clone, Copy)]
pub(super) struct DeclareOptions {
    pub(super) export: bool,
    pub(super) array: bool,
    pub(super) assoc: bool,
    pub(super) integer: bool,
    pub(super) uppercase: bool,
    pub(super) lowercase: bool,
    pub(super) nameref: bool,
    pub(super) readonly: bool,
    pub(super) unset_export: bool,
    pub(super) unset_array: bool,
    pub(super) unset_assoc: bool,
    pub(super) unset_integer: bool,
    pub(super) unset_uppercase: bool,
    pub(super) unset_lowercase: bool,
    pub(super) unset_nameref: bool,
    pub(super) unset_readonly: bool,
}

pub(super) fn apply_declare_attrs<W>(
    names: &[&str],
    variables: &mut HashMap<String, String>,
    options: DeclareOptions,
    mut attr_status: i32,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let DeclareOptions {
        export,
        array,
        assoc,
        integer,
        uppercase,
        lowercase,
        nameref,
        readonly,
        unset_export,
        unset_array,
        unset_assoc,
        unset_integer,
        unset_uppercase,
        unset_lowercase,
        unset_nameref,
        unset_readonly,
    } = options;
    if unset_export
        || unset_array
        || unset_assoc
        || unset_integer
        || unset_uppercase
        || unset_lowercase
        || unset_nameref
        || unset_readonly
    {
        let arrays = marked_vars(variables, ARRAY_VARS);
        let assocs = marked_vars(variables, ASSOC_VARS);
        for name in names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            if unset_readonly && marked_vars(variables, READONLY_VARS).contains(name) {
                writeln!(
                    stderr,
                    "{}declare: {}: readonly variable",
                    diagnostic_prefix(),
                    name
                )?;
                attr_status = EXECUTION_FAILURE;
            }
            if (unset_array && arrays.contains(name)) || (unset_assoc && assocs.contains(name)) {
                writeln!(
                    stderr,
                    "{}declare: {}: cannot destroy array variables in this way",
                    diagnostic_prefix(),
                    name
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
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
            if unset_nameref {
                unmark_typed(variables, NAMEREF_VARS, name);
            }
        }
    }
    if array || assoc {
        for name in names {
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
        for name in names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            mark_typed(variables, INTEGER_VARS, name);
            if let Some(value) = variables.get(name).cloned() {
                let value = if value.starts_with('\x1d') {
                    let mut entries = indexed_array_entries(&value);
                    for element in entries.values_mut() {
                        *element = eval_arith_value(element).to_string();
                    }
                    format_indexed_array_storage(entries)
                } else if value.starts_with('(') && value.ends_with(')') {
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
        for name in names {
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
        for name in names {
            let has_assignment = name.contains('=');
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            if let Some(value) = variables.get(name).cloned().or_else(|| env::var(name).ok()) {
                variables.insert(name.to_string(), value.clone());
                env::set_var(name, value);
                mark_exported(variables, name);
            } else if has_assignment {
                variables.insert((*name).to_string(), String::new());
                env::set_var(name, "");
                mark_exported(variables, name);
            } else {
                mark_exported(variables, name);
            }
        }
    }
    if nameref {
        for name in names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            mark_typed(variables, NAMEREF_VARS, name);
        }
    }
    if readonly {
        for name in names {
            let has_assignment = name.contains('=');
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            if let Some(value) = variables.get(name).cloned().or_else(|| env::var(name).ok()) {
                variables.insert(name.to_string(), value);
            } else if has_assignment {
                variables.entry(name.to_string()).or_default();
            }
            mark_typed(variables, READONLY_VARS, name);
        }
    }
    Ok(attr_status)
}
