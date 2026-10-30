use super::*;
use crate::shell::var_table::VarTable;

pub(in crate::executor) fn validate_local_options(args: &[String]) -> Result<(), char> {
    for arg in args {
        if arg == "--" {
            return Ok(());
        }
        if (!arg.starts_with('-') && !arg.starts_with('+')) || arg == "-" || arg == "+" {
            return Ok(());
        }
        for option in arg[1..].chars() {
            match option {
                'a' | 'A' | 'f' | 'F' | 'g' | 'I' | 'i' | 'l' | 'n' | 'p' | 'r' | 't' | 'u'
                | 'x' => {}
                other => return Err(other),
            }
        }
    }
    Ok(())
}

pub(in crate::executor) fn local_args_request_inherit(args: &[String]) -> bool {
    declare_args_contain_option(args, 'I', true) || declare_args_contain_option(args, 'I', false)
}

pub(in crate::executor) fn declare_args_force_global(args: &[String]) -> bool {
    declare_args_contain_option(args, 'g', true)
}

pub(in crate::executor) fn declare_args_request_print(args: &[String]) -> bool {
    declare_args_contain_option(args, 'p', true)
}

pub(in crate::executor) fn declare_args_contain_option(
    args: &[String],
    option: char,
    set_attr: bool,
) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if (!arg.starts_with('-') && !arg.starts_with('+')) || arg == "-" || arg == "+" {
            return false;
        }
        if arg.starts_with('-') != set_attr {
            continue;
        }
        if arg[1..].chars().any(|current| current == option) {
            return true;
        }
    }
    false
}

pub(in crate::executor) fn local_assignment_name(arg: &str) -> Option<&str> {
    let name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
    let name = name.strip_suffix('+').unwrap_or(name);
    let name = name.split_once('[').map(|(name, _)| name).unwrap_or(name);
    let name = name
        .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
        .unwrap_or(name);
    if is_shell_name(name) {
        Some(name)
    } else {
        None
    }
}

pub(in crate::executor) fn local_names(args: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for arg in args {
        if arg == "--" {
            continue;
        }
        if (arg.starts_with('-') || arg.starts_with('+')) && arg != "-" && arg != "+" {
            continue;
        }
        if let Some(name) = local_assignment_name(arg) {
            names.push(name.to_string());
        }
    }
    names
}

/// All operand words for `local`: flags before `--`/the first operand are
/// skipped, but everything after a bare `--` counts as an operand even when
/// it looks like a flag (`local -- -p` names `-p`, rubash#1377). Used to
/// tell "no operands" apart from "operands with invalid names".
pub(in crate::executor) fn local_operands(args: &[String]) -> Vec<&str> {
    let mut parsing_options = true;
    let mut operands = Vec::new();
    for arg in args {
        if parsing_options && arg == "--" {
            parsing_options = false;
            continue;
        }
        if parsing_options
            && (arg.starts_with('-') || arg.starts_with('+'))
            && arg != "-"
            && arg != "+"
        {
            continue;
        }
        parsing_options = false;
        operands.push(arg.as_str());
    }
    operands
}

pub(in crate::executor) fn local_stderr_from_declare(stderr: Vec<u8>) -> Vec<u8> {
    String::from_utf8(stderr)
        .map(|text| text.replace("declare:", "local:").into_bytes())
        .unwrap_or_default()
}

pub(in crate::executor) fn restore_optional_env_var(
    env_vars: &mut VarTable,
    name: &str,
    value: Option<String>,
) {
    match value {
        Some(value) => {
            env_vars.insert(name.to_string(), value);
        }
        None => {
            env_vars.remove(name);
        }
    }
}

pub(in crate::executor) fn restore_optional_shell_var(
    env_vars: &mut VarTable,
    name: &str,
    value: Option<String>,
) {
    match value {
        Some(value) => {
            env_vars.insert(name.to_string(), value.clone());
            if is_valid_process_env(name, &value) {
                set_process_env(name, value);
            }
        }
        None => {
            env_vars.remove(name);
            unset_process_env(name);
        }
    }
}

pub(in crate::executor) fn capture_var_attrs(
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
) -> VarAttrs {
    env_vars.attr_of(name)
}

pub(in crate::executor) fn set_var_attrs(
    env_vars: &mut crate::shell::var_table::VarTable,
    name: &str,
    attrs: VarAttrs,
) {
    // GNU variables.c pop_scope restores the frame's saved attribute word
    // in one assignment; VarTable::set_attrs rewrites only the serialized
    // lists whose bits actually flipped.
    env_vars.set_attrs(name, attrs);
}

pub(in crate::executor) fn is_valid_process_env(name: &str, value: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0')
}

