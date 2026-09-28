use super::*;
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};

/// Exact-case bare-name fallback into the process environment.
///
/// GNU variables.c: the shell variable table is case-sensitive and the
/// process environment is imported once at startup with its stored
/// spelling (variables.c initialize_shell_variables). Windows
/// `std::env::var` looks up the environment block case-INSENSITIVELY, so
/// using it for the bare `$NAME` fallback resolves `$m2` against an
/// ambient `M2` (e.g. `$username` leaking `USERNAME`). The fallback must
/// match the stored key exactly; a case-variant miss is simply unbound
/// (rubash#273: `M2=x rubash -u -c 'echo $m2'` must fail like GNU).
pub(in crate::executor) fn exact_case_env_var(name: &str) -> Option<String> {
    std::env::vars()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

pub(in crate::executor) fn mark_initial_exported_vars(env_vars: &mut HashMap<String, String>) {
    let mut names: Vec<String> = env_vars
        .keys()
        .filter(|name| is_initial_export_candidate(name))
        .cloned()
        .collect();
    names.sort();
    env_vars.insert(EXPORTED_VARS.to_string(), names.join(DATA_DOLLAR_STR));
}

pub(in crate::executor) fn initialize_shell_level(env_vars: &mut HashMap<String, String>) {
    let next_level = env_vars
        .get("SHLVL")
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|level| *level >= 0)
        .map(|level| level.saturating_add(1))
        .unwrap_or(1);
    env_vars.insert("SHLVL".to_string(), next_level.to_string());
}

pub(in crate::executor) fn is_initial_export_candidate(name: &str) -> bool {
    // Test runs share one process environment; ignore shell-local names that
    // previous Executor instances may have written there.
    if name.starts_with("__RUBASH_") || name.len() <= 1 || is_bash_managed_shell_var(name) {
        return false;
    }
    // GNU variables.c:3307 bind_invalid_envvar: an environment entry whose
    // name is not a valid identifier never becomes a shell variable but
    // stays in the export environment — `env 'v[0]=help' sh -c ...`
    // propagates `v[0]` to the child (varenv13.sub). The uppercase-first
    // heuristic for valid identifiers keeps shell-local names from
    // leaking; invalid names could only have arrived via environ.
    !is_shell_name(name) || name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
}

pub(in crate::executor) fn is_bash_managed_shell_var(name: &str) -> bool {
    matches!(
        name,
        "BASH"
            | "BASHOPTS"
            | "BASH_ALIASES"
            | "BASH_ARGC"
            | "BASH_ARGV"
            | "BASH_CMDS"
            | "BASH_EXECUTION_STRING"
            | "BASH_LINENO"
            | "BASH_SOURCE"
            | "BASH_VERSINFO"
            | "BASH_VERSION"
            | "DIRSTACK"
            | "EUID"
            | "FUNCNAME"
            | "HOSTNAME"
            | "HOSTTYPE"
            | "LINENO"
            | "MACHTYPE"
            | "OLDPWD"
            | "OPTARG"
            | "OPTIND"
            | "OSTYPE"
            | "PIPESTATUS"
            | "PPID"
            | "RANDOM"
            | "SECONDS"
            | "SHELLOPTS"
            | "SRANDOM"
            | "UID"
            | "_"
    )
}

pub(in crate::executor) fn unmark_env_name(
    env_vars: &mut HashMap<String, String>,
    key: &str,
    name: &str,
) {
    let mut names = marked_env_names(env_vars, key);
    names.retain(|current| current != name);
    env_vars.insert(key.to_string(), names.join(DATA_DOLLAR_STR));
}

pub(in crate::executor) fn marked_env_names(
    env_vars: &HashMap<String, String>,
    key: &str,
) -> Vec<String> {
    env_vars
        .get(key)
        .map(|value| {
            value
                .split(DATA_DOLLAR)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(in crate::executor) fn local_export_env_values(
    env_vars: &HashMap<String, String>,
) -> Vec<(String, String)> {
    env_vars
        .get(LOCAL_EXPORT_ENV)
        .map(|value| {
            value
                .split(DATA_DOLLAR)
                .filter_map(|entry| entry.split_once('='))
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub(in crate::executor) fn set_local_export_env_value(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    value: String,
) {
    let mut entries = local_export_env_values(env_vars);
    if let Some((_, entry_value)) = entries
        .iter_mut()
        .find(|(entry_name, _)| entry_name == name)
    {
        *entry_value = value;
    } else {
        entries.push((name.to_string(), value));
    }
    write_local_export_env_values(env_vars, entries);
}

pub(in crate::executor) fn remove_local_export_env_value(
    env_vars: &mut HashMap<String, String>,
    name: &str,
) {
    let entries = local_export_env_values(env_vars)
        .into_iter()
        .filter(|(entry_name, _)| entry_name != name)
        .collect();
    write_local_export_env_values(env_vars, entries);
}

pub(in crate::executor) fn write_local_export_env_values(
    env_vars: &mut HashMap<String, String>,
    entries: Vec<(String, String)>,
) {
    if entries.is_empty() {
        env_vars.remove(LOCAL_EXPORT_ENV);
        return;
    }
    env_vars.insert(
        LOCAL_EXPORT_ENV.to_string(),
        entries
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(DATA_DOLLAR_STR),
    );
}
