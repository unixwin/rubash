use std::env;

/// GNU bash prints diagnostics as "<script>: line <n>: builtin: message"
/// (builtins/common.c builtin_error). The executor tracks the current script
/// and line in its own environment map, which survives nested in-process
/// executions (a command substitution inside an array subscript re-runs the
/// child init that strips these markers from the *process* environment), so
/// prefer the executor map and fall back to the process environment.
pub(super) fn diagnostic_prefix(variables: &crate::shell::var_table::VarTable) -> String {
    let script = variables
        .get("__RUBASH_SCRIPT_NAME")
        .cloned()
        .or_else(|| env::var("__RUBASH_SCRIPT_NAME").ok());
    let line = variables
        .get("__RUBASH_CURRENT_LINE")
        .cloned()
        .or_else(|| env::var("__RUBASH_CURRENT_LINE").ok());
    if let (Some(script), Some(line)) = (script, line) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
