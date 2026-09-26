//! declare module.
//!
//! GNU Bash source ownership:
// - builtins/declare.def

use std::collections::HashMap;
use std::io::{self, Write};

mod assign;
mod attrs;
mod diagnostic;
mod marks;
mod names;
mod output;
mod print;
pub(in crate::builtins) mod storage;

use crate::shell::VariableStore;
use assign::assign_declare_names;
use attrs::{apply_declare_attrs, DeclareOptions};
use diagnostic::diagnostic_prefix;
use marks::{marked_vars, unmark_typed};
use names::{
    check_selfref, declare_base_name, valid_array_reference, valid_declare_name, valid_identifier,
    valid_nameref_value,
};
use storage::{format_array_value, format_assoc_value, indexed_array_entries, parse_assoc_words};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

/// Format an associative array storage value for `set` output
/// (variables.c:1096 print_assignment -> arrayfunc.c:1257 print_assoc_assignment).
/// Used by the `set` builtin to print assoc arrays as `name=(["key"]="value" )`
/// instead of `name='(...)'`.
pub(crate) fn format_assoc_for_output(value: &str, nbuckets: usize) -> String {
    format_assoc_value(value, nbuckets)
}

/// Format an indexed array storage value for `set` output
/// (variables.c:1096 print_assignment -> arrayfunc.c:1239 print_array_assignment).
/// Used by the `set` builtin to print indexed arrays as `name=([0]="value" )`
/// instead of `name='(...)'`.
pub(crate) fn format_array_for_output(value: &str) -> String {
    format_array_value(value)
}

/// Create an associative array from a compound assignment value
/// (arrayfunc.c:630 assign_assoc_from_kvlist). Used by `readonly -A` and
/// `export` when they need to create an associative array from a kvpair
/// compound assignment like `( one 1 two 2 three 3 )`.
pub(crate) fn create_assoc_from_compound(value: &str) -> String {
    storage::append_assoc_value("()", value, false, &HashMap::new())
}
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const ASSOC_128_VARS: &str = "__RUBASH_ASSOC_128_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const CAPCASE_VARS: &str = "__RUBASH_CAPCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const TRACE_VARS: &str = "__RUBASH_TRACE_VARS";
const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::types::COMPOUND_ASSIGNMENT_MARKER;
const EX_USAGE: i32 = 2;

/// Synchronize indexed declarations into the typed variable owner after the
/// legacy builtin path has applied its attribute and encoding rules.
pub(crate) fn sync_typed_assignments(
    args: &[String],
    variables: &HashMap<String, String>,
    store: &mut VariableStore,
) {
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let mut parse_options = true;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        let Some((raw_name, _)) = arg.split_once('=') else {
            continue;
        };
        let name = raw_name.strip_suffix('+').unwrap_or(raw_name);
        let Some(base) = declare_base_name(name) else {
            continue;
        };
        let Some(value) = variables.get(base) else {
            continue;
        };
        if arrays.contains(base) {
            let entries = indexed_array_entries(value);
            let values = entries.into_iter().map(|(_, value)| value);
            let _ = store.replace_indexed_array(base, values);
        } else if assocs.contains(base) {
            let _ = store.replace_associative_array(base, parse_assoc_words(value));
        }
    }
}

