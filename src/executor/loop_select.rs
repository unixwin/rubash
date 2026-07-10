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
            for_command
                .words
                .iter()
                .flat_map(|word| self.expand_for_word_values(word))
                .collect()
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
        let mut for_command = for_command.clone();
        let mut body = Ast {
            commands: for_command.body,
        };
        self.apply_command_output_redirects(cmd, &mut body)?;
        for_command.body = body.commands;

        let old_function_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_function_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        if let Some(input) = self.loop_redirect_input(cmd) {
            self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
            self.env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        }

        let result = self.execute_for_command(&for_command);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_function_stdin);
        restore_optional_env_var(
            &mut self.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_function_stdin_offset,
        );
        result
    }

    pub(in crate::executor) fn loop_redirect_input(&mut self, cmd: &CommandNode) -> Option<String> {
        let redirect = cmd.redirect_in.as_ref()?;
        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            return self.process_substitution_output(source);
        }

        let target = self.expand_word(&redirect.target);
        fs::read_to_string(shell_path_to_windows(&target, &self.env_vars)).ok()
    }
}
