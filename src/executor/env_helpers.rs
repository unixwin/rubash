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

/// rubash#2884 (PUA sweep F6): entry-encode imported environment
/// values before they become shell variables. GNU variables.c:368
/// initialize_shell_variables stores environ verbatim because GNU
/// has no transport domain; this engine does, and every OTHER value
/// entry channel (stdin lines, `read` input, comsub output,
/// assignment RHS) entry-encodes at its boundary — environment
/// import was the one gap, so bare registry chars in inherited user
/// values were read as live protocol downstream (a bare U+E314
/// eaten by the display decoder's guard arm; a U+E400+U+E314 pair
/// collapsing to one char on expansion; a raw ESC byte losing its
/// `$'...'` quoting in `declare -p`). The encoding is per channel,
/// matching the channel that consumes the entry:
/// * `__RUBASH_*` names are the shell's own cross-process
///   transport — the interpretation is deliberate, values pass
///   verbatim.
/// * `BASH_FUNC_*%%` entries are consumed by the function-source
///   channel (import_exported_functions_from_env tokenizes the
///   value as source text), so they take the SOURCE encoding
///   (escape_marker_zone_chars, the bytes_to_script_text parity);
///   raw carrier bytes in a body keep the lexer meaning they
///   would have in a script file.
/// * every other (user) value takes the DATA encoding
///   (entry_encode_value_text, the bytes_to_shell_text parity).
/// Per-name entry encoding for a single RAW host/env value (rubash#3207):
/// the channel rule `entry_encode_imported_env` applies to the whole
/// imported environment, factored out so the host API faces
/// (`SessionBuilder` env seeds, `VariablesView::set` / `set_array` /
/// `set_assoc`) encode each value at their own boundary instead of
/// landing it bare in storage (rubash#3105's contract, extended here).
/// `__RUBASH_*` names are the shell's own cross-process transport and
/// pass verbatim; `BASH_FUNC_*%%` names take the SOURCE encoding; every
/// other (user) value takes the DATA encoding.
pub(crate) fn entry_encode_imported_env_value(name: &str, value: &str) -> String {
    if name.starts_with("__RUBASH_") {
        value.to_string()
    } else if super::function_env::imported_function_name(name).is_some() {
        crate::executor::substitution_metadata::escape_marker_zone_chars(value)
    } else {
        crate::executor::substitution_metadata::entry_encode_value_text(value)
    }
}

