//! zsh-style option compatibility builtins.
//!
//! Winuxsh migration/startup files commonly contain `setopt`/`unsetopt`.
//! Rubash owns shell execution, so these builtins live here and map obvious
//! zsh options onto existing Bash-compatible shell/shopt state.

use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const ZSH_OPTION_STATE: &str = "__RUBASH_ZSH_OPTIONS";

pub fn setopt(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io("setopt", true, args, env_vars, &mut stdout, &mut stderr)
}

pub fn unsetopt(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io("unsetopt", false, args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    command_name: &str,
    enable: bool,
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    if args.is_empty() {
        print_options(env_vars, stdout)?;
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    let mut parse_options = true;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        if parse_options && arg.starts_with('-') {
            match arg.as_str() {
                "-m" | "-o" => continue,
                _ => {
                    writeln!(
                        stderr,
                        "{}{command_name}: unsupported option",
                        diagnostic_prefix()
                    )?;
                    status = EXECUTION_FAILURE;
                    continue;
                }
            }
        }

        let Some((option, option_enable)) = resolve_option_arg(arg, enable) else {
            writeln!(
                stderr,
                "{}{command_name}: no such option: {arg}",
                diagnostic_prefix()
            )?;
            status = EXECUTION_FAILURE;
            continue;
        };
        set_option(env_vars, option, option_enable);
    }

    Ok(status)
}

pub(crate) fn enabled_options(env_vars: &HashMap<String, String>) -> HashSet<String> {
    env_vars
        .get(ZSH_OPTION_STATE)
        .map(|value| {
            value
                .split(DATA_DOLLAR)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn set_option(env_vars: &mut HashMap<String, String>, option: &'static str, enabled: bool) {
    let mut state = enabled_options(env_vars);
    if enabled {
        state.insert(option.to_string());
    } else {
        state.remove(option);
    }
    env_vars.insert(ZSH_OPTION_STATE.to_string(), serialize_state(&state));

    if let Some(shopt) = shopt_equivalent(option) {
        crate::builtins::shopt::set_option(env_vars, shopt, enabled);
    }
    if let Some(shell_option) = shell_option_equivalent(option) {
        crate::builtins::set::set_shell_option(env_vars, shell_option, enabled);
    }
    match option {
        "hist_ignore_dups" => {
            env_vars.insert(
                "WINUXSH_HIST_IGNORE_DUPS".to_string(),
                if enabled { "1" } else { "0" }.to_string(),
            );
        }
        "hist_ignore_space" => {
            env_vars.insert(
                "WINUXSH_HIST_IGNORE_SPACE".to_string(),
                if enabled { "1" } else { "0" }.to_string(),
            );
        }
        _ => {}
    }
}

fn shopt_equivalent(option: &str) -> Option<&'static str> {
    match option {
        "append_history" => Some("histappend"),
        "auto_cd" => Some("autocd"),
        "extended_glob" => Some("extglob"),
        "glob_dots" => Some("dotglob"),
        "hist_verify" => Some("histverify"),
        "nomatch" => Some("failglob"),
        "null_glob" => Some("nullglob"),
        "prompt_subst" => Some("promptvars"),
        _ => None,
    }
}

fn shell_option_equivalent(option: &str) -> Option<&'static str> {
    match option {
        "brace_expand" => Some("braceexpand"),
        "interactive_comments" => Some("interactive-comments"),
        "monitor" => Some("monitor"),
        _ => None,
    }
}

fn print_options<W>(env_vars: &HashMap<String, String>, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let mut options: Vec<String> = enabled_options(env_vars).into_iter().collect();
    options.sort();
    for option in options {
        writeln!(stdout, "{option}")?;
    }
    Ok(())
}

fn resolve_option_arg(name: &str, enable: bool) -> Option<(&'static str, bool)> {
    let key = option_key(name);
    if let Some(stripped) = key.strip_prefix("no") {
        if let Some(option) = canonical_option(stripped) {
            return Some((option, !enable));
        }
    }
    canonical_option(&key).map(|option| (option, enable))
}

fn option_key(name: &str) -> String {
    name.chars()
        .filter(|ch| *ch != '_' && *ch != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn canonical_option(key: &str) -> Option<&'static str> {
    match key {
        "appendhistory" => Some("append_history"),
        "autocd" => Some("auto_cd"),
        "autopushd" => Some("auto_pushd"),
        "arithexpand" | "arithsubst" => Some("arith_expand"),
        "beep" => Some("beep"),
        "braceexpand" => Some("brace_expand"),
        "commandsubst" => Some("command_subst"),
        "completealiases" => Some("complete_aliases"),
        "extendedglob" => Some("extended_glob"),
        "globdots" => Some("glob_dots"),
        "histfindnodups" => Some("hist_find_no_dups"),
        "histignorealldups" => Some("hist_ignore_all_dups"),
        "histignoredups" => Some("hist_ignore_dups"),
        "histignorespace" => Some("hist_ignore_space"),
        "histreduceblanks" => Some("hist_reduce_blanks"),
        "histsavenodups" => Some("hist_save_no_dups"),
        "histverify" => Some("hist_verify"),
        "incappendhistory" => Some("inc_append_history"),
        "interactivecomments" => Some("interactive_comments"),
        "monitor" => Some("monitor"),
        "nomatch" => Some("nomatch"),
        "nullglob" => Some("null_glob"),
        "promptpercent" => Some("prompt_percent"),
        "promptsubst" => Some("prompt_subst"),
        "pushdignoredups" => Some("pushd_ignore_dups"),
        "sharehistory" => Some("share_history"),
        "tildeexpand" => Some("tilde_expand"),
        "variableexpand" => Some("variable_expand"),
        _ => None,
    }
}

fn serialize_state(state: &HashSet<String>) -> String {
    let mut names: Vec<&str> = state.iter().map(String::as_str).collect();
    names.sort();
    names.join(DATA_DOLLAR_STR)
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
