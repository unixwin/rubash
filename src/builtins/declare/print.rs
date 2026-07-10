use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use super::attrs::DeclareOptions;
use super::diagnostic::diagnostic_prefix;
use super::marks::{exported_vars, marked_vars};
use super::output::{print_declaration, print_unset_declaration, DeclarationAttrs};
use super::{
    ARRAY_VARS, ASSOC_VARS, DECLARED_UNSET_VARS, EXECUTION_FAILURE, INTEGER_VARS, LOWERCASE_VARS,
    NAMEREF_VARS, READONLY_VARS, UPPERCASE_VARS,
};

pub(super) fn print_declare_names<W, E>(
    names: &[&str],
    variables: &HashMap<String, String>,
    options: DeclareOptions,
    mut status: i32,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let filter_exported = options.export;
    let filter_readonly = options.readonly;
    let filter_array = options.array;
    let filter_assoc = options.assoc;
    let filter_integer = options.integer;
    let filter_uppercase = options.uppercase;
    let filter_lowercase = options.lowercase;
    let filter_nameref = options.nameref;

    let exported = exported_vars(variables);
    let readonly = marked_vars(variables, READONLY_VARS);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let integers = marked_vars(variables, INTEGER_VARS);
    let uppercase = marked_vars(variables, UPPERCASE_VARS);
    let lowercase = marked_vars(variables, LOWERCASE_VARS);
    let namerefs = marked_vars(variables, NAMEREF_VARS);
    let declared_unset = marked_vars(variables, DECLARED_UNSET_VARS);
    let names_to_print = if names.is_empty() {
        declaration_names_to_print(
            variables,
            filter_exported,
            filter_readonly,
            filter_array,
            filter_assoc,
            filter_integer,
            filter_uppercase,
            filter_lowercase,
            filter_nameref,
            &uppercase,
            &lowercase,
            &namerefs,
        )
    } else {
        names
            .iter()
            .copied()
            .map(|name| name.split_once('=').map(|(name, _)| name).unwrap_or(name))
            .map(|name| name.strip_suffix('+').unwrap_or(name))
            .map(str::to_string)
            .collect()
    };
    for name in names_to_print {
        let attrs = DeclarationAttrs {
            exported: exported.contains(&name),
            readonly: readonly.contains(&name),
            array: arrays.contains(&name),
            assoc: assocs.contains(&name),
            integer: integers.contains(&name),
            uppercase: uppercase.contains(&name),
            lowercase: lowercase.contains(&name),
            nameref: namerefs.contains(&name),
        };
        if let Some(value) = variables.get(&name) {
            print_declaration(&name, value, attrs, stdout)?;
        } else if attrs.has_scalar_attribute() || declared_unset.contains(&name) {
            print_unset_declaration(&name, attrs, stdout)?;
        } else {
            writeln!(
                stderr,
                "{}declare: {}: not found",
                diagnostic_prefix(),
                name
            )?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

pub(super) fn declaration_names_to_print(
    variables: &HashMap<String, String>,
    export: bool,
    readonly: bool,
    array: bool,
    assoc: bool,
    integer: bool,
    uppercase: bool,
    lowercase: bool,
    nameref: bool,
    uppercase_vars: &HashSet<String>,
    lowercase_vars: &HashSet<String>,
    nameref_vars: &HashSet<String>,
) -> Vec<String> {
    let exported = exported_vars(variables);
    let readonly_vars = marked_vars(variables, READONLY_VARS);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let integers = marked_vars(variables, INTEGER_VARS);
    let filter_by_attr =
        export || readonly || array || assoc || integer || uppercase || lowercase || nameref;
    let mut names: Vec<String> = variables
        .keys()
        .filter(|name| !name.starts_with("__RUBASH_"))
        .filter(|name| {
            if !filter_by_attr {
                return true;
            }
            (!export || exported.contains(*name))
                && (!readonly || readonly_vars.contains(*name))
                && (!array || arrays.contains(*name))
                && (!assoc || assocs.contains(*name))
                && (!integer || integers.contains(*name))
                && (!uppercase || uppercase_vars.contains(*name))
                && (!lowercase || lowercase_vars.contains(*name))
                && (!nameref || nameref_vars.contains(*name))
        })
        .cloned()
        .collect();
    for name in exported
        .iter()
        .chain(readonly_vars.iter())
        .chain(nameref_vars.iter())
    {
        if name.starts_with("__RUBASH_") {
            continue;
        }
        if filter_by_attr
            && !((!export || exported.contains(name))
                && (!readonly || readonly_vars.contains(name))
                && (!array || arrays.contains(name))
                && (!assoc || assocs.contains(name))
                && (!integer || integers.contains(name))
                && (!uppercase || uppercase_vars.contains(name))
                && (!lowercase || lowercase_vars.contains(name))
                && (!nameref || nameref_vars.contains(name)))
        {
            continue;
        }
        if !names.iter().any(|current| current == name) {
            names.push(name.clone());
        }
    }
    names.sort();
    names
}
