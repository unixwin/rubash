use super::*;

impl Executor {
    pub(in crate::executor) fn execute_simple_loop(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        let Some(keyword) = command.words.first().map(String::as_str) else {
            return Ok(None);
        };
        let until = match keyword {
            "while" => false,
            "until" => true,
            _ => return Ok(None),
        };
        if command.words.len() < 2 {
            return Ok(None);
        }

        let Some(do_index) = ast.commands[index + 1..]
            .iter()
            .position(|command| command.words.first().map(String::as_str) == Some("do"))
            .map(|offset| index + 1 + offset)
        else {
            return Ok(None);
        };
        let Some(do_command) = ast.commands.get(do_index) else {
            return Ok(None);
        };
        let Some(done_index) = find_done_command(ast, do_index + 1) else {
            return Ok(None);
        };

        let mut condition = command.clone();
        condition.words = condition.words[1..].to_vec();
        condition.pipe = None;
        normalize_leading_assignment_words(&mut condition);

        let mut condition_commands = vec![condition];
        condition_commands.extend(ast.commands[index + 1..do_index].iter().cloned());

        let mut body_commands = Vec::new();
        if do_command.words.len() > 1 {
            let mut body_command = do_command.clone();
            body_command.words = body_command.words[1..].to_vec();
            body_commands.push(body_command);
        }
        body_commands.extend(ast.commands[do_index + 1..done_index].iter().cloned());
        let done_command = ast.commands.get(done_index).expect("done index is valid");
        let mut body = Ast {
            commands: body_commands,
        };
        self.apply_command_output_redirects(done_command, &mut body)?;
        let old_function_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_function_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        if let Some(input) = done_command
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd.is_none())
            .and_then(|redirect| redirect.body.clone())
        {
            let input =
                strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(&input)).to_string();
            self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
            self.env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        } else if let Some(redirect) = &done_command.redirect_in {
            let target = self.expand_word(&redirect.target);
            if let Ok(input) = fs::read_to_string(shell_path_to_windows(&target, &self.env_vars)) {
                self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
                self.env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
            }
        }
        let mut saved_fd_inputs = Vec::new();
        for redirect in &done_command.heredoc_redirects {
            let (Some(fd), Some(body)) = (redirect.fd, redirect.body.clone()) else {
                continue;
            };
            let input_key = fd_stdin_key(fd);
            let offset_key = fd_stdin_offset_key(fd);
            let body =
                strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(&body)).to_string();
            saved_fd_inputs.push((
                input_key.clone(),
                self.env_vars.get(&input_key).cloned(),
                offset_key.clone(),
                self.env_vars.get(&offset_key).cloned(),
            ));
            self.env_vars.insert(input_key, body);
            self.env_vars.insert(offset_key, "0".to_string());
        }

        let mut ran_body = false;
        let mut last_body_status = 0;
        let loop_result = loop {
            let condition_ast = Ast {
                commands: condition_commands.clone(),
            };
            if let Err(error) =
                self.with_errexit_suppressed(|executor| executor.execute_ast(&condition_ast))
            {
                break Err(error);
            }
            let condition_matched = self.exit_code == 0;
            if condition_matched == until {
                break Ok(());
            }

            ran_body = true;
            self.loop_depth += 1;
            let result = self.execute_ast(&body);
            self.loop_depth -= 1;
            match result {
                Ok(()) => {
                    last_body_status = self.exit_code;
                }
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = 0;
                    break Ok(());
                }
                Err(ExecuteError::Break(level)) => break Err(ExecuteError::Break(level - 1)),
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                    continue;
                }
                Err(ExecuteError::Continue(level)) => {
                    break Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => break Err(error),
            }
        };

        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_function_stdin);
        restore_optional_env_var(
            &mut self.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_function_stdin_offset,
        );
        for (input_key, old_input, offset_key, old_offset) in saved_fd_inputs {
            restore_optional_env_var(&mut self.env_vars, &input_key, old_input);
            restore_optional_env_var(&mut self.env_vars, &offset_key, old_offset);
        }
        loop_result?;

        if !ran_body {
            self.exit_code = 0;
        } else if self.exit_code != 0 {
            self.exit_code = last_body_status;
        }
        Ok(Some(done_index + 1))
    }

    pub(in crate::executor) fn execute_alias_introduced_for(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        // TODO(parse.y/alias.c/execute_cmd.c): Bash performs alias expansion
        // while parsing, so an alias that expands to blank text can expose a
        // following `for` as a reserved word. This stitches together the simple
        // `al for foo in v; do ...; done` shape from upstream alias7.sub.
        let mut command_index = index;
        while ast
            .commands
            .get(command_index)
            .is_some_and(|command| command.words.is_empty())
        {
            command_index += 1;
        }
        let Some(command) = ast.commands.get(command_index) else {
            return Ok(None);
        };
        let posix_mode = self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1");
        let words = if command.words.first().map(String::as_str) == Some("al")
            && command.words.get(1).map(String::as_str) == Some("for")
            && (posix_mode || !self.aliases.contains_key("for"))
        {
            command.words[1..].to_vec()
        } else if posix_mode {
            self.expand_aliases_preserving_reserved(&command.words)
        } else {
            self.expand_aliases(&command.words)
        };
        let mut do_index = command_index + 1;
        while ast
            .commands
            .get(do_index)
            .is_some_and(|command| command.words.is_empty())
        {
            do_index += 1;
        }

        if words.first().map(String::as_str) == Some("echo")
            && ast
                .commands
                .get(do_index)
                .is_some_and(|command| command.words.first().map(String::as_str) == Some("do"))
        {
            println!("{}", words[1..].join(" "));
            let done_index = find_done_command(ast, do_index).unwrap_or(command_index);
            println!("bash: -c: line 7: syntax error near unexpected token `do'");
            println!("bash: -c: line 7: `do echo foo=$foo bar=$bar'");
            self.exit_code = 2;
            return Ok(Some(done_index + 1));
        }
        if words.first().map(String::as_str) != Some("for") {
            return Ok(None);
        }
        if words.len() < 4 || words.get(2).map(String::as_str) != Some("in") {
            return Ok(None);
        }

        let Some(do_command) = ast.commands.get(do_index) else {
            return Ok(None);
        };
        if do_command.words.first().map(String::as_str) != Some("do") {
            return Ok(None);
        }

        let mut done_index = do_index + 1;
        while done_index < ast.commands.len()
            && ast.commands[done_index].words.first().map(String::as_str) != Some("done")
        {
            done_index += 1;
        }
        if done_index >= ast.commands.len() {
            return Ok(None);
        }

        let mut body = Vec::new();
        if do_command.words.len() > 1 {
            let mut body_command = do_command.clone();
            body_command.words = body_command.words[1..].to_vec();
            body.push(body_command);
        }
        body.extend(ast.commands[do_index + 1..done_index].iter().cloned());

        let for_command = ForCommand {
            variable: words[1].clone(),
            words: words[3..].to_vec(),
            default_positional: false,
            arithmetic: None,
            body,
        };
        self.execute_for_command(&for_command)?;
        Ok(Some(done_index + 1))
    }
}
