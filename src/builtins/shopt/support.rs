use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

pub(super) const SHOPT_OPTIONS: &[&str] = &[
    "array_expand_once",
    "assoc_expand_once",
    "autocd",
    "bash_source_fullpath",
    "cdable_vars",
    "cdspell",
    "checkhash",
    "checkjobs",
    "checkwinsize",
    "cmdhist",
    "compat31",
    "compat32",
    "compat40",
    "compat41",
    "compat42",
    "compat43",
    "compat44",
    "complete_fullquote",
    "direxpand",
    "dirspell",
    "dotglob",
    "execfail",
    "expand_aliases",
    "extdebug",
    "extglob",
    "extquote",
    "failglob",
    "force_fignore",
    "globasciiranges",
    "globskipdots",
    "globstar",
    "gnu_errfmt",
    "histappend",
    "histreedit",
    "histverify",
    "hostcomplete",
    "huponexit",
    "inherit_errexit",
    "interactive_comments",
    "lastpipe",
    "lithist",
    "localvar_inherit",
    "localvar_unset",
    "login_shell",
    "mailwarn",
    "no_empty_cmd_completion",
    "nocaseglob",
    "nocasematch",
    "noexpand_translation",
    "nullglob",
    "patsub_replacement",
    "progcomp",
    "progcomp_alias",
    "promptvars",
    "restricted_shell",
    "shift_verbose",
    "sourcepath",
    "varredir_close",
    "xpg_echo",
];

pub(super) fn default_state() -> HashSet<String> {
    SHOPT_OPTIONS
        .iter()
        .copied()
        .filter(|name| default_enabled(name))
        .map(str::to_string)
        .collect()
}

pub(super) fn print_all_shopts<W>(
    env_vars: &HashMap<String, String>,
    reusable: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    for name in SHOPT_OPTIONS {
        print_shopt(env_vars, name, reusable, stdout)?;
    }
    Ok(())
}

pub(super) fn print_shopts_by_state<W>(
    env_vars: &HashMap<String, String>,
    enabled: bool,
    reusable: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    for name in SHOPT_OPTIONS {
        if super::option_enabled(env_vars, name) == enabled {
            print_shopt(env_vars, name, reusable, stdout)?;
        }
    }
    Ok(())
}

pub(super) fn print_shopt<W>(
    env_vars: &HashMap<String, String>,
    name: &str,
    reusable: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let enabled = super::option_enabled(env_vars, name);
    if reusable {
        writeln!(stdout, "shopt -{} {name}", if enabled { "s" } else { "u" })
    } else {
        writeln!(stdout, "{name:<15}\t{}", if enabled { "on" } else { "off" })
    }
}

pub(crate) fn is_supported_option(name: &str) -> bool {
    SHOPT_OPTIONS.contains(&name)
}

fn default_enabled(name: &str) -> bool {
    matches!(
        name,
        "cmdhist"
            | "checkwinsize"
            | "complete_fullquote"
            | "extquote"
            | "force_fignore"
            | "globasciiranges"
            | "globskipdots"
            | "hostcomplete"
            | "interactive_comments"
            | "patsub_replacement"
            | "progcomp"
            | "promptvars"
            | "sourcepath"
    )
}
