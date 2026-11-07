use std::io::{self, Write};

use super::marks::{
    mark_array, mark_assoc, mark_exported, mark_readonly, marked_vars, nameref_resolved_cell,
    unmark_exported,
};
use super::value::{
    array_attribute_assignment_value, diagnostic_prefix, readonly_error_prefix,
    readonly_error_subject, split_assignment, valid_identifier,
};
use super::{ExportMode, EXECUTION_FAILURE, EXECUTION_SUCCESS, READONLY_VARS};
use crate::executor::local_helpers::sync_shell_assignment_process_env;

/// The subject GNU's sh_invalidid names for a bad assignment operand
/// (rubash#1348): when the name portion parses as an array-element
/// reference — a valid identifier base followed by a `[...]` subscript —
/// the subject is that name portion (`a[0]` for `a[0]=x`); otherwise the
/// whole operand word is named (`9bad=v`, `9x[0]=v`, `a-b=v`).
fn invalid_id_subject<'a>(arg: &'a str, name: &'a str) -> &'a str {
    if name.ends_with(']') {
        if let Some((base, _)) = name.split_once('[') {
            if valid_identifier(base) {
                return name;
            }
        }
    }
    arg
}

pub(super) fn apply_export_arg<W>(
    arg: &str,
    mode: ExportMode,
    array: bool,
    assoc: bool,
    env_vars: &mut crate::shell::var_table::VarTable,
    stderr: &mut W,
    oob_word: &mut Option<String>,
    traced: Option<&mut Vec<(String, bool, String)>>,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    // The operand as written: GNU's per-assignment xtrace line
    // (subst.c:3580 xtrace_print_assignment via setattr.def's
    // do_assignment_no_expand) names the written name, even when the
    // bind lands on a nameref's resolved cell (rubash#1340).
    let written_name = name.to_string();
    let rhs_text = value.map(str::to_string);
    if !valid_identifier(name) {
        // GNU sh_invalidid's subject: for a word that parses as an
        // array-element assignment (valid identifier base + `[...]`),
        // the name-with-subscript is reported (`a[0]=x' complains about
        // `a[0]'); anything else reports the whole operand word
        // (`9bad=v', `9x[0]=v', `a-b=v') — rubash#1348.
        let diagnostic = format!(
            "{}export: `{}': not a valid identifier\n",
            diagnostic_prefix(),
            invalid_id_subject(arg, name)
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
                writeln!(
                    stderr,
                    "{}{}: readonly variable",
                    readonly_error_prefix(env_vars),
                    name
                )?;
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
            // The bind happened: record it for the caller's xtrace line
            // (subst.c:3580 — one line per performed assignment operand,
            // export/readonly only; declare/typeset/local do not trace
            // theirs). Compound operands are skipped by the emitter: the
            // declaration expansion phase already traced them (rubash#1340).
            if let (Some(traced), Some(rhs)) = (traced, rhs_text) {
                traced.push((written_name, append, rhs));
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
            // rubash#1692: an export VALUE bind is bind_variable —
            // sv_locale fires (variables.c:6272). A name-only export
            // merely marks att_exported in GNU (no bind, no warning),
            // so gate on has_assign like the sv_path flush below.
            if has_assign {
                if let Some(warning) = crate::locale::locale_assignment_warning(name, env_vars) {
                    writeln!(stderr, "{}warning: {}", diagnostic_prefix(), warning)?;
                }
            }
            // GNU variables.c:5819/5919 sv_path -> phash_flush
            // (rubash#1551): an export VALUE bind is a bind. A name-only
            // `export PATH` merely marks att_exported in GNU (no bind,
            // no flush) — the value unwrap above only materializes the
            // existing binding for the mirror, so gate on has_assign.
            if has_assign && name == "PATH" {
                crate::builtins::hash::flush_command_hash_table(env_vars);
            }
            mark_exported(env_vars, name);
        }
        ExportMode::Unset => {
            // GNU setattr.def set_or_show_attributes: an assignment
            // operand is bound FIRST (bind_variable) and only then is
            // att_exported removed, so `export -n name=value` keeps the
            // new value as a plain shell variable (rubash#1330). Dropping
            // the operand entirely lost the value.
            if let Some(assigned) = value {
                if env_vars.is_marked(READONLY_VARS, name) {
                    writeln!(
                        stderr,
                        "{}{}: readonly variable",
                        readonly_error_prefix(env_vars),
                        name
                    )?;
                    return Ok(EXECUTION_FAILURE);
                }
                let converted = env_vars.contains_key(name);
                let (new_value, bound_array) = array_attribute_assignment_value(
                    assigned, array, assoc, append, env_vars, name, oob_word,
                );
                env_vars.insert(name.to_string(), new_value);
                // GNU setattr.def binds the operand before removing
                // att_exported, so sv_path fires here too (rubash#1551).
                if name == "PATH" {
                    crate::builtins::hash::flush_command_hash_table(env_vars);
                }
                if assoc {
                    mark_assoc(env_vars, name, converted);
                } else if array || bound_array {
                    mark_array(env_vars, name);
                }
                if oob_word.is_some() {
                    return Ok(EXECUTION_FAILURE);
                }
            }
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
    traced: Option<&mut Vec<(String, bool, String)>>,
) -> io::Result<i32>
where
    W: Write,
{
    let (name, append, value) = split_assignment(arg);
    // Written operand name + RHS for the xtrace record (rubash#1340, see
    // apply_export_arg).
    let written_name = name.to_string();
    let rhs_text = value.map(str::to_string);
    if !valid_identifier(name) {
        // Same sh_invalidid subject rule as apply_export_arg
        // (rubash#1348): name-with-subscript for a valid-base element
        // form, the whole operand word otherwise.
        let diagnostic = format!(
            "{}readonly: `{}': not a valid identifier\n",
            diagnostic_prefix(),
            invalid_id_subject(arg, name)
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
            writeln!(
                stderr,
                "{}{}: readonly variable",
                readonly_error_prefix(env_vars),
                name
            )?;
        }
        return Ok(EXECUTION_FAILURE);
    }

    // #916: same fresh-name short-circuit as export above.
    if value.is_none() && !env_vars.contains_key(name) {
        // GNU set_var_attribute creates the missing variable through
        // bind_variable (setattr.def, variables.c:3211-3212), so under
        // set -a the creation bind also applies the mark_modified_vars
        // export mark (rubash#2944: `set -a; readonly r` -> declare -rx r).
        if crate::builtins::set::shell_option_enabled(env_vars, "allexport") {
            mark_exported(env_vars, name);
        }
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
    // The bind happened: record it for the caller's xtrace line
    // (subst.c:3580, rubash#1340 — see apply_export_arg).
    if let (Some(traced), Some(rhs)) = (traced, rhs_text) {
        traced.push((written_name, append, rhs));
    }
    // Same array-export contract as apply_export_arg above (GNU variables.c
    // make_env_array_from_var_list, rubash#534): a readonly array keeps the
    // attribute but its storage value never enters the child env. Mirrored
    // through the same gate as every other write (#916).
    sync_shell_assignment_process_env(env_vars, name, &value);
    // rubash#1692: a readonly VALUE bind is bind_variable — sv_locale
    // fires (variables.c:6272); name-only marks att_readonly only
    // (same has_assign gate as the sv_path flush below).
    if has_assign {
        if let Some(warning) = crate::locale::locale_assignment_warning(name, env_vars) {
            writeln!(stderr, "{}warning: {}", diagnostic_prefix(), warning)?;
        }
    }
    // GNU variables.c:5819/5919 sv_path -> phash_flush (rubash#1551):
    // a readonly VALUE bind is a bind; a name-only `readonly PATH`
    // only sets the attribute in GNU and must not flush (same
    // has_assign gate as export above).
    if has_assign && name == "PATH" {
        crate::builtins::hash::flush_command_hash_table(env_vars);
    }
    // rubash#2944: a valued readonly word binds through
    // do_assignment_no_expand -> bind_variable, whose tail applies the
    // mark_modified_vars export mark under set -a (variables.c:3211-3212)
    // — including a compound value bound without -a/-A (GNU shows
    // `readonly arr=(1 2)` as -arx). The -a/-A forms instead rewrite to
    // an array creation (setattr.def:240-258), which never marks, and a
    // name-only operand on an existing variable performs no bind at all
    // (set_var_attribute only sets the attribute), so neither marks here.
    if has_assign
        && !(array || assoc)
        && crate::builtins::set::shell_option_enabled(env_vars, "allexport")
    {
        mark_exported(env_vars, name);
    }
    mark_readonly(env_vars, name);
    Ok(EXECUTION_SUCCESS)
}
