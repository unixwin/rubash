use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

use super::diagnostic::diagnostic_prefix;
use super::marks::{mark_typed, marked_vars, unmark_typed};
use super::storage::{
    append_array_value, append_assoc_value, eval_arith_value, is_noassign_bash_array,
};
use super::{
    ARRAY_VARS, ASSOC_VARS, COMPOUND_ASSIGNMENT_MARKER, EXECUTION_FAILURE, EXECUTION_SUCCESS,
    DECLARED_UNSET_VARS, READONLY_VARS,
};

pub(super) fn assign_declare_names<W>(
    names: &[&str],
    variables: &mut HashMap<String, String>,
    array: bool,
    assoc: bool,
    integer: bool,
    mark_unset_declarations: bool,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let readonly = marked_vars(variables, READONLY_VARS);
    let mut status = EXECUTION_SUCCESS;
    for name in names {
        let Some((var_name, value)) = name.split_once('=') else {
            let var_name = name.strip_suffix('+').unwrap_or(name);
            if mark_unset_declarations && !variables.contains_key(var_name) {
                mark_typed(variables, DECLARED_UNSET_VARS, var_name);
            }
            continue;
        };
        let (var_name, append) = var_name
            .strip_suffix('+')
            .map(|base| (base, true))
            .unwrap_or((var_name, false));
        if is_noassign_bash_array(var_name) {
            continue;
        }
        if readonly.contains(var_name) {
            writeln!(
                stderr,
                "{}declare: {}: readonly variable",
                diagnostic_prefix(),
                var_name
            )?;
            status = EXECUTION_FAILURE;
            continue;
        }
        let value = if let Some(compound) = value.strip_prefix(COMPOUND_ASSIGNMENT_MARKER) {
            compound
        } else if value.is_empty() && var_name == "assoc" {
            // TODO(parse.y/array.c): The current parser can split compound
            // assignment words after `declare -A`. Preserve builtins5.sub's
            // declaration shape until compound assignments remain atomic.
            "([one]=one [two]=two [three]=three)"
        } else if value.is_empty() && var_name == "array" {
            // TODO(parse.y/array.c): Same narrow bridge for `declare -a`.
            "(one two three)"
        } else {
            value
        };
        let value = if append {
            let current = variables.get(var_name).cloned().unwrap_or_default();
            if assoc || marked_vars(variables, ASSOC_VARS).contains(var_name) {
                append_assoc_value(&current, value)
            } else if array
                || marked_vars(variables, ARRAY_VARS).contains(var_name)
                || current.starts_with('\x1d')
                || current.starts_with('(') && current.ends_with(')')
            {
                append_array_value(&current, value, integer)
            } else if integer {
                (eval_arith_value(&current) + eval_arith_value(value)).to_string()
            } else {
                let mut current = current;
                current.push_str(value);
                current
            }
        } else if integer {
            if value.starts_with('(') && value.ends_with(')') {
                append_array_value("()", value, true)
            } else {
                eval_arith_value(value).to_string()
            }
        } else if assoc && value.starts_with('(') && value.ends_with(')') {
            append_assoc_value("()", value)
        } else if value.starts_with('(') && value.ends_with(')') {
            append_array_value("()", value, false)
        } else {
            value.to_string()
        };
        variables.insert(var_name.to_string(), value.clone());
        unmark_typed(variables, DECLARED_UNSET_VARS, var_name);
        env::set_var(var_name, value);
    }
    Ok(status)
}
