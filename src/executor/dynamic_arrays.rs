use super::*;

/// Lazy dynamic-parameter context for arithmetic evaluation — the port of
/// GNU's read-time `dynamic_value` getters (variables.c:1844
/// initialize_dynamic_variables / INIT_DYNAMIC_VAR at :1202, resolved
/// through find_variable at expr.c:1150 expr_streval). See
/// [`Executor::arith_dynamic_context`] for the cost model.
#[derive(Debug, Clone)]
pub(crate) struct ArithDynamicContext {
    bashpid: u32,
    subshell_depth: usize,
    histcmd: usize,
    function_depth: usize,
    funcname_top: Option<String>,
    debug_trap_command: Option<String>,
    pipestatus_first: i32,
    /// Sparse (index, status) snapshot of the live PIPESTATUS cell
    /// (rubash#1364 R3); empty unless the cell shape is LiveArray.
    /// Arithmetic element reads consult it because the env mirror
    /// never carries pipeline results — the completion writer updates
    /// the cell state only.
    pipestatus_entries: Vec<(i64, i32)>,
    /// The PIPESTATUS cell's scalar text when its shape is Scalar
    /// (rubash#1555): a tempenv/local scalar shadow is VT_VARIABLE in
    /// arithmetic too (get_array_value -> array_value_internal), so
    /// element 0 is the text and a negative subscript is INDEX_ERROR
    /// rather than a dense rebase onto the value.
    pipestatus_scalar: Option<String>,
}

impl ArithDynamicContext {
    /// Resolve one of the nine Executor-injected dynamic names, mirroring
    /// the arms of [`Executor::dynamic_parameter_value`] for exactly those
    /// names (RANDOM/SRANDOM/LINENO/SECONDS/EPOCH* are resolved by the
    /// evaluator itself and never reach here). `env_vars` is the
    /// evaluator's own map — SHELLOPTS/BASHOPTS read the maintained entry
    /// first (identical to what `dynamic_parameter_value` returned), with
    /// the pure-env recompute as the absent-entry fallback for unit-test
    /// maps that never ran the Executor::new binding.
    pub(in crate::executor) fn resolve(
        &self,
        env_vars: &HashMap<String, String>,
        name: &str,
    ) -> Option<String> {
        match name {
            "BASHPID" => Some(self.bashpid.to_string()),
            "BASH_SUBSHELL" => Some(self.subshell_depth.to_string()),
            "HISTCMD" => Some(self.histcmd.to_string()),
            // variables.c:1526 get_bash_argv0 returns dollar_vars[0]; the
            // script-name chain below is rubash's dollar_vars[0] model.
            "BASH_ARGV0" => Some(script_name_value_from_env(env_vars)),
            // GNU FUNCNAME carries att_invisible and reads unset at
            // function depth 0 (variables.c:1812 make_funcname_visible);
            // the dynamic snapshot omitted it there — funcname_top is None.
            "FUNCNAME" => self.funcname_top.clone(),
            // groups_words() is the single GROUPS source (general.c
            // get_groupset port); keep both readers on it.
            "GROUPS" => Some(groups_words().get(0).cloned().unwrap_or_default()),
            "BASH_COMMAND" => Some(
                self.debug_trap_command
                    .clone()
                    .or_else(|| env_vars.get("__RUBASH_CURRENT_COMMAND").cloned())
                    .unwrap_or_default(),
            ),
            "SHELLOPTS" => Some(
                env_vars
                    .get("SHELLOPTS")
                    .cloned()
                    .unwrap_or_else(|| crate::builtins::set::shellopts_value(env_vars)),
            ),
            "BASHOPTS" => Some(
                env_vars
                    .get("BASHOPTS")
                    .cloned()
                    .unwrap_or_else(|| crate::builtins::shopt::bashopts_value(env_vars)),
            ),
            "PIPESTATUS" => Some(self.pipestatus_first.to_string()),
            _ => None,
        }
    }

    /// Arithmetic `${PIPESTATUS[i]}` / `PIPESTATUS[i]` element read
    /// over the live-cell snapshot (rubash#1364 R3): the outer None
    /// means "no live snapshot — fall back to the env mirror" (scalar
    /// or frozen shapes keep their existing arithmetic posture); the
    /// inner Option is the element verdict, with a negative subscript
    /// resolving against the maximum index exactly like
    /// array_reference (a miss reads as unset, i.e. empty text).
    pub(in crate::executor) fn pipestatus_element(&self, index: i128) -> Option<Option<String>> {
        if let Some(scalar) = &self.pipestatus_scalar {
            // VT_VARIABLE cell (arrayfunc.c array_value_internal via
            // get_array_value): element 0 is the whole string, every
            // other subscript is unset — negatives never rebase.
            return Some(if index == 0 {
                Some(scalar.clone())
            } else {
                None
            });
        }
        if self.pipestatus_entries.is_empty() {
            return None;
        }
        let resolved: i128 = if index < 0 {
            let max_index = self
                .pipestatus_entries
                .last()
                .map(|(index, _)| *index as i128)?;
            max_index + 1 + index
        } else {
            index
        };
        if resolved < 0 {
            return Some(None);
        }
        Some(
            self.pipestatus_entries
                .iter()
                .find(|(entry_index, _)| *entry_index as i128 == resolved)
                .map(|(_, status)| status.to_string()),
        )
    }

    /// Whether a negative subscript on the PIPESTATUS cell is the
    /// INDEX_ERROR case (arrayfunc.c:1580-1586 via get_array_value):
    /// scalar and absent/valueless cells never rebase, so every
    /// negative subscript is bad; a live array rebases against its
    /// maximum index and is bad only when the resolution still
    /// falls below zero (rubash#1555).
    pub(in crate::executor) fn pipestatus_negative_is_bad(&self, index: i128) -> bool {
        if index >= 0 {
            return false;
        }
        if self.pipestatus_scalar.is_some() {
            return true;
        }
        match self.pipestatus_entries.last() {
            Some((max_index, _)) => *max_index as i128 + 1 + index < 0,
            None => true,
        }
    }
}

/// Env-only form of [`Executor::script_name_value`] (GNU
/// variables.c:1526-1545 get_bash_argv0/assign_bash_argv0): the
/// dollar_vars[0] chain lives entirely in env entries, so the arithmetic
/// context can resolve BASH_ARGV0 without an Executor handle.
pub(in crate::executor) fn script_name_value_from_env(
    env_vars: &HashMap<String, String>,
) -> String {
    env_vars
        .get("BASH_ARGV0")
        .or_else(|| env_vars.get("__RUBASH_ARGV0_AFTER_UNSET"))
        .or_else(|| env_vars.get("__RUBASH_TOP_LEVEL_NAME"))
        .or_else(|| env_vars.get("__RUBASH_SCRIPT_NAME"))
        .or_else(|| env_vars.get("__RUBASH_SHELL_NAME"))
        .cloned()
        .unwrap_or_else(|| "rubash".to_string())
}

