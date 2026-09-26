use super::*;

impl Executor {
    pub(in crate::executor) fn execute_materialized_command(
        &mut self,
        cmd: &CommandNode,
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
        let ambiguous_redirect = self.reject_ambiguous_redirects(cmd)?;
        let temporary_assignments = if standalone_assignments {
            self.apply_permanent_assignments(&cmd.assignments);
            Vec::new()
        } else {
            self.apply_temporary_assignments(&cmd.assignments)
        };
        if self.xtrace_enabled() && cmd.arithmetic_command.is_none() {
            // GNU dispatches `(( ))` to execute_arith_command, whose own
            // xtrace (execute_cmd.c:3940) prints the raw between-parens text;
            // the generic simple-command trace (execute_cmd.c:4480) never
            // fires for it. rubash's words ["((", expr, "))"] would print a
            // second, normalized line (issue: gnu-compat set-x G16).
            let prefix = self.xtrace_prefix();
            let mut xtrace_output = Vec::new();
            if !cmd.assignments.is_empty() && !cmd.words.is_empty() {
                // GNU traces the assignment prefix on its own line before the
                // command words (`foo=one echo hi` → `+ foo=one` `+ echo hi`).
                let assignments = self.xtrace_assignment_text(cmd);
                writeln!(xtrace_output, "{prefix}{}", assignments.join(" ")).ok();
                writeln!(
                    xtrace_output,
                    "{prefix}{}",
                    cmd.words
                        .iter()
                        .map(|word| {
                            super::prompt_expansion::xtrace_quote_word(
                                crate::builtins::arrayref::take_arrayref_flag(word).1,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                )
                .ok();
            } else {
                let text = self.xtrace_command_text(cmd);
                writeln!(xtrace_output, "{prefix}{text}").ok();
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
        let result = if ambiguous_redirect {
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
        let outcome = if result.is_ok()
            && self.special_builtin_failed.get()
            && self.posix_mode_enabled()
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_INTERACTIVE")
                .map(String::as_str)
                != Some("1")
        {
            Err(ExecuteError::ExitCode(self.exit_code))
        } else if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
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
