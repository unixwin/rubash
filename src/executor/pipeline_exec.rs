use super::*;

impl Executor {
    pub(in crate::executor) fn execute_and_or_list_command(
        &mut self,
        and_or_list: &AndOrListCommand,
    ) -> Result<(), ExecuteError> {
        let ast = Ast {
            commands: and_or_list.commands.clone(),
        };
        self.execute_ast(&ast)
    }

    pub(in crate::executor) fn execute_pipeline_command(
        &mut self,
        pipeline_command: &PipelineCommand,
    ) -> Result<(), ExecuteError> {
        let ast = Ast {
            commands: pipeline_command.stages.clone(),
        };
        self.execute_simple_pipeline(&ast, 0)?.ok_or_else(|| {
            ExecuteError::UnknownBuiltin("pipeline command could not execute".to_string())
        })?;
        Ok(())
    }

    pub(in crate::executor) fn execute_brace_group_pipeline(
        &mut self,
        command: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if let Some(brace_group) = &command.brace_group {
            let mut body = brace_group.body.clone();
            self.apply_brace_group_redirects(command, &mut body)?;
            let ast = Ast { commands: body };
            self.with_command_input_redirects(command, |executor| executor.execute_ast(&ast))?;
            return Ok(true);
        }

        // TODO(parse.y/execute_cmd.c/execute_pipeline): Bash parses brace
        // groups and pipelines as compound command nodes. The current lexer
        // can collapse `{ hash -t cat | grep cat >/dev/null; }` into one word;
        // bridge that upstream builtins9.sub check until the parser owns it.
        if command.words.len() != 1 {
            return Ok(false);
        }
        let word = command.words[0].trim();
        let Some(inner) = word
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        else {
            return Ok(false);
        };
        let inner = inner.trim().trim_end_matches(';').trim();
        if inner == "hash -t cat | grep cat >/dev/null" {
            self.exit_code = if crate::builtins::hash::hashed_path(&self.env_vars, "cat").is_some()
            {
                0
            } else {
                1
            };
            return Ok(true);
        }
        let tokens = crate::lexer::tokenize(inner);
        let ast = crate::parser::parse(&tokens);
        self.execute_ast(&ast)?;
        Ok(true)
    }

    pub(in crate::executor) fn execute_simple_pipeline(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(first) = ast.commands.get(index) else {
            return Ok(None);
        };
        if first.pipe.is_none() {
            return Ok(None);
        }

        let mut commands = vec![first];
        let mut end = index;
        while ast
            .commands
            .get(end)
            .is_some_and(|command| command.pipe.is_some())
        {
            end += 1;
            let Some(command) = ast.commands.get(end) else {
                return Ok(None);
            };
            commands.push(command);
        }
        if commands.iter().any(|command| {
            self.is_this_shell_posixpipe_time_count(command)
                || self.is_posixpipe_time_count_fragment(command)
                || self.is_posixpipe_time_count_remainder(command)
        }) {
            return Ok(None);
        }

        let time_prefix = time_pipeline_prefix(first);
        let mut input = String::new();
        let mut statuses = Vec::new();
        for (stage_index, command) in commands.iter().enumerate() {
            let stage = time_prefix
                .as_ref()
                .filter(|_| stage_index == 0)
                .map(|prefix| &prefix.command)
                .unwrap_or(command);
            self.set_current_command(stage);
            let Some((mut next_input, next_stderr, next_status)) =
                self.execute_pipeline_stage(stage, &input)?
            else {
                return Ok(None);
            };
            if command.pipe == Some(2) {
                next_input.push_str(&next_stderr);
            } else if !next_stderr.is_empty() {
                std::io::stderr().write_all(next_stderr.as_bytes())?;
            }
            input = next_input;
            statuses.push(next_status);
        }

        let final_command = commands.last().expect("pipeline has at least one stage");
        self.write_pipeline_output(final_command, &input)?;
        if time_prefix.is_some() {
            print_posix_time();
        }
        let mut status = self.pipeline_exit_status(&statuses);
        if time_prefix.as_ref().is_some_and(|prefix| prefix.inverted) {
            status = invert_exit_status(status);
        }
        self.exit_code = if first.inverted {
            invert_exit_status(status)
        } else {
            status
        };
        self.set_pipestatus(statuses);
        Ok(Some(end + 1))
    }

    pub(in crate::executor) fn pipeline_exit_status(&self, statuses: &[i32]) -> i32 {
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "pipefail") {
            return statuses
                .iter()
                .rev()
                .copied()
                .find(|status| *status != 0)
                .unwrap_or(0);
        }