pub(in crate::executor) fn entry_encode_imported_env(
    values: HashMap<String, String>,
) -> HashMap<String, String> {
    values
        .into_iter()
        .map(|(name, value)| {
            let value = entry_encode_imported_env_value(&name, &value);
            (name, value)
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
/// one. Since rubash#2723 the embedded path
/// (is_initial_export_candidate) applies the same rule, so both
/// predicates are one: only rubash's internal transport names and
/// bash-managed names stay unmarked.
pub(in crate::executor) fn is_process_startup_export_candidate(name: &str) -> bool {
    is_initial_export_candidate(name)
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
    // GNU variables.c:368 initialize_shell_variables: EVERY environment
    // entry that becomes a shell variable is bound with
    // att_exported|att_imported (the ordinary-variable branch,
    // variables.c:513-516), whatever the name's case or length — the
    // environment's exported-ness is imported as-is. An earlier
    // uppercase-first heuristic (plus a length<=1 exclusion) de-exported
    // every lowercase-first name for executors constructed inside a
    // shared process, on the theory that a sibling executor's shell-local
    // mirror writes could accumulate in the process environment; that
    // surface no longer exists — set_process_env mirrors only a fixed
    // uppercase/internal whitelist (shell_assignment_needs_process_env),
    // so no lowercase name in the environment can be a mirror write
    // (rubash#2723: `http_proxy`, `my_var`, `x=1` reached GNU's children
    // but not an embedded session's). What stays excluded is only what
    // never came from the caller's environment at all: rubash's internal
    // transport names and the bash-managed names the shell re-derives.
    // GNU variables.c:3307 bind_invalid_envvar covers the invalid-name
    // arm of the same rule: an entry whose name is not a valid
    // identifier never becomes a shell variable but stays in the export
    // environment — `env 'v[0]=help' sh -c ...` propagates `v[0]` to the
    // child (varenv13.sub).
    !(name.starts_with("__RUBASH_") || is_bash_managed_shell_var(name))
}

/// Names the shell re-derives at birth and that an inherited environment
/// value does not mark exported.
///
/// SHELLOPTS and BASHOPTS are deliberately NOT here (rubash#3517): GNU
/// variables.c:506-517 initialize_shell_variables binds an imported
/// SHELLOPTS/BASHOPTS like every other valid-name entry, with
/// `att_exported | att_imported` (the `ro` branch only adds ASS_FORCE and
/// re-asserts att_readonly), and the rebuilds that follow
/// (builtins/set.def set_shellopts, builtins/shopt.def set_bashopts) go
/// through bind_variable, which keeps the attributes. The imported value
/// is consumed by the option replay in initialize_fresh_shell_env_vars and
/// then rebuilt in place, so the export mark survives and `set -o` /
/// `shopt -s` changes reach child shells through it.
pub(in crate::executor) fn is_bash_managed_shell_var(name: &str) -> bool {
    matches!(
        name,
        "BASH"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(names: &[&str]) -> VarTable {
        let mut table = VarTable::default();
        for name in names {
            table.insert((*name).to_string(), "v".to_string());
        }
        table
    }

    /// rubash#2723: GNU initialize_shell_variables imports the
    /// environment's exported-ness as-is (variables.c:513-516) — on the
    /// EMBEDDED path (process_startup=false) as much as on the
    /// standalone one. Lowercase-first, underscore-first and
    /// single-character names are export candidates exactly like
    /// uppercase ones; only rubash-internal transport names and
    /// bash-managed names stay unmarked.
    #[test]
    fn initial_export_candidates_follow_gnu_import() {
        for process_startup in [false, true] {
            let mut table = table_with(&[
                "http_proxy",
                "my_var",
                "_myvar",
                "foo",
                "x",
                "PATH",
                "__RUBASH_SESSION_SCOPE",
                "SECONDS",
                "PWD",
            ]);
            mark_initial_exported_vars(&mut table, process_startup);
            for expected in ["http_proxy", "my_var", "_myvar", "foo", "x", "PATH"] {
                assert!(
                    table.is_marked(EXPORTED_VARS, expected),
                    "{expected} must be marked exported (process_startup={process_startup})"
                );
            }
            for excluded in ["__RUBASH_SESSION_SCOPE", "SECONDS"] {
                assert!(
                    !table.is_marked(EXPORTED_VARS, excluded),
                    "{excluded} must stay unmarked (process_startup={process_startup})"
                );
            }
        }
    }

    /// rubash#2884: the import funnel encodes per channel — user
    /// values take the data encoding (bare registry chars E400-escaped,
    /// raw carrier bytes as raw-byte marker pairs), `__RUBASH_*`
    /// transport stays verbatim, and BASH_FUNC bodies take only the
    /// source encoding (marker-zone escape; carrier bytes keep their
    /// script-file lexer meaning).
    #[test]
    fn entry_encode_imported_env_follows_the_consuming_channel() {
        let encoded = entry_encode_imported_env(HashMap::from([
            ("PLAIN".to_string(), "hello world".to_string()),
            ("GUARD".to_string(), "\u{E314}".to_string()),
            ("PAIR".to_string(), "\u{E400}\u{E314}".to_string()),
            ("ESC".to_string(), "A\u{1B}B".to_string()),
            (
                "__RUBASH_TRANSPORT".to_string(),
                "\u{E314}\u{1B}".to_string(),
            ),
            (
                "BASH_FUNC_f%%".to_string(),
                "() { echo '\u{E314}\u{1B}'; }".to_string(),
            ),
        ]));
        assert_eq!(encoded["PLAIN"], "hello world");
        assert_eq!(encoded["GUARD"], "\u{E400}\u{E314}");
        assert_eq!(encoded["PAIR"], "\u{E400}\u{E400}\u{E400}\u{E314}");
        assert_eq!(encoded["ESC"], "A\u{E000}\u{E01C}B");
        assert_eq!(encoded["__RUBASH_TRANSPORT"], "\u{E314}\u{1B}");
        assert_eq!(
            encoded["BASH_FUNC_f%%"],
            "() { echo '\u{E400}\u{E314}\u{1B}'; }"
        );
    }
}