/// Synchronize the final declare attribute markers into the typed variable owner.
pub(crate) fn sync_typed_attributes(
    args: &[String],
    variables: &HashMap<String, String>,
    store: &mut VariableStore,
) {
    let exported = marked_vars(variables, EXPORTED_VARS);
    let readonly = marked_vars(variables, READONLY_VARS);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let integer = marked_vars(variables, INTEGER_VARS);
    let uppercase = marked_vars(variables, UPPERCASE_VARS);
    let lowercase = marked_vars(variables, LOWERCASE_VARS);
    let namerefs = marked_vars(variables, NAMEREF_VARS);

    for arg in args {
        let raw_name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
        let name = raw_name.strip_suffix('+').unwrap_or(raw_name);
        let Some(base) = declare_base_name(name) else {
            continue;
        };
        // If the variable doesn't exist in the typed store, create it from env_vars.
        if store.get(base).is_none() {
            let value = variables.get(base).cloned().unwrap_or_default();
            let _ = store.set_scalar(base, value);
        } else if arg.contains('=') {
            // When there's an assignment in a declare/typed command and the
            // variable already exists in the typed store, update its value
            // to match env_vars (this handles local shadowing of parent vars).
            if let Some(value) = variables.get(base) {
                if let Some(variable) = store.get_mut(base) {
                    // Update the scalar value while preserving the variable's type state.
                    variable.value = crate::shell::ShellValue::Scalar(value.clone());
                }
            }
        }
        if arrays.contains(base)
            && !matches!(
                store.get(base).map(|v| &v.value),
                Some(crate::shell::ShellValue::IndexedArray(_))
            )
        {
            // GNU arrayfunc.c convert_var_to_array: a scalar's value moves to
            // element 0 when the variable gains att_array — with
            // localvar_inherit that scalar is the inherited caller's value
            // (`declare -a v` on v=7 keeps [0]="7"). Serialized storage text
            // is parsed back into entries.
            let env_value = variables.get(base).cloned().unwrap_or_default();
            let elements: Vec<String> = if env_value.starts_with(STORAGE_WORD_PREFIX)
                || (env_value.starts_with('(') && env_value.ends_with(')'))
            {
                indexed_array_entries(&env_value).into_values().collect()
            } else if env_value.is_empty() {
                Vec::new()
            } else {
                vec![env_value]
            };
            let _ = store.replace_indexed_array(base, elements);
        } else if assocs.contains(base)
            && !matches!(
                store.get(base).map(|v| &v.value),
                Some(crate::shell::ShellValue::AssociativeArray(_))
            )
        {
            // GNU arrayfunc.c:111-140 convert_var_to_assoc: a scalar's value
            // moves to element "0" when the variable gains att_assoc —
            // `declare -A v` on an inherited scalar v=7 keeps [0]="7" so a
            // later `v+=(1 one)` merges instead of replacing. Serialized
            // assoc storage text is parsed back into entries.
            let env_value = variables.get(base).cloned().unwrap_or_default();
            let entries: Vec<(String, String)> = if env_value.starts_with(STORAGE_WORD_PREFIX)
                || (env_value.starts_with('(') && env_value.ends_with(')'))
            {
                parse_assoc_words(
                    env_value
                        .strip_prefix(STORAGE_WORD_PREFIX)
                        .unwrap_or(&env_value),
                )
            } else if env_value.is_empty() {
                Vec::new()
            } else {
                vec![("0".to_string(), env_value)]
            };
            let _ = store.replace_associative_array(base, entries);
        }
        if let Some(variable) = store.get_mut(base) {
            variable.exported = exported.contains(base);
            variable.readonly = readonly.contains(base);
            variable.integer = integer.contains(base);
            variable.uppercase = uppercase.contains(base);
            variable.lowercase = lowercase.contains(base);
            variable.nameref = if namerefs.contains(base) {
                arg.split_once('=').map(|(_, value)| value.to_string())
            } else {
                None
            };
        }
    }
}

pub fn execute(args: &[String], variables: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr();
    execute_with_io(args, variables, &mut stdout, &mut stderr)
}

/// Minimal flag scan used by the executor to decide whether a declare/typeset
/// invocation follows nameref chains (declare.def:704-806 applies when the
/// command has neither -n nor +n among its options).
pub(crate) fn declare_nameref_flags(args: &[String]) -> (bool, bool) {
    let mut nameref = false;
    let mut unset_nameref = false;
    let mut parse_options = true;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        if parse_options
            && (arg.starts_with('-') || arg.starts_with('+'))
            && arg != "-"
            && arg != "+"
        {
            let set_attr = arg.starts_with('-');
            for option in arg[1..].chars() {
                match option {
                    'n' if set_attr => nameref = true,
                    'n' => unset_nameref = true,
                    _ => {}
                }
            }
            continue;
        }
        break;
    }
    (nameref, unset_nameref)
}

