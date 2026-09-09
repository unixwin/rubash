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
mod storage;

use crate::shell::VariableStore;
use assign::assign_declare_names;
use attrs::{apply_declare_attrs, DeclareOptions};
use diagnostic::diagnostic_prefix;
use marks::{marked_vars, unmark_typed};
use names::{
    check_selfref, declare_base_name, valid_array_reference, valid_declare_name,
    valid_nameref_value,
};
use storage::{indexed_array_entries, parse_assoc_words};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const CAPCASE_VARS: &str = "__RUBASH_CAPCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
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
            let _ = store.replace_indexed_array(base, std::iter::empty::<String>());
        } else if assocs.contains(base)
            && !matches!(
                store.get(base).map(|v| &v.value),
                Some(crate::shell::ShellValue::AssociativeArray(_))
            )
        {
            let _ = store.replace_associative_array(base, std::iter::empty::<(String, String)>());
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
    execute_with_io_named_in_context(command_name, args, variables, stdout, stderr, false)
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
    let mut attr_status = EXECUTION_SUCCESS;
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let namerefs = marked_vars(variables, NAMEREF_VARS);
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
                    !current.starts_with('\x1d')
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
            if arrays.contains(lhs) || assocs.contains(lhs) {
                writeln!(
                    stderr,
                    "{}{command_name}: {}: reference variable cannot be an array",
                    diagnostic_prefix(variables),
                    lhs
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
            }
        }
        if !valid_declare_name(name) {
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
                // Unresolved chain (valueless or invalid cell): GNU rejects
                // the assignment through the unusable reference.
                writeln!(
                    stderr,
                    "{}{command_name}: {}: reference variable cannot be an array",
                    diagnostic_prefix(variables),
                    var_name
                )?;
                attr_status = EXECUTION_FAILURE;
                continue;
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
            writeln!(
                stderr,
                "{}{command_name}: {}: cannot convert associative to indexed array",
                diagnostic_prefix(variables),
                var_name
            )?;
            variables.insert(var_name.to_string(), String::new());
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
    if assign_declare_names(
        command_name,
        &attr_names,
        variables,
        array,
        assoc,
        integer,
        !print,
        stderr,
    )? != EXECUTION_SUCCESS
    {
        attr_status = EXECUTION_FAILURE;
    }
    let names = attr_names;
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
    };
    // GNU declare.def: -p is display-only; `declare -np b` must not create
    // or mark b (nameref23.sub:41 -- GNU prints "b: not found" for a failed
    // `declare -n b="1"` followed by `declare -np b`, while an attr pass
    // would leave a nameref mark behind).
    if !print {
        attr_status = apply_declare_attrs(command_name, &names, variables, options, attr_status, stderr)?;
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
