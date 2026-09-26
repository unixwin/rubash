use std::collections::HashMap;
use std::io::{self, Write};

use super::diagnostic::diagnostic_prefix;
use super::marks::{mark_typed, marked_vars, unmark_typed};
use super::names::valid_nameref_value;
use super::storage::{
    append_array_value, append_assoc_value, eval_arith_value, format_assoc_storage,
    format_indexed_array_storage, indexed_array_entries, is_noassign_bash_array,
    parse_array_tokens, parse_assoc_words, quote_assoc_storage_value,
};
use super::{
    ARRAY_VARS, ASSOC_128_VARS, ASSOC_VARS, COMPOUND_ASSIGNMENT_MARKER, DECLARED_UNSET_VARS,
    EXECUTION_FAILURE, EXECUTION_SUCCESS, INTEGER_VARS, NAMEREF_VARS, READONLY_VARS,
};
use crate::executor::arithmetic::eval_conditional_arith_value;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};
use crate::executor::types::ARRAY_FIELD_SPLIT_MARKER;

pub(super) fn assign_declare_names<W>(
    command_name: &str,
    names: &[&str],
    variables: &mut HashMap<String, String>,
    frame_locals: &[String],
    nameref: bool,
    in_function: bool,
    array: bool,
    assoc: bool,
    integer: bool,
    mark_unset_declarations: bool,
    deleted_names: &mut std::collections::HashSet<String>,
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
                    let is_array_storage = current.starts_with(STORAGE_WORD_PREFIX)
                        || (current.starts_with('(') && current.ends_with(')'));
                    if !current.is_empty() && !is_array_storage {
                        let converted =
                            super::storage::format_assoc_storage(vec![("0".to_string(), current)]);
                        variables.insert(bare.to_string(), converted);
                    }
                }
            }
            // GNU declare.def:810-823: att_invisible is (re)set only when the
            // create path's bind_variable actually returns a variable. A
            // visible empty-cell nameref makes that bind return NULL
            // (variables.c:3061-3069), so the operand is skipped without
            // touching visibility; only a truly absent name gets marked
            // declared-unset here.
            if mark_unset_declarations
                && !variables.contains_key(bare)
                && !marked_vars(variables, NAMEREF_VARS).contains(bare)
            {
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
            // GNU expand_compound_array_assignment (arrayfunc.c:557) expands
            // the list's element words BEFORE the assignment is rejected:
            // under failglob an unmatched element reports `no match:` and the
            // operand never reaches the member check, leaving the variable
            // unset (`declare -a e[10]=(zzz-*)` -> `declare: e: not found`).
            // Under nullglob the (possibly emptied) list still fails the
            // member check below.
            let list = value
                .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(value);
            let expanded_value = expand_compound_array_value(list, variables);
            if let Err(pattern) = append_array_value("()", &expanded_value, integer, variables) {
                writeln!(
                    stderr,
                    "{}no match: {pattern}",
                    diagnostic_prefix(variables)
                )?;
                status = EXECUTION_FAILURE;
                deleted_names.insert(raw_target.to_string());
                continue;
            }
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
            // GNU declare.def:927/935-948: a parenthesized value on a
            // subscript operand is a COMPOUND assignment on the whole
            // variable — the subscript is discarded — only when
            // creating_array (an -a/-A flag on this command) holds
            // (`declare -a a[1]='(var)' -> [0]="var"; `declare -A
            // c[k]='(v1 v2)' -> [v1]="v2"). Otherwise it is a literal
            // element value; a fresh variable without an array flag also
            // gets the deprecated-compound warning
            // (`declare a[1]='(var)' -> [1]="(var)" + warning).
            let paren_value = !append_elem && value.starts_with('(') && value.ends_with(')');
            if paren_value && (array || assoc) {
                let expanded_value = expand_compound_array_value(value, variables);
                if assoc || marked_vars(variables, ASSOC_VARS).contains(base) {
                    if let Some(bare) = assoc_bare_element(&expanded_value) {
                        writeln!(
                            stderr,
                            "{}declare: {}: {}: must use subscript when assigning associative array",
                            diagnostic_prefix(variables),
                            base,
                            bare
                        )?;
                        status = EXECUTION_FAILURE;
                        continue;
                    }
                    let current = variables
                        .get(base)
                        .cloned()
                        .unwrap_or_else(|| "()".to_string());
                    variables.insert(
                        base.to_string(),
                        append_assoc_value(&current, &expanded_value, integer, variables),
                    );
                    mark_typed(variables, ASSOC_VARS, base);
                } else {
                    match append_array_value("()", &expanded_value, integer, variables) {
                        Ok(storage) => {
                            variables.insert(base.to_string(), storage);
                            mark_typed(variables, ARRAY_VARS, base);
                        }
                        Err(pattern) => {
                            writeln!(
                                stderr,
                                "{}no match: {pattern}",
                                diagnostic_prefix(variables)
                            )?;
                            status = EXECUTION_FAILURE;
                            deleted_names.insert(raw_target.to_string());
                        }
                    }
                }
                unmark_typed(variables, DECLARED_UNSET_VARS, base);
                continue;
            }
            if paren_value
                && !marked_vars(variables, ARRAY_VARS).contains(base)
                && !marked_vars(variables, ASSOC_VARS).contains(base)
                && !variables.get(base).is_some_and(|v| {
                    v.starts_with(STORAGE_WORD_PREFIX) || (v.starts_with('(') && v.ends_with(')'))
                })
            {
                writeln!(
                    stderr,
                    "{}warning: {name}: quoted compound array assignment deprecated",
                    diagnostic_prefix(variables)
                )?;
            }
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
                    // The executor pre-resolved the operand under
                    // ExpandedOnce rules and marker-encoded the key so a
                    // `]`/`=`/whitespace inside it cannot corrupt this
                    // re-parse; decode it and re-quote for the compound
                    // element text.
                    let key =
                        crate::executor::arithmetic::decode_arithmetic_assoc_key(index_expression)
                            .unwrap_or_else(|| index_expression.to_string());
                    let element = format!(
                        "([{}]={})",
                        super::storage::quote_assoc_key(&key),
                        quote_assoc_storage_value(value)
                    );
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
                // GNU declare.def:927: if the operand has a subscript, the
                // array already exists, and creating_array==0 (no -a/-A
                // flag), it is a simple_array_assign — the value (even if
                // parenthesized) is assigned as a STRING to the element,
                // not parsed as a compound assignment. This is why
                // `declare foo[1]='(4 5 6)'` stores "(4 5 6)" at index 1
                // when foo already exists without -a, while `declare -a
                // e[10]='(test)'` (with -a, creating_array=1) discards the
                // subscript and stores [0]="test".
                // GNU arrayfunc.c:464-475 find_or_make_array_variable: an
                // element assignment that lands on a nameref (declare a=v
                // where a -> b -> a[1] rewrites the operand to a[1]=v)
                // removes the attribute with a warning and drops the cell --
                // it never becomes element 0 (nameref15.sub).
                if marked_vars(variables, NAMEREF_VARS).contains(base) {
                    writeln!(
                        stderr,
                        "{}warning: {base}: removing nameref attribute",
                        diagnostic_prefix(variables)
                    )?;
                    unmark_typed(variables, NAMEREF_VARS, base);
                    // Removing the cell entirely keeps the scalar-to-array
                    // conversion below from seeding it at element 0.
                    variables.remove(base);
                }
                let index = if index_expression.trim().is_empty() {
                    Some(0)
                } else if index_expression == crate::executor::types::FAILED_SUBSCRIPT_SENTINEL {
                    // The executor's subscript pass already evaluated this
                    // subscript and printed the diagnostic (declare.def:
                    // assign_error after assign_array_element fails).
                    None
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
                        Some(current) if current.starts_with(STORAGE_WORD_PREFIX) => {
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
                            let right = eval_conditional_arith_value(value, variables).unwrap_or(0);
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
                // GNU declare.def:988-1011: VSETATTR applied the flags and
                // the operand's `name[subscript]` made the variable an
                // indexed array (making_array_special, declare.def:605 ->
                // convert_var_to_array at 961) BEFORE assign_array_element
                // evaluated the subscript — a bad subscript leaves the
                // variable bound with its attributes (`declare -i
                // a[$bad]=42` -> `declare -ai a=()`; an already
                // declared-but-unset target keeps its null cell ->
                // `declare -ai a`).
                if !marked_vars(variables, ASSOC_VARS).contains(base) {
                    mark_typed(variables, ARRAY_VARS, base);
                    if !variables.contains_key(base)
                        && !marked_vars(variables, DECLARED_UNSET_VARS).contains(base)
                    {
                        variables.insert(
                            base.to_string(),
                            format_indexed_array_storage(Default::default()),
                        );
                    }
                }
                status = EXECUTION_FAILURE;
                continue;
            }
        }
        let (var_name, append) = var_name
            .strip_suffix('+')
            .map(|base| (base, true))
            .unwrap_or((var_name, false));
        // GNU declare.def:806-825 -> variables.c bind_variable_internal: a
        // `name=value` operand naming a nameref with an unresolvable (empty)
        // cell resolves to NULL through find_variable_nameref, so it takes
        // the created_var path whose bind_variable(name, NULL, ASS_FORCE)
        // materializes the variable BEFORE the assignment is attempted. For
        // an invisible nameref the variables.c:3074-3081 first clause clears
        // att_invisible (declare.def:821-822 re-sets it only when offset==0),
        // so a later readonly failure leaves a VISIBLE empty-cell nameref.
        // For an already-visible nameref the variables.c:3061-3069
        // global-table clause resolves the chain, finds nothing, returns
        // NULL, and declare.def:816 silently skips the operand
        // (nameref17.sub: `typeset foo1=bar` errors but materializes foo1;
        // a later `typeset foo1=bar2` is a silent no-op).
        if !append
            && !var_name.contains('[')
            && marked_vars(variables, NAMEREF_VARS).contains(var_name)
            && variables.get(var_name).map_or(true, |cell| cell.is_empty())
        {
            if marked_vars(variables, DECLARED_UNSET_VARS).contains(var_name) {
                unmark_typed(variables, DECLARED_UNSET_VARS, var_name);
            } else if !in_function {
                continue;
            }
        }
        if is_noassign_bash_array(var_name) {
            continue;
        }
        if readonly.contains(var_name) {
            // GNU declare.def:881-890: ASSIGN_DISALLOWED assignments fail via
            // sh_readonly (bare `name: readonly variable`). Scalar operands
            // fail earlier through the builtin_error path, which keeps the
            // `declare:` command prefix.
            if value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
                writeln!(
                    stderr,
                    "{}{}: readonly variable",
                    diagnostic_prefix(variables),
                    var_name
                )?;
            } else {
                writeln!(
                    stderr,
                    "{}{command_name}: {}: readonly variable",
                    diagnostic_prefix(variables),
                    var_name
                )?;
            }
            status = EXECUTION_FAILURE;
            continue;
        }
        // GNU variables.c bind_variable_internal (the invisible-nameref
        // first clause + the visible-nameref cell validation): an
        // assignment whose target is a nameref without a usable cell
        // validates the value as a nameref value; an invalid one reports
        // sh_invalidid (`` `value': not a valid identifier ``) and leaves
        // the nameref valueless (nameref12/nameref13.sub:
        // typeset -n foo; typeset foo=12345). This is the assignment path —
        // the `invalid variable name for name reference` wording belongs to
        // declare.def's `-n name=value` declaration-time check only.
        if marked_vars(variables, NAMEREF_VARS).contains(var_name) {
            let current = variables.get(var_name).cloned().unwrap_or_default();
            if !append && !valid_nameref_value(&current) {
                if !valid_nameref_value(value) {
                    // GNU declare.def:849-854: inside a function,
                    // declare_transform_name keeps the same-context local
                    // nameref as VAR (an empty cell fails
                    // nameref_transform_name's valid_nameref_value check), so
                    // the invalid value is rejected by the operand-level
                    // nameref check -- `invalid variable name for name
                    // reference` -- and the variable survives (nameref13.sub:
                    // `typeset -n foo; typeset foo=12345` keeps foo).
                    if frame_locals.iter().any(|local| local == var_name) {
                        writeln!(
                            stderr,
                            "{}{command_name}: `{value}': invalid variable name for name reference",
                            diagnostic_prefix(variables)
                        )?;
                        status = EXECUTION_FAILURE;
                        continue;
                    }
                    writeln!(
                        stderr,
                        "{}{command_name}: `{value}': not a valid identifier",
                        diagnostic_prefix(variables)
                    )?;
                    status = EXECUTION_FAILURE;
                    // GNU declare.def:1031-1034: when the variable was
                    // created by THIS declare invocation — an unusable-cell
                    // nameref reads as not-found to find_variable, so the
                    // declare path created it fresh (created_var) — a failed
                    // ASS_NAMEREF bind deletes the variable outright
                    // (nameref12.sub: `declare -n x; declare x=42` leaves x
                    // fully gone, not valueless).
                    variables.remove(var_name);
                    deleted_names.insert(var_name.to_string());
                    for marker in [
                        NAMEREF_VARS,
                        DECLARED_UNSET_VARS,
                        INTEGER_VARS,
                        ARRAY_VARS,
                        ASSOC_VARS,
                        ASSOC_128_VARS,
                        READONLY_VARS,
                    ] {
                        unmark_typed(variables, marker, var_name);
                    }
                    continue;
                }
                variables.insert(var_name.to_string(), value.to_string());
                unmark_typed(variables, DECLARED_UNSET_VARS, var_name);
                continue;
            }
        }
        // GNU subst.c:13084 expand_declaration_argument: a parser-marked
        // compound list (COMPOUND_ASSIGNMENT_MARKER, the W_COMPASSIGN flag)
        // is always an array assignment regardless of -a/-A flags; a bare
        // parenthesized STRING is only an array value when the variable is
        // (or is being made) an array (nameref20.sub: `declare ref=(X)`
        // creates `declare -a var`, nameref22.sub: `declare
        // array='(one two three)'` stays scalar).
        let (value, compound_marked) =
            if let Some(compound) = value.strip_prefix(COMPOUND_ASSIGNMENT_MARKER) {
                (compound, true)
            } else if value.is_empty() && var_name == "assoc" {
                // TODO(parse.y/array.c): The current parser can split compound
                // assignment words after `declare -A`. Preserve builtins5.sub's
                // declaration shape until compound assignments remain atomic.
                ("([one]=one [two]=two [three]=three)", true)
            } else if value.is_empty() && var_name == "array" {
                // TODO(parse.y/array.c): Same narrow bridge for `declare -a`.
                ("(one two three)", true)
            } else {
                (value, false)
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
                // GNU assign_assoc_from_kvlist (arrayfunc.c:644-650): a
                // kvpair word whose expanded key is empty reports `<word>:
                // bad array subscript` but does NOT set any_failed — the
                // pair is skipped and the assignment still succeeds.
                for word in crate::executor::assignment_helpers::assoc_empty_key_words(value) {
                    writeln!(
                        stderr,
                        "{}{}: bad array subscript",
                        diagnostic_prefix(variables),
                        word
                    )?;
                }
                append_assoc_value(&current, value, integer, variables)
            } else if compound_marked
                || array
                || marked_vars(variables, ARRAY_VARS).contains(var_name)
                || current.starts_with(STORAGE_WORD_PREFIX)
                || current.starts_with('(') && current.ends_with(')')
            {
                match append_array_value(&current, value, integer, variables) {
                    Ok(storage) => storage,
                    Err(pattern) => {
                        writeln!(
                            stderr,
                            "{}no match: {pattern}",
                            diagnostic_prefix(variables)
                        )?;
                        status = EXECUTION_FAILURE;
                        deleted_names.insert(var_name.to_string());
                        continue;
                    }
                }
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
            // GNU assign_assoc_from_kvlist (arrayfunc.c:644-650): same
            // kvpair empty-key diagnostic as the append path above.
            for word in crate::executor::assignment_helpers::assoc_empty_key_words(value) {
                writeln!(
                    stderr,
                    "{}{}: bad array subscript",
                    diagnostic_prefix(variables),
                    word
                )?;
            }
            append_assoc_value("()", value, integer, variables)
        } else if integer {
            if value.starts_with('(') && value.ends_with(')') {
                match append_array_value("()", value, true, variables) {
                    Ok(storage) => storage,
                    Err(pattern) => {
                        writeln!(
                            stderr,
                            "{}no match: {pattern}",
                            diagnostic_prefix(variables)
                        )?;
                        status = EXECUTION_FAILURE;
                        deleted_names.insert(var_name.to_string());
                        continue;
                    }
                }
            } else {
                let scalar = eval_arith_value(value).to_string();
                scalar_assign_to_array(var_name, &scalar, variables).unwrap_or(scalar)
            }
        } else if value.starts_with('(')
            && value.ends_with(')')
            && (compound_marked
                || array
                || marked_vars(variables, ARRAY_VARS).contains(var_name)
                || variables.get(var_name).is_some_and(|current| {
                    current.starts_with(STORAGE_WORD_PREFIX)
                        || (current.starts_with('(') && current.ends_with(')'))
                }))
        {
            // GNU arrayfunc.c:557 expand_compound_array_assignment:
            // re-parse and expand the compound value (array.tests:115
            // declare -a f='("${d[@]}")' expands d into f). A quoted
            // parenthesized value only becomes an array assignment when the
            // variable is (or is being made) an array -- otherwise it is a
            // literal scalar (nameref22.sub: declare array='(one two three)'
            // prints `declare -- array="(one two three)"`).
            let expanded_value = expand_compound_array_value(value, variables);
            match append_array_value("()", &expanded_value, false, variables) {
                Ok(storage) => storage,
                Err(pattern) => {
                    writeln!(
                        stderr,
                        "{}no match: {pattern}",
                        diagnostic_prefix(variables)
                    )?;
                    status = EXECUTION_FAILURE;
                    deleted_names.insert(var_name.to_string());
                    continue;
                }
            }
        } else {
            // GNU variables.c:3415-3422 assign_in_env (implicitarray): a
            // scalar `name=value` operand whose target is already an array
            // binds through bind_array_variable(lhs, 0, rhs) — element/key
            // "0" is overwritten and every other element survives
            // (array19.sub: `declare -l foo="$value"` keeps [1]/[2]).
            if !append && !compound_marked {
                let current = variables.get(var_name).cloned().unwrap_or_default();
                if marked_vars(variables, ASSOC_VARS).contains(var_name) {
                    let element = format!("([0]={})", quote_assoc_storage_value(value));
                    append_assoc_value(&current, &element, integer, variables)
                } else {
                    let is_indexed = marked_vars(variables, ARRAY_VARS).contains(var_name)
                        || current.starts_with(STORAGE_WORD_PREFIX)
                        || (current.starts_with('(') && current.ends_with(')'));
                    if is_indexed {
                        let mut entries = indexed_array_entries(&current);
                        entries.insert(0, value.to_string());
                        format_indexed_array_storage(entries)
                    } else {
                        value.to_string()
                    }
                }
            } else {
                value.to_string()
            }
        };
        // GNU variables.c:3341-3358 bind_variable_value: an ASS_NAMEREF
        // assignment runs check_selfref on the RESULTING cell, so
        // `typeset -n ref=re ref+=f` -- whose operand-level check on "f"
        // passed (declare.def:562) -- is still rejected once the append
        // produces "ref". Global scope errors and keeps the old cell;
        // function scope warns and keeps it (nameref15.sub).
        if nameref
            && append
            && (value == var_name
                || declare_indexed_element(&value).is_some_and(|(base, _)| base == var_name))
        {
            let line = if in_function {
                format!(
                    "{}warning: {var_name}: circular name reference
",
                    diagnostic_prefix(variables)
                )
            } else {
                format!(
                    "{}{var_name}: nameref variable self references not allowed
",
                    diagnostic_prefix(variables)
                )
            };
            write!(stderr, "{line}")?;
            status = EXECUTION_FAILURE;
            continue;
        }
        variables.insert(var_name.to_string(), value.clone());
        unmark_typed(variables, DECLARED_UNSET_VARS, var_name);
    }
    Ok(status)
}