/// G-03: the process-env write journal. Every process-env mirror write an
/// executor lineage performs is recorded in its journal (name -> final
/// written state, None = removed); Executor::drop then restores only the
/// keys whose current value still equals the journal's final state, so a
/// dropping session can no longer roll back a live sibling session's
/// variables (the pre-G-03 Drop restored the whole birth snapshot).
/// The stack routes writes to the lineage currently executing on this
/// thread: execute_ast / call_function_owned push the running executor's
/// journal, and Executor::new_inner pushes its own around the init-time
/// clears. Writes outside any push (host bookkeeping transients) are
/// simply not journaled.
pub(crate) type EnvWriteJournal =
    std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, Option<String>>>>;

thread_local! {
    static ENV_JOURNAL_STACK: std::cell::RefCell<Vec<EnvWriteJournal>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// RAII push of an executor's journal onto this thread's stack. A nested
/// push of the SAME journal (recursive execute_ast on one executor) is a
/// no-op: the writes belong to the same lineage either way.
pub(crate) struct EnvJournalGuard {
    pushed: bool,
}

pub(crate) fn push_env_journal(journal: &EnvWriteJournal) -> EnvJournalGuard {
    ENV_JOURNAL_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        if stack
            .last()
            .is_some_and(|top| std::rc::Rc::ptr_eq(top, journal))
        {
            return EnvJournalGuard { pushed: false };
        }
        stack.push(journal.clone());
        EnvJournalGuard { pushed: true }
    })
}

