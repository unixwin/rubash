use std::collections::HashMap;
use std::io::{self, Write};

use super::diagnostic::diagnostic_prefix;
use super::marks::{mark_typed, marked_vars, unmark_typed};
use super::storage::{
    append_array_value, append_assoc_value, eval_arith_value, format_indexed_array_storage,
    indexed_array_entries, is_noassign_bash_array, parse_array_tokens,
};
use super::{
    ARRAY_VARS, ASSOC_VARS, COMPOUND_ASSIGNMENT_MARKER, DECLARED_UNSET_VARS, EXECUTION_FAILURE,
    EXECUTION_SUCCESS, INTEGER_VARS, NAMEREF_VARS, READONLY_VARS,
};
use super::names::valid_nameref_value;
use crate::executor::arithmetic::eval_conditional_arith_value;

pub(super) fn assign_declare_names<W>(
    command_name: &str,
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
            // GNU declare.def: a "name[subscript]" operand without '=' is a
            // declaration with a size hint; the subscript is discarded and
            // the variable is recorded under the bare name ("declare -a
            // b[256]" then prints as "declare -a b"). declare.def:605 sets
            // making_array_special for ANY subscripted operand and 959-962
            // converts the variable to an indexed array even without -a, so
            // "declare -r c[100]" lists as "declare -ar c" (array.tests:62).
            let stripped = name.strip_suffix('+').unwrap_or(name);
            let bare = declare_indexed_element(stripped)
                .map(|(base, _)| base)
                .unwrap_or(stripped);
            if bare != stripped && !marked_vars(variables, ASSOC_VARS).contains(bare) {
                mark_typed(variables, ARRAY_VARS, bare);
            }
            // GNU declare.def -> get_universal_initial_value / assocconvert:
            // converting an existing scalar to an associative array moves
            // the value into element "0" (assoc.tests: assoc=assoc;
            // declare -A assoc then prints [0]="assoc" and ${assoc[@]}).
            if marked_vars(variables, ASSOC_VARS).contains(bare) {
                if let Some(current) = variables.get(bare).cloned() {
                    let is_array_storage = current.starts_with('\x1d')
                        || (current.starts_with('(') && current.ends_with(')'));
                    if !current.is_empty() && !is_array_storage {
                        let converted = super::storage::format_assoc_storage(vec![(
                            "0".to_string(),
                            current,
                        )]);
                        variables.insert(bare.to_string(), converted);
                    }
                }
            }
            if mark_unset_declarations && !variables.contains_key(bare) {
                mark_typed(variables, DECLARED_UNSET_VARS, bare);
            }
            continue;
        };
        let (raw_target, append_elem) = var_name
            .strip_suffix('+')
            .map(|base| (base, true))
            .unwrap_or((var_name, false));
        // GNU subst.c:13084 expand_declaration_argument performs an unquoted
        // compound list (parser CA-marked) as a separate assignment before
        // the builtin sees the operand, and subst.c:3599-3605 rejects a list
        // to a subscripted member ("cannot assign list to array member");
        // the word is then truncated to the bare name[sub], so the array is
        // only declared and no element is assigned (array.tests:
        // declare -a e[10]=(test) leaves "declare -a e").
        if value.starts_with(COMPOUND_ASSIGNMENT_MARKER) && raw_target.contains('[') {
            let (base, _) = declare_indexed_element(raw_target).unwrap_or((raw_target, ""));
            writeln!(
                stderr,
                "{}{raw_target}: cannot assign list to array member",
                diagnostic_prefix(variables)
            )?;
            status = EXECUTION_FAILURE;
            if !assoc && !marked_vars(variables, ASSOC_VARS).contains(base) {
                mark_typed(variables, ARRAY_VARS, base);
            }
            if mark_unset_declarations && !variables.contains_key(base) {
                mark_typed(variables, DECLARED_UNSET_VARS, base);
            }
            continue;
        }
        if let Some((base, index_expression)) = declare_indexed_element(raw_target) {
            if assoc || marked_vars(variables, ASSOC_VARS).contains(base) {
                if readonly.contains(base) {
                    writeln!(
                        stderr,
                        "{}{command_name}: {}: readonly variable",
                        diagnostic_prefix(variables),
                        base
                    )?;
                    status = EXECUTION_FAILURE;
                } else {
                    let current = variables
                        .get(base)
                        .cloned()
                        .unwrap_or_else(|| "()".to_string());
                    let element = format!("([{index_expression}]={value})");
                    variables.insert(
                        base.to_string(),
                        append_assoc_value(&current, &element, integer, variables),
                    );
                    mark_typed(variables, ASSOC_VARS, base);
                    unmark_typed(variables, DECLARED_UNSET_VARS, base);
                }
                continue;
            }
            if !assoc {
                // GNU declare.def:935-943 (compat > 43: a parenthesized value
                // on a declare operand is a compound array assignment, taken
                // without the deprecated warning while the array is being
                // created) and 992-993 (compound_array_assign outranks the
                // subscript path): the subscript is discarded and the
                // compound replaces the array (array.tests:112
                // declare -a e[10]='(test)' stores [0]="test").
                if !append_elem && value.starts_with('(') && value.ends_with(')') {
                    let storage = append_array_value("()", value, integer);
                    variables.insert(base.to_string(), storage);
                    mark_typed(variables, ARRAY_VARS, base);
                    unmark_typed(variables, DECLARED_UNSET_VARS, base);
                    continue;
                }
                let index = if index_expression.trim().is_empty() {
                    Some(0)
                } else {
                    eval_conditional_arith_value(index_expression, variables)
                        .and_then(|value| usize::try_from(value).ok())
                };
                if let Some(index) = index {
                    // GNU make_new_array_variable creates an indexed array
                    // with no elements; only convert_var_to_array (an existing
                    // scalar promoted by the declare operand) lands the scalar
                    // in [0] (array.tests: m=; declare -a m[10]=v keeps
                    // [0]=""). Parsing "" as one empty word materialized a
                    // phantom [0]="" element in every freshly created array.
                    let arrays = marked_vars(variables, ARRAY_VARS);
                    let mut entries = match variables.get(base).cloned() {
                        Some(current) if current.starts_with('\x1d') => {
                            indexed_array_entries(&current)
                        }
                        Some(current) if !current.is_empty() || !arrays.contains(base) => {
                            indexed_array_entries(&current)
                        }
                        _ => Default::default(),
                    };
                    // GNU bind_array_element: appends through a nameref land
                    // on the referenced element, arithmetically when the
                    // referenced array carries the integer attribute
                    // (nameref23.sub: declare -ai a; a[0]=4; declare -n
                    // b='a[0]'; declare b+=1 bumps a[0] to 5).
                    let element = if append_elem {
                        let current_element = entries.get(&index).cloned().unwrap_or_default();
                        if integer || marked_vars(variables, INTEGER_VARS).contains(base) {
                            let left = eval_conditional_arith_value(&current_element, variables)
                                .unwrap_or(0);
                            let right =
                                eval_conditional_arith_value(value, variables).unwrap_or(0);
                            (left + right).to_string()
                        } else {
                            format!("{current_element}{value}")
                        }
                    } else {
                        value.to_string()
                    };
                    entries.insert(index, element);
                    variables.insert(base.to_string(), format_indexed_array_storage(entries));
                    mark_typed(variables, ARRAY_VARS, base);
                    continue;
                }
            }
        }
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
                "{}{command_name}: {}: readonly variable",
                diagnostic_prefix(variables),
                var_name
            )?;
            status = EXECUTION_FAILURE;
            continue;
        }
        // GNU variables.c bind_variable_internal (the invisible-nameref
        // first clause + the visible-nameref cell validation): an
        // assignment whose target is a nameref without a usable cell
        // validates the value as a nameref value; an invalid one reports
        // sh_invalidid and leaves the nameref valueless
        // (nameref12/nameref13.sub: typeset -n foo; typeset foo=12345).
        if marked_vars(variables, NAMEREF_VARS).contains(var_name) {
            let current = variables.get(var_name).cloned().unwrap_or_default();
            if !append && !valid_nameref_value(&current) {
                if !valid_nameref_value(value) {
                    writeln!(
                        stderr,
                        "{}{command_name}: `{value}': not a valid identifier",
                        diagnostic_prefix(variables)
                    )?;
                    status = EXECUTION_FAILURE;
                    continue;
                }
                variables.insert(var_name.to_string(), value.to_string());
                unmark_typed(variables, DECLARED_UNSET_VARS, var_name);
                continue;
            }
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
                if value.starts_with('(') && value.ends_with(')') {
                    if let Some(bare) = assoc_bare_element(value) {
                        writeln!(
                            stderr,
                            "{}declare: {}: {}: must use subscript when assigning associative array",
                            diagnostic_prefix(variables),
                            var_name,
                            bare
                        )?;
                        status = EXECUTION_FAILURE;
                        continue;
                    }
                }
                append_assoc_value(&current, value, integer, variables)
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
        } else if (assoc || marked_vars(variables, ASSOC_VARS).contains(var_name))
            && value.starts_with('(')
            && value.ends_with(')')
        {
            // GNU arrayfunc.c: assoc-ness decides the compound form first; the
            // integer attribute then only evaluates the element values, so
            // `declare -Ai chaff=([one]=3+7)` stores an associative 10 rather
            // than routing the compound through the indexed path.
            //
            // GNU Bash (array.c/arrayassign.c): every element of an associative
            // array compound assignment must use the [key]=value form. A bare
            // word (no subscript) is rejected with the must-use-subscript error.
            if let Some(bare) = assoc_bare_element(value) {
                writeln!(
                    stderr,
                    "{}declare: {}: {}: must use subscript when assigning associative array",
                    diagnostic_prefix(variables),
                    var_name,
                    bare
                )?;
                status = EXECUTION_FAILURE;
                continue;
            }
            append_assoc_value("()", value, integer, variables)
        } else if integer {
            if value.starts_with('(') && value.ends_with(')') {
                append_array_value("()", value, true)
            } else {
                eval_arith_value(value).to_string()
            }
        } else if value.starts_with('(') && value.ends_with(')') {
            append_array_value("()", value, false)
        } else {
            value.to_string()
        };
        variables.insert(var_name.to_string(), value.clone());
        unmark_typed(variables, DECLARED_UNSET_VARS, var_name);
    }
    Ok(status)
}