/// Resolves dynamic parameters whose values derive solely from `env_vars`
/// (plus the wall clock). Shared between `$SECONDS`-style parameter
/// expansion (via [`Executor::dynamic_parameter_value`]) and arithmetic
/// evaluation, whose parser carries no [`Executor`] handle.
pub(in crate::executor) fn env_derived_dynamic_parameter_value(
    env_vars: &HashMap<String, String>,
    name: &str,
) -> Option<String> {
    match name {
        "EPOCHSECONDS" => Some(current_epoch_seconds().to_string()),
        "EPOCHREALTIME" => {
            let micros = current_epoch_micros();
            Some(format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000))
        }
        // rubash#1649: GNU variables.c get_monoseconds (bash 5.3) —
        // CLOCK_MONOTONIC seconds, gettimeofday fallback inside
        // current_monotonic_seconds.
        "BASH_MONOSECONDS" => Some(current_monotonic_seconds().to_string()),
        "SECONDS" => {
            let start = env_vars
                .get(SHELL_START_EPOCH)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_else(current_epoch_seconds);
            let offset = env_vars
                .get(SECONDS_OFFSET)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            // GNU variables.c get_seconds: the value is a plain
            // time_t difference plus the assignment offset — a negative
            // assignment (`SECONDS=-100`) reads back negative and ticks
            // up through zero; there is no clamp at 0 (rubash#1083).
            Some((current_epoch_seconds() - start + offset).to_string())
        }
        _ => None,
    }
}

/// GROUPS word list — the process group set (GNU general.c:1281
/// initialize_group_array / variables.c:1637 get_groupset). Shared by
/// the Executor accessors and the arithmetic dynamic context.
/// shquote.c sh_double_quote: wrap in `"` and backslash-escape `\`, `"`,
/// `` ` `` and `$` — the declare -p renderer for dynamic variable values.
fn sh_double_quote_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if matches!(ch, '\\' | '"' | '`' | '$') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

pub(in crate::executor) fn groups_words() -> Vec<String> {
    // GNU general.c:1281 initialize_group_array feeding variables.c:1637
    // get_groupset: the process group set from getgroups(), with the
    // primary gid guaranteed present and moved to element 0 (an empty
    // getgroups result degrades to just the primary gid). Cached by GNU
    // for the shell's lifetime (group_set is built once); the process
    // group set cannot change under a running shell except through
    // setgroups by the process itself, which rubash never calls.
    // rubash#1046 (the old stub always returned ["0"]).
    #[cfg(unix)]
    {
        use std::sync::OnceLock;
        static GROUPS: OnceLock<Vec<String>> = OnceLock::new();
        GROUPS
            .get_or_init(|| {
                let gid = unsafe { libc::getgid() };
                let mut groups: Vec<libc::gid_t> = vec![0; 64];
                let count = unsafe { libc::getgroups(64, groups.as_mut_ptr()) };
                let mut gids: Vec<u32> = if count > 0 {
                    groups.truncate(count as usize);
                    groups
                } else {
                    Vec::new()
                };
                if gids.is_empty() {
                    gids.push(gid);
                } else if let Some(pos) = gids.iter().position(|&g| g == gid) {
                    if pos != 0 {
                        gids.swap(0, pos);
                    }
                } else {
                    gids.insert(0, gid);
                }
                gids.iter().map(u32::to_string).collect()
            })
            .clone()
    }
    #[cfg(not(unix))]
    {
        vec!["0".to_string()]
    }
}

