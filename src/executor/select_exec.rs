use super::*;

enum SelectBodyFlow {
    ContinueLoop,
    BreakLoop,
}

impl Executor {
    pub(in crate::executor) fn execute_select_command(
        &mut self,
        cmd: &CommandNode,
        select_command: &SelectCommand,
    ) -> Result<(), ExecuteError> {
        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut select_command = select_command.clone();
        let mut body = Ast {
            commands: select_command.body,
        };
        let result = self.apply_command_output_redirects(&redirect_cmd, &mut body);
        let status = self.exit_code;
        if let Err(error) = result {
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            self.exit_code = status;
            finish_result?;
            return Err(error);
        }
        select_command.body = body.commands;

        let values: Vec<String> = if select_command.default_positional {
            self.positional_params.clone()
        } else {
            let mut values = Vec::new();
            for word in &select_command.words {
                match self.expand_for_word_values_result(word) {
                    Ok(expanded) => values.extend(expanded),
                    Err(pattern) => {
                        self.report_failglob(&pattern);
                        self.finish_compound_output_process_substitutions(group_outputs)?;
                        return Err(ExecuteError::ExitCode(1));
                    }
                }
            }
            values
        };

        if values.is_empty() {
            self.exit_code = 0;
            self.finish_compound_output_process_substitutions(group_outputs)?;
            return Ok(());
        }

        let result = self.with_command_input_redirects(cmd, |executor| {
            executor.execute_select_loop(&select_command, &values)
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn execute_select_loop(
        &mut self,
        select_command: &SelectCommand,
        values: &[String],
    ) -> Result<(), ExecuteError> {
        let ps3 = self
            .env_vars
            .get("PS3")
            .cloned()
            .unwrap_or_else(|| "#? ".to_string());
        let has_stdin = self.env_vars.contains_key(FUNCTION_STDIN);
        let mut stdin_offset = self.select_stdin_offset(has_stdin);

        loop {
            for (i, value) in values.iter().enumerate() {
                eprintln!("{}{}", i + 1, value);
            }
            eprint!("{}", ps3);

            let Some(input) = self.read_select_input(has_stdin, &mut stdin_offset) else {
                self.exit_code = 0;
                return Ok(());
            };
            if input.is_empty() {
                continue;
            }

            let selected = input
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1 && *n <= values.len())
                .map(|n| values[n - 1].clone())
                .unwrap_or_default();
            self.env_vars
                .insert(select_command.variable.clone(), selected.clone());
            set_process_env(&select_command.variable, selected);

            match self.execute_select_body(&select_command.body)? {
                SelectBodyFlow::ContinueLoop => continue,
                SelectBodyFlow::BreakLoop => break,
            }
        }

        Ok(())
    }

    fn select_stdin_offset(&self, has_stdin: bool) -> usize {
        if has_stdin {
            self.env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
        } else {
            0
        }
    }

    fn read_select_input(&mut self, has_stdin: bool, stdin_offset: &mut usize) -> Option<String> {
        if !has_stdin {
            let mut input = String::new();
            return match std::io::stdin().read_line(&mut input) {
                Ok(0) => {
                    eprintln!();
                    None
                }
                Ok(_) => Some(input.trim().to_string()),
                Err(_) => {
                    self.exit_code = 1;
                    Some(String::new())
                }
            };
        }

        let stdin_content = self
            .env_vars
            .get(FUNCTION_STDIN)
            .cloned()
            .unwrap_or_default();
        if *stdin_offset >= stdin_content.len() {
            eprintln!();
            return None;
        }
        let remaining = &stdin_content[*stdin_offset..];
        let input = if let Some(newline_pos) = remaining.find('\n') {
            *stdin_offset += newline_pos + 1;
            remaining[..newline_pos].to_string()
        } else {
            *stdin_offset = stdin_content.len();
            remaining.to_string()
        };
        self.env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), stdin_offset.to_string());
        Some(input)
    }

    fn execute_select_body(
        &mut self,
        body: &[CommandNode],
    ) -> Result<SelectBodyFlow, ExecuteError> {
        let body = Ast {
            commands: body.to_vec(),
        };
        self.loop_depth += 1;
        let result = self.execute_ast(&body);
        self.loop_depth -= 1;
        match result {
            Ok(()) => Ok(SelectBodyFlow::ContinueLoop),
            Err(ExecuteError::Break(level)) if level <= 1 => {
                self.exit_code = 0;
                Ok(SelectBodyFlow::BreakLoop)
            }
            Err(ExecuteError::Break(level)) => Err(ExecuteError::Break(level - 1)),
            Err(ExecuteError::Continue(level)) if level <= 1 => {
                self.exit_code = 0;
                Ok(SelectBodyFlow::ContinueLoop)
            }
            Err(ExecuteError::Continue(level)) => Err(ExecuteError::Continue(level - 1)),
            Err(error) => Err(error),
        }
    }
}
