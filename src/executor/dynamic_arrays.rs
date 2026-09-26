use super::*;

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

impl Executor {
    pub(in crate::executor) fn dynamic_parameter_value(&self, name: &str) -> Option<String> {
        match name {
            "SECONDS" | "EPOCHSECONDS" | "EPOCHREALTIME" => {
                env_derived_dynamic_parameter_value(&self.shell_state.env_vars, name)
            }
            "RANDOM" => Some(self.next_random_value().to_string()),
            "SRANDOM" => Some(self.next_srandom_value().to_string()),
            "BASHPID" => Some(self.bashpid_value().to_string()),
            "BASH_SUBSHELL" => Some(self.shell_state.subshell_depth.get().to_string()),
            "BASH_ARGV0" => Some(self.script_name_value()),
            "FUNCNAME" => Some(self.funcname_stack().first().cloned().unwrap_or_default()),
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
            "SHELLOPTS" => Some(crate::builtins::set::shellopts_value(
                &self.shell_state.env_vars,
            )),
            "BASHOPTS" => Some(crate::builtins::shopt::bashopts_value(
                &self.shell_state.env_vars,
            )),
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

    /// GNU keeps a bottom BASH_LINENO frame of "0" for the main script frame
    /// in script-file mode only: dbg-support.tests reports
    /// BASH_LINENO=("0") at the top level and BASH_LINENO[3]=0 inside nested
    /// calls ("main called from ... at line 0"), while `bash -c` has no main
    /// frame and reports BASH_LINENO with no trailing "0" (cli function
    /// stack probes: "inner outer|environment environment|1 1"). The main
    /// script frame exists exactly when __RUBASH_SCRIPT_NAME is bound, the
    /// same condition FUNCNAME's synthetic "main" uses.
    pub(in crate::executor) fn bash_lineno_view(&self) -> Vec<String> {
        let mut stack = self.shell_state.bash_lineno_stack.clone();
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_SCRIPT_NAME")
            && stack.last().map(String::as_str) != Some("0")
        {
            stack.push("0".to_string());
        }
        stack
    }

    pub(in crate::executor) fn parameter_array_storage(&self, name: &str) -> Option<String> {
        let name = self.resolved_variable_name(name)?;
        let name = name.as_str();
        match name {
            "PIPESTATUS" => return Some(format_indexed_array_values(self.pipestatus_values())),
            "FUNCNAME" => {
                let mut stack = self.shell_state.function_name_stack.clone();
                // Bash exposes the script's top-level frame as `main`, but
                // `bash -c` reports only real function frames.
                if self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_SCRIPT_NAME")
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
                return Some(format_indexed_array_values(
                    self.shell_state.bash_source_stack.clone(),
                ))
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
        self.shell_state
            .env_vars
            .insert("BASH_ALIASES".to_string(), self.bash_aliases_storage());
        mark_env_name(&mut self.shell_state.env_vars, ASSOC_VARS, "BASH_ALIASES");
        self.shell_state
            .env_vars
            .insert("BASH_CMDS".to_string(), self.bash_cmds_storage());
        mark_env_name(&mut self.shell_state.env_vars, ASSOC_VARS, "BASH_CMDS");
    }

    /// Materializes the DIRSTACK array cell from the live directory stack.
    /// GNU runs this getter only when DIRSTACK itself is named by a command
    /// (named `declare -p DIRSTACK`, `declare -a DIRSTACK`, subscript
    /// access, assignment) -- pushd/popd/dirs and unrelated declares leave
    /// the stored cell untouched (variables.c:1618 get_dirstack,
    /// builtins/pushd.def:669 get_directory_stack).
    pub(in crate::executor) fn sync_dirstack_cell(&mut self) {
        self.shell_state
            .env_vars
            .insert("DIRSTACK".to_string(), self.dirstack_storage());
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
        self.shell_state
            .env_vars
            .get("BASH_ARGV0")
            .or_else(|| self.shell_state.env_vars.get("__RUBASH_TOP_LEVEL_NAME"))
            .or_else(|| self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME"))
            // Embedded hosts provide their public shell identity here. Keep
            // this after script names so `niu foo.sh` still reports foo.sh.
            .or_else(|| self.shell_state.env_vars.get("__RUBASH_SHELL_NAME"))
            .cloned()
            .unwrap_or_else(|| "rubash".to_string())
    }

    pub(in crate::executor) fn groups_words(&self) -> Vec<String> {
        vec!["0".to_string()]
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

    pub(in crate::executor) fn evaluate_declare_integer_assignment_args(
        &self,
        args: &[String],
    ) -> Vec<String> {
        args.iter()
            .map(|arg| {
                let Some((name, value)) = split_assignment_word(arg) else {
                    return arg.clone();
                };
                if value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                    || value.starts_with('(') && value.ends_with(')')
                {
                    return arg.clone();
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
                    return arg.clone();
                }
                format!("{name}={}", self.eval_integer_assignment_value(value))
            })
            .collect()
    }

    /// GNU variables.c:3320-3345 bind_variable_value → make_variable_value
    /// (variables.c:2920-2937): an assignment operand whose target carries
    /// the integer attribute evaluates the RHS with evalexp. readonly and
    /// export share declare.def's operand handling (setattr.def), so
    /// `readonly i=100+42` on an int-attributed local binds 142
    /// (varenv25.sub init_vars2).
    pub(in crate::executor) fn evaluate_integer_attribute_assignment_args(
        &self,
        args: &[String],
    ) -> Vec<String> {
        args.iter()
            .map(|arg| {
                let Some((name, value)) = split_assignment_word(arg) else {
                    return arg.clone();
                };
                if value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                    || value.starts_with('(') && value.ends_with(')')
                {
                    return arg.clone();
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
                if target_is_integer {
                    format!("{name}={}", self.eval_integer_assignment_value(value))
                } else {
                    arg.clone()
                }
            })
            .collect()
    }
}