/// Resolve the nameref assignment targets for an upcoming declare invocation:
/// for every `name=value` argument whose base is a marked nameref, returns
/// `(base, final_target)` from declare_nameref_chain so the executor can
/// mirror the new target value into the typed owner (variables.c:2051
/// find_variable_last_nameref traversal).
pub(crate) fn nameref_assignment_targets(
    args: &[String],
    variables: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut targets = Vec::new();
    for arg in args {
        let Some((raw_lhs, _)) = arg.split_once('=') else {
            continue;
        };
        let lhs = raw_lhs.strip_suffix('+').unwrap_or(raw_lhs);
        let Some(base) = declare_base_name(lhs) else {
            continue;
        };
        if let Some((_, target)) = declare_nameref_chain(variables, base) {
            targets.push((base.to_string(), target));
        }
    }
    targets
}

/// GNU variables.c:2051 find_variable_last_nameref: walk the nameref chain
/// while each cell is itself a nameref value (identifier or array reference)
/// and return `(last_nameref_name, final_target_name)`. Returns None when
/// NAME is not a nameref or the chain does not resolve to a variable-like
/// cell (GNU then keeps operating on the nameref itself).
fn declare_nameref_chain(
    variables: &HashMap<String, String>,
    name: &str,
) -> Option<(String, String)> {
    if !marked_vars(variables, NAMEREF_VARS).contains(name) {
        return None;
    }
    let namerefs = marked_vars(variables, NAMEREF_VARS);
    let mut last = name.to_string();
    let mut current = name.to_string();
    for _ in 0..16 {
        if !namerefs.contains(current.as_str()) {
            break;
        }
        let cell = variables.get(&current)?.clone();
        if !names::valid_nameref_value(&cell) {
            return None;
        }
        last = current;
        current = cell;
    }
    if last == current {
        return None;
    }
    Some((last, current))
}

/// GNU declare.def:623-640 declare_transform_name: at function scope every
/// operand (bare or `name=value`) resolves through namerefs to the name that
/// make_local_variable actually localizes -- `declare -a ref` on ref->var
/// creates a local array `var`, and a bare `declare ref` creates local `var`.
/// Returns the resolved operand names (bare names of `x[i]` cells included).
pub(crate) fn nameref_resolved_operand_names(
    args: &[String],
    variables: &HashMap<String, String>,
) -> Vec<String> {
    let mut resolved = Vec::new();
    for arg in args {
        if arg == "--" || arg == "-" || arg == "+" {
            continue;
        }
        if arg.starts_with('-') || arg.starts_with('+') {
            continue;
        }
        let (raw_lhs, _) = arg.split_once('=').unwrap_or((arg.as_str(), ""));
        let lhs = raw_lhs.strip_suffix('+').unwrap_or(raw_lhs);
        let Some(base) = declare_base_name(lhs) else {
            continue;
        };
        if let Some((_, target)) = declare_nameref_chain(variables, base) {
            resolved.push(target);
        }
    }
    resolved
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    variables: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_with_io_named("declare", args, variables, stdout, stderr)
}

pub(crate) fn execute_with_io_named<W, E>(
    command_name: &str,
    args: &[String],
    variables: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_with_io_named_in_context(command_name, args, variables, stdout, stderr, false, &[])
}

