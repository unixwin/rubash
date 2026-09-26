use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

use super::marks::{
    mark_array, mark_assoc, mark_exported, mark_readonly, marked_vars, nameref_resolved_cell,
    unmark_exported,
};
use super::value::{
    array_attribute_assignment_value, diagnostic_prefix, readonly_error_subject, split_assignment,
    valid_identifier,
};
use super::{ExportMode, EXECUTION_FAILURE, EXECUTION_SUCCESS, READONLY_VARS};

pub(super) fn apply_export_arg<W>(
    arg: &str,
    mode: ExportMode,
    array: bool,
    assoc: bool,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    if !valid_identifier(name) {
        let diagnostic = format!(
            "{}export: `{}': not a valid identifier\n",
            diagnostic_prefix(),
            arg
        );
        stderr.write_all(diagnostic.as_bytes())?;
        return Ok(EXECUTION_FAILURE);
    }
    let resolved_name = nameref_resolved_cell(env_vars, name).unwrap_or_else(|| name.to_string());
    let name = resolved_name.as_str();
    // GNU builtins/setattr.def:651 + variables.c:2201-2204: attribute
    // builtins applied THROUGH a nameref validate the resolved cell —
    // `typeset -n ref='var[0]'; export ref' reports
    // `export: 'var[0]': not a valid identifier` (sh_invalidid on the
    // cell), not a silent mark on a literal "var[0]" key.
    if !valid_identifier(name) {
        let diagnostic = format!(
            "{}export: `{name}': not a valid identifier
",
            diagnostic_prefix()
        );
        stderr.write_all(diagnostic.as_bytes())?;
        // set_var_attribute returns void here without bumping
        // any_failed, so set_or_show_attributes reports SUCCESS.
        return Ok(EXECUTION_SUCCESS);
    }

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

            // GNU setattr.def:240-258: `export -a/-A name=value` is
            // rewritten as `declare -gx{a,A} name=value`, so the array
            // attribute is applied only for an explicit assignment word;
            // `export -a name` alone just marks att_exported.
            let has_assign = value.is_some();
            let converted = env_vars.contains_key(name) || env::var(name).is_ok();
            let (value, bound_array) = value
                .map(|value| {
                    array_attribute_assignment_value(value, array, assoc, append, env_vars, name)
                })
                .unwrap_or_else(|| {
                    (
                        env_vars
                            .get(name)
                            .cloned()
                            .or_else(|| env::var(name).ok())
                            .unwrap_or_default(),
                        array || assoc,
                    )
                });
            env_vars.insert(name.to_string(), value.clone());
            env::set_var(name, value);
            mark_exported(env_vars, name);
            if assoc && has_assign {
                mark_assoc(env_vars, name, converted);
            } else if (array && has_assign) || bound_array {
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
    assoc: bool,
    env_vars: &mut HashMap<String, String>,
    stderr: &mut W,
    context_name: Option<&str>,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    if !valid_identifier(name) {
        if name.ends_with(']') {
            if let Some((base, _)) = name.split_once('[') {
                if valid_identifier(base) {
                    let diagnostic = format!(
                        "{}readonly: `{}': not a valid identifier\n",
                        diagnostic_prefix(),
                        arg
                    );
                    stderr.write_all(diagnostic.as_bytes())?;
                    return Ok(EXECUTION_FAILURE);
                }
            }
        }
        let diagnostic = format!(
            "{}readonly: `{}': not a valid identifier\n",
            diagnostic_prefix(),
            arg
        );
        stderr.write_all(diagnostic.as_bytes())?;
        return Ok(EXECUTION_FAILURE);
    }
    let resolved_name = nameref_resolved_cell(env_vars, name).unwrap_or_else(|| name.to_string());
    let name = resolved_name.as_str();
    // Same through-nameref cell validation as apply_export_arg above
    // (setattr.def:651 -> variables.c:2201-2204): `readonly ref` with
    // ref -> 'var[0]' reports `readonly: 'var[0]': not a valid
    // identifier` and leaves the nameref untouched.
    if !valid_identifier(name) {
        let diagnostic = format!(
            "{}readonly: `{name}': not a valid identifier
",
            diagnostic_prefix()
        );
        stderr.write_all(diagnostic.as_bytes())?;
        // set_var_attribute returns void here without bumping
        // any_failed, so set_or_show_attributes reports SUCCESS.
        return Ok(EXECUTION_SUCCESS);
    }

    let readonly = marked_vars(env_vars, READONLY_VARS);
    if readonly.contains(name) && value.is_some() {
        if let Some(subject) =
            readonly_error_subject(value.unwrap_or_default(), array || assoc, context_name)
        {
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

    // GNU setattr.def:240-258: `readonly -a/-A name=value` is rewritten as
    // `declare -gr{a,A} name=value`; without `=` the flags only filter the
    // printed list, so the marks below need the assignment word.
    let has_assign = value.is_some();
    let converted = env_vars.contains_key(name) || env::var(name).is_ok();
    let (value, bound_array) = value
        .map(|value| array_attribute_assignment_value(value, array, assoc, append, env_vars, name))
        .unwrap_or_else(|| {
            (
                env_vars
                    .get(name)
                    .cloned()
                    .or_else(|| env::var(name).ok())
                    .unwrap_or_default(),
                array || assoc,
            )
        });
    env_vars.insert(name.to_string(), value.clone());
    env::set_var(name, value);
    mark_readonly(env_vars, name);
    if assoc && has_assign {
        mark_assoc(env_vars, name, converted);
    } else if (array && has_assign) || bound_array {
        mark_array(env_vars, name);
    }
    Ok(EXECUTION_SUCCESS)
}
