//! The single argv[0]-derived fallback for the builtin layer's
//! env-map diagnostic prefixes (rubash#2791).
//!
//! GNU has exactly one name source for diagnostics:
//! `get_name_for_error` (error.c:90-120) — BASH_SOURCE[0] (the
//! innermost executing file), else dollar_vars[0] ($0) VERBATIM,
//! else `base_pathname(shell_name)`, else PROGRAM ("bash") — where
//! `shell_name` itself is argv[0] verbatim (shell.c:1779-1806
//! `set_shell_name`). `builtin_error_prolog`
//! (builtins/common.c:83-96) renders that name plus the executing
//! line for non-interactive shells.
//!
//! The executor owns that chain for its own diagnostics
//! (`Executor::error_report_name` / `diagnostic_prefix`). The
//! builtin layer's private prefix helpers historically each
//! re-implemented only the (script, line) arm and fell back to a
//! hard-coded `"bash: "` / `"rubash: "` literal when the pair was
//! incomplete — a second, divergent voice: a shell invoked under
//! any other name (renamed binary, host embedding) whose builtin
//! diagnostic reached such a helper with only one arm present (or
//! neither) reported the wrong name. Those helpers now all derive
//! the missing arm from the same carriers the executor reads, via
//! this module; no builtin-local literal remains on these paths.
//! The only literal left is GNU's own terminal fallback, "bash"
//! (error.c's PROGRAM), reached when no carrier names the shell
//! at all.
//!
//! The carriers are read through a getter so one implementation
//! serves the three storage shapes the builtin layer uses: the
//! executor env map (`HashMap`), the `VarTable` value view, and
//! the process environment (the executor mirrors the current
//! file/line carriers there; see `Executor::set_current_line`).

/// A carrier lookup: the value bound to `key`, if any.
pub(crate) type CarrierGet<'a> = dyn Fn(&str) -> Option<String> + 'a;

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

/// The executing frame's file: `__RUBASH_CURRENT_FILE` (the
/// BASH_SOURCE[0] face) when present, else the top-level
/// `__RUBASH_SCRIPT_NAME` — the same pairing the executor's
/// `current_error_file_name` and `builtins::diagnostic` use.
fn error_file_name(get: &CarrierGet<'_>) -> Option<String> {
    non_empty(get("__RUBASH_CURRENT_FILE")).or_else(|| non_empty(get("__RUBASH_SCRIPT_NAME")))
}

