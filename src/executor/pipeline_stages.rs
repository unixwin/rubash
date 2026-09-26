use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::pipeline_exec::command_is_compound_pipeline_stage;

impl Executor {
    pub(in crate::executor) fn execute_lastpipe_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<(String, String, i32), ExecuteError> {
        let old_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
        let old_stdin_offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .cloned();
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        let saved_stdout_capture = self.stdout_capture.take();
        let saved_stderr_capture = self.stderr_capture.take();
        self.stdout_capture = Some(Vec::new());
        self.stderr_capture = Some(Vec::new());

        // The element's own output redirections stay on the command: the
        // lastpipe element runs in the current shell, and its `>file` /
        // `>/dev/null` must rebind fd 1 for the body the way GNU's
        // do_redirection_internal does inside the element (see
        // execute_compound_pipeline_stage).
        // Direct-stdout builtins in the stage consult the thread-local
        // capture, which belongs to an enclosing capture when this pipeline
        // runs inside one; give the stage its own capture.
        let (thread_captured, result) =
            crate::executor::shell_options::capture_stdout(|| self.execute_command(command));
        let mut output = self.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&thread_captured);
        let stderr = self.stderr_capture.take().unwrap_or_default();
        self.stdout_capture = saved_stdout_capture;
        self.stderr_capture = saved_stderr_capture;
        // In-shell stage: the cursor visible on self is the element's real
        // fd-0 consumption within `input` (execute_cmd.c:2758 lastpipe).
        let consumed = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));
        restore_optional_env_var(&mut self.shell_state.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_stdin_offset,
        );
        // GNU execute_cmd.c:2758: the lastpipe stage runs in the current
        // shell.  `exit N` must therefore exit the current shell, not just
        // set the pipeline's exit status.  Convert ExitCode to LastpipeExit
        // so it propagates past the pipeline-status match in execute_ast_inner.
        match result {
            Ok(()) => {}
            Err(ExecuteError::ExitCode(code)) => {
                self.exit_code = code;
                return Err(ExecuteError::LastpipeExit(code));
            }
            Err(error) => return Err(error),
        }

        Ok((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            self.last_exit_code(),
        ))
    }

    pub(in crate::executor) fn execute_compound_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<(String, String, i32), ExecuteError> {
        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
        // A pipeline member runs in a subshell: caught signal traps reset to
        // the inherited disposition (execute_cmd.c subshell trap reset), so
        // only traps set inside the member run at its exit.
        crate::builtins::trap::reset_for_subshell(&mut subshell.shell_state.env_vars);
        // GNU execute_cmd.c execute_pipeline (2702-2708 left elements,
        // 2722-2723 rightmost) propagates the pipeline command's
        // CMD_IGNORE_RETURN into EVERY element, and a group command pushes
        // it into its inner list (1104-1108). The stage subshell therefore
        // inherits the parent's errexit suppression verbatim: at top level
        // (ignore_return clear) `{ false; echo x; } | cat` dies on `false`,
        // while under `!`/if/while/&&/||/comsub suppression the same group
        // runs to completion (set-e1.sub:40 prints `A 1`).
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        for (name, value) in &command.assignments {
            let (base_name, _) = assignment_name_and_append(name);
            let expanded_value = subshell.expand_assignment_value(name, value);
            subshell
                .shell_state
                .env_vars
                .insert(base_name.to_string(), expanded_value);
        }

        subshell.stdout_capture = Some(Vec::new());
        subshell.stderr_capture = Some(Vec::new());

        // GNU execute_cmd.c execute_pipeline + redir.c do_redirection_internal:
        // inside the element's subshell the pipe is bound to fd 1 first and
        // the element's own redirections run after it — `( inner ) >/dev/null`
        // rebinds fd 1 for the whole body, so an inner `3>&1` duplicates the
        // null device, not the pipe. Keep redirect_out/append on the stage
        // command so the compound-redirect propagation applies them inside.
        // Builtins that write the process stdout directly (notably the exec
        // builtin's printenv/external fallback via io::stdout()) bypass the
        // subshell's stdout_capture field, so their output would leak past a
        // downstream pipe element. Wrap the stage in the same thread-local
        // stdout capture execute_builtin_pipeline_stage uses and merge both
        // capture sources.
        let saved_capture = crate::executor::shell_options::begin_stdout_capture();

        let result = if command.brace_group.is_some() {
            subshell.execute_brace_group_pipeline(command).map(|_| ())
        } else {
            subshell.execute_command(command)
        };
        // GNU execute_cmd.c:1761-1763 (execute_in_subshell): a pipeline
        // element is a forked subshell whose exit — implicit after the body
        // as much as via `exit` — runs the subshell's EXIT trap
        // (subshell_exit -> run_exit_trap; trap4.sub's group without a
        // trailing `exit` still prints its "in trap EXIT"). The trap table
        // was reset at entry (reset_for_subshell above), so only traps the
        // body itself installed can fire; the explicit-`exit` case already
        // consumed the action through the exit builtin, and take_exit_trap
        // leaves nothing for this second run.
        let body_status = subshell.last_exit_code();
        let trap_result = subshell.run_exit_trap_for_status(body_status);
        let mut thread_output = crate::executor::shell_options::take_stdout_capture();
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        let stderr = subshell.stderr_capture.take().unwrap_or_default();

        crate::executor::shell_options::restore_stdout_capture(saved_capture);
        output.append(&mut thread_output);

        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }
        let status = match (result, trap_result) {
            (Ok(()), Ok(trap_status)) => trap_status,
            (Err(ExecuteError::ExitCode(code)), Ok(_)) => code,
            (Err(ExecuteError::ExpansionFailure(code)), Ok(_)) => code,
            (_, Err(error)) | (Err(error), _) => return Err(error),
        };
        // The subshell's FUNCTION_STDIN cursor is the element's fd-0
        // consumption within `input`; report it for the driver writeback.
        let consumed = subshell
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));

        Ok((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            status,
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
        // GNU subst.c process_substitute: a <( ) word in the command's
        // argument list materializes before the command runs
        // (procsub.tests: count_lines <(date) | ... must pass the
        // substitution result as \$1, not the raw <(date) text). The
        // top-level function path does this through
        // command_with_process_substitution_files; pipeline stages skipped
        // it, leaving $1 as the literal <(date) string.
        let (materialized_command, procsub_files) =
            self.command_with_process_substitution_files(command)?;
        let args = materialized_command.words[1..]
            .iter()
            .map(|word| self.expand_word(word))
            .collect::<Vec<_>>();
        let mut call = materialized_command.clone();
        call.words = std::iter::once(function_name.clone())
            .chain(args.iter().cloned())
            .collect();
        call.redirect_out = None;
        call.append = None;

        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
        // Function pipeline stages are compound command bodies for errexit
        // (execute_cmd.c execute_function: the body inherits the element's
        // CMD_IGNORE_RETURN), so the subshell keeps the parent's
        // suppress_errexit like the compound-stage path above.
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        subshell.stdout_capture = Some(Vec::new());
        subshell.stderr_capture = Some(Vec::new());
        // Direct-stdout builtins in the function consult the thread-local
        // capture, which belongs to an enclosing capture when this stage
        // runs inside one; give the call its own capture.
        let (thread_captured, result) = crate::executor::shell_options::capture_stdout(|| {
            subshell.execute_function(&function_name, &args, &call)
        });
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&thread_captured);
        let stderr = subshell.stderr_capture.take().unwrap_or_default();
        let status = subshell.last_exit_code();

        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }
        let status = match result {
            Ok(()) => status,
            Err(ExecuteError::ExitCode(code)) | Err(ExecuteError::ExpansionFailure(code)) => code,
            Err(error) => return Err(error),
        };
        let consumed = subshell
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));
        // Drain output substitutions (none expected for <( ) arguments) and
        // delete the materialized input temp files after the call.
        self.finish_process_substitutions(procsub_files)?;
        Ok(Some((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            status,
        )))
    }

    /// Runs a builtin command inside a subshell for a pipeline stage and
    /// captures its stdout/stderr/status. Bash runs every pipeline element in
    /// a subshell, so special builtins like `set`, `export`, `type`, `trap`
    /// work as pipeline stages without leaking state to the parent.
    pub(in crate::executor) fn execute_builtin_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(None);
        };
        // GNU bash field-splits the expanded first word before command
        // dispatch, so `v="echo hi"; $v | cat` runs the builtin echo in this
        // stage. expand_word alone would classify the joined "echo hi" string
        // as a non-builtin name and fall through to the external stage.
        let first_raw = command
            .word_metadata
            .first()
            .map(|metadata| metadata.raw.as_str());
        let expanded_first = self.expand_command_word(command, 0, name, first_raw);
        let Some(expanded_name) = expanded_first.first() else {
            return Ok(None);
        };
        if !crate::executor::builtin_names::is_shell_builtin_name(expanded_name) {
            return Ok(None);
        }

        let mut call = command.clone();
        call.redirect_out = None;
        call.append = None;

        let saved_capture = crate::executor::shell_options::begin_stdout_capture();

        // Bash applies a pipeline element's redirections before running the
        // command (redir.c do_redirection_internal runs for every command). A
        // failing dup must abort the command with its diagnostic routed through
        // the already-redirected fd2 -- which, after a 2>&1 dup, is this
        // stage's pipe. The stripped clone below never reached that machinery,
        // so "echo foo 2>&1 >&$v | cat" silently ran echo instead of failing
        // with a $v: Bad file descriptor diagnostic down the pipe. Run the same
        // ordered redirect preflight the top-level builtin path uses, while the
        // stdout capture is active so a 2>&1-routed diagnostic lands in this
        // stage's stdout (the pipe) exactly like GNU's.
        if self.command_output_redirect_fails(&call)? {
            let captured = crate::executor::shell_options::take_stdout_capture();
            crate::executor::shell_options::restore_stdout_capture(saved_capture);
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&captured),
                String::new(),
                1,
            )));
        }

        let mut subshell = self.command_substitution_executor();
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        subshell.stderr_capture = Some(Vec::new());
        let result = subshell.execute_command(&call);
        let output = crate::executor::shell_options::take_stdout_capture();
        let stderr = subshell.stderr_capture.take().unwrap_or_default();
        let status = subshell.last_exit_code();

        crate::executor::shell_options::restore_stdout_capture(saved_capture);
        let status = match result {
            Ok(()) => status,
            Err(ExecuteError::ExitCode(code)) | Err(ExecuteError::ExpansionFailure(code)) => code,
            Err(error) => return Err(error),
        };
        let consumed = subshell
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));
        Ok(Some((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            status,
        )))
    }

    pub(in crate::executor) fn execute_external_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let (command, process_substitution_files) =
            self.command_with_process_substitution_files(command)?;
        let result = self.execute_external_pipeline_stage_inner(&command, input, stdin_inherit);
        let finish_result = self.finish_process_substitutions(process_substitution_files);
        let output = result?;
        finish_result?;
        Ok(output)
    }

    fn execute_external_pipeline_stage_inner(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(Some((String::new(), String::new(), 0)));
        };
        if let Some(output) = self.invoke_host_external_command(command) {
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout),
                crate::executor::substitution_metadata::bytes_to_shell_text(&output.stderr),
                output.status,
            )));
        }
        // Field-split the expanded first word like GNU bash: `v="echo -n hi
        // there"; $v | cat` dispatches on the first field "echo" and passes
        // the remaining fields as leading arguments. expand_word alone joined
        // the whole expansion into one command name and reported
        // "echo -n hi there: command not found".
        let first_raw = command
            .word_metadata
            .first()
            .map(|metadata| metadata.raw.as_str());
        let mut first_fields = self
            .expand_command_word(command, 0, name, first_raw)
            .into_iter()
            .map(|word| {
                crate::executor::command_prepare::restore_pathname_escape_markers(
                    &word
                        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
                        .replace(crate::executor::markers::DATA_BACKSLASH, "\\"),
                )
            });
        let expanded_name = first_fields.next().unwrap_or_default();
        let leading_args: Vec<String> = first_fields.collect();
        let Some(program) = find_user_command(&expanded_name, &self.shell_state.env_vars) else {
            let diagnostic = format!(
                "{}{}: command not found\n",
                self.diagnostic_prefix(),
                super::execution_misc::printable_filename(&expanded_name)
            );
            // Bash applies a pipeline element's redirections before the
            // command lookup fails (redir.c do_redirection_internal runs for
            // every command): `2>/dev/null` discards the diagnostic,
            // `2>file` writes it there, and `2>&1` sends it down this
            // stage's pipe (issue #70's `git ... 2>/dev/null | sed` idiom).
            if let Some(redirect) = &command.redirect_err {
                let target = self.expand_redirect_target(redirect);
                if redirect_target_fd(&target) == Some(1) {
                    return Ok(Some((diagnostic, String::new(), 127)));
                }
                if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                    if let Ok(mut file) = self.create_redirect_output(&target, redirect.clobber) {
                        let _ = file.write_all(diagnostic.as_bytes());
                    }
                    return Ok(Some((String::new(), String::new(), 127)));
                }
            } else if let Some(redirect) = &command.redirect_err_append {
                let target = self.expand_redirect_target(redirect);
                if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                    if let Ok(mut file) = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                    {
                        let _ = file.write_all(diagnostic.as_bytes());
                    }
                    return Ok(Some((String::new(), String::new(), 127)));
                }
            }
            return Ok(Some((String::new(), diagnostic, 127)));
        };

        let mut args: Vec<String> = leading_args;
        // GNU execute_simple_command runs pathname expansion on every
        // argument of an external command in a pipeline element, so
        // `ls *` hands ls the directory listing rather than a literal
        // `*`. Without this the pattern reached the host binary verbatim
        // (probe 2026-09-09: `ls * | wc -c` gave 2 bytes instead of 15,
        // while `ls -1 | wc -c` was correct).
        for (offset, word) in command.words[1..].iter().enumerate() {
            let index = offset + 1;
            let raw = command
                .word_metadata
                .get(index)
                .map(|metadata| metadata.raw.as_str());
            for expanded in self.expand_command_word(command, index, word, raw) {
                let value = crate::executor::command_prepare::restore_pathname_escape_markers(
                    &expanded
                        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
                        .replace(crate::executor::markers::DATA_BACKSLASH, "\\"),
                )
                .to_string();
                // \x1d marks a fully quoted word and \x1b a quoted tilde;
                // both stay literal, as command_prepare does.
                if expanded.starts_with(STORAGE_WORD_PREFIX)
                    || expanded.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                {
                    args.push(value);
                    continue;
                }
                // Quoted words (e.g. "*.txt") must not be glob-expanded.
                let metadata = command.word_metadata.get(index);
                let suppress =
                    crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                        raw, metadata,
                    );
                if suppress {
                    args.push(value);
                    continue;
                }
                match glob::pathname_expand_word(&value, &self.shell_state.env_vars) {
                    glob::PathnameExpansion::Matches(matches) => args.extend(matches),
                    glob::PathnameExpansion::NoMatch | glob::PathnameExpansion::Fail(_) => {
                        args.push(value)
                    }
                }
            }
        }
        // GNU execute_cmd.c:6139-6233 (shell_execve): a file the OS cannot
        // exec natively is classified by its first bytes before the
        // shell-script fallback: an unresolvable #! interpreter is refused
        // ("bad interpreter", EX_NOEXEC) and a binary first line is refused
        // ("cannot execute binary file", EX_BINARY_FILE), both with status
        // 126. Plain text falls through to the shell-script execution.
        if crate::executor::path::should_run_with_shell(&program) {
            if let Some((diagnostic, status)) = self.exec_format_refusal(command, &program) {
                return Ok(Some((String::new(), format!("{diagnostic}\n"), status)));
            }
        }

        // The stage's fd 0 is the buffered `input` payload fed through a
        // pipe below (or the live inherited stdin for an unbound stage 0)
        // and fd 1 is captured by the caller — /dev/fd operands
        // materialize against those endpoints, not the fd table.
        let dev_stdin = if stdin_inherit {
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable
        } else {
            crate::executor::dev_fd_operands::DevOperandStdin::Payload(
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(input),
            )
        };
        let (dev_args, dev_ops) = self.materialize_dev_fd_operands(
            &args,
            dev_stdin,
            crate::executor::dev_fd_operands::DevOperandStdout::Capture,
        );
        let (mut process, _) = external_command_for_named_program(
            &program,
            Some(&expanded_name),
            &dev_args,
            &self.shell_state.env_vars,
        );

        self.apply_child_environment(&mut process);
        for (var_name, var_value) in &command.assignments {
            let (base_name, _) = assignment_name_and_append(var_name);
            let expanded_value = self.expand_assignment_value(var_name, var_value);
            if is_valid_process_env(base_name, &expanded_value) {
                process.env(base_name, expanded_value);
            }
        }
        if stdin_inherit {
            process.stdin(Stdio::inherit());
        } else {
            process.stdin(Stdio::piped());
        }
        process.stdout(Stdio::piped());

        // Bash applies a pipeline element's redirections before running the
        // command (redir.c do_redirection_internal). The child's stderr must
        // follow the stage's parsed `2>`/`2>>`/`2>&1` redirect instead of
        // inheriting the shell's stderr, or diagnostics such as ls's
        // "cannot access" leak past `2>/dev/null` (issue #70).
        let mut stderr_merges_into_stdout = false;
        if let Some(redirect) = &command.redirect_err {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target) == Some(1) {
                stderr_merges_into_stdout = true;
                process.stderr(Stdio::piped());
            } else if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                if let Ok(file) = self.create_redirect_output(&target, redirect.clobber) {
                    process.stderr(Stdio::from(file));
                }
            }
        } else if let Some(redirect) = &command.redirect_err_append {
            let target = self.expand_redirect_target(redirect);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                if let Ok(file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                {
                    process.stderr(Stdio::from(file));
                }
            }
        } else if command.pipe == Some(2) {
            // GNU pipe-and-ampersand is "2>&1 |" (redir.c): an external
            // producer's stderr belongs to the pipe payload. Without the
            // explicit pipe the child inherits the shell's stderr, so the
            // invocation.tests option-error producers fed their consumers
            // an empty stream.
            stderr_merges_into_stdout = true;
            process.stderr(Stdio::piped());
        }

        let mut child = process.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(
                &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
            )?;
            // The child was handed the whole payload; model it as consumed
            // (same approximation as comsub_stdin_writeback — a spawned
            // process's read() calls are not observable here).
            self.pipeline_stdin_consumed.set(Some(input.len()));
        }
        let output = child.wait_with_output()?;

        let mut stdout_bytes = output.stdout;
        stdout_bytes.extend(self.finish_dev_fd_operands(dev_ops));
        let mut stderr_bytes = output.stderr;
        if stderr_merges_into_stdout {
            // 2>&1: the stage pipe is fd 1, so the captured stderr belongs
            // to the pipe payload, not to the shell's stderr.
            stdout_bytes.extend_from_slice(&stderr_bytes);
            stderr_bytes.clear();
        }

        Ok(Some((
            crate::executor::substitution_metadata::bytes_to_shell_text(&stdout_bytes),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr_bytes),
            crate::executor::wait_status::process_exit_status(&output.status),
        )))
    }

    pub(in crate::executor) fn write_pipeline_output(
        &mut self,
        command: &CommandNode,
        output: &str,
        already_routed: bool,
    ) -> Result<(), ExecuteError> {
        // Final pipeline output is a byte boundary: decode owner-tagged
        // raw-byte markers exactly once before reaching files, captures,
        // or stdout.
        // Compound stages propagated their own output redirections into the
        // body already (execute_compound_pipeline_stage keeps redirect_out so
        // inner `3>&1` resolves the redirected fd 1), so only what escaped to
        // fd 1 remains here. Simple stages still carry theirs — resolve them
        // like the intermediate-stage routing does, covering `>f`, `1>&2`,
        // `>&-`, and numbered fds the way do_redirection_internal would (the
        // old mirror-only path once wrote `cat 1>&2`'s payload to a literal
        // file named `&2`). Callers that already ran
        // route_pipeline_stage_streams for the stage (the sequential stage
        // loop, and lastpipe/compound stages whose redirections were applied
        // inside) pass already_routed — a second pass would reopen `>f` and
        // truncate what the first pass wrote.
        if !already_routed && !command_is_compound_pipeline_stage(command) {
            let mut stdout = output.to_string();
            let mut stderr = String::new();
            let mut status = 0;
            if self.route_pipeline_stage_streams(command, &mut stdout, &mut stderr, &mut status)? {
                if status != 0 {
                    self.exit_code = status;
                }
                if !stdout.is_empty() {
                    let payload =
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stdout);
                    // One fd-1 stream: the thread-local capture is the
                    // innermost live fd 1 while a stage body runs, so route
                    // through write_default_stdout — writing the field
                    // capture first would reorder against thread-local
                    // writes from spawned children in the same body
                    // (procsub.tests: `x <(date) | cat` emitted the inner
                    // pipeline's wc output before the earlier spawned wc).
                    self.write_default_stdout(&payload)?;
                }
                if !stderr.is_empty() {
                    let payload =
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stderr);
                    self.write_default_stderr(&payload)?;
                }
                return Ok(());
            }
        }
        let payload = crate::executor::substitution_metadata::shell_text_to_raw_bytes(output);
        // Same single-fd-1 ordering: the live thread-local capture (if any)
        // is the stage's fd 1, ahead of self.stdout_capture.
        self.write_default_stdout(&payload)?;
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

        let Some(alias) = self.shell_state.aliases.get(&command.words[0]) else {
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
