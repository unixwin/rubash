//! `export` and `readonly` attribute builtins.
//!
//! GNU Bash source ownership:
//! - builtins/setattr.def (`export_builtin`, `readonly_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

mod apply;
mod marks;
mod print;
mod value;

use apply::{apply_export_arg, apply_readonly_arg};
use print::{print_exported, print_readonly};
use value::diagnostic_prefix;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
use crate::executor::types::COMPOUND_ASSIGNMENT_MARKER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExportMode {
    Set,
    Unset,
}

pub fn export(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    export_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

/// Execute `readonly` with arguments after the command name.
pub fn readonly(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr().lock();
    readonly_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
        None,
    )
}

pub(crate) fn export_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut mode = ExportMode::Set;
    let mut print = false;
    let mut array = false;
    let mut assoc = false;
    let mut func_export = false;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'n' => mode = ExportMode::Unset,
                'p' => print = true,
                'a' => array = true,
                'A' => assoc = true,
                'f' => {
                    // export -f: mark function for export
                    // Store the function definition in an environment variable
                    func_export = true;
                }
                other => {
                    writeln!(stderr, "rubash: export: -{}: invalid option", other)?;
                    writeln!(
                        stderr,
                        "export: usage: export [-fn] [name[=value] ...] or export -p"
                    )?;
                    return Ok(EX_USAGE);
                }
            }
        }

        index += 1;
    }

    // GNU setattr.def set_or_show_attributes: the listing branch runs only
    // when NO operand names remain (loptend == NULL, the `else` arm at
    // setattr.def:291-345). With names present the -p flag is ignored —
    // each name is marked/assigned silently (`export -p NAME` prints
    // nothing), exactly like readonly.
    if index >= args.len() {
        let _ = print;
        print_exported(env_vars, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    // Handle export -f: mark functions for export
    if func_export {
        for arg in &args[index..] {
            let func_name = arg.to_string();
            // Store the function name in BASH_FUNC_<name>%% environment variable
            // This is the standard way bash exports functions
            let env_key = format!("BASH_FUNC_{}%%", func_name);
            env_vars.insert(env_key, "() { :; }".to_string());
        }
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in &args[index..] {
        if apply_export_arg(arg, mode, array, assoc, env_vars, stderr)? != EXECUTION_SUCCESS {
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

pub(crate) fn readonly_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
    context_name: Option<&str>,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut print = false;
    let mut array = false;
    let mut assoc = false;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || *arg == "-" {
            break;
        }
        for option in arg[1..].chars() {
            match option {
                'p' => print = true,
                'a' => array = true,
                'A' => assoc = true,
                'f' => {}
                // GNU setattr.def ATTROPTS includes `n` (undo attribute).
                // For `readonly`, undoing att_readonly is silently disallowed
                // (setattr.def lines 174-176), so `-n` is an accepted no-op.
                'n' => {}
                other => {
                    writeln!(
                        stderr,
                        "{}readonly: -{}: invalid option",
                        diagnostic_prefix(),
                        other
                    )?;
                    writeln!(
                        stderr,
                        "readonly: usage: readonly [-aAf] [name[=value] ...] or readonly -p"
                    )?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    // GNU setattr.def set_or_show_attributes: with operand names the -p
    // flag is ignored — the operand branch (setattr.def:174-291) only calls
    // set_var_attribute, which never prints. Only the nameless invocation
    // (the else arm at :291-345) lists; without names, -a/-A restrict the
    // printed set to readonly indexed/associative arrays.
    if index >= args.len() {
        let _ = print;
        print_readonly(
            env_vars,
            array,
            assoc,
            env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1"),
            stdout,
        )?;
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    for arg in &args[index..] {
        if apply_readonly_arg(arg, array, assoc, env_vars, stderr, context_name)?
            != EXECUTION_SUCCESS
        {
            status = EXECUTION_FAILURE;
        }
    }
    Ok(status)
}

#[cfg(test)]
#[path = "setattr_tests.rs"]
mod tests;