/// GNU builtins/declare.def decides between the global-scope self-reference
/// error and the function-scope circular-reference warning with
/// `variable_context` (declare.def:565). Rubash threads the executor's
/// function depth through here so `typeset -n` inside a function warns and
/// continues instead of erroring.
pub(crate) fn execute_with_io_named_in_context<W, E>(
    command_name: &str,
    args: &[String],
    variables: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
    in_function: bool,
    frame_locals: &[String],
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut print = false;
    let mut export = false;
    let mut array = false;
    let mut assoc = false;
    let mut integer = false;
    let mut uppercase = false;
    let mut lowercase = false;
    let mut capcase = false;
    let mut nameref = false;
    let mut trace = false;
    let mut unset_trace = false;
    let mut readonly = false;
    let mut unset_export = false;
    let mut unset_array = false;
    let mut unset_assoc = false;
    let mut unset_integer = false;
    let mut unset_uppercase = false;
    let mut unset_lowercase = false;
    let mut unset_capcase = false;
    let mut unset_nameref = false;
    let mut unset_readonly = false;
    let mut names = Vec::new();

    let mut parse_options = true;
    let mut saw_option = false;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        if parse_options
            && (arg.starts_with('-') || arg.starts_with('+'))
            && arg != "-"
            && arg != "+"
        {
            let set_attr = arg.starts_with('-');
            saw_option = true;
            for option in arg[1..].chars() {
                match option {
                    'p' => print = true,
                    'x' if set_attr => export = true,
                    'x' => unset_export = true,
                    'a' if set_attr => array = true,
                    'a' => unset_array = true,
                    'A' if set_attr => assoc = true,
                    'A' => unset_assoc = true,
                    'i' if set_attr => integer = true,
                    'i' => unset_integer = true,
                    'u' => {
                        if set_attr {
                            uppercase = true;
                            lowercase = false;
                            capcase = false;
                        } else {
                            unset_uppercase = true;
                        }
                    }
                    'l' => {
                        if set_attr {
                            lowercase = true;
                            uppercase = false;
                            capcase = false;
                        } else {
                            unset_lowercase = true;
                        }
                    }
                    'c' if set_attr => {
                        capcase = true;
                        uppercase = false;
                        lowercase = false;
                    }
                    'c' => unset_capcase = true,
                    'n' if set_attr => nameref = true,
                    'n' => unset_nameref = true,
                    'r' if set_attr => readonly = true,
                    'r' => unset_readonly = true,
                    // GNU declare.def option string "aAfFgiIlnrtux":
                    // -t sets att_trace (printed by declare -p); +t
                    // clears it.
                    't' if set_attr => trace = true,
                    't' => unset_trace = true,
                    'g' | 'G' | 'I' => {
                        // TODO(variables.c/builtins/declare.def): `-g` forces
                        // global scope inside functions. Rubash has one
                        // variable table for now. `-I` is a local inheritance
                        // attribute; outside local it is accepted but does not
                        // add a printable variable attribute.
                    }
                    _ => {
                        writeln!(
                            stderr,
                            "{}{command_name}: -{option}: invalid option",
                            diagnostic_prefix(variables),
                        )?;
                        print_declare_usage(command_name, stderr)?;
                        return Ok(EX_USAGE);
                    }
                }
            }
        } else {
            names.push(arg.as_str());
        }
    }

    let had_name_args = !names.is_empty();
    // GNU declare.def: an operand name[subscript] without = declares or
    // re-declares the array; the subscript is only a size hint and is
    // discarded ("declare -a b[256]" declares b, and "declare -p b" prints
    // "declare -a b"). Strip it before any downstream name processing.
    if array || assoc {
        for name in names.iter_mut() {
            if name.contains('=') {
                continue;
            }
            if let Some((base, _subscript)) = name.split_once('[') {
                if !base.is_empty() && name.ends_with(']') && !base.contains('[') {
                    *name = base;
                }
            }
        }
    }
    let mut assign_names = Vec::new();
    let mut drop_nameref_attrs: Vec<String> = Vec::new();
    let mut attr_status = EXECUTION_SUCCESS;
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let namerefs = marked_vars(variables, NAMEREF_VARS);
    let readonly_vars = marked_vars(variables, READONLY_VARS);
    for name in &names {
        // GNU builtins/declare.def:549-580 runs the nameref-specific lexical
        // checks on the LHS and RHS before the generic identifier check, and
        // reports with the invoked builtin name (declare or typeset).
        let (lhs, value) = match name.split_once('=') {
            Some((lhs, value)) => (lhs, value),
            None => (*name, ""),
        };
        let append = lhs.strip_suffix('+').is_some();
        let lhs = lhs.strip_suffix('+').unwrap_or(lhs);
        // GNU 5.2.21 (declare.def:651 invisible-husk creation + ksh93 onref
        // deferral): `declare -in name=value` -- integer and nameref together
        // WITH an assignment -- leaves nothing observable: no value, no
        // attributes, no message (probe: `declare -p a b` afterwards reports
        // "b: not found" and a later `b+=1` builds a plain b).
        if nameref && integer && !value.is_empty() {
            continue;
        }
        // GNU declare.def:554-558 + 966-980: a real compound assignment
        // operand under -n reports `name: reference variable cannot be an
        // array`, the array attribute wins so the compound still assigns,
        // and onref is dropped so the nameref attribute is never set
        // (nameref22.sub:74 `declare -n array=(one two three)` errors yet
        // leaves `declare -a array=([0]="one" ...)`).
        if nameref && value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
            writeln!(
                stderr,
                "{}{command_name}: {}: reference variable cannot be an array",
                diagnostic_prefix(variables),
                lhs
            )?;
            attr_status = EXECUTION_FAILURE;
            drop_nameref_attrs.push(lhs.to_string());
            assign_names.push(*name);
            continue;
        }
        if nameref {
            // declare.def:554: a nameref cannot be declared as an array
            // reference name (x[3]).
            if valid_array_reference(lhs) {
                writeln!(
                    stderr,
                    "{}{command_name}: {}: reference variable cannot be an array",
                    diagnostic_prefix(variables),
                    lhs
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // declare.def:562-573: disallow self references at global scope,
            // warn at function scope and continue creating the nameref.
            if check_selfref(lhs, value) {
                if in_function {
                    writeln!(
                        stderr,
                        "{}{command_name}: warning: {}: circular name reference",
                        diagnostic_prefix(variables),
                        lhs
                    )?;
                    // bind_variable_value's ASS_NAMEREF check_selfref
                    // (variables.c:3324) reports the same self-reference via
                    // internal_warning — a second, unqualified diagnostic —
                    // then still stores the cell.
                    writeln!(
                        stderr,
                        "{}warning: {}: circular name reference",
                        diagnostic_prefix(variables),
                        lhs
                    )?;
                } else {
                    writeln!(
                        stderr,
                        "{}{command_name}: {}: nameref variable self references not allowed",
                        diagnostic_prefix(variables),
                        lhs
                    )?;
                    attr_status = EXECUTION_FAILURE;
                    continue;
                }
            }
            // declare.def:574-579: the value must be a valid identifier o
            // array reference when it will be used as a nameref target.
            if !value.is_empty()
                && !append
                && !value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                && !valid_nameref_value(value)
            {
                writeln!(
                    stderr,
                    "{}{command_name}: `{value}': invalid variable name for name reference",
                    diagnostic_prefix(variables)
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // declare.def:854-862: converting an existing non-nameref variable
            // to a nameref without an assignment fails when its current value
            // is not a valid nameref value.
            if !append
                && value.is_empty()
                && !namerefs.contains(lhs)
                && variables.get(lhs).is_some_and(|current| {
                    // A compound/array-storage cell is not a nameref value at
                    // all -- GNU reaches the array-rejection diagnostic for
                    // those (nameref22.sub:50), not the invalid-value error.
                    !current.starts_with(STORAGE_WORD_PREFIX)
                        && !current.starts_with('(')
                        && !current.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                        && !valid_nameref_value(current)
                })
            {
                let current = variables.get(lhs).cloned().unwrap_or_default();
                writeln!(
                    stderr,
                    "{}{command_name}: `{current}': invalid variable name for name reference",
                    diagnostic_prefix(variables)
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // declare.def:841: applying -n to an existing array variable is
            // rejected, but only after the value checks above -- GNU reports
            // the invalid-value error first (nameref22.sub:69 reports
            // `(one two three)': invalid variable name, then :70 reports the
            // array rejection for the same variable).
            if (arrays.contains(lhs) || assocs.contains(lhs))
                && !(namerefs.contains(lhs) && value.is_empty() && !variables.contains_key(lhs))
            {
                writeln!(
                    stderr,
                    "{}{command_name}: {}: reference variable cannot be an array",
                    diagnostic_prefix(variables),
                    lhs
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // GNU variables.c bind_variable_value ASS_NAMEREF path: an
            // explicit `declare -n name=` assignment binds "" as the cell,
            // which fails valid_nameref_value and reports
            // `` `': not a valid identifier `` -- the variable is not
            // created, and an existing nameref keeps its cell
            // (nameref24.sub:24 `declare -n name3=`).
            if !append && name.contains('=') && value.is_empty() {
                writeln!(
                    stderr,
                    "{}{command_name}: `': not a valid identifier",
                    diagnostic_prefix(variables)
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            if readonly_vars.contains(lhs) {
                writeln!(
                    stderr,
                    "{}{command_name}: {}: readonly variable",
                    diagnostic_prefix(variables),
                    lhs
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
        }
        if !valid_declare_name(name) {
            if print && !name.contains('=') {
                // GNU declare.def: -p is display-only; the identifier check
                // gates attribute changes, not display, so an invalid name
                // reaches find_variable and reports "not found" — never the
                // full listing fallback (niubash issue #102 probe: bash 5.3.0
                // prints `declare: X(BR): not found` for `declare -p X(BR)`,
                // `not a valid identifier` only without -p).
                writeln!(
                    stderr,
                    "{}{command_name}: {}: not found",
                    diagnostic_prefix(variables),
                    name
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            let diagnostic = format!(
                "{}{command_name}: `{}': not a valid identifier\n",
                diagnostic_prefix(variables),
                name
            );
            stderr.write_all(diagnostic.as_bytes())?;
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        assign_names.push(*name);
    }
    let mut valid_assign_names = Vec::new();
    for name in assign_names {
        let Some(var_name) = declare_base_name(name) else {
            valid_assign_names.push(name);
            continue;
        };
        if (array || assoc) && namerefs.contains(var_name) {
            let operand_has_subscript = name.starts_with(var_name)
                && name.len() > var_name.len()
                && name[var_name.len()..].starts_with('[');
            if operand_has_subscript {
                // GNU declare.def:737-752: an array assignment to a nameref
                // WITH a subscript removes the nameref attribute (with an
                // internal warning) and applies the array declaration to the
                // name itself (nameref15.sub: declare -a xref[1]=one).
                writeln!(
                    stderr,
                    "{}warning: {var_name}: removing nameref attribute",
                    diagnostic_prefix(variables)
                )?;
                unmark_typed(variables, NAMEREF_VARS, var_name);
                variables.remove(var_name);
            } else if declare_nameref_chain(variables, var_name).is_some() {
                // GNU declare.def:197-216 declare_transform_name + 764-806:
                // without a subscript the -a/-A flag and the compound
                // assignment follow the chain onto the referenced variable
                // (nameref20/nameref21.sub); the effective_assign_names stage
                // below performs the rewrite, so nothing is rejected here.
            } else {
                // GNU declare.def: a valueless nameref (no target value
                // stored) can be converted to an array by removing the
                // nameref attribute (nameref12.sub: declare -n foo;
                // declare -a foo; declare -p foo shows declare -a foo).
                unmark_typed(variables, NAMEREF_VARS, var_name);
                variables.remove(var_name);
            }
        }
        if assoc && arrays.contains(var_name) && !assocs.contains(var_name) {
            writeln!(
                stderr,
                "{}{command_name}: {}: cannot convert indexed to associative array",
                diagnostic_prefix(variables),
                var_name
            )?;
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        if array && assocs.contains(var_name) && !arrays.contains(var_name) {
            // GNU declare.def -> convert_var_to_array refusal: the operand
            // errors and the variable keeps its assoc value untouched —
            // clearing it would print `declare -A m` where GNU still shows
            // the elements (varenv14.sub declare -a assoc case).
            writeln!(
                stderr,
                "{}{command_name}: {}: cannot convert associative to indexed array",
                diagnostic_prefix(variables),
                var_name
            )?;
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        valid_assign_names.push(name);
    }
    let assign_names = valid_assign_names;
    // GNU builtins/declare.def:704-738 (turning off the nameref attribute with
    // an assignment: assign through the chain, then remove the attribute while
    // leaving the nameref's value in place) and declare.def:764-806 (without
    // -n/+n the attributes and assignment apply to the variable the nameref
    // chain references, via find_variable_last_nameref). `declare -p` never
    // follows the chain (nameref18.sub prints the nameref itself).
    let mut effective_assign_names: Vec<String> = Vec::new();
    if !print {
        for name in &assign_names {
            let (raw_lhs, value) = match name.split_once('=') {
                Some((lhs, value)) => (lhs, Some(value)),
                None => (*name, None),
            };
            let append = raw_lhs.strip_suffix('+').is_some();
            let lhs = raw_lhs.strip_suffix('+').unwrap_or(raw_lhs);
            let chain = declare_nameref_chain(variables, lhs);
            if let Some((last_nameref, target)) = chain {
                if unset_nameref {
                    // GNU declare.def:678-682/716-724 (ksh93 compat): the
                    // nameref attribute cannot be removed from a readonly
                    // nameref that carries a cell. At global scope the
                    // last-nameref lookup always lands on the nameref, so
                    // any non-empty cell errors. Inside a function the
                    // operand is first resolved to a local var; when the
                    // cell's target exists the same readonly check fires,
                    // but when the target is missing the rewrite silently
                    // keeps the nameref (nameref17.sub: typeset +n foo4 with
                    // cell -> existing bar4 errors, cell -> missing stays).
                    let cell = variables.get(&last_nameref).cloned().unwrap_or_default();
                    let readonly_with_cell = marked_vars(variables, READONLY_VARS)
                        .contains(last_nameref.as_str())
                        && !cell.is_empty();
                    if readonly_with_cell {
                        let cell_base = cell.split('[').next().unwrap_or(cell.as_str());
                        let target_exists = !in_function
                            || variables.contains_key(cell_base)
                            || marked_vars(variables, ARRAY_VARS).contains(cell_base)
                            || marked_vars(variables, ASSOC_VARS).contains(cell_base);
                        if target_exists {
                            writeln!(
                                stderr,
                                "{}{command_name}: {}: readonly variable",
                                diagnostic_prefix(variables),
                                last_nameref
                            )?;
                            attr_status = EXECUTION_FAILURE;
                        }
                        continue;
                    }
                    if let Some(value) = value {
                        effective_assign_names.push(if append {
                            format!("{target}+={value}")
                        } else {
                            format!("{target}={value}")
                        });
                    } else {
                        effective_assign_names.push((*name).to_string());
                    }
                    // declare.def:985-986: only the last nameref in the chain
                    // loses the attribute; its cell value is kept as the new
                    // plain variable value.
                    unmark_typed(variables, NAMEREF_VARS, &last_nameref);
                    continue;
                }
                if !nameref {
                    // GNU arrayfunc.c:454 find_or_make_array_variable ->
                    // variables.c:2201 find_variable_nameref_for_create: a
                    // compound `=(...)` value through a nameref requires a
                    // bare-identifier cell; an element cell like `D[2]`
                    // fails sh_invalidid. An attribute-only `declare -a/-A`
                    // instead applies the flag to the cell's array BASE
                    // name (nameref18.sub: `declare -A r` with r -> `A[0]`
                    // yields `declare -A A`).
                    let compound_value =
                        value.is_some_and(|v| v.starts_with(COMPOUND_ASSIGNMENT_MARKER));
                    if compound_value && !valid_identifier(&target) {
                        writeln!(
                            stderr,
                            "{}`{target}': not a valid identifier",
                            diagnostic_prefix(variables)
                        )?;
                        attr_status = EXECUTION_FAILURE;
                        continue;
                    }
                    let target = if value.is_none() && (array || assoc) {
                        target
                            .split('[')
                            .next()
                            .unwrap_or(target.as_str())
                            .to_string()
                    } else {
                        target
                    };
                    match value {
                        Some(value) => effective_assign_names.push(if append {
                            format!("{target}+={value}")
                        } else {
                            format!("{target}={value}")
                        }),
                        None => effective_assign_names.push(target),
                    }
                    continue;
                }
            }
            effective_assign_names.push((*name).to_string());
        }
    } else {
        effective_assign_names.extend(assign_names.iter().map(|name| (*name).to_string()));
    }
    let attr_names: Vec<&str> = effective_assign_names.iter().map(String::as_str).collect();
    // GNU declare.def:1031-1034: a failed ASS_NAMEREF assignment to a
    // freshly-created (invisible/empty-cell) variable deletes it outright;
    // the attribute pass must not resurrect the name.
    let mut deleted_names = std::collections::HashSet::new();
    // GNU declare.def:810-823 checks whether the variable is already bound
    // BEFORE the command's own assignment runs: `declare -A foo` on an
    // existing variable converts it through convert_var_to_assoc
    // (arrayfunc.c:111, a 128-bucket table), while an unbound name takes
    // make_new_assoc_variable (variables.c:2851, 1024 buckets). Snapshot
    // operand names now -- after assign_declare_names an operand like
    // `declare -A foo=v` would look pre-existing even though GNU created it.
    let preexisting_vars: std::collections::HashSet<String> = attr_names
        .iter()
        .map(|name| {
            let base = name
                .split_once('=')
                .map(|(base, _)| base)
                .unwrap_or(name)
                .trim_end_matches('+');
            let base = base.split('[').next().unwrap_or(base);
            base.to_string()
        })
        .filter(|base| variables.contains_key(base) || frame_locals.iter().any(|n| n == base))
        .collect();
    if assign_declare_names(
        command_name,
        &attr_names,
        variables,
        frame_locals,
        nameref,
        in_function,
        array,
        assoc,
        integer,
        !print,
        &mut deleted_names,
        stderr,
    )? != EXECUTION_SUCCESS
    {
        attr_status = EXECUTION_FAILURE;
    }
    let names = attr_names;
    // GNU declare.def: with explicit operands, -p prints exactly those
    // names and reports per-name errors ("not a valid identifier", "not
    // found"); it never falls back to a full listing. Operands rejected in
    // the loop above (e.g. `declare -p 'X(BR)'` on an inherited
    // CommonProgramFiles(x86)-style name) leave nothing to print — the
    // empty-names listing below is only for the no-operand invocation.
    if names.is_empty() && had_name_args {
        return Ok(attr_status);
    }
    let options = DeclareOptions {
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
    };
    // GNU declare.def: -p is display-only; `declare -np b` must not create
    // or mark b (nameref23.sub:41 -- GNU prints "b: not found" for a failed
    // `declare -n b="1"` followed by `declare -np b`, while an attr pass
    // would leave a nameref mark behind).
    if !print {
        attr_status = apply_declare_attrs(
            command_name,
            &names,
            variables,
            options,
            attr_status,
            &deleted_names,
            &preexisting_vars,
            in_function,
            stderr,
        )?;
        // declare.def:978-981: array/compound operands drop the deferred -n.
        for name in &drop_nameref_attrs {
            unmark_typed(variables, NAMEREF_VARS, name);
        }
    }

    let plain = names.is_empty() && !had_name_args && !print && !saw_option;
    if names.is_empty() && !had_name_args {
        print = true;
    }

    if !print {
        return Ok(attr_status);
    }

    print::print_declare_names(
        command_name,
        &names,
        variables,
        options,
        plain,
        attr_status,
        stdout,
        stderr,
    )
}

fn print_declare_usage<W>(command_name: &str, stderr: &mut W) -> io::Result<()>
where
    W: Write,
{
    if command_name == "typeset" {
        writeln!(
            stderr,
            "typeset: usage: typeset [-aAfFgiIlnrtux] name[=value] ... or typeset -p [-aAfFilnrtux] [name ...]"
        )
    } else {
        writeln!(
            stderr,
            "declare: usage: declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]"
        )
    }
}

#[cfg(test)]
#[path = "declare_tests.rs"]
mod tests;
