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
        let old_function_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_function_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        if let Some(input) = self.loop_redirect_input(cmd) {
            self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
            self.env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        }

        let result = self.execute_for_command(for_command);
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

    pub(in crate::executor) fn execute_select_command(
        &mut self,
        cmd: &CommandNode,
        select_command: &SelectCommand,
    ) -> Result<(), ExecuteError> {
        // `select name [in words ...]; do body; done`
        // Displays a numbered menu of words, prompts for selection, and executes body
        // with the selected word assigned to the variable.
        let values: Vec<String> = select_command
            .words
            .iter()
            .flat_map(|word| self.expand_for_word_values(word))
            .collect();

        if values.is_empty() {
            self.exit_code = 0;
            return Ok(());
        }

        let ps3 = self
            .env_vars
            .get("PS3")
            .cloned()
            .unwrap_or_else(|| "#? ".to_string());

        // Check for stdin from here-string, here-doc, or redirect
        let has_stdin = self.env_vars.contains_key(FUNCTION_STDIN)
            || cmd.here_string.is_some()
            || cmd.heredoc_redirects.iter().any(|r| r.body.is_some());
        let mut stdin_offset = 0usize;
        if has_stdin && !self.env_vars.contains_key(FUNCTION_STDIN) {
            // Set up stdin from here-string or here-doc
            if let Some(ref here_string) = cmd.here_string {
                let input = self.expand_word(here_string);
                self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
                self.env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
            } else if let Some(input) = cmd
                .heredoc_redirects
                .iter()
                .rev()
                .find(|r| r.fd.is_none())
                .and_then(|r| r.body.clone())
            {
                let input = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(&input))
                    .to_string();
                self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
                self.env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
            }
        }
        if has_stdin {
            stdin_offset = self
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
        }

        loop {
            // Display menu
            for (i, value) in values.iter().enumerate() {
                eprintln!("{}{}", i + 1, value);
            }

            // Display prompt
            eprint!("{}", ps3);

            // Read user input
            let mut input = String::new();
            if has_stdin {
                // Read from FUNCTION_STDIN (heredoc/redirect)
                let stdin_content = self
                    .env_vars
                    .get(FUNCTION_STDIN)
                    .cloned()
                    .unwrap_or_default();
                if stdin_offset >= stdin_content.len() {
                    // EOF
                    eprintln!();
                    self.exit_code = 0;
                    return Ok(());
                }
                let remaining = &stdin_content[stdin_offset..];
                if let Some(newline_pos) = remaining.find('\n') {
                    input = remaining[..newline_pos].to_string();
                    stdin_offset += newline_pos + 1;
                } else {
                    input = remaining.to_string();
                    stdin_offset = stdin_content.len();
                }
                self.env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), stdin_offset.to_string());
            } else {
                match std::io::stdin().read_line(&mut input) {
                    Ok(0) => {
                        // EOF
                        eprintln!();
                        self.exit_code = 0;
                        return Ok(());
                    }
                    Ok(_) => {
                        input = input.trim().to_string();
                    }
                    Err(_) => {
                        self.exit_code = 1;
                        return Ok(());
                    }
                }
                input = input.trim().to_string();
            }

            // If input is empty, re-display menu
            if input.is_empty() {
                continue;
            }

            // Parse selection number
            match input.parse::<usize>() {
                Ok(n) if n >= 1 && n <= values.len() => {
                    // Valid selection
                    self.env_vars
                        .insert(select_command.variable.clone(), values[n - 1].clone());
                    set_process_env(&select_command.variable, values[n - 1].clone());

                    let body = Ast {
                        commands: select_command.body.clone(),
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
                        Err(ExecuteError::Break(level)) => {
                            return Err(ExecuteError::Break(level - 1));
                        }
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
                _ => {
                    // Invalid selection - set variable to empty
                    self.env_vars
                        .insert(select_command.variable.clone(), String::new());

                    let body = Ast {
                        commands: select_command.body.clone(),
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
                        Err(ExecuteError::Break(level)) => {
                            return Err(ExecuteError::Break(level - 1));
                        }
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
            }
        }

        self.exit_code = 0;
        Ok(())
    }
}