impl Executor {
    pub(in crate::executor) fn dynamic_parameter_value(&self, name: &str) -> Option<String> {
        // Hot-path reorder: the UNSET_DYNAMIC_VARS probe below is a
        // SipHash over the name, paid on EVERY variable read (the assoc
        // `k$i` key/value fast lanes resolve `i` through here twice per
        // iteration). A non-dynamic name can never produce a value here —
        // the match below yields None for it either way — so admit only
        // the dynamic-name class first and keep the mark check (GNU
        // variables.c:3839 unbind_variable) for that class. Outcomes are
        // identical in all four combinations (dynamic/non-dynamic x
        // marked/unmarked); only the hash count changes.
        if !matches!(
            name,
            "SECONDS"
                | "EPOCHSECONDS"
                | "EPOCHREALTIME"
                | "BASH_MONOSECONDS"
                | "RANDOM"
                | "SRANDOM"
                | "BASHPID"
                | "BASH_SUBSHELL"
                | "BASH_ARGV0"
                | "FUNCNAME"
                | "BASH_SOURCE"
                | "GROUPS"
                | "LINENO"
                | "BASH_COMMAND"
                | "SHELLOPTS"
                | "BASHOPTS"
                | "PIPESTATUS"
                | "HISTCMD"
        ) {
            return None;
        }
        // GNU variables.c:3839 unbind_variable: `unset -v` unbinds a
        // dynamic variable for the rest of the shell's lifetime.
        if is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, name) {
            return None;
        }
        match name {
            "SECONDS" | "EPOCHSECONDS" | "EPOCHREALTIME" | "BASH_MONOSECONDS" => {
                env_derived_dynamic_parameter_value(&self.shell_state.env_vars, name)
            }
            "RANDOM" => Some(self.next_random_value().to_string()),
            "SRANDOM" => Some(self.next_srandom_value().to_string()),
            "BASHPID" => Some(self.bashpid_value().to_string()),
            "BASH_SUBSHELL" => Some(self.shell_state.subshell_depth.get().to_string()),
            // GNU bashhist.c: HISTCMD is always defined — the history
            // number of the current command, i.e. of the most recent
            // history entry (history_base + history_length - 1); the
            // pre-execution add makes that the current line whenever the
            // line was recorded, and it is 0 before anything is recorded
            // (rubash#1071: it read as entirely unset).
            "HISTCMD" => Some(self.histcmd_value().to_string()),
            "BASH_ARGV0" => Some(self.script_name_value()),
            // GNU FUNCNAME carries att_invisible and only becomes visible
            // while a function is executing (variables.c:1812
            // make_funcname_visible, set from execute_cmd.c:5257 on entry
            // and cleared at 5167 on unwind). The "source" frame pushed by
            // evalfile.c:257 does NOT flip visibility, so at the top level
            // (even mid-source) $FUNCNAME reads as unset.
            "FUNCNAME" if self.shell_state.function_depth > 0 => {
                Some(self.funcname_stack().first().cloned().unwrap_or_default())
            }
            // GNU variables.c:1892: BASH_SOURCE is an INIT_DYNAMIC_VAR
            // whose get_self reads the live ARRAY object — a scalar read
            // is element 0 of the same array element reads see. Rubash's
            // env mirror cell is only refreshed where the stack is pushed
            // for a `source` frame or a script-name bind, NOT on function
            // entry/exit, so after any function call the scalar read kept
            // returning the frame the cell last froze (rubash#1872:
            // bats' DEBUG trap argument "$BASH_SOURCE" stuck at the
            // loader's file for the whole run). Read the composed view,
            // exactly like the element/storage arms below.
            "BASH_SOURCE" => Some(
                self.dynamic_stack_array_values("BASH_SOURCE")
                    .unwrap_or_default()
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
            ),
            "GROUPS" => self.group_value_at(0),
            "LINENO" => Some(
                self.shell_state
                    .env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .cloned()
                    .unwrap_or_else(|| "1".to_string()),
            ),
            "BASH_COMMAND" => Some(
                self.shell_state
                    .debug_trap_command
                    .borrow()
                    .clone()
                    .or_else(|| {
                        self.shell_state
                            .env_vars
                            .get("__RUBASH_CURRENT_COMMAND")
                            .cloned()
                    })
                    .unwrap_or_default(),
            ),
            // GNU has no per-read re-render for SHELLOPTS/BASHOPTS: they are
            // ordinary (readonly) variables whose VALUES are rewritten at
            // every option change (builtins/set.def set_option ->
            // reset_option_vars rebinds SHELLOPTS; builtins/shopt.def
            // toggle_shopts -> set_bashopts rebinds BASHOPTS), and every
            // read — parameter expansion or find_variable via
            // expr.c:1150 expr_streval — returns the stored value. Rubash's
            // flip sites maintain the stored entry the same way
            // (set_shell_option / sync_shell_option_flag rewrite
            // SHELLOPTS after every __RUBASH_SETOPT_* write — those two are
            // the only production writers of the option keys; shopt.rs's
            // single SHOPT_STATE mutation site rewrites BASHOPTS at :383).
            // Read the maintained entry; render only when it is absent
            // (fresh unit-test maps without Executor::new's init binding).
            // This keeps `(( x ))`/`$(( SHELLOPTS ... ))` from re-walking
            // the whole option table on every arithmetic evaluation.
            "SHELLOPTS" => Some(
                self.shell_state
                    .env_vars
                    .get("SHELLOPTS")
                    .cloned()
                    .unwrap_or_else(|| {
                        crate::builtins::set::shellopts_value(&self.shell_state.env_vars)
                    }),
            ),
            "BASHOPTS" => Some(
                self.shell_state
                    .env_vars
                    .get("BASHOPTS")
                    .cloned()
                    .unwrap_or_else(|| {
                        crate::builtins::shopt::bashopts_value(&self.shell_state.env_vars)
                    }),
            ),
            // GNU: $PIPESTATUS is element 0 of the array cell — an
            // absent or valueless cell (before the first command, or
            // frozen by a valueless declaration) reads as unset, not
            // as a fabricated 0 (rubash#1363/#1364).
            "PIPESTATUS" => match self.shell_state.pipestatus_shape {
                // Element 0 BY INDEX (rubash#1364 R3): on a sparse
                // live array whose smallest index is above 0 the
                // element is unset — the first entry's value is some
                // other pipeline slot's status, never $PIPESTATUS.
                crate::shell::state::PipestatusShape::LiveArray => self
                    .shell_state
                    .pipestatus
                    .iter()
                    .find(|(index, _)| *index == 0)
                    .map(|(_, status)| status.to_string()),
                crate::shell::state::PipestatusShape::Scalar => {
                    self.shell_state.env_vars.get("PIPESTATUS").cloned()
                }
                crate::shell::state::PipestatusShape::Valueless
                | crate::shell::state::PipestatusShape::ValuelessArray
                | crate::shell::state::PipestatusShape::Absent => None,
            },
            _ => None,
        }
    }

    pub(in crate::executor) fn shell_pid_value(&self) -> u32 {
        self.shell_pid
    }

    pub(in crate::executor) fn last_background_pid_value(&self) -> String {
        self.shell_state
            .last_background_pid
            .map(|pid| pid.to_string())
            .unwrap_or_default()
    }

    /// Joins positional parameters with the first character of IFS, matching
    /// Bash's `$*` / `"$*"` semantics: default IFS => space, empty IFS =>
    /// no separator.
    pub(in crate::executor) fn positional_params_star_joined(&self) -> String {
        let ifs = self
            .shell_state
            .env_vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());
        match ifs.chars().next() {
            Some(separator) => self
                .shell_state
                .positional_window()
                .join(&separator.to_string()),
            None => self.shell_state.positional_window().concat(),
        }
    }

    /// GNU variables.c:1393-1408 assign_random: a numeric bind to RANDOM
    /// reseeds the generator (rseed = seed, last_random_value = 0); a
    /// non-numeric value fails valid_number and leaves the state alone.
    /// Applied on the declare/typeset/local bind path, whose assignment
    /// operands bypass apply_shell_assignment's own seed (rubash#541).
    /// A RANDOM seed written as a canonical decimal integer (optional
    /// sign, no leading zero) seeds directly; anything else (`010`,
    /// `0x10`, `abc`, `42x`) goes through evalexp like GNU assign_random
    /// (rubash#1082).
    pub(in crate::executor) fn canonical_decimal_seed(value: &str) -> Option<i64> {
        let trimmed = value.trim();
        let digits = trimmed
            .strip_prefix('-')
            .or_else(|| trimmed.strip_prefix('+'))
            .unwrap_or(trimmed);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if digits.len() > 1 && digits.starts_with('0') {
            return None;
        }
        trimmed.parse::<i64>().ok()
    }

    pub(in crate::executor) fn apply_declare_random_seeds(&self, args: &[String]) {
        for arg in args {
            let Some((name, value)) = arg.split_once('=') else {
                continue;
            };
            if name.ends_with('+') || name != "RANDOM" {
                continue;
            }
            if let Some(seed) = Self::canonical_decimal_seed(value) {
                self.shell_state.random_state.rseed.set(seed as u32);
                self.shell_state.random_state.last_value.set(0);
            } else if let Some(seed) = crate::executor::arithmetic::eval_conditional_arith_value(
                value,
                &self.shell_state.env_vars,
            ) {
                // Same evalexp seeding as the plain assignment path
                // (rubash#1082); a failed evaluation leaves the seed
                // alone — declare's caller surfaces the evalerror.
                self.shell_state.random_state.rseed.set(seed as u32);
                self.shell_state.random_state.last_value.set(0);
            }
            // A failed evaluation leaves the seed alone; the evalerror
            // record stays for the declare caller's diagnostic path.
        }
    }

    /// GNU variables.c initialize_dynamic_variables registers every dynamic
    /// parameter as a hash-table cell carrying a dynamic_value getter
    /// (INIT_DYNAMIC_VAR), so `declare -p NAME` renders the CURRENT dynamic
    /// value with the cell's attributes — `declare -i RANDOM="…"`,
    /// `declare -- LINENO="…"`, `declare -a GROUPS=([0]="0")` — re-evaluated
    /// at display time (rubash#541). None when the name is not a registered
    /// dynamic variable or was `unset` (variables.c:3839 unbind_variable
    /// removes the cell for the rest of the shell's lifetime).
    pub(in crate::executor) fn dynamic_declaration_view(&self, name: &str) -> Option<String> {
        if is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, name) {
            return None;
        }
        // rubash#1394: BASH_ARGC/BASH_ARGV are dynamic stack arrays
        // whose scalar getter does not exist (dynamic_parameter_value
        // has no arm for them and returns None), so render them here
        // from the composed views BEFORE the scalar-value gate below —
        // the same views element reads use (variables.c get_bashargcv
        // materializes the bottom frame on a top-level read; inside a
        // function the arrays stay unmaterialized without extdebug and
        // render as ()).
        if matches!(name, "BASH_ARGC" | "BASH_ARGV") {
            let exported = if is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, name) {
                "x"
            } else {
                ""
            };
            let readonly = if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name) {
                "r"
            } else {
                ""
            };
            let values = if name == "BASH_ARGC" {
                self.bash_argc_view()
            } else {
                self.bash_argv_view()
            };
            let elements = values
                .iter()
                .enumerate()
                .map(|(index, value)| format!("[{index}]={}", sh_double_quote_text(value)))
                .collect::<Vec<_>>()
                .join(" ");
            return Some(format!(
                "declare -a{readonly}{exported} {name}=({elements})"
            ));
        }
        // rubash#1872: BASH_SOURCE/BASH_LINENO are the same class —
        // dynamic stack arrays (variables.c push_source / execute_
        // function array_push) whose env mirror cells only refresh at
        // source-frame push/pop, so a declare after any function call
        // printed the frozen cell (a returned function's frame on top,
        // the pushed call line missing). Render from the composed
        // views element reads use, before the scalar gate below.
        if matches!(name, "BASH_SOURCE" | "BASH_LINENO") {
            let exported = if is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, name) {
                "x"
            } else {
                ""
            };
            let readonly = if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name) {
                "r"
            } else {
                ""
            };
            let values = if name == "BASH_SOURCE" {
                self.dynamic_stack_array_values("BASH_SOURCE")
                    .unwrap_or_default()
            } else {
                self.bash_lineno_view()
            };
            let elements = values
                .iter()
                .enumerate()
                .map(|(index, value)| format!("[{index}]={}", sh_double_quote_text(value)))
                .collect::<Vec<_>>()
                .join(" ");
            return Some(format!(
                "declare -a{readonly}{exported} {name}=({elements})"
            ));
        }
        let value = match self.dynamic_parameter_value(name) {
            Some(value) => value,
            // PIPESTATUS renders from the cell shape in the match
            // below, not from this scalar getter: the getter returns
            // None for an absent or valueless cell (the $PIPESTATUS
            // read view, variables.c:6291), which must not gate the
            // declaration view off — a valueless cell still renders
            // (`declare -r PIPESTATUS` after a bare `readonly
            // PIPESTATUS`), and the env-table fallback would bake the
            // static ARRAY mark into its flags (rubash#1364).
            None if name == "PIPESTATUS" => String::new(),
            None => return None,
        };
        let quoted = sh_double_quote_text(&value);
        // mk_env_string/declare -p read the getter's CURRENT value through
        // the cell's attributes: an exported dynamic variable renders
        // `declare -ix …` (GNU: `export RANDOM=42; declare -p RANDOM` shows
        // the next draw with -ix, rubash#541).
        let exported = if is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, name) {
            "x"
        } else {
            ""
        };
        // rubash#1366: readonly flag must also be displayed (GNU shows
        // `declare -ar GROUPS` and `declare -r LINENO` after `readonly`).
        let readonly = if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name) {
            "r"
        } else {
            ""
        };
        match name {
            // GNU variables.c:1858-1875 initialize_dynamic_variables:
            // RANDOM/SRANDOM/BASHPID each get VSETATTR(att_integer), and
            // HISTCMD's INIT_DYNAMIC_VAR (variables.c:1873) is likewise
            // followed by VSETATTR(v, att_integer) — `declare -p HISTCMD`
            // is `declare -i HISTCMD="..."`, not `declare --`
            // (rubash#1492).
            "RANDOM" | "SECONDS" | "SRANDOM" | "BASHPID" | "HISTCMD" => {
                Some(format!("declare -i{readonly}{exported} {name}={quoted}"))
            }
            "GROUPS" => {
                let elements = groups_words()
                    .iter()
                    .enumerate()
                    .map(|(index, group)| format!("[{index}]={}", sh_double_quote_text(group)))
                    .collect::<Vec<_>>()
                    .join(" ");
                Some(format!(
                    "declare -a{readonly}{exported} {name}=({elements})"
                ))
            }
            // GNU: `declare -p PIPESTATUS` renders the live array as
            // `declare -a PIPESTATUS=([0]="1" [1]="0")` (rubash#1363),
            // a scalar cell as `declare -- PIPESTATUS="99"`, and a
            // valueless cell as its attribute string (`declare -r
            // PIPESTATUS` for a readonly declaration, `declare -a
            // PIPESTATUS` when created with -a). An absent cell is
            // not found. Every non-absent shape renders here: the
            // print fallback consults the table's static array mark
            // and would emit `-a` for all of them (rubash#1364).
            "PIPESTATUS" => {
                // Attribute flags in declaration_scalar_attrs order
                // (builtins/declare/output.rs): the static ARRAY mark
                // init sets names PIPESTATUS's nominal type, so the
                // table fallback would render `-a` for every shape —
                // only a live array (or the att_array valueless cell)
                // carries it in GNU (rubash#1363/#1364).
                let flag = |key: &str, letter: &'static str| -> &'static str {
                    if is_marked_var(&self.shell_state.env_vars, key, "PIPESTATUS") {
                        letter
                    } else {
                        ""
                    }
                };
                let iflag = flag(INTEGER_VARS, "i");
                let nflag = flag(NAMEREF_VARS, "n");
                let rflag = flag(READONLY_VARS, "r");
                let tflag = flag(TRACE_VARS, "t");
                let lflag = flag(LOWERCASE_VARS, "l");
                let uflag = flag(UPPERCASE_VARS, "u");
                let cflag = flag(CAPCASE_VARS, "c");
                match self.shell_state.pipestatus_shape {
                    crate::shell::state::PipestatusShape::LiveArray => {
                        // Real indices (rubash#1364 R3): a sparse
                        // cell declares as ([2]="0"), never renumbered.
                        let elements = self
                            .pipestatus_entries()
                            .iter()
                            .map(|(index, status)| {
                                format!("[{index}]={}", sh_double_quote_text(status))
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        Some(format!(
                            "declare -a{rflag}{exported} PIPESTATUS=({elements})"
                        ))
                    }
                    crate::shell::state::PipestatusShape::Scalar => {
                        let flags =
                            format!("{iflag}{nflag}{rflag}{tflag}{exported}{lflag}{uflag}{cflag}");
                        let flags = if flags.is_empty() {
                            "--".to_string()
                        } else {
                            format!("-{flags}")
                        };
                        let value = self
                            .shell_state
                            .env_vars
                            .get("PIPESTATUS")
                            .cloned()
                            .unwrap_or_default();
                        Some(format!(
                            "declare {flags} PIPESTATUS={}",
                            sh_double_quote_text(&value)
                        ))
                    }
                    crate::shell::state::PipestatusShape::Valueless
                    | crate::shell::state::PipestatusShape::ValuelessArray => {
                        let aflag = if self.shell_state.pipestatus_shape
                            == crate::shell::state::PipestatusShape::ValuelessArray
                        {
                            "a"
                        } else {
                            ""
                        };
                        let flags = format!(
                            "{aflag}{iflag}{nflag}{rflag}{tflag}{exported}{lflag}{uflag}{cflag}"
                        );
                        let flags = if flags.is_empty() {
                            "--".to_string()
                        } else {
                            format!("-{flags}")
                        };
                        Some(format!("declare {flags} PIPESTATUS"))
                    }
                    crate::shell::state::PipestatusShape::Absent => None,
                }
            }
            "EPOCHSECONDS" | "EPOCHREALTIME" | "BASH_MONOSECONDS" | "LINENO" | "BASH_SUBSHELL"
            | "BASH_ARGV0" | "BASH_COMMAND" => {
                let flags = format!("{readonly}{exported}");
                let flags = if flags.is_empty() {
                    "--".to_string()
                } else {
                    format!("-{flags}")
                };
                Some(format!("declare {flags} {name}={quoted}"))
            }
            _ => None,
        }
    }

    /// GNU bashhist.c histcmd: the history number of the most recent
    /// entry (history_base + history_length - 1); 0 when nothing has
    /// been recorded or no session list is attached (rubash#1071).
    pub(in crate::executor) fn histcmd_value(&self) -> usize {
        // GNU variables.c histcmd stays 0 while history is disabled: the
        // dynamic value tracks the readline-maintained list position, and
        // with `set +o history` the list (even when `history -s` appended
        // to it) never advances the variable (verified GNU 5.3 script:
        // -s a / -s b / -c / -s z all leave $HISTCMD at 0).
        if self
            .shell_state
            .env_vars
            .get("__RUBASH_SETOPT_history")
            .map(String::as_str)
            != Some("1")
        {
            return 0;
        }
        match self.shell_state.session_history.as_ref() {
            Some(session) => {
                let session = session.borrow();
                (session.base + session.entries.len()).saturating_sub(1)
            }
            None => 0,
        }
    }

    pub(in crate::executor) fn dynamic_parameter_is_set(&self, name: &str) -> bool {
        if is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, name) {
            return false;
        }
        if name == "PIPESTATUS" {
            // A PIPESTATUS cell counts as set only once it holds a
            // value: a live array or a scalar (variables.c var_isset —
            // an att_invisible valueless cell is not set).
            return matches!(
                self.shell_state.pipestatus_shape,
                crate::shell::state::PipestatusShape::LiveArray
                    | crate::shell::state::PipestatusShape::Scalar
            );
        }
        matches!(
            name,
            "EPOCHSECONDS"
                | "EPOCHREALTIME"
                | "BASH_MONOSECONDS"
                | "SECONDS"
                | "RANDOM"
                | "SRANDOM"
                | "BASHPID"
                | "BASH_SUBSHELL"
                | "BASH_ARGV0"
                | "FUNCNAME"
                | "GROUPS"
                | "LINENO"
                | "BASH_COMMAND"
                | "SHELLOPTS"
                | "BASHOPTS"
                | "PIPESTATUS"
                | "HISTCMD"
        )
    }

    /// Snapshot the dynamic parameters arithmetic evaluation cannot reach
    /// on its own. GNU expr.c:1150 expr_streval resolves operand names
    /// through find_variable, which sees dynamic variables exactly like
    /// `$name` expansion; rubash's evaluator only carries `env_vars`, so
    /// the Executor injects this map at each evaluation entry point.
    /// RANDOM/SRANDOM are excluded (reading them must advance the RNG
    /// state held inside the evaluator), as are the env-derived names
    /// (SECONDS/EPOCH*) and LINENO that value.rs resolves directly.
    /// GNU variables.c:1844 `initialize_dynamic_variables` installs the
    /// dynamic parameter set as ordinary hash-table entries carrying a
    /// `dynamic_value` getter (INIT_DYNAMIC_VAR, variables.c:1202) that
    /// find_variable materializes AT READ TIME — expr.c:1150 expr_streval
    /// resolves an operand name through find_variable, so GNU builds no
    /// snapshot per arithmetic evaluation and pays nothing for dynamic
    /// names an expression never references. This context is that model:
    /// the Executor captures the Copy-valued dynamic state once per
    /// evaluation (nothing mutates it mid-parse — no user code runs
    /// during evaluation; command substitutions expand before the parser
    /// starts), and the evaluator resolves the nine injected names
    /// against it lazily, name by name, only when an operand actually
    /// spells one. The two Option strings cost nothing at the top level
    /// (FUNCNAME is unset outside functions, the DEBUG-trap command is
    /// absent outside trap execution) and clone only in the contexts
    /// where the eager 9-entry HashMap snapshot cloned them anyway.
    pub(crate) fn arith_dynamic_context(&self) -> ArithDynamicContext {
        ArithDynamicContext {
            bashpid: self.bashpid_value(),
            subshell_depth: self.shell_state.subshell_depth.get(),
            histcmd: self.histcmd_value(),
            function_depth: self.shell_state.function_depth,
            funcname_top: (self.shell_state.function_depth > 0).then(|| {
                self.shell_state
                    .function_name_stack
                    .first()
                    .cloned()
                    .unwrap_or_default()
            }),
            debug_trap_command: self.shell_state.debug_trap_command.borrow().clone(),
            // Arithmetic $PIPESTATUS is element 0 BY INDEX
            // (rubash#1364 R3), 0 when that element is unset.
            pipestatus_first: self
                .shell_state
                .pipestatus
                .iter()
                .find(|(index, _)| *index == 0)
                .map(|(_, status)| *status)
                .unwrap_or(0),
            pipestatus_entries: if self.shell_state.pipestatus_shape
                == crate::shell::state::PipestatusShape::LiveArray
            {
                self.shell_state.pipestatus.clone()
            } else {
                Vec::new()
            },
            pipestatus_scalar: if self.shell_state.pipestatus_shape
                == crate::shell::state::PipestatusShape::Scalar
            {
                Some(
                    self.shell_state
                        .env_vars
                        .get("PIPESTATUS")
                        .cloned()
                        .unwrap_or_default(),
                )
            } else {
                None
            },
        }
    }

    /// GNU shell.c:1635-1650 (shell_execscript): the synthetic bottom
    /// frames — FUNCNAME "main", the script's BASH_SOURCE entry, and
    /// BASH_LINENO "0" — are pushed ONLY when the shell reads a SCRIPT
    /// FILE. `bash -c` (error.c get_name_for_error reports dollar_vars[0])
    /// and stdin-script mode (`bash -s`, `bash < file`; shell.c:780-786
    /// read_from_stdin) never enter shell_execscript, so they have no main
    /// frame: probe 2026-09-27 — `bash -c 't2; ...'` → FUNCNAME=(t2),
    /// BASH_SOURCE=(bash), BASH_LINENO=(2), all empty at top level;
    /// `printf 'f\n' | bash -s` → FUNCNAME=(f), BASH_SOURCE=().
    pub(in crate::executor) fn has_script_main_frame(&self) -> bool {
        self.shell_state
            .env_vars
            .contains_key("__RUBASH_SCRIPT_NAME")
            && !self.shell_state.env_vars.contains_key("__RUBASH_IS_C")
            && !self
                .shell_state
                .env_vars
                .contains_key(crate::script_driver::READ_STDIN_MARKER)
    }

    /// GNU keeps a bottom BASH_LINENO frame of "0" for the main script frame
    /// in script-file mode only: dbg-support.tests reports
    /// BASH_LINENO=("0") at the top level and BASH_LINENO[3]=0 inside nested
    /// calls ("main called from ... at line 0"), while `bash -c` has no main
    /// frame and reports BASH_LINENO with no trailing "0" (probe
    /// 2026-09-27: "inner outer|bash bash|1 1"). The main script frame
    /// exists exactly when has_script_main_frame() is true.
    /// Effective BASH_ARGV contents: the extdebug call frames plus the
    /// bottom frame (the shell's own positional parameters, reversed)
    /// that GNU's init_bash_argv installs when the arrays materialize
    /// (variables.c get_bashargcv / execute_cmd.c:5337). The bottom
    /// frame's content depends on how the arrays materialized:
    /// startup / extdebug enable / a top-level read capture the
    /// then-current positional parameters, while a compat <= 44
    /// function entry installs an empty frame (see function entry).
    /// A top-level read before any materialization synthesizes the
    /// frame from the current positional window, mirroring GNU's
    /// read-time initialization at variable_context 0
    /// (variables.c get_bashargcv): GNU shows the bottom frame at
    /// the top level regardless of extdebug (oracle: script and -c,
    /// args X Y -> BASH_ARGC=(2), BASH_ARGV=(Y X)), while inside a
    /// function the arrays stay unmaterialized until a
    /// materializing call (rubash#1051, rubash#1110 item 4).
    pub(in crate::executor) fn bash_argv_view(&self) -> Vec<String> {
        let mut view = self.shell_state.bash_argv_stack.clone();
        if let Some(bottom) = self.bash_arg_bottom_frame() {
            view.extend(bottom.iter().cloned());
        }
        view
    }

    /// Effective BASH_ARGC contents — one count per BASH_ARGV frame,
    /// the bottom frame's count last (see [`Self::bash_argv_view`]).
    pub(in crate::executor) fn bash_argc_view(&self) -> Vec<String> {
        let mut view = self.shell_state.bash_argc_stack.clone();
        if let Some(bottom) = self.bash_arg_bottom_frame() {
            view.push(bottom.len().to_string());
        }
        view
    }

    /// The installed BASH_ARGV bottom frame, or None while the arg
    /// arrays have never materialized. A read at the top level
    /// materializes the frame on the spot, exactly like variables.c
    /// get_bashargcv (variable_context == 0 && debugging_mode == 0 ->
    /// init_bash_argv): the frame captures the positional parameters
    /// current at that first read and persists for the shell's
    /// lifetime, so a later function entry (even a compat <= 44 one,
    /// whose own init is then a no-op) observes the same frame.
    /// Inside a function an unmaterialized array stays unmaterialized.
    fn bash_arg_bottom_frame(&self) -> Option<Vec<String>> {
        if let Some(bottom) = self.shell_state.bash_arg_bottom.borrow().as_ref() {
            return Some(bottom.clone());
        }
        if self.shell_state.function_depth > 0 {
            return None;
        }
        let frame: Vec<String> = self
            .shell_state
            .positional_window()
            .iter()
            .rev()
            .cloned()
            .collect();
        *self.shell_state.bash_arg_bottom.borrow_mut() = Some(frame.clone());
        Some(frame)
    }

    pub(in crate::executor) fn bash_lineno_view(&self) -> Vec<String> {
        let mut stack = self.shell_state.bash_lineno_stack.clone();
        if self.has_script_main_frame() && stack.last().map(String::as_str) != Some("0") {
            stack.push("0".to_string());
        }
        stack
    }

    /// Direct element-list view of the six dynamic stack arrays, mirroring
    /// exactly what the matching `parameter_array_storage` arms render —
    /// WITHOUT the storage-string round trip. GNU maintains BASH_LINENO,
    /// BASH_SOURCE, FUNCNAME, BASH_ARGV and BASH_ARGC as real ARRAY objects
    /// (variables.c INIT_DYNAMIC_VAR / push_call_frame), so an element read
    /// `${BASH_LINENO[$i]}` is an O(1) `array_reference` (array.c) — GNU
    /// never re-renders the whole array per subscript. rubash's scalar
    /// table stores these arrays only as rendered storage strings, so every
    /// indexed read, length read or set-ness test rebuilt and re-parsed the
    /// full `[0]=v [1]=v ...` text — measured at ~50µs per read under a
    /// DEBUG trap (bats' `bats_capture_stack_trace` reads three of these
    /// arrays per stack frame per firing; rubash#375). This view feeds the
    /// read-only fast paths; every writer still goes through the storage
    /// form, so the two representations cannot diverge (the view is rebuilt
    /// from the same live stacks on every call — content parity is by
    /// construction, not by cache).
    ///
    /// Returns None for names that are NOT one of the six dynamic stack
    /// arrays (or when nameref resolution fails, mirroring
    /// `parameter_array_storage`'s `resolved_variable_name` gate).
    pub(in crate::executor) fn dynamic_stack_array_values(
        &self,
        name: &str,
    ) -> Option<Vec<String>> {
        let name = self.resolved_variable_name(name)?;
        let values = match name.as_str() {
            "PIPESTATUS" => self.pipestatus_values(),
            "FUNCNAME" => {
                // Same att_invisibility as the storage arm
                // (variables.c:1812 make_funcname_visible): outside any
                // function the whole array reads as unset. `unset FUNCNAME`
                // unbinds the cell and function entry never re-creates it
                // (rubash#541).
                if self.shell_state.function_depth == 0
                    || is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, "FUNCNAME")
                {
                    Vec::new()
                } else {
                    let mut stack = self.shell_state.function_name_stack.clone();
                    if self.has_script_main_frame()
                        && !stack.is_empty()
                        && stack.last().map(String::as_str) != Some("main")
                    {
                        stack.push("main".to_string());
                    }
                    stack
                }
            }
            // rubash#1110 item 4 + rubash#1051: execute_cmd.c's
            // push_args runs only in debugging_mode, so without
            // extdebug no call frame is ever pushed; the view still
            // shows the bottom frame once the arrays materialize
            // (a compat <= 44 call materializes it — see function
            // entry), which is why BASH_ARGV reads empty in an
            // arg-less shell at compat <= 44 without extdebug while
            // BASH_ARGC reads (0).
            "BASH_ARGV" => self.bash_argv_view(),
            "BASH_ARGC" => self.bash_argc_view(),
            "BASH_LINENO" => self.bash_lineno_view(),
            "BASH_SOURCE" => {
                let mut stack = self.shell_state.bash_source_stack.clone();
                // Without a main frame (`bash -c`, stdin scripts) the
                // script-name bottom entry installed with $0 is not a
                // BASH_SOURCE frame (see the storage arm's probe note).
                if !self.has_script_main_frame() && !stack.is_empty() {
                    stack.pop();
                }
                stack
            }
            _ => return None,
        };
        Some(values)
    }

    /// Indexed element read of a dynamic stack array,
    /// `${BASH_LINENO[$i]}` shape. Dense-subscript semantics mirror
    /// `resolve_indexed_array_subscript` + `array_value_at` over the
    /// rendered storage (the six views are contiguous 0..n renders, so
    /// max_index+1 == len): a non-negative index passes through, a
    /// negative index counts back from the end, out-of-range and
    /// unresolvable-subscript reads return None — callers keep their own
    /// diagnostic posture (the element-value path reports "bad array
    /// subscript", the braced-operator paths stay silent, exactly as their
    /// storage-string counterparts do).
    pub(in crate::executor) fn dynamic_stack_array_element(
        &self,
        name: &str,
        index: i128,
    ) -> Option<String> {
        // PIPESTATUS is the one dynamic array whose elements are NOT
        // a contiguous 0..n render (rubash#1364 R3): subscripts name
        // sparse indices and resolve against the maximum index.
        if self.resolved_variable_name(name).as_deref() == Some("PIPESTATUS") {
            return self.pipestatus_element(index);
        }
        let values = self.dynamic_stack_array_values(name)?;
        dense_view_element(&values, index)
    }

    /// `${name[@]}` / `${name[*]}` joined read of a dynamic stack array —
    /// the fast twin of `parameter_array_storage` + `join_array_parameter_
    /// values` for the six dynamic names: normalize each element (the
    /// storage round trip's unquote/normalize pass is identity on the raw
    /// view values) and join with the same `[@]`-space / `[*]`-IFS[0]
    /// rule (`join_expanded_array_values`). `expression` carries the
    /// `[@]`/`[*]` suffix, like every existing join call site. Returns
    /// None when the base name is not one of the six dynamic arrays.
    pub(in crate::executor) fn dynamic_array_joined(&self, expression: &str) -> Option<String> {
        let array_name = expression
            .strip_suffix("[@]")
            .or_else(|| expression.strip_suffix("[*]"))?;
        let values = self.dynamic_stack_array_values(array_name)?;
        let values = values
            .into_iter()
            .map(normalize_array_expanded_value)
            .collect::<Vec<_>>();
        Some(self.join_expanded_array_values(values, expression))
    }

    pub(in crate::executor) fn parameter_array_storage(&self, name: &str) -> Option<String> {
        let name = self.resolved_variable_name(name)?;
        let name = name.as_str();
        match name {
            // Real indices in the storage render (rubash#1364 R3):
            // every storage-mediated read — ${!PIPESTATUS[@]}, slices
            // via array_parameter_slice, mirror subscripts — then
            // sees the sparse cell as GNU's ARRAY object.
            "PIPESTATUS" => {
                let entries = self
                    .pipestatus_entries()
                    .into_iter()
                    .map(|(index, value)| (index as usize, value))
                    .collect();
                return Some(format_indexed_array_storage(entries));
            }
            "FUNCNAME" => {
                // Same att_invisibility as the scalar read above
                // (variables.c:1812 make_funcname_visible): outside any
                // function the whole array reads as unset, including the
                // evalfile.c:257 "source" frame at the top level. `unset
                // FUNCNAME` unbinds the cell permanently (rubash#541).
                if self.shell_state.function_depth == 0
                    || is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, "FUNCNAME")
                {
                    return Some(String::new());
                }
                let mut stack = self.shell_state.function_name_stack.clone();
                // The "main" bottom frame exists only in script-file mode
                // (shell.c:1647 array_push(funcname_a, "main") inside
                // shell_execscript); `bash -c` and stdin scripts have only
                // real function frames.
                if self.has_script_main_frame()
                    && !stack.is_empty()
                    && stack.last().map(String::as_str) != Some("main")
                {
                    stack.push("main".to_string());
                }
                return Some(format_indexed_array_values(stack));
            }
            "BASH_ARGC" => return Some(format_indexed_array_values(self.bash_argc_view())),
            "BASH_ARGV" => {
                // rubash#1110 item 4: same composed view as the direct
                // arm in dynamic_stack_array_values (no call frames
                // without extdebug; bottom frame once materialized).
                return Some(format_indexed_array_values(self.bash_argv_view()));
            }
            "BASH_LINENO" => return Some(format_indexed_array_values(self.bash_lineno_view())),
            "BASH_SOURCE" => {
                let mut stack = self.shell_state.bash_source_stack.clone();
                // Without a main frame (`bash -c`, stdin scripts) the
                // script-name bottom entry installed with $0 is not a
                // BASH_SOURCE frame — GNU reports only real call frames
                // (probe: `bash -c 't2; ...'` → BASH_SOURCE=(bash), top
                // level ()). The bottom sits at the end because function
                // calls insert at index 0.
                if !self.has_script_main_frame() && !stack.is_empty() {
                    stack.pop();
                }
                return Some(format_indexed_array_values(stack));
            }
            _ => {}
        }
        if name == "GROUPS" {
            return Some(format_indexed_array_storage(
                self.groups_words().into_iter().enumerate().collect(),
            ));
        }
        if name == "DIRSTACK" {
            return Some(self.dirstack_storage());
        }
        if name == "BASH_ALIASES" {
            return Some(self.bash_aliases_storage());
        }
        if name == "BASH_CMDS" {
            return Some(self.bash_cmds_storage());
        }
        self.shell_state.env_vars.get(name).cloned()
    }

    pub(in crate::executor) fn is_assoc_parameter_array(&self, name: &str) -> bool {
        self.resolved_variable_name(name)
            .as_deref()
            .is_some_and(|name| {
                is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
                    || self
                        .shell_state
                        .variables
                        .get(name)
                        .is_some_and(|variable| {
                            matches!(
                                variable.value,
                                crate::shell::ShellValue::AssociativeArray(_)
                            )
                        })
            })
    }

    pub(in crate::executor) fn dirstack_storage(&self) -> String {
        format_indexed_array_storage(
            crate::builtins::pushd::load_stack(&self.shell_state.env_vars)
                .into_iter()
                .enumerate()
                .collect(),
        )
    }

    pub(in crate::executor) fn bashpid_value(&self) -> u32 {
        let pid = std::process::id();
        let depth = self.shell_state.subshell_depth.get();
        // rubash#1095: `origin` is the subshell level realized by this
        // process's own spawn (0 for ordinary shells): deeper in-process
        // levels fake fork-distinctness with pid + offset, while the
        // spawn-realized level and below report the bare pid — GNU
        // variables.c:1511 get_bashpid is getpid(), and the async wrapper
        // spawn already is the fork whose pid is the parent's $!.
        pid.saturating_add(
            u32::try_from(depth.saturating_sub(self.bashpid_origin_depth)).unwrap_or(u32::MAX),
        )
    }

    pub(in crate::executor) fn bash_aliases_storage(&self) -> String {
        let mut entries: Vec<_> = self
            .shell_state
            .aliases
            .iter()
            .map(|(name, alias)| (name.clone(), alias.value.clone()))
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        format_assoc_storage(entries)
    }

    pub(in crate::executor) fn bash_cmds_storage(&self) -> String {
        format_assoc_storage(crate::builtins::hash::hashed_entries(
            &self.shell_state.env_vars,
        ))
    }

    pub(in crate::executor) fn sync_dynamic_assoc_vars(&mut self) {
        // DIRSTACK is deliberately NOT synced here. GNU materializes the
        // DIRSTACK array cell only when the variable is directly named:
        // variables.c get_dirstack (1618-1630) rebuilds the array from
        // get_directory_stack on each named access, while a bare list-all
        // `declare -a` prints the last materialized cell -- which stays
        // empty in a shell that never named DIRSTACK (array.tests
        // `declare -a | ignore_builtin_arrays` lines). See
        // sync_dirstack_cell for the named-access materialization.
        let storage = self.bash_aliases_storage();
        self.shell_state
            .env_vars
            .insert("BASH_ALIASES".to_string(), storage);
        mark_env_name(&mut self.shell_state.env_vars, ASSOC_VARS, "BASH_ALIASES");
        let storage = self.bash_cmds_storage();
        self.shell_state
            .env_vars
            .insert("BASH_CMDS".to_string(), storage);
        mark_env_name(&mut self.shell_state.env_vars, ASSOC_VARS, "BASH_CMDS");
    }

    /// Materializes the DIRSTACK array cell from the live directory stack.
    /// GNU runs this getter only when DIRSTACK itself is named by a command
    /// (named `declare -p DIRSTACK`, `declare -a DIRSTACK`, subscript
    /// access, assignment) -- pushd/popd/dirs and unrelated declares leave
    /// the stored cell untouched (variables.c:1618 get_dirstack,
    /// builtins/pushd.def:669 get_directory_stack).
    pub(in crate::executor) fn sync_dirstack_cell(&mut self) {
        let storage = self.dirstack_storage();
        self.shell_state
            .env_vars
            .insert("DIRSTACK".to_string(), storage);
        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, "DIRSTACK");
    }

    pub(in crate::executor) fn funcname_stack(&self) -> Vec<String> {
        self.shell_state.function_name_stack.clone()
    }

    pub(in crate::executor) fn current_bash_source(&self) -> String {
        self.shell_state
            .bash_source_stack
            .first()
            .cloned()
            .or_else(|| {
                self.shell_state
                    .env_vars
                    .get("__RUBASH_SCRIPT_NAME")
                    .cloned()
            })
            .unwrap_or_default()
    }

    pub(in crate::executor) fn next_random_value(&self) -> u32 {
        next_random_from_state(&self.shell_state.random_state, &self.shell_state.env_vars)
    }

    pub(in crate::executor) fn next_srandom_value(&self) -> u32 {
        next_srandom_from_state(&self.shell_state.random_state)
    }

    pub(in crate::executor) fn script_name_value(&self) -> String {
        script_name_value_from_env(&self.shell_state.env_vars)
    }

    pub(in crate::executor) fn groups_words(&self) -> Vec<String> {
        groups_words()
    }

    pub(in crate::executor) fn group_value_at(&self, index: usize) -> Option<String> {
        self.groups_words().get(index).cloned()
    }

    pub(in crate::executor) fn expand_declare_assignment_args(
        &mut self,
        args: &[String],
    ) -> Vec<String> {
        // TODO(builtins/declare.def/subst.c): `declare` and `typeset` perform
        // assignment-word RHS expansion before the builtin applies attributes.
        // General word expansion has already handled parameters and unquoted
        // tilde prefixes, so this bridge only removes Rubash's temporary quote
        // marker before declare.rs mirrors declare.def's bookkeeping. It must
        // not re-expand the RHS: by the time the builtin runs, the value is
        // fully expanded and a quoted literal `~` (from `PPATH="$XPATH:~/bin"`)
        // would wrongly undergo tilde expansion on the second pass.
        let mut expanded_args = Vec::new();
        for arg in args {
            // W_ARRAYREF (in-band ARRAYREF_FLAG) is a no-op for declare —
            // GNU marks the operand (execute_cmd.c:4366) but declare.def
            // never consults builtin_arrayref_flags, so the byte is
            // stripped here rather than used.
            let (_, arg) = crate::builtins::arrayref::take_arrayref_flag(arg);
            let Some((name, value)) = split_assignment_word(arg) else {
                expanded_args.push(arg.to_string());
                continue;
            };
            let value = crate::expand::tilde::tilde::strip_assignment_quote_marker(value);
            expanded_args.push(format!("{name}={value}"));
        }
        expanded_args
    }

    /// GNU variables.c:2920-2937 make_variable_value → 2938-2944: when the
    /// RHS arithmetic evaluation fails (evalexp sets expok=0 and there is no
    /// ASS_NOLONGJMP), the builtin prints the evalerror with its own
    /// command name, the failing operand and every operand after it are
    /// discarded, and jump_to_top_level(DISCARD) aborts the rest of the
    /// command list. Returns the rewritten args (with the failing word and
    /// its successors removed, so the builtin still binds the earlier words
    /// — `declare -i a=1 b=8+2x c=3` binds a, skips c, rc=1) plus the
    /// rendered evalerror diagnostic for the caller to emit.
    fn integer_assignment_eval_failure(&self, value: &str) -> Option<String> {
        if eval_conditional_arith_value(value, &self.shell_state.env_vars).is_some() {
            return None;
        }
        let message = crate::executor::arithmetic::take_arith_eval_error()
            .map(|record| record.render())
            .or_else(|| {
                crate::executor::arithmetic::arithmetic_error_message(
                    value,
                    false,
                    &self.shell_state.env_vars,
                )
            });
        // The abort is armed by the caller through raise_evalerror_abort
        // (this helper is &self and shared with non-aborting re-use).
        Some(message.unwrap_or_default())
    }

    pub(in crate::executor) fn evaluate_declare_integer_assignment_args(
        &self,
        args: &[String],
    ) -> (Vec<String>, Option<String>) {
        let mut failure: Option<String> = None;
        let mut evaluated: Vec<String> = Vec::with_capacity(args.len());
        for arg in args {
            if failure.is_some() {
                // GNU's evalerror longjmp discards the failing operand and
                // every remaining word of the command.
                break;
            }
            let Some((name, value)) = split_assignment_word(arg) else {
                evaluated.push(arg.clone());
                continue;
            };
            if value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                || value.starts_with('(') && value.ends_with(')')
            {
                evaluated.push(arg.clone());
                continue;
            }
            // GNU variables.c bind_variable_internal: the nameref cell
            // check runs on the RAW operand text before
            // bind_variable_value applies the integer evaluation, so
            // `declare -i foo=7*6` on a valueless nameref reports
            // `` `7*6': not a valid identifier `` — not `` `42' ``
            // (nameref12.sub:58). Only a chain that resolves keeps the
            // early evaluation; an unusable cell (Unresolved/MaxDepth)
            // must reach declare.rs's valid_nameref_value check raw.
            let base = name
                .strip_suffix('+')
                .unwrap_or(name)
                .split('[')
                .next()
                .unwrap_or(name);
            if matches!(
                self.nameref_resolution(base),
                NamerefResolution::Unresolved | NamerefResolution::MaxDepth
            ) {
                evaluated.push(arg.clone());
                continue;
            }
            if let Some(message) = self.integer_assignment_eval_failure(value) {
                // GNU's word loop applied the attributes (and created the
                // variable) before bind_variable_value's evalexp jumped:
                // the failing name survives as a valueless declare operand
                // (`declare -p nx` later prints `declare -i nx`).
                evaluated.push(name.strip_suffix('+').unwrap_or(name).to_string());
                failure = Some(message);
                continue;
            }
            evaluated.push(format!(
                "{name}={}",
                self.eval_integer_assignment_value(value)
            ));
        }
        (evaluated, failure)
    }

    /// GNU variables.c:3320-3345 bind_variable_value → make_variable_value
    /// (variables.c:2920-2937): an assignment operand whose target carries
    /// the integer attribute evaluates the RHS with evalexp. readonly and
    /// export share declare.def's operand handling (setattr.def), so
    /// `readonly i=100+42` on an int-attributed local binds 142
    /// (varenv25.sub init_vars2). An evaluation failure follows
    /// variables.c:2938-2944: evalerror + jump_to_top_level(DISCARD) —
    /// the failing word and its successors are removed and the rendered
    /// diagnostic returned for the caller to emit.
    pub(in crate::executor) fn evaluate_integer_attribute_assignment_args(
        &self,
        args: &[String],
    ) -> (Vec<String>, Option<String>) {
        let mut failure: Option<String> = None;
        let mut evaluated: Vec<String> = Vec::with_capacity(args.len());
        for arg in args {
            if failure.is_some() {
                break;
            }
            let Some((name, value)) = split_assignment_word(arg) else {
                evaluated.push(arg.clone());
                continue;
            };
            if value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                || value.starts_with('(') && value.ends_with(')')
            {
                evaluated.push(arg.clone());
                continue;
            }
            let base = name
                .strip_suffix('+')
                .unwrap_or(name)
                .split('[')
                .next()
                .unwrap_or(name);
            let target_is_integer = match self.nameref_resolution(base) {
                NamerefResolution::Target(ref target) => {
                    is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, target)
                }
                _ => is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base),
            };
            if !target_is_integer {
                evaluated.push(arg.clone());
                continue;
            }
            if let Some(message) = self.integer_assignment_eval_failure(value) {
                // Same variables.c word-loop rule as the declare path: the
                // attributes were applied before the evalexp jump, so the
                // failing name survives as a valueless operand.
                evaluated.push(name.strip_suffix('+').unwrap_or(name).to_string());
                failure = Some(message);
                continue;
            }
            evaluated.push(format!(
                "{name}={}",
                self.eval_integer_assignment_value(value)
            ));
        }
        (evaluated, failure)
    }
}

