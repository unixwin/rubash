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
        if !is_shell_name(&select_command.variable) {
            // GNU execute_cmd.c reports via builtin_error with no command
            // segment: "./errors.tests: line 50: `1': not a valid identifier".
            eprintln!(
                "{}`{}': not a valid identifier",
                self.diagnostic_prefix(),
                select_command.variable
            );
            self.exit_code = if self.posix_mode_enabled() { 2 } else { 1 };
            if self.posix_mode_enabled() {
                return Err(ExecuteError::ExitCode(2));
            }
            return Ok(());
        }

        // GNU execute_cmd.c:3525-3528: the select head is printed
        // (print_select_command_head, print_cmd.c:656 -> `select NAME in
        // WORDS`) and run_debug_trap fires once before the word list is
        // expanded; the implicit `select x; do` form prints the default
        // `"$@"` list, the same as the `for` head.
        if self.debug_trap_in_scope() {
            let words_text = if select_command.default_positional {
                "\"$@\"".to_string()
            } else {
                crate::executor::command_text::command_words_source_text(
                    &select_command.words,
                    &select_command.word_metadata,
                )
            };
            let _ = self.run_debug_trap(&format!(
                "select {} in {}",
                select_command.variable, words_text
            ))?;
        }

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
            self.shell_state.positional_params.clone()
        } else {
            let mut values = Vec::new();
            for (index, word) in select_command.words.iter().enumerate() {
                let raw = select_command
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str());
                let metadata = select_command.word_metadata.get(index);
                match self.expand_for_word_values_result(word, raw, metadata) {
                    Ok(expanded) => values.extend(expanded),
                    Err(pattern) => {
                        self.report_failglob(&pattern);
                        self.finish_compound_output_process_substitutions(group_outputs)?;
                        // failglob is a fatal word-expansion error (GNU):
                        // the select command fails with status 1 and the
                        // next line runs.
                        return Err(ExecuteError::ExpansionFailure(1));
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

        // GNU execute_cmd.c:3513 `line_number = select_command->line`: the
        // `select` keyword's line is the ambient while the body runs.
        let select_line = cmd.line;
        let result = self.with_command_input_redirects(cmd, |executor| {
            executor.with_ambient_line(select_line, |executor| {
                executor.execute_select_loop(&select_command, &values)
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

    fn execute_select_loop(
        &mut self,
        select_command: &SelectCommand,
        values: &[String],
    ) -> Result<(), ExecuteError> {
        let ps3 = self
            .shell_state
            .env_vars
            .get("PS3")
            .cloned()
            .unwrap_or_else(|| "#? ".to_string());
        let has_stdin = self.shell_state.env_vars.contains_key(FUNCTION_STDIN);
        let mut stdin_offset = self.select_stdin_offset(has_stdin);

        loop {
            let mut prompt = Vec::new();
            for (i, value) in values.iter().enumerate() {
                writeln!(&mut prompt, "{}) {}", i + 1, value)?;
            }
            write!(&mut prompt, "{ps3}")?;
            self.write_default_stderr(&prompt)?;

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
            // GNU execute_cmd.c execute_select_command binds the reply
            // through bind_variable: nameref select variables follow full
            // assignment semantics (empty-cell nameref stores the cell,
            // invalid values report `` `x': not a valid identifier `` —
            // nameref11.sub `select r in /` on `declare -n r`).
            if !self.apply_shell_assignment(&select_command.variable, selected.clone()) {
                self.exit_code = 1;
                return Ok(());
            }
            set_process_env(&select_command.variable, selected);

            let body_flow = self.execute_select_body(&select_command.body)?;
            // GNU execute_cmd.c:3606 REAP() (execute_select_command).
            self.reap_dead_jobs_after_loop_body();
            match body_flow {
                SelectBodyFlow::ContinueLoop => continue,
                SelectBodyFlow::BreakLoop => break,
            }
        }

        Ok(())
    }

    fn select_stdin_offset(&self, has_stdin: bool) -> usize {
        if has_stdin {
            self.shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
        } else {
            0
        }
    }

    fn read_select_input(&mut self, has_stdin: bool, stdin_offset: &mut usize) -> Option<String> {
        if !has_stdin {
            // Raw handle read like GNU zread: io::stdin() owns a process-wide
            // BufReader that would prefetch the whole stream and break the
            // shared kernel offset for sibling reads.
            let stdin_handle = crate::fd::process_std_handle(0);
            let mut line: Vec<u8> = Vec::new();
            let result = loop {
                match crate::fd::read_some(stdin_handle, 1) {
                    Ok(buf) if buf.is_empty() => break Ok(0_usize),
                    Ok(buf) => {
                        line.push(buf[0]);
                        if buf[0] == b'\n' {
                            break Ok(1_usize);
                        }
                    }
                    Err(e) => break Err(e),
                }
            };
            return match result {
                Ok(0) => {
                    eprintln!();
                    None
                }
                Ok(_) => Some(
                    crate::executor::bytes_to_shell_text(&line)
                        .trim()
                        .to_string(),
                ),
                Err(_) => {
                    self.exit_code = 1;
                    Some(String::new())
                }
            };
        }

        let stdin_content = self
            .shell_state
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
        self.shell_state
            .env_vars
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
        self.shell_state.loop_depth += 1;
        let result = self.execute_ast(&body);
        self.shell_state.loop_depth -= 1;
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
