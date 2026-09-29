use std::collections::HashMap;
use std::io::{self, Write};

use crate::executor::mark_env_name;

#[derive(Clone, Copy)]
struct ShellOption {
    name: &'static str,
    default_enabled: bool,
}

const SHELL_OPTIONS: &[ShellOption] = &[
    ShellOption {
        name: "allexport",
        default_enabled: false,
    },
    ShellOption {
        name: "braceexpand",
        default_enabled: true,
    },
    ShellOption {
        name: "emacs",
        // Non-interactive Bash starts with both readline editing modes off.
        // The executor uses this same state for `bash -c`/script execution.
        default_enabled: false,
    },
    ShellOption {
        name: "errexit",
        default_enabled: false,
    },
    ShellOption {
        name: "errtrace",
        default_enabled: false,
    },
    ShellOption {
        name: "functrace",
        default_enabled: false,
    },
    ShellOption {
        name: "hashall",
        default_enabled: true,
    },
    ShellOption {
        name: "histexpand",
        default_enabled: false,
    },
    ShellOption {
        name: "history",
        // History is an interactive-shell feature and is off for scripts.
        default_enabled: false,
    },
    // GNU builtins/set.def:194-237 (o_options): `igncr` is a Cygwin-only
    // option and does not exist upstream.
    ShellOption {
        name: "ignoreeof",
        default_enabled: false,
    },
    ShellOption {
        name: "interactive-comments",
        default_enabled: true,
    },
    ShellOption {
        name: "keyword",
        default_enabled: false,
    },
    ShellOption {
        name: "monitor",
        default_enabled: false,
    },
    ShellOption {
        name: "noclobber",
        default_enabled: false,
    },
    ShellOption {
        name: "noexec",
        default_enabled: false,
    },
    ShellOption {
        name: "noglob",
        default_enabled: false,
    },
    ShellOption {
        name: "nolog",
        default_enabled: false,
    },
    ShellOption {
        name: "notify",
        default_enabled: false,
    },
    ShellOption {
        name: "nounset",
        default_enabled: false,
    },
    ShellOption {
        name: "onecmd",
        default_enabled: false,
    },
    ShellOption {
        name: "physical",
        default_enabled: false,
    },
    ShellOption {
        name: "pipefail",
        default_enabled: false,
    },
    ShellOption {
        name: "posix",
        default_enabled: false,
    },
    ShellOption {
        name: "privileged",
        default_enabled: false,
    },
    // GNU builtins/set.def:194-237 (o_options): `restricted` is not a
    // `set -o` option upstream — restriction is a separate shell flag set
    // by `set -r`/`bash -r` (flags.c change_flag) and never appears in the
    // o_options enumeration or SHELLOPTS.
    ShellOption {
        name: "verbose",
        default_enabled: false,
    },
    ShellOption {
        name: "vi",
        default_enabled: false,
    },
    ShellOption {
        name: "xtrace",
        default_enabled: false,
    },
];

// GNU prints `set -o` listings with MINUS_O_FORMAT "%-15s\t%s\n"
// (builtins/set.def:281) and `shopt -o` listings with OPTFMT "%-20s\t%s\n"
// (builtins/shopt.def:73) — the caller supplies the column width.
pub(crate) const SET_O_PRINT_WIDTH: usize = 15;
pub(crate) const SHOPT_O_PRINT_WIDTH: usize = 20;

pub(crate) fn print_shell_options<W>(
    env_vars: &HashMap<String, String>,
    recreate: bool,
    width: usize,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    for option in SHELL_OPTIONS.iter().map(|option| option.name) {
        let enabled = shell_option_enabled(env_vars, option);
        if recreate {
            writeln!(
                stdout,
                "set {}o {}",
                if enabled { "-" } else { "+" },
                option
            )?;
        } else {
            writeln!(
                stdout,
                "{:<width$}\t{}",
                option,
                if enabled { "on" } else { "off" },
                width = width
            )?;
        }
    }

    Ok(())
}

pub(crate) fn print_shell_options_by_state<W>(
    env_vars: &HashMap<String, String>,
    enabled_state: bool,
    recreate: bool,
    width: usize,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    for option in SHELL_OPTIONS.iter().map(|option| option.name) {
        if shell_option_enabled(env_vars, option) != enabled_state {
            continue;
        }
        if recreate {
            writeln!(
                stdout,
                "set {}o {}",
                if enabled_state { "-" } else { "+" },
                option
            )?;
        } else {
            writeln!(
                stdout,
                "{:<width$}\t{}",
                option,
                if enabled_state { "on" } else { "off" },
                width = width
            )?;
        }
    }
    Ok(())
}

pub(crate) fn print_shell_option<W>(
    env_vars: &HashMap<String, String>,
    name: &str,
    recreate: bool,
    width: usize,
    stdout: &mut W,
) -> io::Result<Option<()>>
where
    W: Write,
{
    if !is_shell_option(name) {
        return Ok(None);
    }
    let enabled = shell_option_enabled(env_vars, name);
    if recreate {
        writeln!(stdout, "set {}o {}", if enabled { "-" } else { "+" }, name)?;
    } else {
        writeln!(
            stdout,
            "{:<width$}\t{}",
            name,
            if enabled { "on" } else { "off" },
            width = width
        )?;
    }
    Ok(Some(()))
}