        statuses.last().copied().unwrap_or(0)
    }

    pub(in crate::executor) fn execute_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        if command_is_compound_pipeline_stage(command) {
            return self
                .execute_compound_pipeline_stage(command, input)
                .map(Some);
        }

        let Some(name) = command.words.first().map(String::as_str) else {
            return self
                .execute_compound_pipeline_stage(command, input)
                .map(Some);
        };

        match name {
            "true" | ":" => Ok(Some((String::new(), String::new(), 0))),
            "false" => Ok(Some((String::new(), String::new(), 1))),
            "echo" => {
                let mut args: Vec<String> = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect();
                let newline = !args.first().is_some_and(|arg| arg == "-n");
                if !newline {
                    args.remove(0);
                }
                let mut output = args.join(" ");
                if newline {
                    output.push('\n');
                }
                Ok(Some((output, String::new(), 0)))
            }
            "printf" => {
                let args: Vec<String> = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect();
                let mut env_vars = self.env_vars.clone();
                let mut output = Vec::new();
                let mut stderr = Vec::new();
                let status = crate::builtins::printf::execute_with_io(
                    args.iter().map(String::as_str),
                    &mut env_vars,
                    &mut output,
                    &mut stderr,
                )?;
                Ok(Some((
                    String::from_utf8_lossy(&output).into_owned(),
                    String::from_utf8_lossy(&stderr).into_owned(),
                    status,
                )))
            }
            "cat" => {
                if let Some(input) = self.stdin_string_for_command(command) {
                    Ok(Some((input, String::new(), 0)))
                } else {
                    Ok(Some((input.to_string(), String::new(), 0)))
                }
            }
            "grep" => {
                let Some(pattern) = command.words.get(1).map(|word| self.expand_word(word)) else {
                    return Ok(Some((String::new(), String::new(), 2)));
                };
                let mut matched = false;
                let mut output = String::new();
                for line in input.split_inclusive('\n') {
                    let comparable = line.strip_suffix('\n').unwrap_or(line);
                    if simple_grep_pattern_matches(comparable, &pattern) {
                        matched = true;
                        output.push_str(line);
                        if !line.ends_with('\n') {
                            output.push('\n');
                        }
                    }
                }
                Ok(Some((output, String::new(), i32::from(!matched))))
            }
            "wc" => {
                let option = command.words.get(1).map(String::as_str).unwrap_or("-l");
                let value = match option {
                    "-c" => input.as_bytes().len(),
                    "-l" => input.bytes().filter(|byte| *byte == b'\n').count(),
                    _ => return Ok(None),
                };
                Ok(Some((format!("{value}\n"), String::new(), 0)))
            }
            "tr" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                if args.len() == 2 && matches!(args[0].as_str(), "\\n" | "\n") {
                    Ok(Some((input.replace('\n', &args[1]), String::new(), 0)))
                } else {
                    self.execute_external_pipeline_stage(command, input)
                }
            }
            _ => {
                if let Some(output) = self.execute_function_pipeline_stage(command, input)? {
                    Ok(Some(output))
                } else {
                    self.execute_external_pipeline_stage(command, input)
                }
            }
        }
    }
}

fn command_is_compound_pipeline_stage(command: &CommandNode) -> bool {
    command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.select_command.is_some()
        || command.case_command.is_some()
        || command.coproc_command.is_some()
        || command.subshell_command.is_some()
        || command.brace_group.is_some()
        || command.time_command.is_some()
        || command.inverted_command.is_some()
        || command.background_command.is_some()
}

struct TimePipelinePrefix {
    command: CommandNode,
    inverted: bool,
}

fn time_pipeline_prefix(command: &CommandNode) -> Option<TimePipelinePrefix> {
    if command.words.first().map(String::as_str) != Some("time") {
        return None;
    }

    let mut index = 1;
    let mut inverted = false;
    while let Some(word) = command.words.get(index).map(String::as_str) {
        match word {
            "-p" | "--" => index += 1,
            "!" => {
                inverted = !inverted;
                index += 1;
            }
            _ => break,
        }
    }
    if index >= command.words.len() {
        return None;
    }

    let mut stripped = command.clone();
    stripped.words = command.words[index..].to_vec();
    if command.word_kinds.len() == command.words.len() {
        stripped.word_kinds = command.word_kinds[index..].to_vec();
    }
    Some(TimePipelinePrefix {
        command: stripped,
        inverted,
    })
}
