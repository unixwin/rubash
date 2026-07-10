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

use assign::assign_declare_names;
use attrs::{apply_declare_attrs, DeclareOptions};
use diagnostic::diagnostic_prefix;
use marks::marked_vars;
use names::{declare_base_name, nameref_self_reference, valid_declare_name};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
const COMPOUND_ASSIGNMENT_MARKER: char = '\x1e';

pub fn execute(args: &[String], variables: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, variables, &mut stdout, &mut stderr)
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
    let mut print = false;
    let mut export = false;
    let mut array = false;
    let mut assoc = false;
    let mut integer = false;
    let mut uppercase = false;
    let mut lowercase = false;
    let mut nameref = false;
    let mut readonly = false;
    let mut unset_export = false;
    let mut unset_array = false;
    let mut unset_assoc = false;
    let mut unset_integer = false;
    let mut unset_uppercase = false;
    let mut unset_lowercase = false;
    let mut unset_nameref = false;
    let mut unset_readonly = false;
    let mut names = Vec::new();

    for arg in args {
        if (arg.starts_with('-') || arg.starts_with('+')) && arg != "-" && arg != "+" {
            let set_attr = arg.starts_with('-');
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
                        } else {
                            unset_uppercase = true;
                        }
                    }
                    'l' => {
                        if set_attr {
                            lowercase = true;
                            uppercase = false;
                        } else {
                            unset_lowercase = true;
                        }
                    }
                    'n' if set_attr => nameref = true,
                    'n' => unset_nameref = true,
                    'r' if set_attr => readonly = true,
                    'r' => unset_readonly = true,
                    'g' => {
                        // TODO(variables.c/builtins/declare.def): `-g` forces
                        // global scope inside functions. Rubash has one
                        // variable table for now.
                    }
                    _ => {
                        writeln!(
                            stderr,
                            "{}declare: {}: unsupported option",
                            diagnostic_prefix(),
                            arg
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                }
            }
        } else {
            names.push(arg.as_str());
        }
    }

    let had_name_args = !names.is_empty();
    let mut assign_names = Vec::new();
    let mut attr_status = EXECUTION_SUCCESS;
    for name in &names {
        if !valid_declare_name(name) {
            writeln!(
                stderr,
                "{}declare: `{}`: not a valid identifier",
                diagnostic_prefix(),
                name
            )?;
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        if nameref && nameref_self_reference(name) {
            let name = name.split_once('=').map(|(name, _)| name).unwrap_or(name);
            writeln!(
                stderr,
                "{}declare: {}: nameref variable self references not allowed",
                diagnostic_prefix(),
                name
            )?;
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        assign_names.push(*name);
    }
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let mut valid_assign_names = Vec::new();
    for name in assign_names {
        let Some(var_name) = declare_base_name(name) else {
            valid_assign_names.push(name);
            continue;
        };
        if assoc && arrays.contains(var_name) && !assocs.contains(var_name) {
            writeln!(
                stderr,
                "{}declare: {}: cannot convert indexed to associative array",
                diagnostic_prefix(),
                var_name
            )?;
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        if array && assocs.contains(var_name) && !arrays.contains(var_name) {
            writeln!(
                stderr,
                "{}declare: {}: cannot convert associative to indexed array",
                diagnostic_prefix(),
                var_name
            )?;
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        valid_assign_names.push(name);
    }
    let assign_names = valid_assign_names;
    if assign_declare_names(&assign_names, variables, array, assoc, integer, stderr)?
        != EXECUTION_SUCCESS
    {
        attr_status = EXECUTION_FAILURE;
    }
    let names = assign_names;
    let options = DeclareOptions {
        export,
        array,
        assoc,
        integer,
        uppercase,
        lowercase,
        nameref,
        readonly,
        unset_export,
        unset_array,
        unset_assoc,
        unset_integer,
        unset_uppercase,
        unset_lowercase,
        unset_nameref,
        unset_readonly,
    };
    attr_status = apply_declare_attrs(&names, variables, options, attr_status, stderr)?;

    if names.is_empty() && !had_name_args {
        print = true;
    }

    if !print {
        return Ok(attr_status);
    }

    print::print_declare_names(&names, variables, options, attr_status, stdout, stderr)
}

#[cfg(test)]
#[path = "declare_tests.rs"]
mod tests;
