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

use std::collections::HashMap;

/// The executing frame's file for diagnostics: the executor-stamped
/// `__RUBASH_CURRENT_FILE` when present and non-empty, else the
/// top-level `__RUBASH_SCRIPT_NAME` (the pre-frame contexts: -c,
/// startup, and any path that never crossed a command stamp).
fn error_file_name<'a>(env_vars: &'a HashMap<String, String>) -> Option<&'a str> {
    env_vars
        .get("__RUBASH_CURRENT_FILE")
        .map(String::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| env_vars.get("__RUBASH_SCRIPT_NAME").map(String::as_str))
}

/// GNU builtins/common.c:83-94 builtin_error_prolog: the frame file,
/// the executing line for scripts, then the builtin name (the caller
/// appends the builtin name and message).
pub(crate) fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(file), Some(line)) = (
        error_file_name(env_vars),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{file}: line {line}: ");
    }
    // rubash#3355: with a line but no file — the -c face when $0 is
    // empty (the reader-name carrier stays absent there) — the name
    // is get_name_for_error's $0 verdict, not the "rubash" brand: an
    // empty $0 prints an empty name (error.c:90-120). The no-line arm
    // below stays "rubash: ": no line is stamped interactively, so
    // it is the interactive shape, out of this issue's scope.
    if let Some(line) = env_vars.get("__RUBASH_CURRENT_LINE") {
        let name =
            crate::builtins::prefix::fallback_error_name(&|key: &str| env_vars.get(key).cloned());
        return format!("{name}: line {line}: ");
    }
    "rubash: ".to_string()
}

/// GNU error.c:275-281 sys_error: error_prolog (0) — get_name_for_error
/// alone, never a line segment. The one shell_execve classification
/// reported through sys_error is the bad-interpreter refusal
/// (execute_cmd.c:6184); the `exec` builtin face reaches it through
/// the shared spawn classifier (rubash#3148), mirroring the executor's
/// own `sys_error_prefix`.
pub(crate) fn sys_error_prefix(env_vars: &HashMap<String, String>) -> String {
    if let Some(file) = error_file_name(env_vars) {
        return format!("{file}: ");
    }
    // rubash#3355: the no-file arm is get_name_for_error alone
    // (error.c:275-281 sys_error) — the $0 verdict, not the brand:
    // an empty $0 prints an empty name. Interactive keeps today's
    // "rubash: " (no line is stamped there, and the interactive
    // basename face is out of this issue's scope).
    if env_vars.contains_key("__RUBASH_INTERACTIVE") {
        return "rubash: ".to_string();
    }
    let name =
        crate::builtins::prefix::fallback_error_name(&|key: &str| env_vars.get(key).cloned());
    format!("{name}: ")
}

/// GNU error.c:453 err_readonly (report_error family): the bare
/// `VAR: readonly variable` bind diagnostic honors gnu_errfmt
/// (rubash#1038), unlike the builtin_error lines.
pub(crate) fn readonly_error_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(file), Some(line)) = (
        error_file_name(env_vars),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        if crate::builtins::shopt::option_enabled(env_vars, "gnu_errfmt") {
            return format!("{file}:{line}: ");
        }
        return format!("{file}: line {line}: ");
    }
    // rubash#3355: same $0-verdict arm as diagnostic_prefix above —
    // a line but no file is the -c face with an empty $0, whose name
    // is empty (error.c:90-120). The no-line arm stays "rubash: "
    // (the interactive shape).
    if let Some(line) = env_vars.get("__RUBASH_CURRENT_LINE") {
        let name =
            crate::builtins::prefix::fallback_error_name(&|key: &str| env_vars.get(key).cloned());
        if crate::builtins::shopt::option_enabled(env_vars, "gnu_errfmt") {
            return format!("{name}:{line}: ");
        }
        return format!("{name}: line {line}: ");
    }
    "rubash: ".to_string()
}