/// The names GNU keeps as INIT_DYNAMIC_VAR arrays
/// (variables.c:1844/1202): rendered by [`Executor::parameter_array_storage`]
/// or [`Executor::dynamic_stack_array_values`] but not necessarily carrying
/// an ARRAY/ASSOC mark, so the type-aware
/// [`Executor::parameter_is_array_variable`] must name them explicitly
/// rather than parse their rendered storage text (rubash#755).
pub(in crate::executor) fn is_dynamic_array_name(name: &str) -> bool {
    matches!(
        name,
        "PIPESTATUS"
            | "FUNCNAME"
            | "BASH_ARGC"
            | "BASH_ARGV"
            | "BASH_LINENO"
            | "BASH_SOURCE"
            | "GROUPS"
            | "DIRSTACK"
            | "BASH_ALIASES"
            | "BASH_CMDS"
    )
}

/// Element fetch over a dense dynamic-stack view with the storage path's
/// subscript semantics (`resolve_indexed_array_subscript` +
/// `array_value_at` over a contiguous 0..n render): non-negative indices
/// pass through, negative indices count back from the end
/// (max_index+1+index == len+index), an unresolvable negative index and an
/// out-of-range index both read as None. Callers own the diagnostic
/// posture, mirroring their storage counterparts.
pub(in crate::executor) fn dense_view_element(values: &[String], index: i128) -> Option<String> {
    let resolved = if index >= 0 {
        usize::try_from(index).ok()?
    } else {
        i128::try_from(values.len())
            .ok()?
            .checked_add(index)
            .and_then(|resolved| usize::try_from(resolved).ok())?
    };
    values
        .get(resolved)
        .map(|value| normalize_array_expanded_value(value.clone()))
}
