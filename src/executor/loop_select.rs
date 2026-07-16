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

        let values = if for_command.default_positional {
            self.positional_params.clone()
        } else {
            let mut values = Vec::new();
            for word in &for_command.words {
                match self.expand_for_word_values_result(word) {
                    Ok(expanded) => values.extend(expanded),
                    Err(pattern) => {
                        self.report_failglob(&pattern);
                        return Err(ExecuteError::ExitCode(1));
                    }
                }
            }
            values
        };
        let mut ran_body = false;
        for value in values {
            ran_body = true;
            self.env_vars
                .insert(for_command.variable.clone(), value.clone());
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
