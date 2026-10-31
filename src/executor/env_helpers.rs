use super::*;
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use crate::shell::var_table::VarTable;

/// Ingest the process environment as (name, value) pairs, dropping the
/// entries Unicode cannot represent.
///
/// `std::env::vars()` panics when its iteration reaches an entry whose
/// name or value is not valid Unicode (an unpaired surrogate on Windows,
/// arbitrary non-UTF-8 bytes on Unix). A host process can hand an
/// embedded Session such an entry at any moment, so every
/// full-environment read in the engine goes through this filter rather
/// than `vars()` (rubash#2064; problem definition from dsh-bash-native
/// patch 0002). GNU carries environ as bytes; this engine's variable
/// table is String, so the honest equivalent of an undecodable entry is
/// absence — a lossy U+FFFD conversion would forge data and hand the
/// forgery to children. Decodable entries pass through exactly, in
/// environ order.
pub(crate) fn process_env_pairs() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.to_str()?.to_owned();
            let value = value.to_str()?.to_owned();
            Some((name, value))
        })
        .collect()
}

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
    process_env_pairs()
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/// The export candidates for the standalone shell PROCESS's own
/// startup import (rubash#2212, bats-core's BATS_TMPDIR case): GNU
/// variables.c initialize_shell_variables binds EVERY imported
/// environment entry as an exported shell variable, whatever the
/// name's case — a lowercase `expected`/`http_proxy` set by the
/// parent reaches the shell's children exactly like an uppercase
/// one. The uppercase-first heuristic in is_initial_export_candidate
/// exists only for executors constructed INSIDE a shared process
/// (unit tests, embedded sessions), where the process environment
/// accumulates other executors' shell-local mirror writes; a real
/// shell process has one session and no such leak surface, so only
/// rubash's internal transport names and bash-managed names stay
/// unmarked there.
pub(in crate::executor) fn is_process_startup_export_candidate(name: &str) -> bool {
    !(name.starts_with("__RUBASH_") || name.len() <= 1 || is_bash_managed_shell_var(name))
}

pub(in crate::executor) fn mark_initial_exported_vars(
    env_vars: &mut crate::shell::var_table::VarTable,
    process_startup: bool,
) {
    let mut names: Vec<String> = env_vars
        .keys()
        .filter(|name| {
            if process_startup {
                is_process_startup_export_candidate(name)
            } else {
                is_initial_export_candidate(name)
            }
        })
        .cloned()
        .collect();
    names.sort();
    // replace_attr_list writes the serialized string AND the structured
    // exported bits together (the two forms never diverge).
    env_vars.replace_attr_list(EXPORTED_VARS, names);
}

pub(in crate::executor) fn initialize_shell_level(env_vars: &mut VarTable) {
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
    env_vars: &mut crate::shell::var_table::VarTable,
    key: &str,
    name: &str,
) {
    env_vars.unmark_name(key, name);
}

pub(in crate::executor) fn marked_env_names(
    env_vars: &crate::shell::var_table::VarTable,
    key: &str,
) -> Vec<String> {
    env_vars.marked_names(key)
}

/// Whether a marked-exported variable may carry its value into a child
/// process environment. GNU variables.c make_env_array_from_var_list skips
/// array and associative variables when the child env list is assembled
/// (ARRAY_EXPORT off): `declare -a A=(x y); export A; printenv A` prints
/// nothing — the exported attribute stays on the cell (declare -p shows
/// -a -x) but the name never crosses the boundary. rubash#534: the
/// storage-encoded array value must not leak into the child env block.
pub(in crate::executor) fn is_exportable_env_name(
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
) -> bool {
    !env_vars.is_marked(crate::executor::types::ARRAY_VARS, name)
        && !env_vars.is_marked(crate::executor::types::ASSOC_VARS, name)
}

/// Whether a LOCAL_EXPORT_ENV stash entry for `name` still applies to a
/// child environment. The stash stands in for the shadowed outer
/// binding's export-env slot while a local covers it; GNU
/// variables.c:4336 export_environment_candidate collects candidates
/// innermost-context-first and dedupes by name (variables.c:4149
/// vlist_add), so the innermost visible exported binding wins and an
/// invisible (declared-unset) or unexported local drops out — letting
/// the shadowed outer value pass through. The stash therefore applies
/// only while the live binding is NOT itself a live export candidate:
/// bound in env_vars, marked exported, and of an exportable shape
/// (rubash#970: `local FOO; FOO=localval` exports `localval`, not the
/// inherited `parent`).
pub(in crate::executor) fn local_export_env_entry_applies(
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
) -> bool {
    !(env_vars.is_marked(EXPORTED_VARS, name)
        && is_exportable_env_name(env_vars, name)
        && env_vars.get(name).is_some())
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
    env_vars: &mut VarTable,
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

pub(in crate::executor) fn remove_local_export_env_value(env_vars: &mut VarTable, name: &str) {
    let entries = local_export_env_values(env_vars)
        .into_iter()
        .filter(|(entry_name, _)| entry_name != name)
        .collect();
    write_local_export_env_values(env_vars, entries);
}

pub(in crate::executor) fn write_local_export_env_values(
    env_vars: &mut VarTable,
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
