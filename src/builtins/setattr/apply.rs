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
use crate::executor::local_helpers::sync_shell_assignment_process_env;

pub(super) fn apply_export_arg<W>(
    arg: &str,
    mode: ExportMode,
    array: bool,
    assoc: bool,
    env_vars: &mut crate::shell::var_table::VarTable,
    stderr: &mut W,
    oob_word: &mut Option<String>,
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
            if value.is_some() && env_vars.is_marked(READONLY_VARS, name) {
                writeln!(stderr, "{}{}: readonly variable", diagnostic_prefix(), name)?;
                return Ok(EXECUTION_FAILURE);
            }

            // #916: fresh names short-circuit on contains_key; env_vars is seeded
            // from the process env at startup, so a preset name is always present
            // here and never reaches the fallback. The old env::var probe paid a
            // locked O(n) environ scan per fresh export and resurrected stale
            // process values after unset (bash leaves them empty).
            if value.is_none() && !env_vars.contains_key(name) {
                mark_exported(env_vars, name);
                return Ok(EXECUTION_SUCCESS);
            }

            // GNU setattr.def:240-258: `export -a/-A name=value` is
            // rewritten as `declare -gx{a,A} name=value`, so the array
            // attribute is applied only for an explicit assignment word;
            // `export -a name` alone just marks att_exported.
            let has_assign = value.is_some();
            let converted = env_vars.contains_key(name);
            let (value, bound_array) = value
                .map(|value| {
                    array_attribute_assignment_value(
                        value, array, assoc, append, env_vars, name, oob_word,
                    )
                })
                .unwrap_or_else(|| {
                    (
                        env_vars.get(name).cloned().unwrap_or_default(),
                        array || assoc,
                    )
                });
            env_vars.insert(name.to_string(), value.clone());
            if assoc && has_assign {
                mark_assoc(env_vars, name, converted);
            } else if (array && has_assign) || bound_array {
                mark_array(env_vars, name);
            }
            // GNU err_badarraysub + any_failed (arrayfunc.c:804-809,
            // rubash#635): the partial store and marks above stay; fail the
            // builtin and stop here. The recorded word travels up for the
            // bare report_error diagnostic plus the DISCARD list-abort at
            // the Executor layer, exactly like declare/local (subst.c:11169
            // bash_variable_assignment_error).
            if oob_word.is_some() {
                return Ok(EXECUTION_FAILURE);
            }
            // GNU variables.c make_env_array_from_var_list: array and
            // associative variables never cross the child env boundary
            // (ARRAY_EXPORT off), so `export A` / `export -a A=(x y)` on an
            // array keeps the exported attribute (declare -p shows -a -x)
            // while the storage-encoded value stays out of the child env
            // (rubash#534: printenv A must print nothing). Scalar values
            // cross via the explicit child env; mirror to the process env
            // only when exported-or-special, like plain assignments (#916).
            sync_shell_assignment_process_env(env_vars, name, &value);
            mark_exported(env_vars, name);
        }
        ExportMode::Unset => {
            // Existence-gated process-env removal (#916/#917): the name
            // gate only applies to writes; an inherited copy must be
            // dropped so table-miss fallbacks do not resurrect it.
            crate::executor::local_helpers::unset_process_env(name);
            unmark_exported(env_vars, name);
        }
    }

    Ok(EXECUTION_SUCCESS)
}

pub(super) fn apply_readonly_arg<W>(
    arg: &str,
    array: bool,
    assoc: bool,
    env_vars: &mut crate::shell::var_table::VarTable,
    stderr: &mut W,
    context_name: Option<&str>,
    oob_word: &mut Option<String>,
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

    // #916: same fresh-name short-circuit as export above.
    if value.is_none() && !env_vars.contains_key(name) {
        mark_readonly(env_vars, name);
        return Ok(EXECUTION_SUCCESS);
    }

    // GNU setattr.def:240-258: `readonly -a/-A name=value` is rewritten as
    // `declare -gr{a,A} name=value`; without `=` the flags only filter the
    // printed list, so the marks below need the assignment word.
    let has_assign = value.is_some();
    let converted = env_vars.contains_key(name);
    let (value, bound_array) = value
        .map(|value| {
            array_attribute_assignment_value(value, array, assoc, append, env_vars, name, oob_word)
        })
        .unwrap_or_else(|| {
            (
                env_vars.get(name).cloned().unwrap_or_default(),
                array || assoc,
            )
        });
    env_vars.insert(name.to_string(), value.clone());
    if assoc && has_assign {
        mark_assoc(env_vars, name, converted);
    } else if (array && has_assign) || bound_array {
        mark_array(env_vars, name);
    }
    // Same OOB stop as export above (arrayfunc.c:804-809, rubash#635):
    // fail here, diagnostic + DISCARD abort at the Executor layer.
    if oob_word.is_some() {
        return Ok(EXECUTION_FAILURE);
    }
    // Same array-export contract as apply_export_arg above (GNU variables.c
    // make_env_array_from_var_list, rubash#534): a readonly array keeps the
    // attribute but its storage value never enters the child env. Mirrored
    // through the same gate as every other write (#916).
    sync_shell_assignment_process_env(env_vars, name, &value);
    mark_readonly(env_vars, name);
    Ok(EXECUTION_SUCCESS)
}
