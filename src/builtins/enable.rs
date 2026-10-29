//! `enable` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/enable.def (`enable_builtin`)

use crate::shell::var_table::VarTable;
use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
/// GNU shell.h EX_BADUSAGE: usage errors exit 2.
const EX_USAGE: i32 = 2;

const DISABLED_BUILTINS: &str = "__RUBASH_DISABLED_BUILTINS";

pub fn execute(args: &[String], env_vars: &mut VarTable) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut VarTable,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // TODO(builtins/enable.def/builtins.c): Bash can dynamically load/unload
    // builtins. This implementation tracks the builtins that rubash dispatches.
    let mut list_all = false;
    let mut disable = false;
    let mut reusable = false;
    let mut special_only = false;
    let mut delete = false;
    let mut load_file = None;
    let mut operands = Vec::new();

    let mut index = 0;
    // GNU builtins/bashgetopt.c:76 (NOTOPT) and :84-89: option parsing ends
    // at the first non-option word or at a bare `--` (consumed, not an
    // operand); the rest are names. (rubash#1375)
    let mut parsing_options = true;
    while index < args.len() {
        let arg = &args[index];
        if parsing_options && arg == "--" {
            parsing_options = false;
            index += 1;
            continue;
        }
        if parsing_options && arg.starts_with('-') && arg != "-" {
            let mut options = arg[1..].char_indices().peekable();
            while let Some((offset, option)) = options.next() {
                match option {
                    'a' => list_all = true,
                    'n' => disable = true,
                    'p' => reusable = true,
                    's' => special_only = true,
                    'd' => delete = true,
                    'f' => {
                        let value_start = 1 + offset + option.len_utf8();
                        if value_start < arg.len() {
                            load_file = Some(arg[value_start..].to_string());
                            break;
                        }
                        let Some(value) = args.get(index + 1) else {
                            writeln!(
                                stderr,
                                "{}enable: -f: option requires an argument",
                                diagnostic_prefix(env_vars)
                            )?;
                            writeln!(
                                stderr,
                                "enable: usage: enable [-a] [-dnps] [-f filename] [name ...]"
                            )?;
                            // rubash#705 R7.4: GNU enable.def routes a
                            // missing -f operand through internal_getopt's
                            // `?` into the default arm — builtin_usage()
                            // + `return (EX_USAGE)` (enable.def:163): the
                            // usage failure is 2, not EXECUTION_FAILURE.
                            // (GNU 5.3.0 probe: `enable -f` exits 2.)
                            return Ok(EX_USAGE);
                        };
                        load_file = Some(value.clone());
                        index += 1;
                        break;
                    }
                    _ => {
                        writeln!(
                            stderr,
                            "{}enable: -{option}: invalid option",
                            diagnostic_prefix(env_vars)
                        )?;
                        writeln!(
                            stderr,
                            "enable: usage: enable [-a] [-dnps] [-f filename] [name ...]"
                        )?;
                        // rubash#705 R7.4: the default getopt arm in
                        // enable.def:161-163 returns EX_USAGE for an
                        // invalid option. (GNU 5.3.0 probe: `enable -Q`
                        // exits 2.)
                        return Ok(EX_USAGE);
                    }
                }
            }
        } else {
            parsing_options = false;
            operands.push(arg.as_str());
        }
        index += 1;
    }

    if let Some(file) = load_file {
        if operands.is_empty() {
            writeln!(
                stderr,
                "{}enable: {file}: dynamic loading not supported",
                diagnostic_prefix(env_vars)
            )?;
            return Ok(EXECUTION_FAILURE);
        }
        for name in operands {
            writeln!(
                stderr,
                "{}enable: {name}: dynamic loading from {file} not supported",
                diagnostic_prefix(env_vars)
            )?;
        }
        return Ok(EXECUTION_FAILURE);
    }

    if delete {
        let mut status = EXECUTION_SUCCESS;
        for name in operands {
            if !is_builtin(name) {
                writeln!(
                    stderr,
                    "{}enable: {name}: not a shell builtin",
                    diagnostic_prefix(env_vars)
                )?;
                status = EXECUTION_FAILURE;
            } else {
                writeln!(
                    stderr,
                    "{}enable: {name}: not dynamically loaded",
                    diagnostic_prefix(env_vars)
                )?;
                status = EXECUTION_FAILURE;
            }
        }
        return Ok(status);
    }

    // GNU builtins/enable.def (`if (list == 0 || (flags & PFLAG))`): `-p`
    // lists even when operands are present — the operands are ignored
    // (rubash#1397). Plain `enable` with no operands lists in every mode.
    if operands.is_empty() || reusable {
        if special_only {
            let disabled = disabled_builtins(env_vars);
            for name in crate::executor::command_registry::SPECIAL_BUILTINS {
                if disable {
                    if disabled.iter().any(|disabled| disabled == name) {
                        writeln!(stdout, "enable -n {name}")?;
                    }
                } else {
                    writeln!(stdout, "enable {name}")?;
                }
            }
        } else if disable {
            for name in disabled_builtins(env_vars) {
                writeln!(stdout, "enable -n {name}")?;
            }
        } else {
            let disabled = disabled_builtins(env_vars);
            for name in crate::executor::command_registry::introspection_builtin_names() {
                if disabled.iter().any(|disabled| disabled == name) {
                    if list_all {
                        writeln!(stdout, "enable -n {name}")?;
                    }
                } else {
                    writeln!(stdout, "enable {name}")?;
                }
            }
        }
        return Ok(EXECUTION_SUCCESS);
    }

    let mut disabled = disabled_builtins(env_vars);
    let mut status = EXECUTION_SUCCESS;
    for name in operands {
        if !is_builtin(name) {
            writeln!(
                stderr,
                "{}enable: {name}: not a shell builtin",
                diagnostic_prefix(env_vars)
            )?;
            status = EXECUTION_FAILURE;
            continue;
        }

        if disable {
            if !disabled.iter().any(|disabled| disabled == name) {
                disabled.push(name.to_string());
            }
        } else {
            disabled.retain(|disabled| disabled != name);
        }
    }
    set_disabled_builtins(env_vars, &disabled);
    Ok(status)
}