pub(crate) fn is_shell_option(name: &str) -> bool {
    SHELL_OPTIONS.iter().any(|option| option.name == name)
}

// Consumed previously by the completion setopt action, which now carries its
// own GNU-aligned table (see builtins/complete.rs).
#[allow(dead_code)]
pub(crate) fn shell_option_names() -> impl Iterator<Item = &'static str> {
    SHELL_OPTIONS.iter().map(|option| option.name)
}

pub(crate) fn shell_option_enabled(env_vars: &HashMap<String, String>, name: &str) -> bool {
    // The option key (`__RUBASH_SETOPT_<name>` with `-` folded to `_`) is
    // short ASCII; build it in a stack buffer so the per-command/per-word
    // consults (is_brace_expand_enabled during word expansion, the
    // errexit/xtrace/noexec preambles) do not allocate a String per probe.
    // Same key bytes and same lookups as shell_option_key — names longer
    // than the buffer (none in SHELL_OPTIONS) fall back to it.
    const PREFIX: &str = "__RUBASH_SETOPT_";
    let mut buf = [0u8; 48];
    if PREFIX.len() + name.len() <= buf.len() {
        buf[..PREFIX.len()].copy_from_slice(PREFIX.as_bytes());
        let mut end = PREFIX.len();
        for &byte in name.as_bytes() {
            buf[end] = if byte == b'-' { b'_' } else { byte };
            end += 1;
        }
        if let Some(key) = std::str::from_utf8(&buf[..end]).ok() {
            return match env_vars.get(key) {
                Some(value) => value == "1",
                None => shell_option_default(name),
            };
        }
    }
    let key = shell_option_key(name);
    env_vars
        .get(&key)
        .map(|value| value == "1")
        .unwrap_or_else(|| shell_option_default(name))
}

fn shell_option_default(name: &str) -> bool {
    SHELL_OPTIONS
        .iter()
        .find(|option| option.name == name)
        .map(|option| option.default_enabled)
        .unwrap_or(false)
}

pub(crate) fn shellopts_value(env_vars: &HashMap<String, String>) -> String {
    SHELL_OPTIONS
        .iter()
        .map(|option| option.name)
        .filter(|name| shellopts_includes_option(name))
        .filter(|name| shell_option_enabled(env_vars, name))
        .collect::<Vec<_>>()
        .join(":")
}

/// GNU variables.c:6205-6217 sv_ignoreeof direction: set only the option
/// flag (+ SHELLOPTS) without running the binary-option side effects —
/// used when the VARIABLE was assigned/unset rather than the option.
pub(crate) fn sync_shell_option_flag(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    enabled: bool,
) {
    env_vars.insert(
        shell_option_key(name),
        if enabled { "1" } else { "0" }.to_string(),
    );
    let shelopts = shellopts_value(env_vars);
    env_vars.insert("SHELLOPTS".to_string(), shelopts);
}

pub(crate) fn set_shell_option(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    enabled: bool,
) {
    env_vars.insert(
        shell_option_key(name),
        if enabled { "1" } else { "0" }.to_string(),
    );
    let shelopts = shellopts_value(env_vars);
    env_vars.insert("SHELLOPTS".to_string(), shelopts);
    // GNU builtins/set.def:388-399 set_ignoreeof: `set -o ignoreeof` binds
    // IGNOREEOF=10 (which sv_ignoreeof then reads back); `set +o` unbinds
    // the variable entirely.
    if name == "ignoreeof" {
        if enabled {
            env_vars.insert("IGNOREEOF".to_string(), "10".to_string());
        } else {
            env_vars.remove("IGNOREEOF");
        }
    }
    if name == "restricted" && enabled {
        // GNU shell.c maybe_make_restricted (shell.c:1278-1299): PATH, SHELL,
        // ENV, BASH_ENV and HISTFILE become read-only; CDPATH is untouched.
        for variable in ["PATH", "SHELL", "ENV", "BASH_ENV", "HISTFILE"] {
            mark_env_name(env_vars, super::READONLY_VARS, variable);
        }
    }
    // GNU builtins/shopt.def:617-627 (shopt_set_debug_mode):
    // error_trace_mode = function_trace_mode = debugging_mode. Turning
    // extdebug on enables functrace (and errtrace); turning it off clears
    // them, so a later \`set +T\` still stops DEBUG/RETURN inheritance into
    // functions even with extdebug active (dbg-support.tests sets extdebug
    // at line 19 and set +T at line 91).
    if name == "extdebug" {
        for option in ["functrace", "errtrace"] {
            env_vars.insert(
                shell_option_key(option),
                if enabled { "1" } else { "0" }.to_string(),
            );
        }
        let shelopts = shellopts_value(env_vars);
        env_vars.insert("SHELLOPTS".to_string(), shelopts);
    }
}

fn shell_option_key(name: &str) -> String {
    format!("__RUBASH_SETOPT_{}", name.replace('-', "_"))
}

fn shellopts_includes_option(name: &str) -> bool {
    !matches!(name, "emacs" | "history" | "vi")
}
