use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

use super::marks::{
    mark_array, mark_exported, mark_readonly, marked_vars, nameref_target_name, unmark_exported,
};
use super::value::{
    array_attribute_assignment_value, diagnostic_prefix, eval_arith_value, is_array_value,
    readonly_error_subject, split_assignment, valid_identifier,
};
use super::{ExportMode, EXECUTION_FAILURE, EXECUTION_SUCCESS, INTEGER_VARS, READONLY_VARS};

pub(super) fn apply_export_arg<W>(
    arg: &str,
    mode: ExportMode,
    array: bool,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    if !valid_identifier(name) {
        writeln!(stderr, "rubash: export: `{}`: not a valid identifier", arg)?;
        return Ok(EXECUTION_FAILURE);
    }
    let resolved_name = nameref_target_name(env_vars, name).unwrap_or_else(|| name.to_string());
    let name = resolved_name.as_str();

    match mode {
        ExportMode::Set => {
            if value.is_some() && marked_vars(env_vars, READONLY_VARS).contains(name) {
                writeln!(stderr, "{}{}: readonly variable", diagnostic_prefix(), name)?;
                return Ok(EXECUTION_FAILURE);
            }

            if value.is_none() && !env_vars.contains_key(name) && env::var(name).is_err() {
                mark_exported(env_vars, name);
                return Ok(EXECUTION_SUCCESS);
            }

            let value = value
                .map(|value| array_attribute_assignment_value(value, array, env_vars, name))
                .or_else(|| env_vars.get(name).cloned())
                .or_else(|| env::var(name).ok())
                .unwrap_or_default();
            let value = if append {
                let mut current = env_vars.get(name).cloned().unwrap_or_default();
                if marked_vars(env_vars, INTEGER_VARS).contains(name) {
                    (eval_arith_value(&current) + eval_arith_value(&value)).to_string()
                } else {
                    current.push_str(&value);
                    current
                }
            } else {
                value
            };
            env_vars.insert(name.to_string(), value.clone());
            env::set_var(name, value);
            mark_exported(env_vars, name);
            if array || is_array_value(env_vars.get(name).map(String::as_str).unwrap_or("")) {
                mark_array(env_vars, name);
            }
        }
        ExportMode::Unset => {
            env::remove_var(name);
            unmark_exported(env_vars, name);
        }
    }

    Ok(EXECUTION_SUCCESS)
}

pub(super) fn apply_readonly_arg<W>(
    arg: &str,
    array: bool,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    if !valid_identifier(name) {
        writeln!(
            stderr,
            "{}readonly: `{}`: not a valid identifier",
            diagnostic_prefix(),
            arg
        )?;
        return Ok(EXECUTION_FAILURE);
    }
    let resolved_name = nameref_target_name(env_vars, name).unwrap_or_else(|| name.to_string());
    let name = resolved_name.as_str();

    let readonly = marked_vars(env_vars, READONLY_VARS);
    if readonly.contains(name) && value.is_some() {
        if let Some(subject) = readonly_error_subject(value.unwrap_or_default(), array) {
            writeln!(
                stderr,
                "{}{}: {}: readonly variable",
                diagnostic_prefix(),
                subject,
                name
            )?;
        } else {
            writeln!(stderr, "{}{}: readonly variable", diagnostic_prefix(), name)?;
        }
        return Ok(EXECUTION_FAILURE);
    }

    if value.is_none() && !env_vars.contains_key(name) && env::var(name).is_err() {
        mark_readonly(env_vars, name);
        return Ok(EXECUTION_SUCCESS);
    }

    let value = value
        .map(|value| array_attribute_assignment_value(value, array, env_vars, name))
        .or_else(|| env_vars.get(name).cloned())
        .or_else(|| env::var(name).ok())
        .unwrap_or_default();
    let value = if append {
        let mut current = env_vars.get(name).cloned().unwrap_or_default();
        if marked_vars(env_vars, INTEGER_VARS).contains(name) {
            (eval_arith_value(&current) + eval_arith_value(&value)).to_string()
        } else {
            current.push_str(&value);
            current
        }
    } else {
        value
    };
    env_vars.insert(name.to_string(), value.clone());
    env::set_var(name, value);
    mark_readonly(env_vars, name);
    if array || is_array_value(env_vars.get(name).map(String::as_str).unwrap_or("")) {
        mark_array(env_vars, name);
    }
    Ok(EXECUTION_SUCCESS)
}
