//! `enable` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/enable.def (`enable_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

const DISABLED_BUILTINS: &str = "__RUBASH_DISABLED_BUILTINS";
const ALL_BUILTINS: &[&str] = &[
    ".",
    ":",
    "[",
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "help",
    "history",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "wait",
];
const SPECIAL_BUILTINS: &[&str] = &[
    ".", ":", "break", "continue", "eval", "exec", "exit", "export", "readonly", "return", "set",
    "shift", "source", "times", "trap", "unset",
];

pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut HashMap<String, String>,
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
    let mut operands = Vec::new();

    for arg in args {
        if arg.starts_with('-') && arg != "-" {
            for option in arg[1..].chars() {
                match option {
                    'a' => list_all = true,
                    'n' => disable = true,
                    'p' => reusable = true,
                    's' => special_only = true,
                    'd' => delete = true,
                    _ => {
                        writeln!(stderr, "rubash: enable: -{option}: invalid option")?;
                        return Ok(EXECUTION_FAILURE);
                    }
                }
            }
        } else {
            operands.push(arg.as_str());
        }
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
            } else {
                writeln!(
                    stderr,
                    "{}enable: {name}: not dynamically loaded",
                    diagnostic_prefix(env_vars)
                )?;
            }
            status = EXECUTION_FAILURE;
        }
        return Ok(status);
    }

    if operands.is_empty() || reusable || list_all {
        if special_only {
            let disabled = disabled_builtins(env_vars);
            for name in SPECIAL_BUILTINS {
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
            for name in ALL_BUILTINS {
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

fn set_disabled_builtins(env_vars: &mut HashMap<String, String>, disabled: &[String]) {
    if disabled.is_empty() {
        env_vars.remove(DISABLED_BUILTINS);
    } else {
        env_vars.insert(DISABLED_BUILTINS.to_string(), disabled.join(":"));
    }
}

fn is_builtin(name: &str) -> bool {
    ALL_BUILTINS.contains(&name)
}

fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
