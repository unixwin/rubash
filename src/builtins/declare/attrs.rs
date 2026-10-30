use std::env;
use std::io::{self, Write};

use super::diagnostic::diagnostic_prefix;
use super::marks::{mark_array, mark_assoc, mark_exported, mark_typed, unmark_typed};
use super::{
    ARRAY_VARS, ASSOC_128_VARS, ASSOC_VARS, CAPCASE_VARS, EXECUTION_FAILURE, EXPORTED_VARS,
    INTEGER_VARS, LOWERCASE_VARS, NAMEREF_VARS, READONLY_VARS, TRACE_VARS, UPPERCASE_VARS,
};
use crate::executor::local_helpers::sync_shell_assignment_process_env;
use crate::executor::markers::STORAGE_WORD_PREFIX;

#[derive(Clone, Copy)]
pub(super) struct DeclareOptions {
    pub(super) export: bool,
    pub(super) array: bool,
    pub(super) assoc: bool,
    pub(super) integer: bool,
    pub(super) uppercase: bool,
    pub(super) lowercase: bool,
    pub(super) capcase: bool,
    pub(super) nameref: bool,
    pub(super) readonly: bool,
    pub(super) trace: bool,
    pub(super) unset_export: bool,
    pub(super) unset_array: bool,
    pub(super) unset_assoc: bool,
    pub(super) unset_integer: bool,
    pub(super) unset_uppercase: bool,
    pub(super) unset_lowercase: bool,
    pub(super) unset_capcase: bool,
    pub(super) unset_nameref: bool,
    pub(super) unset_readonly: bool,
    pub(super) unset_trace: bool,
}

