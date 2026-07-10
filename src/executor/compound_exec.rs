use super::*;

impl Executor {
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

    pub(in crate::executor) fn execute_coproc_command(
        &mut self,
        _cmd: &CommandNode,
        coproc_cmd: &crate::parser::CoprocCommand,
    ) -> Result<(), ExecuteError> {
        let array_name = coproc_cmd
            .name
            .clone()
            .unwrap_or_else(|| "COPROC".to_string());
        use std::process::{Command, Stdio};
        let exe = std::env::current_exe().unwrap_or_else(|_| "rubash".into());

        let mut child = if let Some(body) = &coproc_cmd.body {
            // Compound command body: coproc [NAME] { body; } or ( body )
            let body_text = body
                .iter()
                .map(|c| c.words.join(" "))
                .collect::<Vec<_>>()
                .join("; ");
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
                        crate::executor::conditional::extglob_case_pattern_matches(&pattern, &word)
                    } else {
                        case_pattern_matches(&pattern, &word)
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
        let mut case_command = case_command.clone();
        self.apply_case_command_redirects(cmd, &mut case_command)?;
        self.execute_case_command(&case_command)
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
