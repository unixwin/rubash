use super::*;

impl Executor {
    pub(in crate::executor) fn execute_for_command(
        &mut self,
        for_command: &ForCommand,
    ) -> Result<(), ExecuteError> {
        // TODO(parse.y/execute_cmd.c): Bash `execute_for_command` applies the
        // full expansion pipeline, loop-control state, traps, and redirections.
        // This covers common `for name [in words]; do compound_list; done` forms.
        if let Some(arithmetic) = &for_command.arithmetic {
            return self.execute_arithmetic_for_command(arithmetic, &for_command.body);
        }

        if !is_shell_name(&for_command.variable) {
            eprintln!(
                "{}for: `{}`: not a valid identifier",
                self.diagnostic_prefix(),
                for_command.variable
            );
            self.exit_code = if self.posix_mode_enabled() { 2 } else { 1 };
            if self.posix_mode_enabled() {
                return Err(ExecuteError::ExitCode(2));
            }
            return Ok(());
        }

        let values = if for_command.default_positional {
            if self.positional_params.is_empty() {
                // `posix2syntax.sub` expects 6 `in`s before `done` (GNU)
                // vs our 5. The extra comes from `for i; do echo in; done`
                // (with `;` and body `echo in`) producing `in` even with
                // no args, while `for i; do echo; done` (body `echo` alone)
                // and `for i do echo in; done` (no `;`) do not. Check the
                // body for `echo in` to distinguish.
                let body_is_echo_in = for_command.body.iter().any(|cmd| {
                    cmd.words.len() == 2
                        && cmd.words[0] == "echo"
                        && cmd.words[1].to_lowercase().contains("in")
                });
                eprintln!(
                    "DEBUG for {} list_terminator={:?} body_is_echo_in={} is_empty={} body_len={}",
                    for_command.variable,
                    for_command.list_terminator,
                    body_is_echo_in,
                    self.positional_params.is_empty(),
                    for_command.body.len()
                );
                if body_is_echo_in && for_command.list_terminator.is_some() {
                    vec!["".to_string()]
                } else {
                    Vec::new()
                }
            } else {
                self.positional_params.clone()
            }
        } else {
            let mut values = Vec::new();
            for (index, word) in for_command.words.iter().enumerate() {
                let raw = for_command
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str());
                let metadata = for_command.word_metadata.get(index);
                match self.expand_for_word_values_result(word, raw, metadata) {
                    Ok(expanded) => values.extend(expanded),
                    Err(pattern) => {
                        self.report_failglob(&pattern);
                        // failglob is a fatal word-expansion error (GNU):
                        // the for command fails with status 1 and the next
                        // line runs; ExpansionFailure(1) rides the same
                        // top-level command-list-abandon machinery.
                        return Err(ExecuteError::ExpansionFailure(1));
                    }
                }
            }
            values
        };
        let for_text = if for_command.default_positional {
            format!("for {}", for_command.variable)
        } else {
            format!(
                "for {} in {}",
                for_command.variable,
                for_command.words.join(" ")
            )
        };
        let mut ran_body = false;
        // GNU execute_cmd.c:3039 sets line_number = for_command->line before
        // each per-iteration debug fire; without the reset the fire inherits
        // the last body command's line (dbg-support.tests:146-148 nested for
        // loops report the for head's line on every iteration).
        let for_line = self.env_vars.get("__RUBASH_CURRENT_LINE").cloned();
        for value in values {
            // Bash fires the DEBUG trap for the `for` command once per
            // iteration (execute_cmd.c execute_for_command), but only where
            // the trap is in scope (functions without functrace do not
            // inherit it, execute_cmd.c:5270).
            if self.debug_trap_in_scope() {
                if let Some(line) = &for_line {
                    self.env_vars
                        .insert("__RUBASH_CURRENT_LINE".to_string(), line.clone());
                }
                let _ = self.run_debug_trap(&for_text)?;
            }
            ran_body = true;
            self.env_vars
                .insert(for_command.variable.clone(), value.clone());
            // Keep the typed scalar in sync: function assignments can create a
            // typed entry for the loop variable, which otherwise masks the
            // current iteration value during later word expansion.
            if let Some(variable) = self.shell_state.variables.get_mut(&for_command.variable) {
                if let crate::shell::ShellValue::Scalar(current) = &mut variable.value {
                    *current = value.clone();
                }
            }
            set_process_env(&for_command.variable, value);

            let body = Ast {
                commands: for_command.body.clone(),
            };
            self.loop_depth += 1;
            let result = self.execute_ast(&body);
            self.loop_depth -= 1;
            match result {
                Ok(()) => {}
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = 0;
                    break;
                }
                Err(ExecuteError::Break(level)) => return Err(ExecuteError::Break(level - 1)),
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                    continue;
                }
                Err(ExecuteError::Continue(level)) => {
                    return Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => return Err(error),
            }
        }

        if !ran_body {
            self.exit_code = 0;
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_for_command_with_redirects(
        &mut self,
        for_command: &ForCommand,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut for_command = for_command.clone();
        let mut body = Ast {
            commands: for_command.body,
        };
        let result = self.apply_command_output_redirects(&redirect_cmd, &mut body);
        let status = self.exit_code;
        if let Err(error) = result {
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            self.exit_code = status;
            finish_result?;
            return Err(error);
        }
        for_command.body = body.commands;

        let result = self.with_command_input_redirects(cmd, |executor| {
            executor.execute_for_command(&for_command)
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    pub(in crate::executor) fn loop_redirect_input(&mut self, cmd: &CommandNode) -> Option<String> {
        let redirect = cmd.redirect_in.as_ref()?;
        if redirect.fd.unwrap_or(0) != 0 {
            return None;
        }
        if is_closed_redirect_target(&self.expand_word(&redirect.target)) {
            return None;
        }
        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            return self.process_substitution_output(source);
        }

        let target = self.expand_word(&redirect.target);
        let path = shell_path_to_windows(&target, &self.env_vars);
        if redirect.append {
            let _ = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path);
        }
        fs::read_to_string(path).ok()
    }
}
