use super::*;

impl Executor {
    pub(in crate::executor) fn execute_materialized_command(
        &mut self,
        cmd: &CommandNode,
        // DBG
        process_substitution_files: ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_MATCMD);
        let standalone_assignments = cmd.words.is_empty() && !cmd.assignments.is_empty();
        let keep_temporary_assignments = self.keeps_temporary_assignments(cmd);
        // GNU execute_cmd.c: this_command_name is the command word for the
        // duration of the command — including its leading tempenv
        // assignments — so assignment diagnostics carry the command segment
        // (`FOO=x getopts ...` reports under `getopts:`). A command with no
        // words is a bare assignment list and reports no command segment.
        let previous_command_name = std::mem::replace(
            &mut self.assignment_command_name,
            cmd.words.first().cloned(),
        );
        if self.posix_function_declare_prefix_assignments_are_local(cmd) {
            self.save_assignment_local_names(&cmd.assignments);
        }
        // GNU redir.c:298 redirection_expand uses expand_words_no_vars:
        // redirect words expand with the temp environment IGNORED
        // (`a=9 echo >&$(echo $a)` sees the outer a, not 9). Expanding here —
        // before apply_temporary_assignments — seeds the redirect-target memo
        // with the correct pre-binding values; the apply sites reuse them.
        let redirect_rejection = self.reject_invalid_redirects(cmd)?;
        let mut skip_command = redirect_rejection.is_some();
        if redirect_rejection == Some(super::redirection::RedirectRejection::FatalNounset) {
            // rubash#535: an unbound variable in a redirect word is a fatal
            // word expansion error classified by where do_redirections runs
            // (finish_heredoc_expansion_error — the same main-shell/child
            // split the heredoc bodies use): a builtin/function target ends
            // the noninteractive script, an external target contains the
            // jump in the forked child. The diagnostic and status 1 are
            // already in place from the walker.
            self.finish_heredoc_expansion_error(cmd, Self::FATAL_PARAMETER_EXPANSION_STATUS)?;
            skip_command = true;
        }
        let redirect_rejected = skip_command;
        let assignment_raws = (cmd.assignment_raws.len() == cmd.assignments.len())
            .then(|| cmd.assignment_raws.as_slice());
        let temporary_assignments = if standalone_assignments {
            // GNU subst.c expand_word_error + execute_cmd.c: a failed
            // assignment with no command word aborts the command list in
            // every mode (the DISCARD jump); POSIX mode additionally EXITS
            // the non-interactive shell (bash_variable_assignment_error,
            // posix-mode notes item 35 — `ro=2` where ro is readonly).
            let applied = self.apply_permanent_assignments(&cmd.assignments, assignment_raws);
            if !applied && self.posix_assignment_failure_is_fatal() {
                self.exit_code = 1;
                return Err(ExecuteError::ExitCode(1));
            }
            Vec::new()
        } else {
            let (applied, assignments_failed) =
                self.apply_temporary_assignments(&cmd.assignments, assignment_raws);
            // POSIX-mode notes item 36, GNU 5.3 verified cell-by-cell with
            // unbuffered per-process probes (the early "every command class
            // exits" reading was a stdout pipe-buffering artifact — GNU's
            // jump-exit does not flush it): a failed readonly tempenv
            // assignment before a SPECIAL builtin exits the shell (status 1,
            // `ro=2 :` — no further statement runs), while before an
            // ordinary builtin, external or function the command is SKIPPED
            // and the rest of the command list is abandoned with status 1
            // (ksh93 emulation — GNU errors7.sub: `x=8 echo` runs nothing,
            // `after non-special builtin: 1`; subshell forms exit the
            // subshell with 1 and the parent continues). ExpansionFailure(1)
            // is exactly that skip-and-abandon contract; the default mode
            // keeps the historical apply-and-run.
            if assignments_failed && self.posix_assignment_failure_is_fatal() {
                self.exit_code = 1;
                let special = cmd
                    .words
                    .first()
                    .is_some_and(|word| is_posix_special_builtin(word));
                // Restore the partial tempenv binds before unwinding: the
                // skip-and-abandon path keeps the shell running, and an
                // unbalanced tempenv mark would corrupt every later restore.
                self.restore_temporary_assignments(applied);
                if special {
                    return Err(ExecuteError::ExitCode(1));
                }
                return Err(ExecuteError::ExpansionFailure(1));
            }
            applied
        };
        if self.xtrace_enabled() && cmd.arithmetic_command.is_none() {
            // GNU dispatches `(( ))` to execute_arith_command, whose own
            // xtrace (execute_cmd.c:3940) prints the raw between-parens text;
            // the generic simple-command trace (execute_cmd.c:4480) never
            // fires for it. rubash's words ["((", expr, "))"] would print a
            // second, normalized line (issue: gnu-compat set-x G16).
            let prefix = self.xtrace_prefix();
            let mut xtrace_output = Vec::new();
            for line in self.xtrace_command_lines(cmd) {
                writeln!(xtrace_output, "{prefix}{line}").ok();
            }
            // GNU bash emits xtrace after applying the command's redirects
            // (execute_cmd.c:4480+), so the trace goes to the redirected
            // stderr. Rubash emits xtrace before redirect application, so
            // check redirect_err_append for an inherited 2>&1 (fd-reference
            // target) and route the trace to that fd's endpoint. This
            // preserves the `2>&1` semantics for nested same-shell scripts.
            if let Some(redirect) = &cmd.redirect_err_append {
                let target = self.expand_redirect_target(redirect);
                if self.has_output_fd_target(&target) {
                    let _ = self.write_output_fd_redirect(&target, &xtrace_output);
                } else {
                    self.xtrace_write(&xtrace_output);
                }
            } else {
                self.xtrace_write(&xtrace_output);
            }
        }

