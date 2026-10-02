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
    function_depth: usize,
    funcname_top: Option<String>,
    debug_trap_command: Option<String>,
    pipestatus_first: i32,
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
            // variables.c:1526 get_bash_argv0 returns dollar_vars[0]; the
            // script-name chain below is rubash's dollar_vars[0] model.
            "BASH_ARGV0" => Some(script_name_value_from_env(env_vars)),
            // GNU FUNCNAME carries att_invisible and reads unset at
            // function depth 0 (variables.c:1812 make_funcname_visible);
            // the dynamic snapshot omitted it there — funcname_top is None.
            "FUNCNAME" => self.funcname_top.clone(),
            // groups_words() is the current stub (vec!["0"]); keep both
            // readers on one source so a real port changes them together.
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
        "SECONDS" => {
            let start = env_vars
                .get(SHELL_START_EPOCH)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_else(current_epoch_seconds);
            let offset = env_vars
                .get(SECONDS_OFFSET)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            Some(
                (current_epoch_seconds() - start + offset)
                    .max(0)
                    .to_string(),
            )
        }
        _ => None,
    }
}

/// GROUPS word list — stub returning a single "0" (a real port would walk
/// the process group set, GNU variables.c:1886 get_groupset). Shared by
/// the Executor accessors and the arithmetic dynamic context.
pub(in crate::executor) fn groups_words() -> Vec<String> {
    vec!["0".to_string()]
}

impl Executor {
    pub(in crate::executor) fn dynamic_parameter_value(&self, name: &str) -> Option<String> {
        // GNU variables.c:3839 unbind_variable: `unset -v` unbinds a
        // dynamic variable for the rest of the shell's lifetime.
        if is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, name) {
            return None;
        }
        match name {
            "SECONDS" | "EPOCHSECONDS" | "EPOCHREALTIME" => {
                env_derived_dynamic_parameter_value(&self.shell_state.env_vars, name)
            }
            "RANDOM" => Some(self.next_random_value().to_string()),
            "SRANDOM" => Some(self.next_srandom_value().to_string()),
            "BASHPID" => Some(self.bashpid_value().to_string()),
            "BASH_SUBSHELL" => Some(self.shell_state.subshell_depth.get().to_string()),
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
            "PIPESTATUS" => Some(
                self.shell_state
                    .pipestatus
                    .first()
                    .copied()
                    .unwrap_or(0)
                    .to_string(),
            ),
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
                .positional_params
                .join(&separator.to_string()),
            None => self.shell_state.positional_params.concat(),
        }
    }

    pub(in crate::executor) fn dynamic_parameter_is_set(&self, name: &str) -> bool {
        if is_marked_var(&self.shell_state.env_vars, UNSET_DYNAMIC_VARS, name) {
            return false;
        }
        matches!(
            name,
            "EPOCHSECONDS"
                | "EPOCHREALTIME"
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
            function_depth: self.shell_state.function_depth,
            funcname_top: (self.shell_state.function_depth > 0).then(|| {
                self.shell_state
                    .function_name_stack
                    .first()
                    .cloned()
                    .unwrap_or_default()
            }),
            debug_trap_command: self.shell_state.debug_trap_command.borrow().clone(),
            pipestatus_first: self.shell_state.pipestatus.first().copied().unwrap_or(0),
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
    fn has_script_main_frame(&self) -> bool {
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
                // function the whole array reads as unset.
                if self.shell_state.function_depth == 0 {
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
            "BASH_ARGV" => self.shell_state.bash_argv_stack.clone(),
            "BASH_ARGC" => self.shell_state.bash_argc_stack.clone(),
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
            "PIPESTATUS" => return Some(format_indexed_array_values(self.pipestatus_values())),
            "FUNCNAME" => {
                // Same att_invisibility as the scalar read above
                // (variables.c:1812 make_funcname_visible): outside any
                // function the whole array reads as unset, including the
                // evalfile.c:257 "source" frame at the top level.
                if self.shell_state.function_depth == 0 {
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
            "BASH_ARGC" => {
                return Some(format_indexed_array_values(
                    self.shell_state.bash_argc_stack.clone(),
                ))
            }
            "BASH_ARGV" => {
                return Some(format_indexed_array_values(
                    self.shell_state.bash_argv_stack.clone(),
                ))
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
        if depth == 0 {
            pid
        } else {
            pid.saturating_add(u32::try_from(depth).unwrap_or(u32::MAX))
        }
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
        next_random_from_state(&self.shell_state.random_state)
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
