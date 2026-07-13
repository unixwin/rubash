use super::*;

impl Executor {
    pub(in crate::executor) fn execute_inverted_ast_command(
        &mut self,
        inverted_command: &InvertedCommand,
    ) -> Result<(), ExecuteError> {
        let ast = Ast {
            commands: vec![(*inverted_command.command).clone()],
        };
        self.with_errexit_suppressed(|executor| executor.execute_ast(&ast))?;
        self.exit_code = invert_exit_status(self.exit_code);
        Ok(())
    }

    pub(in crate::executor) fn execute_background_ast_command(
        &mut self,
        background_command: &BackgroundCommand,
    ) -> Result<(), ExecuteError> {
        let ast = Ast {
            commands: vec![(*background_command.command).clone()],
        };
        self.execute_ast(&ast)?;
        self.last_background_pid = Some(std::process::id());
        self.exit_code = 0;
        Ok(())
    }

    pub(in crate::executor) fn execute_time_ast_command(
        &mut self,
        time_command: &TimeCommand,
    ) -> Result<(), ExecuteError> {
        if let Some(pipeline_command) = &time_command.command.pipeline_command {
            self.execute_pipeline_command(pipeline_command)?;
        } else if time_command.command.brace_group.is_some() {
            self.execute_brace_group_pipeline(&time_command.command)?;
        } else {
            self.execute_command(&time_command.command)?;
        }
        print_posix_time();
        if time_command.inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_time_prefixed_compound_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let inverted = time_prefix_parts(&cmd.words)
            .map(|parts| parts.inverted)
            .unwrap_or(false);
        let result = if let Some(for_command) = &cmd.for_command {
            self.execute_for_command_with_redirects(for_command, cmd)
        } else if let Some(if_command) = &cmd.if_command {
            self.execute_if_command_with_redirects(cmd, if_command)
        } else if let Some(loop_command) = &cmd.loop_command {
            self.execute_loop_command_with_redirects(cmd, loop_command)
        } else if let Some(select_command) = &cmd.select_command {
            self.execute_select_command(cmd, select_command)
        } else if let Some(case_command) = &cmd.case_command {
            self.execute_case_command_with_redirects(cmd, case_command)
        } else if let Some(coproc_cmd) = &cmd.coproc_command {
            self.execute_coproc_command(cmd, coproc_cmd)
        } else if let Some(subshell_command) = &cmd.subshell_command {
            self.execute_subshell_command_with_redirects(cmd, subshell_command)
        } else if cmd.brace_group.is_some() {
            self.execute_brace_group_pipeline(cmd).map(|_| ())
        } else {
            Ok(())
        };
        print_posix_time();
        result?;
        if inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_time_prefixed_command_sequence(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        let Some(prefix) = time_prefix_parts(&command.words) else {
            return Ok(None);
        };
        if !matches!(
            command.words.get(prefix.command_index).map(String::as_str),
            Some("if" | "while" | "until")
        ) {
            return Ok(None);
        }

        let mut timed_ast = Ast {
            commands: ast.commands[index..].to_vec(),
        };
        if let Some(first) = timed_ast.commands.first_mut() {
            first.words = command.words[prefix.command_index..].to_vec();
            if command.word_kinds.len() == command.words.len() {
                first.word_kinds = command.word_kinds[prefix.command_index..].to_vec();
            }
        }

        let next_index = match timed_ast.commands[0].words.first().map(String::as_str) {
            Some("if") => crate::builtins::source::execute_simple_if(self, &timed_ast, 0)?,
            Some("while" | "until") => self.execute_simple_loop(&timed_ast, 0)?,
            _ => None,
        };
        let Some(next_index) = next_index else {
            return Ok(None);
        };

        print_posix_time();
        if prefix.inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        Ok(Some(index + next_index))
    }

    pub(in crate::executor) fn execute_arithmetic_for_command(
        &mut self,
        arithmetic: &ArithmeticForCommand,
        body: &[CommandNode],
    ) -> Result<(), ExecuteError> {
        if !arithmetic.init.trim().is_empty()
            && self
                .eval_arithmetic_command_value(&arithmetic.init)
                .is_none()
        {
            self.exit_code = 1;
            return Ok(());
        }

        let mut ran_body = false;
        loop {
            if !arithmetic.test.trim().is_empty() {
                match self.eval_arithmetic_command_value(&arithmetic.test) {
                    Some(0) => break,
                    Some(_) => {}
                    None => {
                        self.exit_code = 1;
                        break;
                    }
                }
            }

            ran_body = true;
            let ast = Ast {
                commands: body.to_vec(),
            };
            self.loop_depth += 1;
            let result = self.execute_ast(&ast);
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
                }
                Err(ExecuteError::Continue(level)) => {
                    return Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => return Err(error),
            }

            if !arithmetic.update.trim().is_empty()
                && self
                    .eval_arithmetic_command_value(&arithmetic.update)
                    .is_none()
            {
                self.exit_code = 1;
                break;
            }
        }

        if !ran_body {
            self.exit_code = 0;
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_if_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        if_command: &IfCommand,
    ) -> Result<(), ExecuteError> {
        if self.if_command_needs_alias_scan(if_command) {
            let flat = flatten_if_command_for_alias_scan(cmd, if_command);
            crate::builtins::source::execute_simple_if(self, &Ast { commands: flat }, 0)?;
            return Ok(());
        }

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut if_command = if_command.clone();
        let result = apply_if_redirects(self, &redirect_cmd, &mut if_command).and_then(|()| {
            self.with_command_input_redirects(cmd, |executor| {
                executor.execute_if_command(&if_command)
            })
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    pub(in crate::executor) fn execute_loop_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        loop_command: &LoopCommand,
    ) -> Result<(), ExecuteError> {
        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut loop_command = loop_command.clone();
        let result = apply_redirects_to_commands(self, &redirect_cmd, &mut loop_command.body)
            .and_then(|()| {
                self.with_loop_fd_heredocs(cmd, |executor| {
                    executor.with_command_input_redirects(cmd, |executor| {
                        executor.execute_loop_command(&loop_command)
                    })
                })
            });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    pub(in crate::executor) fn execute_subshell_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        subshell_command: &SubshellCommand,
    ) -> Result<(), ExecuteError> {
        let saved_env = self.env_vars.clone();
        let saved_depth = self.subshell_depth.get();
        self.subshell_depth.set(saved_depth + 1);

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut body = Ast {
            commands: subshell_command.body.clone(),
        };
        let result = self
            .apply_command_output_redirects(&redirect_cmd, &mut body)
            .and_then(|()| {
                self.with_command_input_redirects(cmd, |executor| executor.execute_ast(&body))
            });
        let status = self.exit_code;

        self.restore_shell_env(saved_env);
        self.subshell_depth.set(saved_depth);
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn execute_loop_command(&mut self, loop_command: &LoopCommand) -> Result<(), ExecuteError> {
        let mut ran_body = false;
        let mut last_body_status = 0;

        loop {
            let condition = Ast {
                commands: loop_command.condition.clone(),
            };
            self.with_errexit_suppressed(|executor| executor.execute_ast(&condition))?;
            let condition_matched = self.exit_code == 0;
            if condition_matched == loop_command.until {
                break;
            }

            ran_body = true;
            let body = Ast {
                commands: crate::builtins::source::normalize_inline_compound_commands(
                    loop_command.body.clone(),
                ),
            };
            self.loop_depth += 1;
            let result = self.execute_ast(&body);
            self.loop_depth -= 1;
            match result {
                Ok(()) => {
                    last_body_status = self.exit_code;
                }
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
        } else if self.exit_code != 0 {
            self.exit_code = last_body_status;
        }
        Ok(())
    }

    fn with_loop_fd_heredocs<F>(&mut self, cmd: &CommandNode, f: F) -> Result<(), ExecuteError>
    where
        F: FnOnce(&mut Executor) -> Result<(), ExecuteError>,
    {
        let mut saved_fd_inputs = Vec::new();
        for redirect in &cmd.heredoc_redirects {
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

        let result = f(self);
        for (input_key, old_input, offset_key, old_offset) in saved_fd_inputs {
            restore_optional_env_var(&mut self.env_vars, &input_key, old_input);
            restore_optional_env_var(&mut self.env_vars, &offset_key, old_offset);
        }
        result
    }

    fn if_command_needs_alias_scan(&self, if_command: &IfCommand) -> bool {
        if !self.alias_expansion_enabled() {
            return false;
        }

        if self.commands_contain_alias_if_control(&if_command.then_body) {
            return true;
        }
        if if_command.elif_branches.iter().any(|branch| {
            self.commands_contain_alias_if_control(&branch.condition)
                || self.commands_contain_alias_if_control(&branch.body)
        }) {
            return true;
        }
        if let Some(body) = &if_command.else_body {
            return self.commands_contain_alias_if_control(body);
        }
        false
    }

    fn commands_contain_alias_if_control(&self, commands: &[CommandNode]) -> bool {
        commands.iter().any(|command| {
            let words = self.expand_aliases(&command.words);
            matches!(
                words.first().map(String::as_str),
                Some("if" | "then" | "elif" | "else" | "fi")
            )
        })
    }

    fn execute_if_command(&mut self, if_command: &IfCommand) -> Result<(), ExecuteError> {
        if self.if_condition_matches(&if_command.condition)? {
            return self.execute_ast(&Ast {
                commands: crate::builtins::source::normalize_inline_compound_commands(
                    if_command.then_body.clone(),
                ),
            });
        }

        for branch in &if_command.elif_branches {
            if self.if_condition_matches(&branch.condition)? {
                return self.execute_ast(&Ast {
                    commands: crate::builtins::source::normalize_inline_compound_commands(
                        branch.body.clone(),
                    ),
                });
            }
        }

        if let Some(body) = &if_command.else_body {
            return self.execute_ast(&Ast {
                commands: crate::builtins::source::normalize_inline_compound_commands(body.clone()),
            });
        }

        self.exit_code = 0;
        Ok(())
    }

    fn if_condition_matches(&mut self, condition: &[CommandNode]) -> Result<bool, ExecuteError> {
        let ast = Ast {
            commands: condition.to_vec(),
        };
        self.with_errexit_suppressed(|executor| executor.execute_ast(&ast))?;
        Ok(self.exit_code == 0)
    }

    pub(in crate::executor) fn execute_coproc_command(
        &mut self,
        cmd: &CommandNode,
        coproc_cmd: &crate::parser::CoprocCommand,
    ) -> Result<(), ExecuteError> {
        let array_name = coproc_cmd
            .name
            .clone()
            .unwrap_or_else(|| "COPROC".to_string());
        use std::process::{Command, Stdio};
        let exe = std::env::var_os("CARGO_BIN_EXE_rubash")
            .map(std::path::PathBuf::from)
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());

        let mut child = if let Some(body) = &coproc_cmd.body {
            // Compound command body: coproc [NAME] { body; } or ( body )
            let body_text = bash_command_sequence_text(body);
            let mut child = Command::new(&exe);
            child.arg("--").arg("-c").arg(&body_text);
            child
        } else if !coproc_cmd.words.is_empty() {
            // Simple command: coproc [NAME] command [args...]
            let words: Vec<&str> = coproc_cmd.words.iter().map(|w| w.as_str()).collect();
            let mut child = Command::new(&exe);
            child.arg("--");
            for w in &words {
                child.arg(w);
            }
            child
        } else {
            eprintln!(
                "{}coproc: usage: coproc [NAME] command [args...]",
                self.diagnostic_prefix()
            );
            self.exit_code = 1;
            return Ok(());
        };

        for (key, value) in &self.env_vars {
            if !key.starts_with("__RUBASH_") {
                child.env(key, value);
            }
        }

        // Create pipes for bidirectional communication
        let stdin_result = std::io::pipe();
        let stdout_result = std::io::pipe();

        if let (Ok((stdin_reader, stdin_writer)), Ok((stdout_reader, stdout_writer))) =
            (stdin_result, stdout_result)
        {
            child.stdin(stdin_writer);
            child.stdout(stdout_reader);
            child.stderr(Stdio::inherit());
            self.apply_coproc_redirects(cmd, &mut child)?;

            match child.spawn() {
                Ok(child_proc) => {
                    // stdin_writer and stdout_reader were moved into the child process
                    // stdin_reader and stdout_writer are now unusable (drop them)
                    drop(stdin_reader);
                    drop(stdout_writer);

                    let pid = child_proc.id();
                    // Store the file descriptors in env for COPROC array
                    let stdin_key = format!("__RUBASH_COPROC_STDIN_{}", pid);
                    let stdout_key = format!("__RUBASH_COPROC_STDOUT_{}", pid);
                    self.env_vars.insert(stdin_key, "pipe".to_string());
                    self.env_vars.insert(stdout_key, "pipe".to_string());

                    let array_value = format!("({} {})", 0, 1);
                    self.env_vars.insert(array_name.clone(), array_value);
                    mark_env_name(&mut self.env_vars, "__RUBASH_ARRAY_VARS", &array_name);
                    self.env_vars
                        .insert(format!("{}_PID", array_name), pid.to_string());
                    self.exit_code = 0;
                }
                Err(e) => {
                    eprintln!("{}coproc: failed to spawn: {}", self.diagnostic_prefix(), e);
                    self.exit_code = 126;
                }
            }
        } else {
            eprintln!("{}coproc: failed to create pipes", self.diagnostic_prefix());
            self.exit_code = 1;
        }

        Ok(())
    }

    fn apply_coproc_redirects(
        &self,
        cmd: &CommandNode,
        child: &mut Command,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_word(&redirect.target);
                if is_closed_redirect_target(&target) {
                    child.stdin(Stdio::null());
                } else if redirect_target_fd(&target).is_none() {
                    child.stdin(Stdio::from(File::open(shell_path_to_windows(
                        &target,
                        &self.env_vars,
                    ))?));
                }
            }
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                child.stdout(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stdout(Stdio::from(
                    self.create_redirect_output(&target, redirect.clobber)?,
                ));
            }
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                child.stdout(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stdout(Stdio::from(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.env_vars))?,
                ));
            }
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                child.stderr(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stderr(Stdio::from(
                    self.create_redirect_output(&target, redirect.clobber)?,
                ));
            }
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                child.stderr(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stderr(Stdio::from(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.env_vars))?,
                ));
            }
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_case_command(
        &mut self,
        case_command: &CaseCommand,
    ) -> Result<(), ExecuteError> {
        // TODO(parse.y/execute_cmd.c/pathexp.c): Bash case execution uses the
        // full pattern matcher, fall-through operators, expansion flags, and
        // compound-list control flow. This handles the common shell glob
        // operators used by simple `case` clauses.
        let word = self.expand_case_word(&case_command.word);
        // Strip surrounding quotes from word (bash behavior: quotes are literal in case patterns)
        let word = strip_surrounding_quotes(&word);
        let mut fall_through = false;
        let mut index = 0;
        while let Some(clause) = case_command.clauses.get(index) {
            let matched = fall_through
                || clause.patterns.iter().any(|pattern| {
                    let expanded = self.expand_word(pattern);
                    let decoded = decode_parameter_pattern_quotes(&expanded);
                    let stripped = strip_surrounding_quotes(&decoded);
                    if stripped.contains("@(")
                        || stripped.contains("*(")
                        || stripped.contains("+(")
                        || stripped.contains("?(")
                        || stripped.contains("!(")
                    {
                        crate::executor::conditional::extglob_case_pattern_matches(&stripped, &word)
                    } else {
                        case_pattern_matches(&stripped, &word)
                    }
                });
            if matched {
                let body = Ast {
                    commands: clause.body.clone(),
                };
                self.execute_ast(&body)?;
                match clause.terminator {
                    CaseTerminator::Break => return Ok(()),
                    CaseTerminator::FallThrough => {
                        fall_through = true;
                    }
                    CaseTerminator::TestNext => {
                        fall_through = false;
                    }
                }
            }
            index += 1;
        }

        self.exit_code = 0;
        Ok(())
    }

    pub(in crate::executor) fn execute_case_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        case_command: &CaseCommand,
    ) -> Result<(), ExecuteError> {
        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut case_command = case_command.clone();
        let result = self.apply_case_command_redirects(&redirect_cmd, &mut case_command);
        let status = self.exit_code;
        if let Err(error) = result {
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            self.exit_code = status;
            finish_result?;
            return Err(error);
        }
        let result = self.with_command_input_redirects(cmd, |executor| {
            executor.execute_case_command(&case_command)
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn apply_case_command_redirects(
        &mut self,
        cmd: &CommandNode,
        case_command: &mut CaseCommand,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            for clause in &mut case_command.clauses {
                apply_stdout_append_redirect(&mut clause.body, &append_redirect);
            }
        } else if let Some(redirect) = &cmd.append {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_word(&redirect.target);
            for clause in &mut case_command.clauses {
                apply_stdout_append_redirect(&mut clause.body, &append_redirect);
            }
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if redirect_target_fd(&target).is_none() && !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            for clause in &mut case_command.clauses {
                apply_stderr_append_redirect(&mut clause.body, &append_redirect);
            }
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_word(&redirect.target);
            for clause in &mut case_command.clauses {
                apply_stderr_append_redirect(&mut clause.body, &append_redirect);
            }
        }

        Ok(())
    }
}

pub(in crate::executor) fn command_is_time_prefixed_compound(cmd: &CommandNode) -> bool {
    cmd.words.first().map(String::as_str) == Some("time")
        && (cmd.for_command.is_some()
            || cmd.if_command.is_some()
            || cmd.loop_command.is_some()
            || cmd.select_command.is_some()
            || cmd.case_command.is_some()
            || cmd.coproc_command.is_some()
            || cmd.subshell_command.is_some()
            || cmd.brace_group.is_some())
}

fn apply_if_redirects(
    executor: &mut Executor,
    cmd: &CommandNode,
    if_command: &mut IfCommand,
) -> Result<(), ExecuteError> {
    apply_redirects_to_commands(executor, cmd, &mut if_command.condition)?;
    apply_redirects_to_commands(executor, cmd, &mut if_command.then_body)?;
    for branch in &mut if_command.elif_branches {
        apply_redirects_to_commands(executor, cmd, &mut branch.condition)?;
        apply_redirects_to_commands(executor, cmd, &mut branch.body)?;
    }
    if let Some(body) = &mut if_command.else_body {
        apply_redirects_to_commands(executor, cmd, body)?;
    }
    Ok(())
}

fn apply_redirects_to_commands(
    executor: &mut Executor,
    cmd: &CommandNode,
    commands: &mut Vec<CommandNode>,
) -> Result<(), ExecuteError> {
    let mut ast = Ast {
        commands: std::mem::take(commands),
    };
    executor.apply_command_output_redirects(cmd, &mut ast)?;
    *commands = ast.commands;
    Ok(())
}

fn flatten_if_command_for_alias_scan(
    cmd: &CommandNode,
    if_command: &IfCommand,
) -> Vec<CommandNode> {
    let mut commands = Vec::new();
    push_if_condition(&mut commands, "if", &if_command.condition);
    commands.push(command_with_words(["then"]));
    commands.extend(if_command.then_body.clone());
    for branch in &if_command.elif_branches {
        push_if_condition(&mut commands, "elif", &branch.condition);
        commands.push(command_with_words(["then"]));
        commands.extend(branch.body.clone());
    }
    if let Some(body) = &if_command.else_body {
        commands.push(command_with_words(["else"]));
        commands.extend(body.clone());
    }
    let mut fi = command_with_words(["fi"]);
    fi.redirect_in = cmd.redirect_in.clone();
    fi.redirect_out = cmd.redirect_out.clone();
    fi.append = cmd.append.clone();
    fi.redirect_err = cmd.redirect_err.clone();
    fi.redirect_err_append = cmd.redirect_err_append.clone();
    fi.heredoc = cmd.heredoc.clone();
    fi.heredoc_delimiter = cmd.heredoc_delimiter.clone();
    fi.heredoc_redirects = cmd.heredoc_redirects.clone();
    fi.here_string = cmd.here_string.clone();
    commands.push(fi);
    commands
}

fn push_if_condition(commands: &mut Vec<CommandNode>, keyword: &str, condition: &[CommandNode]) {
    let Some((first, rest)) = condition.split_first() else {
        commands.push(command_with_words([keyword]));
        return;
    };

    let mut first = first.clone();
    first.words.insert(0, keyword.to_string());
    commands.push(first);
    commands.extend(rest.iter().cloned());
}

fn command_with_words<const N: usize>(words: [&str; N]) -> CommandNode {
    let mut command = CommandNode::new();
    command.words = words.iter().map(|word| (*word).to_string()).collect();
    command
}

struct TimePrefixParts {
    command_index: usize,
    inverted: bool,
}

fn time_prefix_parts(words: &[String]) -> Option<TimePrefixParts> {
    if words.first().map(String::as_str) != Some("time") {
        return None;
    }

    let mut index = 1;
    let mut inverted = false;
    while let Some(word) = words.get(index).map(String::as_str) {
        match word {
            "-p" | "--" => index += 1,
            "!" => {
                inverted = !inverted;
                index += 1;
            }
            _ => break,
        }
    }
    Some(TimePrefixParts {
        command_index: index,
        inverted,
    })
}

fn test_rubash_binary_from_current_exe() -> Option<std::path::PathBuf> {
    let current = std::env::current_exe().ok()?;
    let deps = current.parent()?;
    if deps.file_name().and_then(|name| name.to_str()) != Some("deps") {
        return None;
    }
    let debug_dir = deps.parent()?;
    let binary_name = if cfg!(windows) {
        "rubash.exe"
    } else {
        "rubash"
    };
    let candidate = debug_dir.join(binary_name);
    candidate.is_file().then_some(candidate)
}