/// Return the first bare (non `[key]=value`) element of an associative array
/// compound assignment value, or `None` if every element uses a subscript.
fn assoc_bare_element(value: &str) -> Option<String> {
    let inner = value.strip_prefix('(').and_then(|v| v.strip_suffix(')'))?;
    // GNU arrayfunc.c kvpair_assignment_p: the FIRST compound word decides
    // the mode. A leading word without the [key]= assignment shape puts the
    // whole list into alternating key/value mode, so no word is "bare"
    // (assoc11: declare -A inside=(a 1 b 2 c 3)).
    let mut first = true;
    for token in parse_array_tokens(inner) {
        if first {
            first = false;
            if !token.starts_with('[') {
                return None;
            }
        }
        let subscript_end = token.trim_end_matches(']').rfind(']');
        let eq = token.find('=');
        let is_subscript = token.starts_with('[')
            && token.contains('=')
            && subscript_end.map_or(false, |i| eq.map_or(false, |e| i < e));
        if !is_subscript && !token.contains('=') {
            return Some(token);
        }
    }
    None
}

fn declare_indexed_element(name: &str) -> Option<(&str, &str)> {
    let (base, subscript) = name.split_once('[')?;
    let subscript = subscript.strip_suffix(']')?;
    if base.is_empty() || subscript.contains('[') {
        return None;
    }
    Some((base, subscript))
}
