//! shopt module.
//!
//! GNU Bash source ownership:
// - builtins/shopt.def

mod support;

pub(crate) use support::is_supported_option;
use support::{default_state, print_all_shopts, print_shopt, print_shopts_by_state, SHOPT_OPTIONS};

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const SHOPT_STATE: &str = "__RUBASH_SHOPT_STATE";

static XPG_ECHO: AtomicBool = AtomicBool::new(false);
static SOURCEPATH: AtomicBool = AtomicBool::new(true);
static CHECKHASH: AtomicBool = AtomicBool::new(false);

pub(crate) fn xpg_echo_enabled() -> bool {
    XPG_ECHO.load(Ordering::Relaxed)
}

pub(crate) fn sourcepath_enabled() -> bool {
    SOURCEPATH.load(Ordering::Relaxed)
}

pub(crate) fn checkhash_enabled() -> bool {
    CHECKHASH.load(Ordering::Relaxed)
}

pub(crate) fn cdable_vars_enabled(env_vars: &HashMap<String, String>) -> bool {
    option_enabled(env_vars, "cdable_vars")
}

pub(crate) fn bashopts_value(env_vars: &HashMap<String, String>) -> String {
    SHOPT_OPTIONS
        .iter()
        .copied()
        .filter(|name| option_enabled(env_vars, name))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
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
    let mut print = args.is_empty();
    let mut use_set_options = false;
    let mut mode = ShoptMode::List;
    let mut names = Vec::new();
    let mut status = EXECUTION_SUCCESS;

    for arg in args {
        if arg == "--" {
            continue;
        }
        if arg.starts_with('-') && arg != "-" {
            for option in arg[1..].chars() {
                match option {
                    's' => mode = ShoptMode::Set,
                    'u' => mode = ShoptMode::Unset,
                    'q' => mode = ShoptMode::Query,
                    'p' => print = true,
                    'o' => use_set_options = true,
                    other => {
                        writeln!(
                            stderr,
                            "{}shopt: -{}: invalid option",
                            diagnostic_prefix(),
                            other
                        )?;
                        writeln!(stderr, "shopt: usage: shopt [-pqsu] [-o] [optname ...]")?;
                        return Ok(EX_USAGE);
                    }
                }
            }
        } else {
            names.push(arg.as_str());
        }
    }

    if use_set_options {
        return execute_set_option_mode(mode, print, &names, env_vars, stdout, stderr);
    }

    if names.is_empty() {
        match mode {
            ShoptMode::Set if print => {
                print_shopts_by_state(env_vars, true, true, stdout)?;
            }
            ShoptMode::Unset if print => {
                print_shopts_by_state(env_vars, false, true, stdout)?;
            }
            ShoptMode::Unset => {
                print_shopts_by_state(env_vars, false, false, stdout)?;
            }
            ShoptMode::List | ShoptMode::Query => {
                print_all_shopts(env_vars, print, stdout)?;
            }
            ShoptMode::Set => {}
        }
        return Ok(status);
    }

    for name in names {
        if !is_supported_option(name) {
            writeln!(
                stderr,
                "{}shopt: {name}: invalid shell option name",
                diagnostic_prefix()
            )?;
            status = EXECUTION_FAILURE;
            continue;
        }

        match mode {
            ShoptMode::Set => set_option(env_vars, name, true),
            ShoptMode::Unset => set_option(env_vars, name, false),
            ShoptMode::Query if !option_enabled(env_vars, name) => status = EXECUTION_FAILURE,
            ShoptMode::Query => {}
            ShoptMode::List if print => print_shopt(env_vars, name, true, stdout)?,
            ShoptMode::List => {
                if !option_enabled(env_vars, name) {
                    status = EXECUTION_FAILURE;
                }
                print_shopt(env_vars, name, false, stdout)?;
            }
        }
    }

    Ok(status)
}

fn execute_set_option_mode<W, E>(
    mode: ShoptMode,
    print: bool,
    names: &[&str],
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut status = EXECUTION_SUCCESS;
    if names.is_empty() {
        match mode {
            ShoptMode::Set if print => {
                crate::builtins::set::print_shell_options_by_state(env_vars, true, true, stdout)?;
            }
            ShoptMode::Unset if print => {
                crate::builtins::set::print_shell_options_by_state(env_vars, false, true, stdout)?;
            }
            ShoptMode::Unset => {
                crate::builtins::set::print_shell_options_by_state(env_vars, false, false, stdout)?;
            }
            _ => crate::builtins::set::print_shell_options(env_vars, print, stdout)?,
        }
        return Ok(status);
    }

    for name in names {
        if !crate::builtins::set::is_shell_option(name) {
            writeln!(
                stderr,
                "{}shopt: {name}: invalid option name",
                diagnostic_prefix()
            )?;
            status = EXECUTION_FAILURE;
            continue;
        }
        match mode {
            ShoptMode::Set if !print => {
                crate::builtins::set::set_shell_option(env_vars, name, true);
            }
            ShoptMode::Unset if !print => {
                crate::builtins::set::set_shell_option(env_vars, name, false);
            }
            _ if print || mode == ShoptMode::List => {
                crate::builtins::set::print_shell_option(env_vars, name, true, stdout)?;
            }
            ShoptMode::Query if !crate::builtins::set::shell_option_enabled(env_vars, name) => {
                status = EXECUTION_FAILURE;
            }
            ShoptMode::Query => {}
            ShoptMode::List | ShoptMode::Set | ShoptMode::Unset => {}
        }
    }

    Ok(status)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShoptMode {
    List,
    Set,
    Unset,
    Query,
}

pub(crate) fn option_enabled(env_vars: &HashMap<String, String>, name: &str) -> bool {
    match name {
        "xpg_echo" => xpg_echo_enabled(),
        "checkhash" => checkhash_enabled(),
        "sourcepath" => sourcepath_enabled(),
        _ => state(env_vars).contains(name),
    }
}

pub(crate) fn set_option(env_vars: &mut HashMap<String, String>, name: &str, enabled: bool) {
    match name {
        "xpg_echo" => XPG_ECHO.store(enabled, Ordering::Relaxed),
        "sourcepath" => SOURCEPATH.store(enabled, Ordering::Relaxed),
        "checkhash" => {
            CHECKHASH.store(enabled, Ordering::Relaxed);
            if enabled {
                std::env::set_var("__RUBASH_SHOPT_CHECKHASH", "1");
            } else {
                std::env::remove_var("__RUBASH_SHOPT_CHECKHASH");
            }
        }
        _ => {}
    }

    let mut state = state(env_vars);
    if enabled {
        state.insert(name.to_string());
    } else {
        state.remove(name);
    }
    env_vars.insert(SHOPT_STATE.to_string(), serialize_state(&state));
}

fn state(env_vars: &HashMap<String, String>) -> HashSet<String> {
    let Some(value) = env_vars.get(SHOPT_STATE) else {
        return default_state();
    };
    value
        .split('\x1f')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn serialize_state(state: &HashSet<String>) -> String {
    let mut names: Vec<&str> = state.iter().map(String::as_str).collect();
    names.sort();
    names.join("\x1f")
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
