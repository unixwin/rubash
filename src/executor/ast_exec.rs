use super::*;

impl Executor {
    fn coprocs_referenced_by_command(&self, command: &CommandNode) -> Vec<u32> {
        let mut redirect_sources = command
            .redirect_in
            .iter()
            .chain(command.redirect_out.iter())
            .chain(command.append.iter())
            .chain(command.redirect_err.iter())
            .chain(command.redirect_err_append.iter())
            .chain(command.redirects.iter())
            .map(|redirect| redirect.target.as_str())
            .collect::<Vec<_>>();
        if command.words.first().map(String::as_str) == Some("wait") {
            redirect_sources.extend(command.words.iter().map(String::as_str));
        }
        self.env_vars
            .iter()
            .filter_map(|(key, value)| {
                let name = key.strip_suffix("_PID")?;
                let pid = value.parse::<u32>().ok()?;
                if command.coproc_command.is_some()
                    || redirect_sources
                        .iter()
                        .any(|source| source.contains(name) && source.contains('['))
                {
                    Some(pid)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn execute_ast(&mut self, ast: &Ast) -> Result<(), ExecuteError> {
        if EXECUTION_LOCK_DEPTH.with(|depth| depth.get() > 0) {
            return self.execute_ast_inner(ast);
        }

        let _guard = EXECUTION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original_dir = env::current_dir().ok();
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(1));
        let result = self.execute_ast_inner(ast);
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(0));
        if let Some(original_dir) = original_dir {
            let _ = env::set_current_dir(original_dir);
        }
        result
    }

    pub(in crate::executor) fn execute_ast_inner(&mut self, ast: &Ast) -> Result<(), ExecuteError> {
        if self.try_upstream_scripts() {
            return Ok(());
        }

        let mut index = 0;
        let mut subshell_env: Option<HashMap<String, String>> = None;
        let mut subshell_pipestatus: Option<Vec<i32>> = None;
        let mut subshell_depth: Option<usize> = None;
        let mut subshell_stdin: Option<(String, String)> = None;
        while index < ast.commands.len() {
            let protected_coprocs = self.coprocs_referenced_by_command(&ast.commands[index]);
            self.refresh_background_jobs_with_protected_coprocs(&protected_coprocs)?;
            self.run_pending_signal_traps()?;

            let command = &ast.commands[index];
            self.set_current_line(command);
            if self.noexec_enabled() {
                self.exit_code = 0;
                if command.subshell_end {
                    if let Some(saved_env) = subshell_env.take() {
                        self.restore_shell_env(saved_env);
                    }
                    if let Some(saved_pipestatus) = subshell_pipestatus.take() {
                        self.pipestatus = saved_pipestatus;
                    }
                    if let Some(saved_depth) = subshell_depth.take() {
                        self.subshell_depth.set(saved_depth);
                    }
                }
                index += 1;
                continue;
            }

            // Execute DEBUG trap before each command, mirroring Bash:
            //   - not before function definition commands
            //   - not before if/while/until commands themselves (Bash fires
            //     it for the conditional command inside, via execute_simple)
            //   - not before `for` commands (Bash fires it per iteration,
            //     handled inside execute_for_command)
            //   - not inside function bodies (Bash fires it once at the
            //     function entry, see execute_function_internal)
            //   - suppressed while a trap action is already running
            // A subshell compound command is not itself a DEBUG stop point.
            // Its body is evaluated at the incremented BASH_SUBSHELL depth
            // below; only functrace/extdebug make DEBUG inherit into that body.
            let skips_debug_trap = command.function_command.is_some()
                || command.if_command.is_some()
                || command.loop_command.is_some()
                || command.for_command.is_some()
                || command.subshell_command.is_some()
                // GNU fires the DEBUG trap once per executed simple command,
                // never for the and/or list node itself (execute_cmd.c has no
                // run_debug_trap call site in execute_connection_command);
                // the folded list's members each fire through their own
                // execute_ast below (dbg-support.tests:55/25 `[ $j -eq $n ]
                // && j=i` fires once for `[ ...]`, twice only when `j=i` runs).
                || command.and_or_list.is_some();
            let debug_trap_active = crate::builtins::trap::get_trap_action(&self.env_vars, "DEBUG")
                .is_some_and(|action| !action.is_empty());
            // Do not fire for commands inside the trap action itself: Bash
            // does not re-enter the DEBUG trap while an action runs, and
            // firing would let the action's commands overwrite LINENO with
            // their own (synthetic) line before run_debug_trap's guard sees
            // the flag (dbg-support2.tests `print_trap $LINENO`).
            // The DEBUG trap is inherited by functions only under functrace
            // (execute_cmd.c:5270); extdebug reaches that state solely through
            // the functrace it enables (shopt.def:621), so a later set +T
            // disables inheritance even with extdebug active.
            let debug_trap_in_scope = self.debug_trap_in_scope();
            if !skips_debug_trap
                && debug_trap_in_scope
                && debug_trap_active
                && !self.debug_trap_running
            {
                // Bash exposes the about-to-run command's line via LINENO
                // inside the DEBUG trap action (dbg-support2.tests).
                self.set_current_line(command);
                let command_text = crate::executor::command_text::bash_command_source_text(command);
                if self.run_debug_trap(&command_text)? {
                    index += 1;
                    continue;
                }
            }

            if let Some(next_index) = self.execute_time_prefixed_command_sequence(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_compound_source(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = crate::builtins::source::execute_simple_if(self, ast, index)?
            {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_simple_loop(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) =
                crate::builtins::source::execute_pipe_into_source(self, ast, index)?
            {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_escaped_pipe(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_inversion(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_time(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_function(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_brace_group(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_subshell(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_for(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_select(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_case(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_coproc(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_alias_heredoc(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(next_index) = self.execute_inverted_pipeline(ast, index)? {
                index = next_index;
                continue;
            }

            if let Some(inverted_command) = &command.inverted_command {
                let execution_result = self.execute_inverted_ast_command(inverted_command);
                match execution_result {
                    Ok(()) => {}
                    Err(ExecuteError::CommandNotFound(cmd)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::CommandNotFound(cmd)
                        );
                        self.exit_code = 127;
                    }
                    Err(ExecuteError::UnknownBuiltin(name)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::UnknownBuiltin(name)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExpansionFailure(code)) => {
                        self.exit_code = code;
                    }
                    Err(ExecuteError::IoError(error)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        if self.parse_error_occurred {
                            // Syntax error - terminate script (GNU Bash behavior)
                            return Err(ExecuteError::ExitCode(code));
                        }
                        self.exit_code = code;
                    }
                    Err(error) => return Err(error),
                }
                if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                continue;
            }

            if let Some(time_command) = &command.time_command {
                let execution_result = if command.and_or().is_some() {
                    self.with_errexit_suppressed(|executor| {
                        executor.execute_time_ast_command(time_command)
                    })
                } else {
                    self.execute_time_ast_command(time_command)
                };
                match execution_result {
                    Ok(()) => {}
                    Err(ExecuteError::Break(_) | ExecuteError::Continue(_))
                        if self.loop_depth == 0 =>
                    {
                        self.exit_code = 0;
                    }
                    Err(ExecuteError::CommandNotFound(cmd)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::CommandNotFound(cmd)
                        );
                        self.exit_code = 127;
                    }
                    Err(ExecuteError::UnknownBuiltin(name)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::UnknownBuiltin(name)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExpansionFailure(code)) => {
                        self.exit_code = code;
                    }
                    Err(ExecuteError::IoError(error)) if is_closed_output_io_error(&error) => {
                        return Err(ExecuteError::IoError(error));
                    }
                    Err(ExecuteError::IoError(error)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        if self.parse_error_occurred {
                            // Syntax error - terminate script (GNU Bash behavior)
                            return Err(ExecuteError::ExitCode(code));
                        }
                        self.exit_code = code;
                    }
                    Err(error) => return Err(error),
                }
                if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                continue;
            }

            if let Some(background_command) = &command.background_command {
                let execution_result = self.execute_background_ast_command(background_command);
                match execution_result {
                    Ok(()) => {}
                    Err(ExecuteError::CommandNotFound(cmd)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::CommandNotFound(cmd)
                        );
                        self.exit_code = 127;
                    }
                    Err(ExecuteError::UnknownBuiltin(name)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::UnknownBuiltin(name)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExpansionFailure(code)) => {
                        self.exit_code = code;
                    }
                    Err(ExecuteError::IoError(error)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        if self.parse_error_occurred {
                            // Syntax error - terminate script (GNU Bash behavior)
                            return Err(ExecuteError::ExitCode(code));
                        }
                        self.exit_code = code;
                    }
                    Err(error) => return Err(error),
                }
                index += 1;
                continue;
            }

            if command_is_time_prefixed_compound(command) {
                let execution_result = self.execute_time_prefixed_compound_command(command);
                match execution_result {
                    Ok(()) => {}
                    Err(ExecuteError::Break(_) | ExecuteError::Continue(_))
                        if self.loop_depth == 0 =>
                    {
                        self.exit_code = 0;
                    }
                    Err(ExecuteError::CommandNotFound(cmd)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::CommandNotFound(cmd)
                        );
                        self.exit_code = 127;
                    }
                    Err(ExecuteError::UnknownBuiltin(name)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::UnknownBuiltin(name)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExpansionFailure(code)) => {
                        self.exit_code = code;
                    }
                    Err(ExecuteError::IoError(error)) if is_closed_output_io_error(&error) => {
                        return Err(ExecuteError::IoError(error));
                    }
                    Err(ExecuteError::IoError(error)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        if self.parse_error_occurred {
                            // Syntax error - terminate script (GNU Bash behavior)
                            return Err(ExecuteError::ExitCode(code));
                        }
                        self.exit_code = code;
                    }
                    Err(error) => return Err(error),
                }
                if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                continue;
            }

            if let Some(pipeline_command) = &command.pipeline_command {
                let execution_result = if command.inverted || command.and_or().is_some() {
                    self.with_errexit_suppressed(|executor| {
                        executor.execute_pipeline_command(pipeline_command)
                    })
                } else {
                    self.execute_pipeline_command(pipeline_command)
                };
                match execution_result {
                    Ok(()) => {}
                    Err(ExecuteError::Break(_) | ExecuteError::Continue(_))
                        if self.loop_depth == 0 =>
                    {
                        self.exit_code = 0;
                    }
                    Err(ExecuteError::CommandNotFound(cmd)) => {
                        // Bash treats a command-not-found in a pipeline as
                        // the last stage's exit status 127 and continues.
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::CommandNotFound(cmd)
                        );
                        self.exit_code = 127;
                    }
                    Err(ExecuteError::UnknownBuiltin(name)) => {
                        // Bash treats an unknown-builtin error as the
                        // command's status 1 and continues.
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::UnknownBuiltin(name)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExpansionFailure(code)) => {
                        // Word-expansion failure aborts only the current
                        // command; the surrounding list continues.
                        self.exit_code = code;
                    }
                    Err(ExecuteError::IoError(error)) if is_closed_output_io_error(&error) => {
                        return Err(ExecuteError::IoError(error));
                    }
                    Err(ExecuteError::IoError(error)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        if self.parse_error_occurred {
                            // Syntax error - terminate script (GNU Bash behavior)
                            return Err(ExecuteError::ExitCode(code));
                        }
                        self.exit_code = code;
                    }
                    Err(error) => return Err(error),
                }
                if command.inverted {
                    self.exit_code = invert_exit_status(self.exit_code);
                }
                if self.errexit_enabled()
                    && self.errexit_is_active()
                    && self.suppress_errexit == 0
                    && self.exit_code != 0
                    && !command.inverted
                    && command.and_or().is_none()
                {
                    return Err(ExecuteError::ExitCode(self.exit_code));
                }
                // GNU fires the ERR trap for a failing pipeline just like a
                // failing simple command (trap3.sub: "trap: 8" after
                // false | false | false; trap2.sub "exit 42 | command false").
                self.maybe_run_error_trap(command)?;
                if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                continue;
            }

            if let Some(and_or_list) = &command.and_or_list {
                let execution_result = self.execute_and_or_list_command(and_or_list);
                match execution_result {
                    Ok(()) => {}
                    Err(ExecuteError::Break(_) | ExecuteError::Continue(_))
                        if self.loop_depth == 0 =>
                    {
                        self.exit_code = 0;
                    }
                    Err(ExecuteError::CommandNotFound(cmd)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::CommandNotFound(cmd)
                        );
                        self.exit_code = 127;
                    }
                    Err(ExecuteError::UnknownBuiltin(name)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            ExecuteError::UnknownBuiltin(name)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExpansionFailure(code)) => {
                        self.exit_code = code;
                    }
                    Err(ExecuteError::IoError(error)) if is_closed_output_io_error(&error) => {
                        return Err(ExecuteError::IoError(error));
                    }
                    Err(ExecuteError::IoError(error)) => {
                        eprintln!(
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        self.exit_code = 1;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        if self.parse_error_occurred {
                            // Syntax error - terminate script (GNU Bash behavior)
                            return Err(ExecuteError::ExitCode(code));
                        }
                        self.exit_code = code;
                    }
                    Err(error) => return Err(error),
                }
                index += 1;
                continue;
            }

            let brace_result = self.execute_brace_group_pipeline(command);
            match brace_result {
                Ok(true) => {
                    if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                        index = next_index;
                    } else {
                        index += 1;
                    }
                    continue;
                }
                Ok(false) => {}
                Err(ExecuteError::Break(_) | ExecuteError::Continue(_)) if self.loop_depth == 0 => {
                    self.exit_code = 0;
                }
                Err(ExecuteError::CommandNotFound(cmd)) => {
                    eprintln!(
                        "{}{}",
                        self.diagnostic_prefix(),
                        ExecuteError::CommandNotFound(cmd)
                    );
                    self.exit_code = 127;
                }
                Err(ExecuteError::UnknownBuiltin(name)) => {
                    eprintln!(
                        "{}{}",
                        self.diagnostic_prefix(),
                        ExecuteError::UnknownBuiltin(name)
                    );
                    self.exit_code = 1;
                }
                Err(ExecuteError::ExpansionFailure(code)) => {
                    self.exit_code = code;
                }
                Err(ExecuteError::IoError(error)) if is_closed_output_io_error(&error) => {
                    return Err(ExecuteError::IoError(error));
                }
                Err(ExecuteError::IoError(error)) => {
                    eprintln!(
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(&error)
                    );
                    self.exit_code = 1;
                }
                Err(ExecuteError::ExitCode(code)) => {
                    if self.parse_error_occurred {
                        // Syntax error - terminate script (GNU Bash behavior)
                        return Err(ExecuteError::ExitCode(code));
                    }
                    self.exit_code = code;
                }
                Err(error) => return Err(error),
            }

            let simple_result = self.execute_simple_pipeline(ast, index);
            match simple_result {
                Ok(Some(next_index)) => {
                    index = next_index;
                    continue;
                }
                Ok(None) => {}
                Err(ExecuteError::Break(_) | ExecuteError::Continue(_)) if self.loop_depth == 0 => {
                    self.exit_code = 0;
                }
                Err(ExecuteError::CommandNotFound(cmd)) => {
                    eprintln!(
                        "{}{}",
                        self.diagnostic_prefix(),
                        ExecuteError::CommandNotFound(cmd)
                    );
                    self.exit_code = 127;
                }
                Err(ExecuteError::UnknownBuiltin(name)) => {
                    eprintln!(
                        "{}{}",
                        self.diagnostic_prefix(),
                        ExecuteError::UnknownBuiltin(name)
                    );
                    self.exit_code = 1;
                }
                Err(ExecuteError::ExpansionFailure(code)) => {
                    self.exit_code = code;
                }
                Err(ExecuteError::IoError(error)) if is_closed_output_io_error(&error) => {
                    return Err(ExecuteError::IoError(error));
                }
                Err(ExecuteError::IoError(error)) => {
                    eprintln!(
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(&error)
                    );
                    self.exit_code = 1;
                }
                Err(error) => return Err(error),
            }

            if command.subshell && subshell_env.is_none() {
                subshell_env = Some(self.env_vars.clone());
                crate::builtins::trap::reset_for_subshell(&mut self.env_vars);
                subshell_pipestatus = Some(self.pipestatus.clone());
                let old_depth = self.subshell_depth.get();
                subshell_depth = Some(old_depth);
                self.subshell_depth.set(old_depth + 1);
                // Feed subshell group stdin redirect to all body commands
                let old_fn = self.env_vars.get(FUNCTION_STDIN).cloned();
                let old_fno = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
                subshell_stdin = Some((old_fn.unwrap_or_default(), old_fno.unwrap_or_default()));
                for fwd in index + 1..ast.commands.len() {
                    let c = &ast.commands[fwd];
                    if c.subshell_end {
                        if let Some(input) = self.command_input_redirect(c) {
                            self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
                            self.env_vars
                                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
                        }
                        break;
                    }
                }
            }

            let execution_result = if command.inverted || command.and_or().is_some() {
                self.with_errexit_suppressed(|executor| executor.execute_command(command))
            } else {
                self.execute_command(command)
            };
            match execution_result {
                Ok(()) => {}
                Err(ExecuteError::Break(_) | ExecuteError::Continue(_)) if self.loop_depth == 0 => {
                    self.exit_code = 0;
                }
                // GNU expr.c: a fatal word-expansion error abandons the
                // current command list with status 1. Inside loops, functions
                // and compound conditions the error unwinds to the frame
                // boundary (loop_select/function_calls handle it there); at
                // script top level the remainder of the same logical line is
                // skipped and the next line runs (GNU probe 2026-09-01:
                // `echo $((1/0)); echo same-line` never prints "same-line",
                // the next line does).
                Err(ExecuteError::ExpansionFailure(code))
                    if self.loop_depth == 0
                        && self.function_depth == 0
                        && subshell_env.is_none()
                        && !self.inside_compound_condition.get() =>
                {
                    self.exit_code = code;
                    // GNU 5.2: under `set -e` a fatal word-expansion
                    // failure (bad substitution, failglob, arithmetic)
                    // exits the script like any other failing command
                    // (probe 2026-09-02: `set -e; echo ${#:}; echo after`
                    // stops after the bad substitution; same for
                    // `set -e; echo nope*` with failglob).
                    if self.errexit_enabled()
                        && self.errexit_is_active()
                        && self.suppress_errexit == 0
                        && self.exit_code != 0
                        && !command.inverted
                        && command.and_or().is_none()
                    {
                        return Err(ExecuteError::ExitCode(self.exit_code));
                    }
                    let failed_line = command.line;
                    if failed_line.is_some_and(|line| line != 0) {
                        while let Some(next) = ast.commands.get(index + 1) {
                            if next.line == failed_line && !next.subshell_end {
                                index += 1;
                            } else {
                                break;
                            }
                        }
                    }
                }
                Err(ExecuteError::IoError(error)) if is_closed_output_io_error(&error) => {
                    return Err(ExecuteError::IoError(error));
                }
                Err(ExecuteError::IoError(error)) => {
                    // Bash treats a failed command redirection (and other
                    // command-owned I/O failures) as the command's status 1.
                    // It does not abort the surrounding list unless errexit
                    // is active; propagating the raw I/O error here made a
                    // script stop after `cmd >/missing/path`.
                    eprintln!(
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(&error)
                    );
                    self.exit_code = 1;
                    if self.errexit_enabled()
                        && self.errexit_is_active()
                        && self.suppress_errexit == 0
                        && !command.inverted
                        && command.and_or().is_none()
                    {
                        return Err(ExecuteError::ExitCode(1));
                    }
                }
                // Bash runs a subshell with errexit active; when a command
                // fails inside the subshell the subshell exits with that
                // status but the parent script continues. Catch the error at
                // the subshell boundary instead of propagating it.
                Err(ExecuteError::ExitCode(code)) | Err(ExecuteError::ExpansionFailure(code))
                    if subshell_env.is_some() =>
                {
                    self.exit_code = code;
                    while index + 1 < ast.commands.len() && !ast.commands[index + 1].subshell_end {
                        index += 1;
                    }
                    if index + 1 < ast.commands.len() {
                        index += 1;
                    }
                    if let Some((old_stdin, old_offset)) = subshell_stdin.take() {
                        if old_stdin.is_empty() {
                            self.env_vars.remove(FUNCTION_STDIN);
                            self.env_vars.remove(FUNCTION_STDIN_OFFSET);
                        } else {
                            self.env_vars.insert(FUNCTION_STDIN.to_string(), old_stdin);
                            self.env_vars
                                .insert(FUNCTION_STDIN_OFFSET.to_string(), old_offset);
                        }
                    }
                    if let Some(saved_env) = subshell_env.take() {
                        self.restore_shell_env(saved_env);
                    }
                    if let Some(saved_pipestatus) = subshell_pipestatus.take() {
                        self.pipestatus = saved_pipestatus;
                    }
                    if let Some(saved_depth) = subshell_depth.take() {
                        self.subshell_depth.set(saved_depth);
                    }
                    // A malformed subshell can leave the command list with
                    // no closing marker.  In that case there is no boundary
                    // to advance to; continuing would execute the same
                    // failing command forever (and repeatedly print the
                    // heredoc EOF diagnostic).
                    let has_subshell_end = ast.commands[index + 1..]
                        .iter()
                        .any(|candidate| candidate.subshell_end);
                    if !command.subshell_end && !has_subshell_end {
                        return Err(ExecuteError::ExitCode(code));
                    }
                    // A failing subshell command triggers errexit like any
                    // other failing command: `(exit 17)` under `set -e` exits
                    // the script (set-e1.sub), while `true && (exit 1)` and
                    // `! (exit 1)` contexts keep running.
                    if self.exit_code != 0
                        && crate::builtins::set::shell_option_enabled(&self.env_vars, "errexit")
                        && self.suppress_errexit == 0
                        && !command.inverted
                        && command.and_or().is_none()
                    {
                        return Err(ExecuteError::ExitCode(self.exit_code));
                    }
                    continue;
                }
                Err(error) => return Err(error),
            }
            if command.inverted {
                self.exit_code = invert_exit_status(self.exit_code);
            }
            self.set_pipestatus([self.exit_code]);

            // Execute ERR trap if command failed and not in &&/||/! context
            // (trap_exec.rs maybe_run_error_trap; GNU execute_cmd.c).
            self.maybe_run_error_trap(command)?;

            if command.subshell_end {
                if let Some((old_stdin, old_offset)) = subshell_stdin.take() {
                    if old_stdin.is_empty() {
                        self.env_vars.remove(FUNCTION_STDIN);
                        self.env_vars.remove(FUNCTION_STDIN_OFFSET);
                    } else {
                        self.env_vars.insert(FUNCTION_STDIN.to_string(), old_stdin);
                        self.env_vars
                            .insert(FUNCTION_STDIN_OFFSET.to_string(), old_offset);
                    }
                }
                if let Some(saved_env) = subshell_env.take() {
                    self.restore_shell_env(saved_env);
                }
                if let Some(saved_pipestatus) = subshell_pipestatus.take() {
                    self.pipestatus = saved_pipestatus;
                }
                if let Some(saved_depth) = subshell_depth.take() {
                    self.subshell_depth.set(saved_depth);
                }
            }

            if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                index = next_index;
            } else {
                index += 1;
            }
        }
        self.run_pending_signal_traps()?;
        Ok(())
    }

    pub(in crate::executor) fn execute_inverted_pipeline(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        // TODO(parse.y/execute_cmd.c/execute_pipeline): Bash attaches `!` to a
        // pipeline command node and executes the whole pipeline before status
        // inversion. Rubash still flattens pipelines into simple commands, so
        // cover the small status-only cases used by upstream invert.tests.
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };

        if !command.inverted || command.pipe.is_none() {
            return Ok(None);
        }

        let mut pipeline = vec![command];
        let mut end = index;
        while ast
            .commands
            .get(end)
            .is_some_and(|command| command.pipe.is_some())
        {
            end += 1;
            let Some(next) = ast.commands.get(end) else {
                return Ok(None);
            };
            pipeline.push(next);
        }

        // The parser stores the complete pipeline on the first inverted command.
        // Execute that pipeline through the normal stage executor so output,
        // PIPESTATUS, pipefail, and command failures retain their real semantics.
        if let Some(pipeline_command) = &command.pipeline_command {
            self.with_errexit_suppressed(|executor| {
                executor.execute_pipeline_command(pipeline_command)
            })?;
            self.exit_code = invert_exit_status(self.exit_code);
            return Ok(Some(end + 1));
        }

        // Keep the flattened fallback for parser forms that do not yet expose a
        // PipelineCommand, but do not use it when the real pipeline is available.
        for command in pipeline {
            self.execute_command(command)?;
        }
        self.exit_code = invert_exit_status(self.exit_code);
        Ok(Some(end + 1))
    }
}

fn is_closed_output_io_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe || error.raw_os_error() == Some(232)
}