/// `base_pathname` (general.c): the text after the last path
/// separator (both separators; a Windows argv[0] may use either).
fn base_pathname(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// GNU error.c:90-120 `get_name_for_error`'s terminal chain, in
/// the executor `error_report_name` order: $0 verbatim (the
/// BASH_ARGV0 carrier, its post-unset shadow, the top-level name
/// carrier), else `base_pathname(shell_name)` from the
/// `__RUBASH_SHELL_NAME` carrier, else PROGRAM.
pub(crate) fn fallback_error_name(get: &CarrierGet<'_>) -> String {
    for key in [
        "BASH_ARGV0",
        "__RUBASH_ARGV0_AFTER_UNSET",
        "__RUBASH_TOP_LEVEL_NAME",
    ] {
        if let Some(value) = non_empty(get(key)) {
            return value;
        }
    }
    if let Some(shell_name) = non_empty(get("__RUBASH_SHELL_NAME")) {
        return base_pathname(&shell_name).to_string();
    }
    if let Some(argv0) = non_empty(get("BASH_ARGV0")) {
        return base_pathname(&argv0).to_string();
    }
    "bash".to_string()
}

/// `base_pathname(shell_name)` for an interactive `Prolog`: the
/// shell-name carrier's basename, else the argv0 carrier's, else
/// PROGRAM. Used by the standalone (stdio) builtin entry points
/// that build their own flush prolog without an executor.
pub(crate) fn shell_basename(get: &CarrierGet<'_>) -> String {
    if let Some(shell_name) = non_empty(get("__RUBASH_SHELL_NAME")) {
        return base_pathname(&shell_name).to_string();
    }
    if let Some(argv0) = non_empty(get("BASH_ARGV0")) {
        return base_pathname(&argv0).to_string();
    }
    "bash".to_string()
}

/// The builtin_error prolog (builtins/common.c:83-96) built from
/// the carriers: `{file}: line {line}: ` when both arms are
/// present, `{file}: ` with a file but no line, `{name}: line
/// {line}: ` with a line but no file (the `-c` shape, name from
/// [`fallback_error_name`]), `{name}: ` with neither.
pub(crate) fn diagnostic_prefix(get: &CarrierGet<'_>) -> String {
    let file = error_file_name(get);
    let line = non_empty(get("__RUBASH_CURRENT_LINE"));
    match (file, line) {
        (Some(file), Some(line)) => format!("{file}: line {line}: "),
        (Some(file), None) => format!("{file}: "),
        (None, Some(line)) => format!("{}: line {line}: ", fallback_error_name(get)),
        (None, None) => format!("{}: ", fallback_error_name(get)),
    }
}

/// The err_readonly prolog (error.c:453, report_error family):
/// like [`diagnostic_prefix`], but the line segment honors
/// `gnu_errfmt` (`{file}:{line}: `) when the caller reports the
/// option enabled.
pub(crate) fn readonly_error_prefix(get: &CarrierGet<'_>, gnu_errfmt: bool) -> String {
    let file = error_file_name(get);
    let line = non_empty(get("__RUBASH_CURRENT_LINE"));
    match (file, line) {
        (Some(file), Some(line)) => {
            if gnu_errfmt {
                format!("{file}:{line}: ")
            } else {
                format!("{file}: line {line}: ")
            }
        }
        (Some(file), None) => format!("{file}: "),
        (None, Some(line)) => {
            let name = fallback_error_name(get);
            if gnu_errfmt {
                format!("{name}:{line}: ")
            } else {
                format!("{name}: line {line}: ")
            }
        }
        (None, None) => format!("{}: ", fallback_error_name(get)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
        move |key: &str| map.get(key).cloned()
    }

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn full_pair_renders_script_and_line() {
        let env = map(&[
            ("__RUBASH_SCRIPT_NAME", "probe.sh"),
            ("__RUBASH_CURRENT_LINE", "3"),
        ]);
        assert_eq!(diagnostic_prefix(&getter(&env)), "probe.sh: line 3: ");
    }

    #[test]
    fn current_file_wins_over_script_name() {
        let env = map(&[
            ("__RUBASH_CURRENT_FILE", "lib.sh"),
            ("__RUBASH_SCRIPT_NAME", "probe.sh"),
            ("__RUBASH_CURRENT_LINE", "3"),
        ]);
        assert_eq!(diagnostic_prefix(&getter(&env)), "lib.sh: line 3: ");
    }

    #[test]
    fn file_without_line_renders_the_file_alone() {
        let env = map(&[("__RUBASH_SCRIPT_NAME", "probe.sh")]);
        assert_eq!(diagnostic_prefix(&getter(&env)), "probe.sh: ");
    }

    #[test]
    fn line_without_file_uses_the_argv0_name_verbatim() {
        let env = map(&[("__RUBASH_CURRENT_LINE", "1"), ("BASH_ARGV0", "/opt/zimsh")]);
        assert_eq!(diagnostic_prefix(&getter(&env)), "/opt/zimsh: line 1: ");
    }

    #[test]
    fn neither_arm_uses_the_shell_name_basename() {
        let env = map(&[("__RUBASH_SHELL_NAME", "/usr/local/bin/zimsh")]);
        assert_eq!(diagnostic_prefix(&getter(&env)), "zimsh: ");
    }

    #[test]
    fn no_carriers_fall_back_to_program() {
        let env = map(&[]);
        assert_eq!(diagnostic_prefix(&getter(&env)), "bash: ");
        assert_eq!(fallback_error_name(&getter(&env)), "bash");
        assert_eq!(shell_basename(&getter(&env)), "bash");
    }

    #[test]
    fn readonly_prefix_honors_gnu_errfmt_only_with_a_line() {
        let env = map(&[
            ("__RUBASH_SCRIPT_NAME", "probe.sh"),
            ("__RUBASH_CURRENT_LINE", "3"),
        ]);
        assert_eq!(readonly_error_prefix(&getter(&env), true), "probe.sh:3: ");
        assert_eq!(
            readonly_error_prefix(&getter(&env), false),
            "probe.sh: line 3: "
        );
        let bare = map(&[("BASH_ARGV0", "zimsh")]);
        assert_eq!(readonly_error_prefix(&getter(&bare), true), "zimsh: ");
    }
}