        // GNU execute_cmd.c:4480 resets special_builtin_failed before each
        // simple command; builtins that return EX_USAGE/EX_UTILERROR/etc.
        // (> EX_SHERRBASE) set it during dispatch.
        self.special_builtin_failed.set(false);
        let result = if redirect_rejected {
            Ok(())
        } else {
            self.execute_prepared_command(cmd)
        };
        self.finish_process_substitutions(process_substitution_files)?;
        self.finish_assignment_output_process_substitutions_for_command(cmd)?;
        if cmd.background && result.is_ok() {
            self.shell_state.last_background_pid = Some(std::process::id());
            self.exit_code = 0;
        }
        if !keep_temporary_assignments {
            self.restore_temporary_assignments(temporary_assignments);
        } else if Self::attribute_builtin_absorbs_named_tempenv(cmd) {
            // GNU setattr.def:590-614 set_var_attribute ("only called by
            // readonly/export") + declare.def:1105: an attribute builtin
            // absorbs from the temporary environment ONLY the variables its
            // operands name — `FOO=val export BAR` leaves FOO unset while
            // `FOO=val export FOO` persists; every other tempenv binding
            // dies with the temporary environment (rubash#538). `export -f`
            // operands name functions, not variables: nothing absorbs.
            let keep = self.attribute_builtin_operand_names(cmd);
            // GNU's absorption inherits the tempvar's attributes
            // (setattr.def:604 `var->attributes |= tv->attributes &
            // ~att_tempvar`) — assign_in_env marks every tempenv binding
            // exported, so an ABSORBED assignment name keeps att_exported.
            // The __RUBASH_EXPORTED_VARS marker snapshot therefore must not
            // be restored (it predates the tempenv's export marks); the
            // absorbed names are re-marked after the restore instead. Every
            // other real variable entry not named by an operand is restored.
            let absorbed: std::collections::HashSet<String> = cmd
                .assignments
                .iter()
                .filter_map(|(name, _)| {
                    let base = assignment_name_and_append(name).0.to_string();
                    keep.contains(&base).then_some(base)
                })
                .collect();
            let survivors: Vec<_> = temporary_assignments
                .into_iter()
                .filter(|(name, ..)| name != "__RUBASH_EXPORTED_VARS" && !keep.contains(name))
                .collect();
            self.restore_temporary_assignments(survivors);
            for name in &absorbed {
                self.mark_exported(name);
            }
        } else if self.posix_mode_enabled() && self.shell_state.function_depth > 0 {
            // GNU execute_cmd.c:4892-4894 → variables.c:4662
            // merge_temporary_env → variables.c:4485 push_posix_temp_var:
            // a posix special builtin's tempenv binds at the current
            // variable context; a non-local context binding (context>0 &&
            // !local_p) gains att_propagate and descends into the caller's
            // context when the frame pops — surviving the function-call
            // tempenv restore (`var=inner export var` inside `var=func f`).
            // A binding that lands on a local dies with its frame instead
            // (func2's `var=global :` on `local var`).
            for (name, _) in &cmd.assignments {
                let (base, _) = assignment_name_and_append(name);
                let is_local = self
                    .shell_state
                    .local_var_scopes
                    .iter()
                    .any(|scope| scope.contains_key(base));
                let is_function_tempenv = self
                    .function_tempenv_names
                    .iter()
                    .any(|frame| frame.iter().any(|n| n == base));
                if !is_local
                    && is_function_tempenv
                    && !self
                        .tempenv_propagated_names
                        .iter()
                        .any(|(propagated, _)| propagated == base)
                {
                    self.tempenv_propagated_names.push((
                        base.to_string(),
                        capture_var_attrs(&self.shell_state.env_vars, base),
                    ));
                }
            }
        }
        self.update_underscore_parameter(cmd);
        // GNU execute_cmd.c:1004-1017: in POSIX mode, a non-interactive shell
        // exits when a special builtin returned an error status (> EX_SHERRBASE).
        // execute_cmd.c:4657-4666: `builtin_is_special` is only set on the
        // dispatch path where `(cmdflags & CMD_NO_FUNCTIONS) == 0' — the
        // `command' builtin's re-dispatched argument (check_command_builtin
        // sets CMD_NO_FUNCTIONS, execute_cmd.c:4723) never qualifies, so a
        // special builtin executed as `command return ...' cannot mark the
        // failure ("we don't want to exit the shell if a special builtin
        // executed with `command builtin' fails"). The depth gate is the
        // same state GNU tracks as executing_command_builtin
        // (execute_cmd.c:4731-4735; errors8.sub ok 5/ok 6).
        // GNU order (rubash#344): the ERR trap for a failing simple command
        // fires inside execute_simple_command (run_error_trap callers,
        // execute_cmd.c:1000/1166) — DEEPER than the ERREXIT jump in
        // execute_command_internal (execute_cmd.c:624) — so `set -e; trap
        // 'echo E' ERR; false` prints E and exits 1. This dispatch-level
        // errexit conversion is the shell's jump site; fire the trap here
        // when it is about to kill the command, before converting. The
        // non-errexit path keeps firing at the ast_exec loop sites (a fire
        // here would double it once the Ok result reaches them), and
        // maybe_run_error_trap's own gates (inverted / &&-|| context /
        // errtrace function depth) match the errexit conditions.
        let outcome = if result.is_ok()
            && self.special_builtin_failed.get()
            && self.command_builtin_depth == 0
            && self.posix_mode_enabled()
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_INTERACTIVE")
                .map(String::as_str)
                != Some("1")
        {
            self.maybe_run_error_trap(cmd)?;
            Err(ExecuteError::ExitCode(self.exit_code))
        } else if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
            self.maybe_run_error_trap(cmd)?;
            Err(ExecuteError::ExitCode(self.exit_code))
        } else {
            result
        };
        self.assignment_command_name = previous_command_name;
        outcome
    }

    fn execute_prepared_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        let Some(word) = cmd.words.first() else {
            return Ok(());
        };
        // GNU execute_cmd.c:4750-4757: a simple command whose first word
        // starts with `%` is a job spec — dispatch to fg (bg when async)
        // before builtin/function/PATH lookup, with the `%` word itself as
        // the jobspec operand (fg_bg.def get_job_spec sees it in word[0]).
        // The fg builtin itself reports "no job control" when the monitor
        // option is off.
        if word.starts_with('%') {
            let mut fg_cmd = cmd.clone();
            fg_cmd.words.insert(0, "fg".to_string());
            self.exit_code =
                self.execute_fg_bg(&fg_cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
            return Ok(());
        }
        if let Some(message) = self.restricted_command_error(cmd, word) {
            self.write_default_stderr(message.as_bytes())?;
            self.exit_code = 1;
            return Ok(());
        }
        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word) {
            return self.execute_external(cmd);
        }
        if let Some(result) = self.execute_primary_builtin_command(cmd, word)? {
            return result;
        }
        self.execute_late_builtin_command(cmd, word)
    }
}
