//! `enable` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/enable.def (`enable_builtin`)

use crate::builtins::dynamic;
use crate::shell::var_table::VarTable;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
/// GNU shell.h EX_BADUSAGE: usage errors exit 2.
const EX_USAGE: i32 = 2;

const DISABLED_BUILTINS: &str = "__RUBASH_DISABLED_BUILTINS";

pub fn execute(args: &[String], env_vars: &mut VarTable) -> io::Result<i32> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io_in_dir(args, env_vars, &cwd, &mut stdout, &mut stderr)
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
    let cwd = std::env::current_dir().unwrap_or_default();
    execute_with_io_in_dir(args, env_vars, &cwd, stdout, stderr)
}

/// `enable` entry point with the shell's physical cwd.  Loadable objects
/// are resolved against the SHELL cwd, not the host process cwd — rubash
/// `cd` moves `ShellState::cwd` without calling `chdir` (see
/// `src/shell/state.rs`), while GNU resolves `make_absolute(filename,
/// ".")` against the process cwd because its `cd` really chdirs.
pub(crate) fn execute_with_io_in_dir<W, E>(
    args: &[String],
    env_vars: &mut VarTable,
    cwd: &Path,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
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

    // GNU builtins/enable.def (`if (list == 0 || (flags & PFLAG))`): this
    // branch is checked BEFORE -f/-d, so `enable -f file` with no NAME
    // and `enable -p -f file NAME` both list instead of loading. `-p`
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
            // GNU list_some_builtins walks the whole shell_builtins table,
            // so a loadable builtin loaded with `-s` appears here too.
            for name in dynamic::loaded_names() {
                if !dynamic::is_special(&name) || is_static_special_builtin(&name) {
                    continue;
                }
                if disable {
                    if !dynamic::is_enabled(&name) {
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
            // GNU appends dynamically loaded builtins after the compiled
            // table (enable.def:449-466).  Skip a loaded name that already
            // has a static row: a shadowing load replaces that one slot.
            for name in dynamic::loaded_names() {
                if crate::executor::command_registry::is_static_introspection_builtin(&name) {
                    continue;
                }
                if disabled.iter().any(|disabled| disabled == &name) {
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

    if let Some(file) = load_file {
        let names: Vec<String> = operands.iter().map(|name| name.to_string()).collect();
        let loadables_path = env_vars.get("BASH_LOADABLES_PATH").map(String::as_str);
        match dynamic::load_builtin(&file, &names, loadables_path, cwd, special_only, disable) {
            Ok(outcome) => {
                for warning in &outcome.warnings {
                    writeln!(
                        stderr,
                        "{}enable: warning: {warning}",
                        diagnostic_prefix(env_vars)
                    )?;
                }
                for error in &outcome.errors {
                    writeln!(stderr, "{}enable: {error}", diagnostic_prefix(env_vars))?;
                }
                for name in &outcome.loaded {
                    set_builtin_enabled(env_vars, name, !disable);
                }
                // GNU enable.def:420-446: diagnostics for individual
                // names do not turn the whole load into a failure when
                // at least one builtin was installed.
                if outcome.loaded.is_empty() {
                    return Ok(EXECUTION_FAILURE);
                }
                return Ok(EXECUTION_SUCCESS);
            }
            Err(error) => {
                writeln!(stderr, "{}enable: {error}", diagnostic_prefix(env_vars))?;
                return Ok(EXECUTION_FAILURE);
            }
        }
    }

    if delete {
        // GNU enable.def:536-586 (`dyn_unload_builtin`): NAME must be a
        // builtin and must have been loaded with -f; a static builtin is
        // reported "not dynamically loaded".
        let mut status = EXECUTION_SUCCESS;
        for name in operands {
            if !is_builtin(name) {
                writeln!(
                    stderr,
                    "{}enable: {name}: not a shell builtin",
                    diagnostic_prefix(env_vars)
                )?;
                status = EXECUTION_FAILURE;
            } else if !dynamic::is_loaded(name) {
                writeln!(
                    stderr,
                    "{}enable: {name}: not dynamically loaded",
                    diagnostic_prefix(env_vars)
                )?;
                status = EXECUTION_FAILURE;
            } else {
                match dynamic::unload_builtin(name) {
                    Ok(()) => set_builtin_enabled(env_vars, name, true),
                    Err(error) => {
                        writeln!(stderr, "{}enable: {error}", diagnostic_prefix(env_vars))?;
                        status = EXECUTION_FAILURE;
                    }
                }
            }
        }
        return Ok(status);
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
        // Keep a loaded struct's BUILTIN_ENABLED bit in sync.
        dynamic::set_enabled(name, !disable);
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
    dynamic::set_enabled(name, !disabled);
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
    // lacked env is gone. Loadable builtins join the same table from
    // the process-global dynamic registry.
    crate::executor::command_registry::is_introspection_builtin(name) || dynamic::is_loaded(name)
}

fn is_static_special_builtin(name: &str) -> bool {
    crate::executor::command_registry::SPECIAL_BUILTINS.contains(&name)
}

/// Set NAME's enabled state in the existing `enable -n` carrier used by
/// every dispatch path.  Loading with `-n` records the same disabled
/// state as `enable -n NAME` (GNU clears BUILTIN_ENABLED, enable.def:430).
fn set_builtin_enabled(env_vars: &mut VarTable, name: &str, enabled: bool) {
    let mut disabled = disabled_builtins(env_vars);
    if enabled {
        disabled.retain(|disabled| disabled != name);
    } else if !disabled.iter().any(|disabled| disabled == name) {
        disabled.push(name.to_string());
    }
    set_disabled_builtins(env_vars, &disabled);
    dynamic::set_enabled(name, enabled);
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