impl Drop for EnvJournalGuard {
    fn drop(&mut self) {
        if self.pushed {
            ENV_JOURNAL_STACK.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
}

thread_local! {
    static PROCESS_STATE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// G-01: RAII hold on the process-state lock (`super::EXECUTION_LOCK`).
/// The lock used to be held for whole `execute_ast` runs, serializing
/// every session behind the longest command. It now serializes only
/// the points that touch process-global mutable state: env mirror
/// writes (the funnel below), trap disposition reconcile, Executor
/// birth snapshots, the Drop restore, and the fresh-shell env swap.
/// Reentrant per thread — a holder passes through funnel writes
/// without re-locking, and pure session-state execution never takes
/// it at all. The audit of which surfaces stay inside the lock (and
/// which were proven self-synchronized or session-local) is in the
/// ms-stopbleed cluster REPORT.md.
pub(crate) struct ProcessStateGuard {
    owned: Option<std::sync::MutexGuard<'static, ()>>,
}

pub(crate) fn acquire_process_state() -> ProcessStateGuard {
    let depth = PROCESS_STATE_DEPTH.with(|depth| depth.get());
    if depth > 0 {
        PROCESS_STATE_DEPTH.with(|cell| cell.set(depth + 1));
        return ProcessStateGuard { owned: None };
    }
    let guard = super::EXECUTION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    PROCESS_STATE_DEPTH.with(|depth| depth.set(1));
    ProcessStateGuard { owned: Some(guard) }
}

impl Drop for ProcessStateGuard {
    fn drop(&mut self) {
        PROCESS_STATE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        // The MutexGuard drops after the depth clears, so a funnel
        // write triggered by another thread's wakeup can never observe
        // this thread as still holding the lock.
        drop(self.owned.take());
    }
}

pub(crate) fn with_process_state_lock<T>(f: impl FnOnce() -> T) -> T {
    let _guard = acquire_process_state();
    f()
}

fn record_env_write(name: &str, value: Option<&str>) {
    ENV_JOURNAL_STACK.with(|stack| {
        if let Some(top) = stack.borrow().last() {
            top.borrow_mut()
                .insert(name.to_string(), value.map(str::to_string));
        }
    });
}

/// Unconditional process-env write + journal record, for the scoped
/// restore paths (subshell/function-local restores) whose direct writes
/// the #916 assignment gate must not filter: whatever they write is this
/// lineage's final state for the key.
pub(crate) fn mirror_env_set(name: &str, value: &str) {
    // G-01: the mutation + journal record are one process-state
    // critical section — a concurrent Drop restore must never slip
    // between the write and its record.
    with_process_state_lock(|| {
        env::set_var(name, value);
        record_env_write(name, Some(value));
    });
}

/// Removal counterpart of `mirror_env_set`.
pub(crate) fn mirror_env_remove(name: &str) {
    with_process_state_lock(|| {
        env::remove_var(name);
        record_env_write(name, None);
    });
}

/// rubash#2178: birth-value lookup for the G-03 Drop restore — the one
/// place a journal key (shell spelling) is matched against the birth
/// snapshot (process-block spelling). The snapshot is a verbatim import
/// of the process environment block: it also seeds the variable table,
/// whose names are case-sensitive shell names (GNU variables.c
/// initialize_shell_variables imports environ with its stored
/// spelling), so canonicalizing the snapshot itself would corrupt the
/// table import. The two spellings must instead meet here, on the
/// platform's own identity rules. Windows environment names are
/// case-insensitive (std::env's get/set/remove all resolve
/// case-insensitively there) and the block commonly stores "Path"
/// where the journal speaks "PATH", so under `windows_case_rules` an
/// exact-spelling miss falls back to a case-insensitive scan. The
/// exact match is tried first: when the block somehow carries both
/// spellings, the journal's own spelling is the unambiguous one.
/// Without Windows rules a case variant is a DIFFERENT variable
/// (POSIX names are case-sensitive), so the lookup stays exact and
/// POSIX behavior is unchanged. A journaled key whose birth entry
/// exists under any spelling must never fall into the restore's
/// remove branch.
pub(crate) fn snapshot_birth_value<'a>(
    snapshot: &'a std::collections::HashMap<String, String>,
    name: &str,
    windows_case_rules: bool,
) -> Option<&'a String> {
    if let Some(value) = snapshot.get(name) {
        return Some(value);
    }
    if !windows_case_rules {
        return None;
    }
    snapshot
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

pub(crate) fn set_process_env(name: &str, value: impl AsRef<str>) {
    // #916: the process env mirrors exactly the special-name set below.
    // Every other name reaches children through the explicit env built at
    // spawn (env_clear + per-var env) and is read from the table everywhere
    // else, so mirroring it here paid a locked O(n) environ scan per write
    // for nothing. TMPDIR is never mirrored (safe_temp_dir_string reads
    // the inherited value; writing it would shadow sandbox configuration).
    let value = value.as_ref();
    if name != "TMPDIR"
        && shell_assignment_needs_process_env(name)
        && is_valid_process_env(name, value)
    {
        with_process_state_lock(|| {
            env::set_var(name, value);
            record_env_write(name, Some(value));
        });
    }
}

/// Unbinding counterpart of `set_process_env`. The #916 mirror gate only
/// governs WRITES: a name that entered the process env by inheritance
/// predates the mirror, so the special-name gate can never see it, yet the
/// stale copy is read back by every fallback that still consults the
/// process env (exact_case_env_var bare-$NAME reads, the nounset
/// existence checks, the fork+exec env_map base — the #182 revival class,
/// rubash#917). GNU variables.c:3839 unbind_variable drops the name from
/// the export environment unconditionally, so the mirror must too: the
/// existence check is a single non-locking getenv that keeps the common
/// "never mirrored" unset off the locked environ mutation path.
pub(crate) fn unset_process_env(name: &str) {
    if is_valid_process_env(name, "") && env::var_os(name).is_some() {
        with_process_state_lock(|| {
            env::remove_var(name);
            record_env_write(name, None);
        });
    }
}

impl Executor {
    /// rubash#1692: register one just-committed locale-family bind so
    /// GNU's sv_locale consequence runs (variables.c:6272 sv_locale ->
    /// locale.c:185 set_locale_var): the real setlocale attempt and,
    /// on failure, the `{prefix}warning: setlocale: ...` diagnostic.
    /// Callers invoke this AFTER the value lands in the variable store
    /// (sv_locale reads the stored binding). While a simple command's
    /// tempenv binds are deferred (its class is not known yet — see
    /// `defer_locale_assignment_warnings`), the bind is only recorded;
    /// the dispatch then flushes or drops the record.
    pub(in crate::executor) fn note_locale_assignment(&mut self, name: &str) {
        if !crate::locale::is_locale_family_name(name) {
            return;
        }
        if self.defer_locale_assignment_warnings.get() {
            self.deferred_locale_assignment_binds.push(name.to_string());
            return;
        }
        self.emit_locale_assignment_warning(name);
    }

    fn emit_locale_assignment_warning(&mut self, name: &str) {
        if let Some(warning) =
            crate::locale::locale_assignment_warning(name, &self.shell_state.env_vars)
        {
            let line = format!("{}warning: {}\n", self.diagnostic_prefix(), warning);
            let _ = self.write_default_stderr(line.as_bytes());
        }
    }

    /// The deferred tempenv binds belong to a command that DOES bind
    /// them (builtin/function): run GNU's attempts now, before the
    /// command body executes.
    pub(in crate::executor) fn flush_deferred_locale_assignment_binds(&mut self) {
        for name in std::mem::take(&mut self.deferred_locale_assignment_binds) {
            self.emit_locale_assignment_warning(&name);
        }
    }

    /// The deferred tempenv binds belong to a disk command: GNU never
    /// bound them in the shell, so no attempt ever ran.
    pub(in crate::executor) fn drop_deferred_locale_assignment_binds(&mut self) {
        self.deferred_locale_assignment_binds.clear();
    }
}

pub(crate) fn sync_shell_assignment_process_env(
    _env_vars: &crate::shell::var_table::VarTable,
    name: &str,
    value: impl AsRef<str>,
) {
    // #916: mirror iff the name needs the process env (special list
    // below). Exported-but-ordinary names used to mirror too, at O(n) per
    // op; they reach children through the explicit spawn env instead.
    // (`_env_vars` is retained so every assignment-shaped call site keeps
    // the same funnel.)
    set_process_env(name, value);
}

pub(crate) fn shell_assignment_needs_process_env(name: &str) -> bool {
    matches!(
        name,
        "HOME" | "OLDPWD" | "PATH" | "PATHEXT" | "PWD" | "TEMP" | "TMP" | "TMPDIR" | "USERPROFILE"
    ) || name.starts_with("__RUBASH_")
        // rubash#1531: the locale family. GNU variables.c's special_vars
        // table binds LANG and every LC_* category to sv_locale, which
        // fires on ASSIGNMENT (not export) and recomputes the effective
        // locale (locale.c set_locale_var: LC_ALL > LC_<cat> > LANG).
        // Rubash's locale readers resolve the categories from the PROCESS
        // environment (crate::locale collate_locale_name / locale_name /
        // decimal_point), so a plain `LC_COLLATE=de_DE.UTF-8` must mirror
        // immediately or glob order, `[[ a<b ]]`, ${#v} and printf number
        // parsing never see it. The save/restore machinery (temporary
        // assignments, function locals) keys off this same gate, so
        // scoped assignments restore the previous process value too.
        || matches!(
            name,
            "LANG" | "LC_ALL" | "LC_COLLATE" | "LC_CTYPE" | "LC_MESSAGES" | "LC_NUMERIC" | "LC_TIME"
        )
}

pub(in crate::executor) fn safe_temp_dir_string() -> String {
    for name in ["TMPDIR", "TEMP", "TMP"] {
        if let Ok(value) = env::var(name) {
            if !value.is_empty() && !value.contains('\0') {
                return value;
            }
        }
    }

    // GNU coreutils temp-dir resolution (rubash#232): TMPDIR or /tmp on
    // POSIX. The cwd/target fallback keeps the Windows-side habit of
    // keeping scratch files inside the workspace (no /tmp on raw Windows).
    #[cfg(unix)]
    {
        "/tmp".to_string()
    }
    #[cfg(not(unix))]
    {
        env::current_dir()
            .map(|path| path.join("target").to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string())
    }
}

pub(in crate::executor) fn set_marked_var(
    env_vars: &mut crate::shell::var_table::VarTable,
    key: &str,
    name: &str,
    marked: bool,
) {
    if marked {
        mark_env_name(env_vars, key, name);
    } else {
        unmark_env_name(env_vars, key, name);
    }
}

#[cfg(test)]
mod tests {
    use super::snapshot_birth_value;
    use std::collections::HashMap;

    fn snapshot_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn issue2178_snapshot_birth_value_prefers_the_exact_spelling() {
        // When the block somehow carries both spellings, the journal's
        // own spelling is the unambiguous one — the case-insensitive
        // scan must not shadow it.
        let snapshot = snapshot_of(&[("Path", "variant"), ("PATH", "exact")]);
        assert_eq!(
            snapshot_birth_value(&snapshot, "PATH", true).map(String::as_str),
            Some("exact")
        );
        assert_eq!(
            snapshot_birth_value(&snapshot, "Path", true).map(String::as_str),
            Some("variant")
        );
    }

    #[test]
    fn issue2178_snapshot_birth_value_finds_case_variant_under_windows_rules() {
        // The #2178 shape itself: the Windows block spells it "Path",
        // the journal speaks "PATH" — the birth value must be found,
        // or the Drop restore deletes the host's PATH.
        let snapshot = snapshot_of(&[("Path", "birth")]);
        assert_eq!(
            snapshot_birth_value(&snapshot, "PATH", true).map(String::as_str),
            Some("birth")
        );
    }

    #[test]
    fn issue2178_snapshot_birth_value_case_variant_is_distinct_without_windows_rules() {
        // POSIX names are case-sensitive: "Path" and "PATH" are
        // different variables, so the variant must NOT resolve.
        let snapshot = snapshot_of(&[("Path", "birth")]);
        assert_eq!(snapshot_birth_value(&snapshot, "PATH", false), None);
    }

    #[test]
    fn issue2178_snapshot_birth_value_absent_name_stays_absent() {
        // A name genuinely absent at birth must still miss under both
        // rule sets, so the restore keeps removing session-created
        // variables (the G-03 semantics are unchanged).
        let snapshot = snapshot_of(&[("OTHER", "x")]);
        assert_eq!(snapshot_birth_value(&snapshot, "PATH", true), None);
        assert_eq!(snapshot_birth_value(&snapshot, "PATH", false), None);
    }
}
