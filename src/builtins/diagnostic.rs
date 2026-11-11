//! Shared diagnostic prolog builders for the builtin layer.
//!
//! GNU's builtin diagnostics go through builtin_error -> error_prolog
//! (builtins/common.c), whose name is `get_name_for_error`
//! (error.c:90-120): BASH_SOURCE[0] — inside a function, its
//! DEFINITION file (execute_cmd.c:5310-5313 execute_function pushes
//! the function's recorded source_file) — paired with the executing
//! line. The executor maintains that pair as the
//! `__RUBASH_CURRENT_FILE` / `__RUBASH_CURRENT_LINE` carriers,
//! stamped per command in `Executor::set_current_line`.
//!
//! The per-builtin private copies this module replaces read the
//! top-level-only `__RUBASH_SCRIPT_NAME` directly, so an error inside
//! a function defined in a sourced file carried the definition
//! file's LINE under the top-level script's NAME (rubash#3035 — the
//! same pairing the executor's own prefix got right in rubash#2212).
//!
//! The three builders here are thin faces over the single argv[0]
//! derivation chain in `builtins::prefix` (rubash#2791): each reads
//! the same carriers through a map getter and delegates the arm
//! selection to the chain, so an incomplete carrier pair (file
//! without line, line without file, neither) derives its name from
//! the invocation name exactly as the executor's own prefix does —
//! the hard-coded `"rubash: "` fallback these faces used to print
//! for every incomplete pair named a renamed shell by the wrong
//! name (rubash#3256). The one arm the chain's renderers do not
//! carry is the interactive one: `get_name_for_error` skips the
//! BASH_SOURCE/$0 levels in an interactive shell (error.c:98) and
//! reports `base_pathname(shell_name)` with no line segment, so
//! each builder checks the interactive carrier first, in the same
//! shape as the executor's `diagnostic_prefix` / `sys_error_prefix`.

use std::collections::HashMap;

/// The carrier getter over an executor env map, in the shape the
/// `builtins::prefix` chain consumes.
fn map_get<'a>(env_vars: &'a HashMap<String, String>) -> impl Fn(&str) -> Option<String> + 'a {
    move |key: &str| env_vars.get(key).cloned()
}

/// The interactive arm shared by all three builders: GNU
/// error.c:98's `interactive_shell == 0` guard around the
/// BASH_SOURCE/$0 levels of `get_name_for_error` — an interactive
/// shell reports `base_pathname(shell_name)` alone, never a line
/// segment.
fn interactive_name(env_vars: &HashMap<String, String>) -> Option<String> {
    if env_vars.contains_key("__RUBASH_INTERACTIVE") {
        Some(super::prefix::shell_basename(&map_get(env_vars)))
    } else {
        None
    }
}

/// GNU builtins/common.c:83-94 builtin_error_prolog: the frame file,
/// the executing line for scripts, then the builtin name (the caller
/// appends the builtin name and message).
pub(crate) fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let Some(name) = interactive_name(env_vars) {
        return format!("{name}: ");
    }
    super::prefix::diagnostic_prefix(&map_get(env_vars))
}

/// GNU error.c:275-281 sys_error: error_prolog (0) — get_name_for_error
/// alone, never a line segment. The one shell_execve classification
/// reported through sys_error is the bad-interpreter refusal
/// (execute_cmd.c:6184); the `exec` builtin face reaches it through
/// the shared spawn classifier (rubash#3148), mirroring the executor's
/// own `sys_error_prefix`.
pub(crate) fn sys_error_prefix(env_vars: &HashMap<String, String>) -> String {
    if let Some(name) = interactive_name(env_vars) {
        return format!("{name}: ");
    }
    super::prefix::sys_error_prefix(&map_get(env_vars))
}

/// GNU error.c:453 err_readonly (report_error family): the bare
/// `VAR: readonly variable` bind diagnostic honors gnu_errfmt
/// (rubash#1038), unlike the builtin_error lines.
pub(crate) fn readonly_error_prefix(env_vars: &HashMap<String, String>) -> String {
    if let Some(name) = interactive_name(env_vars) {
        return format!("{name}: ");
    }
    super::prefix::readonly_error_prefix(
        &map_get(env_vars),
        crate::builtins::shopt::option_enabled(env_vars, "gnu_errfmt"),
    )
}
