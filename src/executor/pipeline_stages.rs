use super::*;

impl Executor {
    pub(in crate::executor) fn execute_compound_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<(String, String, i32), ExecuteError> {
        let old_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        self.env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        self.env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        let saved_stdout_capture = self.stdout_capture.take();
        let saved_stderr_capture = self.stderr_capture.take();
        self.stdout_capture = Some(Vec::new());
        self.stderr_capture = Some(Vec::new());
        let mut stage_command = command.clone();
        stage_command.redirect_out = None;
        stage_command.append = None;

        let result = if stage_command.brace_group.is_some() {
            self.execute_brace_group_pipeline(&stage_command)
                .map(|_| ())
        } else {
            self.execute_command(&stage_command)
        };
        let output = self.stdout_capture.take().unwrap_or_default();
        let stderr = self.stderr_capture.take().unwrap_or_default();
        self.stdout_capture = saved_stdout_capture;
        self.stderr_capture = saved_stderr_capture;
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN_OFFSET, old_stdin_offset);
        result?;

        Ok((
            String::from_utf8_lossy(&output).into_owned(),
            String::from_utf8_lossy(&stderr).into_owned(),
            self.last_exit_code(),
        ))
    }

    pub(in crate::executor) fn execute_function_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(Some((String::new(), String::new(), 0)));
        };
        let expanded_name = self.expand_word(name);
        let Some(function_name) = self.function_name_for_command_word(&expanded_name) else {
            return Ok(None);
        };
        let args = command.words[1..]
            .iter()
            .map(|word| self.expand_word(word))
            .collect::<Vec<_>>();
        let mut call = command.clone();
        call.words = std::iter::once(function_name.clone())
            .chain(args.iter().cloned())
            .collect();

        let old_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_stdin_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        self.env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        self.env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        let saved_stdout_capture = self.stdout_capture.take();
        let saved_stderr_capture = self.stderr_capture.take();
        self.stdout_capture = Some(Vec::new());
        self.stderr_capture = Some(Vec::new());
        let result = self.execute_function(&function_name, &args, &call);
        let output = self.stdout_capture.take().unwrap_or_default();
        let stderr = self.stderr_capture.take().unwrap_or_default();
        self.stdout_capture = saved_stdout_capture;
        self.stderr_capture = saved_stderr_capture;
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN_OFFSET, old_stdin_offset);
        result?;
        Ok(Some((
            String::from_utf8_lossy(&output).into_owned(),
            String::from_utf8_lossy(&stderr).into_owned(),
            self.last_exit_code(),
        )))
    }

    pub(in crate::executor) fn execute_external_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(Some((String::new(), String::new(), 0)));
        };
        let Some(program) = find_user_command(&self.expand_word(name), &self.env_vars) else {
            return Ok(None);
        };

        let args: Vec<String> = command.words[1..]
            .iter()
            .map(|word| self.expand_word(word))
            .collect();
        let mut process = if should_run_with_shell(&program) {
            if let Some(shell) = find_shell(&self.env_vars) {
                let mut command = Command::new(shell);
                command.arg(&program);
                command.args(&args);
                command
            } else {
                Command::new(&program)
            }
        } else {
            let mut command = Command::new(&program);
            command.args(&args);
            command
        };

        self.apply_child_environment(&mut process);
        for (var_name, var_value) in &command.assignments {
            if is_valid_process_env(var_name, var_value) {
                process.env(var_name, var_value);
            }
        }
        process.stdin(Stdio::piped()).stdout(Stdio::piped());

        let mut child = process.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes())?;
        }
        let output = child.wait_with_output()?;

        Ok(Some((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code().unwrap_or(1),
        )))
    }

    pub(in crate::executor) fn write_pipeline_output(
        &self,
        command: &CommandNode,
        output: &str,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &command.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            file.write_all(output.as_bytes())?;
        } else if let Some(redirect) = &command.append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            file.write_all(output.as_bytes())?;
        } else {
            print!("{output}");
        }
        Ok(())
    }

    pub(in crate::executor) fn skip_and_or_rhs(&self, ast: &Ast, index: usize) -> Option<usize> {
        // TODO(parse.y/execute_cmd.c): Bash executes AND_AND/OR_OR lists from
        // the grammar, not by scanning flattened commands. This narrow bridge
        // keeps `cmd || { echo ...; exit 1; }` failure handlers from running
        // after a successful command in upstream source8.sub.
        let connector = ast.commands.get(index)?.and_or()?;
        let should_skip = (connector && self.exit_code != 0) || (!connector && self.exit_code == 0);
        if !should_skip {
            return None;
        }

        if ast
            .commands
            .get(index)
            .is_some_and(|command| is_arithmetic_command_words(&command.words))
        {
            return Some((index + 2).min(ast.commands.len()));
        }

        let start_line = ast.commands.get(index + 1).and_then(|command| command.line);
        let mut next_index = index + 1;
        while next_index < ast.commands.len()
            && ast.commands[next_index].line == start_line
            && ast.commands[next_index].and_or().is_none()
        {
            next_index += 1;
        }
        Some(next_index.max(index + 1))
    }

    pub(in crate::executor) fn execute_alias_escaped_pipe(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        // TODO(parse.y/alias.c): Bash pushes alias text back to the parser, so
        // an alias ending with backslash can quote the next input character.
        // This covers alias4.sub's `alias a='printf "<%s>\n" \'` followed by
        // `a|cat`, which should pass literal `|cat` to printf.
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        if command.pipe.is_none() || command.words.len() != 1 {
            return Ok(None);
        }

        let Some(alias) = self.aliases.get(&command.words[0]) else {
            return Ok(None);
        };
        if !alias.value.ends_with('\\') {
            return Ok(None);
        }

        let Some(next_command) = ast.commands.get(index + 1) else {
            return Ok(None);
        };
        let mut source = alias.value.trim_end_matches('\\').trim_end().to_string();
        source.push_str(" \\|");
        source.push_str(&next_command.words.join(" "));

        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        self.execute_ast(&reparsed)?;
        Ok(Some(index + 2))
    }
}