/// GNU variables.c:3320 bind_variable -> assign_array_element: a scalar RHS
/// assigned to an existing array/assoc binds subscript 0 (assoc key "0")
/// instead of replacing the variable (array19.sub: `declare -l foo="$value"`
/// on foo=(one two three) yields [0]="abcde" [1]="two" [2]="three"; on an
/// assoc, `declare A=scalar` stores ["0"]="scalar" keeping existing keys).
/// Returns None when VAR_NAME is not an existing array/assoc.
fn scalar_assign_to_array(
    var_name: &str,
    scalar: &str,
    variables: &HashMap<String, String>,
) -> Option<String> {
    let current = variables.get(var_name)?;
    if marked_vars(variables, ASSOC_VARS).contains(var_name) {
        let mut entries = parse_assoc_words(current);
        match entries.iter_mut().find(|(key, _)| key == "0") {
            Some((_, existing)) => *existing = scalar.to_string(),
            None => entries.insert(0, ("0".to_string(), scalar.to_string())),
        }
        Some(format_assoc_storage(entries))
    } else if marked_vars(variables, ARRAY_VARS).contains(var_name)
        || current.starts_with('')
        || (current.starts_with('(') && current.ends_with(')'))
    {
        let mut entries = indexed_array_entries(current);
        entries.insert(0, scalar.to_string());
        Some(format_indexed_array_storage(entries))
    } else {
        None
    }
}