pub(super) fn apply_declare_attrs<W>(
    command_name: &str,
    names: &[&str],
    variables: &mut crate::shell::var_table::VarTable,
    options: DeclareOptions,
    mut attr_status: i32,
    deleted_names: &std::collections::HashSet<String>,
    preexisting_vars: &std::collections::HashSet<String>,
    in_function: bool,
    stderr: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    // GNU declare.def:1031-1034 delete_var: names deleted by a failed
    // created-var assignment must not be resurrected by attribute marking.
    let names: Vec<&str> = if deleted_names.is_empty() {
        names.to_vec()
    } else {
        names
            .iter()
            .copied()
            .filter(|name| {
                let base = name
                    .split_once('=')
                    .map(|(base, _)| base)
                    .unwrap_or(name)
                    .trim_end_matches('+');
                !deleted_names.contains(base)
            })
            .collect()
    };
    let names = &names[..];
    let DeclareOptions {
        export,
        array,
        assoc,
        integer,
        uppercase,
        lowercase,
        capcase,
        nameref,
        readonly,
        unset_export,
        unset_array,
        unset_assoc,
        unset_integer,
        unset_uppercase,
        unset_lowercase,
        unset_capcase,
        unset_nameref,
        unset_readonly,
        trace,
        unset_trace,
    } = options;
    // GNU declare.def:764-806: attribute-only arguments (no \`name=value\`)
    // whose NAME is an existing nameref follow the chain -- the attributes and
    // the created empty variable land on the referenced variable, never on the
    // nameref itself (nameref21.sub: \`declare -A ref\` marks var). Only when
    // the command is not itself toggling the nameref attribute: declare.def
    // 695-701 keeps -n/+n operating on the refvar.
    // GNU declare.def:593-604 truncates a `name[subscript]` operand at the
    // `[' before any attribute work: the subscript belongs to the element
    // assignment, never to the variable name, so every attribute and the
    // variable creation land on the bare name (array.tests: `declare -a
    // e[10]=test` must not leave a variable literally named "e[10]").
    let attr_targets: Vec<(String, bool)> =
        names.iter().map(|name| attr_target_name(name)).collect();
    let attr_names_owned: Vec<String> = if !nameref && !unset_nameref {
        attr_targets
            .iter()
            .map(|(name, _)| {
                if name.contains('=') {
                    return name.clone();
                }
                let base = name.strip_suffix('+').unwrap_or(name);
                if variables.is_marked(NAMEREF_VARS, base) {
                    if let Some((_, target)) = super::declare_nameref_chain(variables, base) {
                        return target;
                    }
                }
                name.clone()
            })
            .collect()
    } else {
        attr_targets.iter().map(|(name, _)| name.clone()).collect()
    };
    // GNU declare.def:764-816 -> variables.c:3061-3069: every attribute
    // operand naming a VISIBLE empty-cell nameref resolves to NULL through
    // find_variable_nameref, and the create-path bind_variable returns NULL
    // for a visible nameref in the global table -- NEXT_VARIABLE silently
    // skips it, leaving all attributes intact (nameref17.sub: `typeset +r
    // foo1` and `typeset +n foo1` after the failed `typeset foo1=bar` keep
    // `declare -nr foo1`). `-n` is exempt: declare.def:684-702 looks the
    // operand up with find_variable_noref and operates on the nameref
    // itself. Function scope binds a different table, where the
    // variables.c:3061 global-table clause does not fire.
    let names: Vec<&str> = if nameref || in_function {
        attr_names_owned.iter().map(String::as_str).collect()
    } else {
        attr_names_owned
            .iter()
            .map(String::as_str)
            .filter(|name| {
                let base = name
                    .split_once('=')
                    .map(|(base, _)| base)
                    .unwrap_or(name)
                    .trim_end_matches('+');
                !(variables.is_marked(NAMEREF_VARS, base)
                    && !variables.is_marked(super::DECLARED_UNSET_VARS, base)
                    && variables.get(base).map_or(true, |cell| cell.is_empty()))
            })
            .collect()
    };
    let names = &names[..];
    if unset_export
        || unset_array
        || unset_assoc
        || unset_integer
        || unset_uppercase
        || unset_lowercase
        || unset_capcase
        || unset_nameref
        || unset_readonly
        || unset_trace
    {
        for name in names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            if unset_readonly && variables.is_marked(READONLY_VARS, name) {
                writeln!(
                    stderr,
                    "{}{command_name}: {}: readonly variable",
                    diagnostic_prefix(variables),
                    name
                )?;
                attr_status = EXECUTION_FAILURE;
                // GNU declare.def refuses ALL attribute removal from a
                // readonly variable (nameref17.sub: typeset +r foo1 keeps
                // declare -nr foo1); report the error and leave the
                // variable untouched instead of unmarking after the error.
                continue;
            }
            if (unset_array && variables.is_marked(ARRAY_VARS, name))
                || (unset_assoc && variables.is_marked(ASSOC_VARS, name))
            {
                writeln!(
                    stderr,
                    "{}{command_name}: {}: cannot destroy array variables in this way",
                    diagnostic_prefix(variables),
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
                unmark_typed(variables, ASSOC_128_VARS, name);
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
            if unset_capcase {
                unmark_typed(variables, CAPCASE_VARS, name);
            }
            if unset_trace {
                unmark_typed(variables, TRACE_VARS, name);
            }
            if unset_nameref {
                // GNU declare.def:704-735 (+n): removing the nameref
                // attribute from a readonly nameref that still carries a
                // cell is refused; a readonly valueless nameref may drop it.
                if variables.is_marked(READONLY_VARS, name)
                    && variables.get(name).map_or(false, |cell| !cell.is_empty())
                {
                    writeln!(
                        stderr,
                        "{}{command_name}: {}: readonly variable",
                        diagnostic_prefix(variables),
                        name
                    )?;
                    attr_status = EXECUTION_FAILURE;
                    continue;
                }
                unmark_typed(variables, NAMEREF_VARS, name);
            }
        }
    }
    for (i, name) in attr_names_owned.iter().enumerate() {
        let made_array_special = attr_targets[i].1;
        if !array && !assoc && !made_array_special {
            continue;
        }
        let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
        let name = name.strip_suffix('+').unwrap_or(name);
        if array {
            mark_array(variables, name);
            // GNU arrayfunc.c convert_var_to_array (declare.def:959-962):
            // applying att_array to a variable holding a scalar moves the
            // value into element 0 — including an EMPTY scalar cell
            // (rubash#1225: `w=; declare -a w` and `declare -a w=` both
            // store [0]=""). A declared-but-unset name has no cell to
            // convert and stays element-less. Mirrors the assoc conversion
            // below so the storage is canonical for element reads.
            if let Some(current) = variables.get(name) {
                let is_storage = current.starts_with(STORAGE_WORD_PREFIX)
                    || (current.starts_with('(') && current.ends_with(')'));
                if !is_storage && !variables.is_marked(super::DECLARED_UNSET_VARS, name) {
                    let mut entries = std::collections::BTreeMap::new();
                    entries.insert(0, current.clone());
                    let converted = super::storage::format_indexed_array_storage(entries);
                    variables.insert(name.to_string(), converted);
                }
            }
        }
        if assoc {
            // GNU declare.def:810-823/953-958: `declare -A` on an unbound name
            // goes through make_new_assoc_variable (variables.c:2851), a
            // fresh ASSOC_HASH_BUCKETS (1024) table; on an existing non-assoc
            // variable it goes through convert_var_to_assoc (arrayfunc.c:111),
            // whose assoc_create(0) yields the 128-bucket default. The bucket
            // count decides iteration order, so record which path ran.
            if !variables.is_marked(ASSOC_VARS, name) {
                if preexisting_vars.contains(name) {
                    mark_typed(variables, ASSOC_128_VARS, name);
                } else {
                    unmark_typed(variables, ASSOC_128_VARS, name);
                }
            }
            mark_assoc(variables, name);
            // GNU arrayfunc.c:111-140 convert_var_to_assoc: applying
            // att_assoc to a variable holding a scalar moves the value into
            // element "0" (`declare -A v` on v=7 stores [0]="7"). Materialize
            // the assoc storage here so element reads, `${v[@]}`, and
            // compound appends all see the canonical form — including the
            // scalar a local inherits under localvar_inherit.
            // rubash#1225: an EMPTY scalar cell converts too — GNU
            // `v=; declare -A v` and `declare -A v=` both store [0]="".
            // Only a declared-but-unset name (no real cell) stays
            // element-less, like a bare `declare -A newvar`.
            if let Some(current) = variables.get(name) {
                let is_storage = current.starts_with(STORAGE_WORD_PREFIX)
                    || (current.starts_with('(') && current.ends_with(')'));
                if !is_storage && !variables.is_marked(super::DECLARED_UNSET_VARS, name) {
                    let converted = super::storage::format_assoc_storage(vec![(
                        "0".to_string(),
                        current.clone(),
                    )]);
                    variables.insert(name.to_string(), converted);
                }
            }
        }
        if made_array_special && !array && !assoc {
            // GNU declare.def:959-962: making_array_special converts the
            // variable to an indexed array even without -a (array.tests:62
            // `declare -r c[100]` lists as "declare -ar c").
            if !variables.is_marked(ASSOC_VARS, name) {
                mark_array(variables, name);
            }
        }
        variables.entry(name.to_string()).or_default();
    }
    if nameref {
        // GNU declare.def:967-968 (ksh93 compat): turning on the nameref
        // attribute clears -i/-u/-l/-c ALREADY on the variable
        // (`declare -i ivar` then `declare -n ivar=foo` lists `declare -n`,
        // nameref19.sub:73). The clear runs before the per-flag mark loops
        // because GNU VUNSETATTR precedes VSETATTR: `-i` given in the same
        // command still lands (`declare -in b` keeps -in, nameref23.sub:28).
        for name in names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            unmark_typed(variables, INTEGER_VARS, name);
            unmark_typed(variables, UPPERCASE_VARS, name);
            unmark_typed(variables, LOWERCASE_VARS, name);
            unmark_typed(variables, CAPCASE_VARS, name);
        }
    }
    if integer {
        for name in names.iter() {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            mark_typed(variables, INTEGER_VARS, name);
            // GNU declare.def:988 VSETATTR applies att_integer without
            // touching the stored value -- evaluation happens only when a
            // value is later assigned through bind_variable_value
            // (variables.c:3358-3360). `x=abc; declare -i x` keeps "abc",
            // and scalar-to-array conversion keeps [0]="one" verbatim
            // (nameref22.sub:84 `declare -i array[64]` on array=one).
        }
    }
    // Effective case attributes: GNU declare.def:525-527 applies
    // VSETATTR(flags_on) and then VUNSETATTR(flags_off), so an attribute
    // that a sibling case option in the same command crossed off
    // (`declare -lu` sets on={lower,upper}, off={upper,lower,capcase})
    // neither survives as a mark nor drives the value conversion below
    // (rubash#1458). At most one effective case attribute can remain:
    // each set case option adds the other two to flags_off.
    let eff_uppercase = uppercase && !unset_uppercase;
    let eff_lowercase = lowercase && !unset_lowercase;
    let eff_capcase = capcase && !unset_capcase;
    if eff_uppercase || eff_lowercase || eff_capcase {
        for name in names {
            let raw = name;
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            // GNU applies the case modification at bind time
            // (variables.c bind_variable_value), never as a rewrite of
            // an existing value when an attribute is merely set:
            // `s=abc; declare -u s` leaves "abc" (and likewise for
            // indexed/associative elements) until the next assignment.
            // The post-pass below therefore runs only for operands
            // that assign in THIS command (`h=...`, `h[k]=...`,
            // `h+=...`); a bare `declare -u h` only sets the mark.
            let lhs = raw.split('=').next().unwrap_or("");
            let assigned = raw.contains('=')
                && (lhs == name
                    || lhs.strip_suffix('+') == Some(name)
                    || lhs.starts_with(&format!("{name}[")));
            if eff_uppercase {
                mark_typed(variables, UPPERCASE_VARS, name);
                unmark_typed(variables, LOWERCASE_VARS, name);
            }
            if eff_lowercase {
                mark_typed(variables, LOWERCASE_VARS, name);
                unmark_typed(variables, UPPERCASE_VARS, name);
                unmark_typed(variables, CAPCASE_VARS, name);
            }
            if eff_capcase {
                mark_typed(variables, CAPCASE_VARS, name);
                unmark_typed(variables, UPPERCASE_VARS, name);
                unmark_typed(variables, LOWERCASE_VARS, name);
            }
            if assigned {
                if let Some(value) = variables.get(name).cloned() {
                    let convert = |value: &str| -> String {
                        if eff_uppercase {
                            value.to_uppercase()
                        } else if eff_lowercase {
                            value.to_lowercase()
                        } else {
                            // GNU capitalize: first character uppercased, rest
                            // lowercased (variables.c capcase, casemod.tests:99-103).
                            let mut chars = value.chars();
                            match chars.next() {
                                Some(first) => {
                                    first.to_uppercase().collect::<String>()
                                        + &chars.as_str().to_lowercase()
                                }
                                None => value.to_string(),
                            }
                        }
                    };
                    // GNU variables.c apply_varcasemod converts each array
                    // ELEMENT VALUE; an associative array's keys are never
                    // case-folded (`declare -uA h=([k]=v)` stores ([k]="V"),
                    // rubash#1398). The storage string holds keys inline, so
                    // folding the whole string would corrupt the keys —
                    // convert per entry and re-render instead. The stored
                    // form may carry the STORAGE_WORD_PREFIX byte before the
                    // `(`; strip it for the parse and restore it after.
                    let value = if variables.is_marked(ASSOC_VARS, name) {
                        let storage = value.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(&value);
                        if storage.starts_with('(') {
                            let entries = super::storage::parse_assoc_words(storage)
                                .into_iter()
                                .map(|(key, element)| (key, convert(&element)))
                                .collect::<Vec<_>>();
                            let rendered = super::storage::format_assoc_storage(entries);
                            if value.starts_with(STORAGE_WORD_PREFIX) {
                                format!("{STORAGE_WORD_PREFIX}{rendered}")
                            } else {
                                rendered
                            }
                        } else {
                            convert(&value)
                        }
                    } else if value.starts_with(STORAGE_WORD_PREFIX)
                        || (variables.is_marked(ARRAY_VARS, name) && value.starts_with('('))
                    {
                        // GNU apply_varcasemod converts each ELEMENT value
                        // of an indexed array too (rubash#1619). Folding
                        // the whole storage string instead capitalizes
                        // nothing — the render's first character is the
                        // prefix/`(` — and lowercases every element, so
                        // `declare -ca a=(hello world)` stored the words
                        // verbatim. Convert per element and re-render,
                        // like the assoc branch above.
                        let entries = super::storage::indexed_array_entries(&value)
                            .into_iter()
                            .map(|(index, element)| (index, convert(&element)))
                            .collect::<std::collections::BTreeMap<_, _>>();
                        super::storage::format_indexed_array_storage(entries)
                    } else {
                        convert(&value)
                    };
                    variables.insert(name.to_string(), value.clone());
                    // Same gate as plain assignments (#916): mirror to the
                    // process env only when exported or special (children get
                    // the table explicitly; an unconditional mirror paid a
                    // locked O(n) environ scan per call).
                    sync_shell_assignment_process_env(variables, name, &value);
                }
            }
        }
    }

    if export {
        for name in names {
            let has_assignment = name.contains('=');
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            if let Some(value) = variables.get(name).cloned().or_else(|| env::var(name).ok()) {
                variables.insert(name.to_string(), value.clone());
                // Same gate as plain assignments (#916): mirror only when
                // exported or special (scalar children get the table via
                // env_clear + explicit env; arrays never cross anyway).
                sync_shell_assignment_process_env(variables, name, &value);
                mark_exported(variables, name);
            } else if has_assignment {
                variables.insert((*name).to_string(), String::new());
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
    if trace {
        for name in names {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            let name = name.strip_suffix('+').unwrap_or(name);
            mark_typed(variables, TRACE_VARS, name);
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

/// GNU declare.def:593-604 truncates a `name[subscript]` operand at the `['
/// before attribute processing and keeps the assignment side intact. Returns
/// the normalized operand plus whether the name carried a subscript
/// (making_array_special). Malformed subscripts pass through unchanged (the
/// caller reports them through valid_declare_name before we get here).
fn attr_target_name(name: &str) -> (String, bool) {
    let (lhs, rhs) = match name.split_once('=') {
        Some((lhs, rhs)) => (lhs, Some(rhs)),
        None => (name, None),
    };
    let append = lhs.strip_suffix('+');
    let lhs = append.unwrap_or(lhs);
    let Some((base, subscript)) = lhs.split_once('[') else {
        return ((*name).to_string(), false);
    };
    if base.is_empty() || base.contains('[') || !subscript.ends_with(']') {
        return ((*name).to_string(), false);
    }
    let mut normalized = String::from(base);
    if append.is_some() {
        normalized.push('+');
    }
    if let Some(rhs) = rhs {
        normalized.push('=');
        normalized.push_str(rhs);
    }
    (normalized, true)
}
