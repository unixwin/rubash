use std::collections::HashMap;
use std::io::{self, Write};

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
        default_enabled: true,
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
        default_enabled: true,
    },
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

pub(crate) fn print_shell_options<W>(
    env_vars: &HashMap<String, String>,
    recreate: bool,
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
                "{:<15}\t{}",
                option,
                if enabled { "on" } else { "off" }
            )?;
        }
    }

    Ok(())
}

pub(crate) fn print_shell_options_by_state<W>(
    env_vars: &HashMap<String, String>,
    enabled_state: bool,
    recreate: bool,
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
                "{:<15}\t{}",
                option,
                if enabled_state { "on" } else { "off" }
            )?;
        }
    }
    Ok(())
}

pub(crate) fn print_shell_option<W>(
    env_vars: &HashMap<String, String>,
    name: &str,
    recreate: bool,
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
            "{:<15}\t{}",
            name,
            if enabled { "on" } else { "off" }
        )?;
    }
    Ok(Some(()))
}

pub(crate) fn is_shell_option(name: &str) -> bool {
    SHELL_OPTIONS.iter().any(|option| option.name == name)
}

pub(crate) fn shell_option_enabled(env_vars: &HashMap<String, String>, name: &str) -> bool {
    let key = shell_option_key(name);
    env_vars
        .get(&key)
        .map(|value| value == "1")
        .unwrap_or_else(|| {
            SHELL_OPTIONS
                .iter()
                .find(|option| option.name == name)
                .map(|option| option.default_enabled)
                .unwrap_or(false)
        })
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

pub(crate) fn set_shell_option(env_vars: &mut HashMap<String, String>, name: &str, enabled: bool) {
    env_vars.insert(
        shell_option_key(name),
        if enabled { "1" } else { "0" }.to_string(),
    );
    env_vars.insert("SHELLOPTS".to_string(), shellopts_value(env_vars));
}

fn shell_option_key(name: &str) -> String {
    format!("__RUBASH_SETOPT_{}", name.replace('-', "_"))
}

fn shellopts_includes_option(name: &str) -> bool {
    !matches!(name, "emacs" | "history" | "vi")
}