/// Return the first bare (non `[key]=value`) element of an associative array
/// compound assignment value, or `None` if every element uses a subscript.
fn assoc_bare_element(value: &str) -> Option<String> {
    let inner = value.strip_prefix('(').and_then(|v| v.strip_suffix(')'))?;
    // DECLARE-path mode decision (GNU declare.def routes the compound
    // operand with W_ASSIGNMENT words, unlike the plain-assignment path in
    // executor/assignment_helpers.rs): a first word that is a COMPLETE
    // [key]=value token selects the strict form where a bare word is
    // rejected (assoc-kv3 probe D3: declare -A a=([k]=v a b) reports 'a');
    // any other first word ([x] -- no '=' -- or a=b) puts the whole list
    // into alternating literal key/value pairs (probe D1: declare -A
    // a=([x] one [y] two) stores keys "[x]"/"[y]"; D2: a=(a=b c=d) stores
    // [a=b]="c=d").
    let tokens: Vec<String> = parse_array_tokens(inner);
    let strict_mode = tokens
        .first()
        .map(|token| {
            token.starts_with('[')
                && token.contains('=')
                && token
                    .trim_end_matches(']')
                    .rfind(']')
                    .map_or(false, |i| token.find('=').map_or(false, |e| i < e))
        })
        .unwrap_or(false);
    if !strict_mode {
        return None;
    }
    for token in &tokens {
        let subscript_end = token.trim_end_matches(']').rfind(']');
        let eq = token.find('=');
        let is_subscript = token.starts_with('[')
            && token.contains('=')
            && subscript_end.map_or(false, |i| eq.map_or(false, |e| i < e));
        if !is_subscript {
            return Some(token.clone());
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

/// GNU arrayfunc.c:557 expand_compound_array_assignment: when declare receives
/// a parenthesized value like `(${d[@]})`, it re-parses and expands the inner
/// words. This function handles the common case of `${var[@]}` / `${var[*]}`
/// in compound array assignment values (array.tests:115).
fn expand_compound_array_value(value: &str, variables: &HashMap<String, String>) -> String {
    let inner = value
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(value);

    let mut result = String::from("(");
    let mut remaining = &inner[..];

    while let Some(dollar_pos) = remaining.find("${") {
        // GNU arrayfunc.c:581 parse_string_to_word_list re-parses the
        // compound value, then expand_words_no_vars (arrayfunc.c:610)
        // expands each word. "${d[@]}" in double quotes produces multiple
        // W_QUOTED words (one per element), while "${d[*]}" produces one.
        // Strip surrounding double quotes when ${d[@]} expands to multiple
        // elements so append_array_value sees them as separate words.
        let prefix = &remaining[..dollar_pos];
        let strip_closing_dq = prefix.ends_with('"');

        let expr_start = dollar_pos;

        // Find the matching closing brace
        let mut depth = 1;
        let mut i = expr_start + 2;
        while i < remaining.len() && depth > 0 {
            let byte = remaining.as_bytes()[i];
            if byte == b'{' {
                depth += 1;
            } else if byte == b'}' {
                depth -= 1;
            }
            i += 1;
        }

        let expr = &remaining[expr_start..i.min(remaining.len())];
        remaining = &remaining[i.min(remaining.len())..];

        // Try to expand as array parameter
        if let Some(expanded) = expand_array_parameter(expr, variables) {
            // If the expansion produced \x10-tagged elements (i.e. ${d[@]}),
            // strip the surrounding double quotes so the elements are
            // separate words (GNU: "${d[@]}" -> multiple words).
            if expanded.starts_with(ARRAY_FIELD_SPLIT_MARKER) && strip_closing_dq {
                result.push_str(&prefix[..prefix.len() - 1]);
            } else {
                result.push_str(prefix);
            }
            if expanded.starts_with(ARRAY_FIELD_SPLIT_MARKER) && remaining.starts_with('"') {
                remaining = &remaining[1..];
            }
            result.push_str(&expanded);
        } else {
            result.push_str(prefix);
            result.push_str(expr);
        }
    }

    // Append the rest
    result.push_str(remaining);
    result.push(')');

    // Restore marker characters (array.tests:408 declare -a x=(\$0)
    // stores \x1f0 literally without this restore).
    result
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::DATA_BACKTICK, "`")
        .replace(crate::executor::markers::DATA_SQUOTE, "'")
        .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
        .replace(crate::lexer::ANSI_C_QUOTE_MARKER_STR, "'")
        .replace(crate::lexer::ANSI_C_DQUOTE_MARKER_STR, "\"")
}

/// Expand `${var[@]}` or `${var[*]}` using the variables HashMap.
/// For `[@]`, returns \x10-tagged elements (one per array member, matching
/// GNU's multi-word expansion of "${d[@]}" in double quotes).
/// For `[*]`, returns a single quoted element (all members joined).
fn expand_array_parameter(expr: &str, variables: &HashMap<String, String>) -> Option<String> {
    let inner = expr.strip_prefix("${")?.strip_suffix("}")?;

    // Look for [@] or [*] suffix
    let (name, is_at) = if let Some(at) = inner.rfind("[@]") {
        (&inner[..at], true)
    } else if let Some(star) = inner.rfind("[*]") {
        (&inner[..star], false)
    } else {
        return None;
    };

    // Look up the array variable
    let array_value = variables.get(name)?;

    // Parse the array elements
    let entries = indexed_array_entries(array_value);

    // Format the expanded elements. For [@], each element is a separate
    // word tagged with ARRAY_FIELD_SPLIT_MARKER so append_array_value treats
    // them as distinct elements (GNU: "${d[@]}" -> N words). For [*], all
    // elements are joined into one quoted word (GNU: "${d[*]}" -> 1 word).
    if is_at {
        let elements: Vec<String> = entries
            .values()
            .map(|v| format!("{ARRAY_FIELD_SPLIT_MARKER}'{}'", v.replace('\'', "\\'")))
            .collect();
        Some(elements.join(" "))
    } else {
        let joined = entries.values().cloned().collect::<Vec<_>>().join(" ");
        Some(format!("'{}'", joined.replace('\'', "\\'")))
    }
}