/// Change a builtin's enabled state without parsing an `enable` command.
/// Embedding hosts use this for policy defaults before user startup code runs.
pub fn set_disabled(env_vars: &mut VarTable, name: &str, disabled: bool) {
    let mut builtins = disabled_builtins(env_vars);
    if disabled {
        if !builtins.iter().any(|builtin| builtin == name) {
            builtins.push(name.to_string());
        }
    } else {
        builtins.retain(|builtin| builtin != name);
    }
    set_disabled_builtins(env_vars, &builtins);
}

pub fn is_disabled(env_vars: &HashMap<String, String>, name: &str) -> bool {
    disabled_builtins(env_vars)
        .iter()
        .any(|disabled| disabled == name)
}

fn disabled_builtins(env_vars: &HashMap<String, String>) -> Vec<String> {
    env_vars
        .get(DISABLED_BUILTINS)
        .map(|value| {
            value
                .split(':')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn set_disabled_builtins(env_vars: &mut VarTable, disabled: &[String]) {
    if disabled.is_empty() {
        env_vars.remove(DISABLED_BUILTINS);
    } else {
        env_vars.insert(DISABLED_BUILTINS.to_string(), disabled.join(":"));
    }
}

fn is_builtin(name: &str) -> bool {
    // The registry projection (rubash#2036 S2): `env` is a builtin
    // here too — KD-1's third face. The hand ALL_BUILTINS copy that
    // lacked env is gone.
    crate::executor::command_registry::is_introspection_builtin(name)
}

fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    // rubash#705 R7.4: GNU's report_prolog falls back to the shell name
    // ("bash:") when neither a script nor a line context exists. The old
    // `rubash:` fallback was a second brand leaking into user-visible
    // diagnostics.
    "bash: ".to_string()
}
