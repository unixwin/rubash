use super::*;

enum CoprocStderrForwardTarget {
    Stdout,
    Stderr,
    File(PathBuf),
    Discard,
    CoprocStdin(std::io::PipeWriter),
}

fn forward_coproc_stderr(
    mut stderr: std::process::ChildStderr,
    mut target: CoprocStderrForwardTarget,
) -> Result<(), std::io::Error> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = stderr.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        match &mut target {
            CoprocStderrForwardTarget::Stdout => {
                std::io::stdout().write_all(&buffer[..count])?;
                std::io::stdout().flush()?;
            }
            CoprocStderrForwardTarget::Stderr => {
                std::io::stderr().write_all(&buffer[..count])?;
                std::io::stderr().flush()?;
            }
            CoprocStderrForwardTarget::File(path) => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(&buffer[..count])?;
            }
            CoprocStderrForwardTarget::Discard => {}
            CoprocStderrForwardTarget::CoprocStdin(writer) => {
                writer.write_all(&buffer[..count])?;
                writer.flush()?;
            }
        }
    }
}

impl Executor {
    /// Allocate a coproc pipe fd the way GNU exposes it: the parent keeps
    /// `rpipe[0]` and `wpipe[1]` from `sh_openpipe`, which moves the pipe
    /// ends to the highest free fds below 64 via `move_to_high_fd(maxfd 64)`.
    /// That yields rpipe 63/62 and wpipe 61/60, of which the parent retains
    /// 63 and 60 — the pair `${COPROC[@]}` prints as "63 60" for every
    /// coproc in coproc.tests. `slot` 0 requests the read end (63) and
    /// `slot` 1 the write end (60). Falls back to the low-first allocator
    /// when the preferred fd is already taken.
    fn allocate_coproc_fd(&mut self, slot: usize) -> u32 {
        let want = if slot == 0 { 63u32 } else { 60u32 };
        let free = self.fd_table.entries.get(&want).map_or(true, |e| {
            e.closed || (e.read.is_none() && e.write.is_none())
        });
        if free {
            want
        } else {
            self.fd_table.allocate_dynamic()
        }
    }

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
        let exe = std::env::var_os("CARGO_BIN_EXE_rubash")
            .map(std::path::PathBuf::from)
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());
        let source = self.background_command_source(&background_command.command);
        let display_source = bash_command_source_text(&background_command.command);
        let mut child = Command::new(&exe);
        child.arg("-c").arg(&source);
        child.stdin(Stdio::null());
        for (key, value) in &self.env_vars {
            if !key.starts_with("__RUBASH_") {
                child.env(key, value);
            }
        }
        // POSIX 2.11 (Signals and Error Handling): caught traps reset to
        // their default in a subshell, and GNU's fork+exec background child
        // never reaches the parent's exit-trap path. The child inherits this
        // process's environ (which carries the trap table), so drop those
        // keys or the background child fires the inherited EXIT trap when
        // its command finishes (trap.tests: three stray "exiting" lines
        // around the monitored `sleep 7 & sleep 6 & sleep 5 & / wait`).
        for key in self
            .env_vars
            .keys()
            .filter(|key| key.starts_with("__RUBASH_TRAP"))
        {
            child.env_remove(key);
        }
        child.env("__RUBASH_SHELL_PID", self.shell_pid.to_string());

        let child = child.spawn()?;
        let pid = child.id();
        self.background_children.insert(pid, child);
        self.job_table
            .register_process(pid, display_source.clone(), true);
        self.background_jobs.insert(pid, display_source);
        self.background_job_order.push(pid);
        self.last_background_pid = Some(pid);
        self.exit_code = 0;
        Ok(())
    }

    fn background_command_source(&self, command: &CommandNode) -> String {
        let mut source = String::new();
        for (name, body) in &self.functions {
            if is_exportable_function_name(name) {
                source.push_str(name);
                source.push_str("() { ");
                source.push_str(&bash_command_sequence_text(&body.commands));
                source.push_str("; }; ");
            }
        }
        source.push_str(&bash_command_source_text(command));
        source
    }

    pub(in crate::executor) fn execute_time_ast_command(
        &mut self,
        time_command: &TimeCommand,
    ) -> Result<(), ExecuteError> {
        let started = time_command_started();
        if let Some(coproc_cmd) = &time_command.command.coproc_command {
            self.execute_coproc_command(&time_command.command, coproc_cmd)?;
        } else if time_command.command.words.is_empty()
            && command_has_no_effect(&time_command.command)
        {
            self.exit_code = 0;
        } else if let Some(pipeline_command) = &time_command.command.pipeline_command {
            self.execute_pipeline_command(pipeline_command)?;
        } else if time_command.command.brace_group.is_some() {
            self.execute_brace_group_pipeline(&time_command.command)?;
        } else if time_command.command.words.first().map(String::as_str) == Some("coproc") {
            self.execute_time_reparsed_coproc(&time_command.command)?;
        } else {
            self.execute_command(&time_command.command)?;
        }
        print_time(&self.env_vars, time_command.posix_format, started);
        if time_command.inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        Ok(())
    }

    fn execute_time_reparsed_coproc(&mut self, command: &CommandNode) -> Result<(), ExecuteError> {
        let source = bash_command_source_text(command);
        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        if let Some(coproc_command) = reparsed
            .commands
            .first()
            .and_then(|command| command.coproc_command.as_ref())
        {
            return self.execute_coproc_command(command, coproc_command);
        }

        self.execute_command(command)
    }

    pub(in crate::executor) fn execute_time_prefixed_compound_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let inverted = time_prefix_parts(&cmd.words)
            .map(|parts| parts.inverted)
            .unwrap_or(false);
        let started = time_command_started();
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
        print_time(
            &self.env_vars,
            time_prefix_parts(&cmd.words).is_some_and(|parts| parts.posix_format),
            started,
        );
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
            if command.word_metadata.len() == command.words.len() {
                first.word_metadata = command.word_metadata[prefix.command_index..].to_vec();
            }
        }

        let started = time_command_started();
        let next_index = match timed_ast.commands[0].words.first().map(String::as_str) {
            Some("if") => crate::builtins::source::execute_simple_if(self, &timed_ast, 0)?,
            Some("while" | "until") => self.execute_simple_loop(&timed_ast, 0)?,
            _ => None,
        };
        let Some(next_index) = next_index else {
            return Ok(None);
        };

        print_time(&self.env_vars, prefix.posix_format, started);
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
        let mut arithmetic_failed = false;
        // GNU execute_arith_for_command:3236 sets line_number = arith_lineno
        // = arith_for_command->line, and eval_arith_for_expr:3187 runs the
        // DEBUG trap before each expression evaluation (init once; test and
        // step once per iteration) with that line restored, so $LINENO inside
        // the fire is the for command's line (dbg-support.tests: the double
        // "debug lineno: 108 main" per iteration).
        let for_line = self.env_vars.get("__RUBASH_CURRENT_LINE").cloned();
        let restore_for_line = |executor: &mut Executor| {
            if let Some(line) = &for_line {
                executor
                    .env_vars
                    .insert("__RUBASH_CURRENT_LINE".to_string(), line.clone());
            }
        };
        if self.debug_trap_in_scope() && !arithmetic.init.trim().is_empty() {
            restore_for_line(self);
            self.run_debug_trap(&arithmetic.init)?;
        }
        if !arithmetic.init.trim().is_empty()
            && self
                .eval_arithmetic_command_value(&arithmetic.init)
                .is_none()
        {
            self.report_arithmetic_error_raw_display(&arithmetic.init_metadata.expression);
            self.exit_code = 1;
            arithmetic_failed = true;
        }

        let mut ran_body = false;
        let body_ast = Ast {
            commands: body.to_vec(),
        };
        while !arithmetic_failed {
            if self.debug_trap_in_scope() && !arithmetic.test.trim().is_empty() {
                restore_for_line(self);
                self.run_debug_trap(&arithmetic.test)?;
            }
            if !arithmetic.test.trim().is_empty() {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_TEST);
                match self.eval_arithmetic_command_value(&arithmetic.test) {
                    Some(0) => break,
                    Some(_) => {}
                    None => {
                        self.report_arithmetic_error_raw_display(
                            &arithmetic.test_metadata.expression,
                        );
                        self.exit_code = 1;
                        arithmetic_failed = true;
                        break;
                    }
                }
            }

            ran_body = true;
            self.loop_depth += 1;
            let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_BODY);
            let result = self.execute_ast(&body_ast);
            drop(_t);
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

            if self.debug_trap_in_scope() && !arithmetic.update.trim().is_empty() {
                restore_for_line(self);
                self.run_debug_trap(&arithmetic.update)?;
            }
            if !arithmetic.update.trim().is_empty() {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_UPDATE);
                if self
                    .eval_arithmetic_command_value(&arithmetic.update)
                    .is_none()
                {
                    self.report_arithmetic_error_raw_display(&arithmetic.update_metadata.expression);
                    self.exit_code = 1;
                    arithmetic_failed = true;
                    break;
                }
            }
        }

        if arithmetic_failed {
            self.exit_code = 1;
        } else if !ran_body {
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
        let saved_pipestatus = self.pipestatus.clone();
        let saved_depth = self.subshell_depth.get();
        // The subshell body runs in place on this executor, so the typed
        // variable store and positional parameters must be saved and restored
        // like env_vars. GNU keeps assignments, set --, and IFS changes
        // local to the subshell.
        let saved_variables = self.shell_state.variables.clone();
        let saved_positional_params = self.positional_params.clone();
        crate::builtins::trap::reset_for_subshell(&mut self.env_vars);
        let saved_loop_depth = self.loop_depth;
        self.subshell_depth.set(saved_depth + 1);
        self.loop_depth = 0;

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut body = Ast {
            commands: subshell_command.body.clone(),
        };
        // Numbered redirects are duplicated in parser stdio fields. Keep only
        // true stdio fields in compound preparation; the original redirect
        // list is applied to each body command below.
        let mut stdio_redirect_cmd = redirect_cmd.clone();
        let is_numbered = |redirect: &Redirect| {
            redirect
                .operator
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
        };
        if stdio_redirect_cmd
            .redirect_in
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_in = None;
        }
        if stdio_redirect_cmd
            .redirect_out
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_out = None;
        }
        if stdio_redirect_cmd.append.as_ref().is_some_and(is_numbered) {
            stdio_redirect_cmd.append = None;
        }
        if stdio_redirect_cmd
            .redirect_err
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_err = None;
        }
        if stdio_redirect_cmd
            .redirect_err_append
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_err_append = None;
        }
        self.apply_command_output_redirects(&stdio_redirect_cmd, &mut body)?;
        // Preserve numbered redirects on every body command so its virtual fd
        // state sees the same left-to-right ordering.
        let numbered_redirects = redirect_cmd
            .redirects
            .iter()
            .filter(|redirect| is_numbered(redirect))
            .cloned()
            .collect::<Vec<_>>();
        if !numbered_redirects.is_empty() {
            for command in &mut body.commands {
                command.redirects.splice(0..0, numbered_redirects.clone());
            }
        }
        let result = self.with_command_input_redirects(cmd, |executor| executor.execute_ast(&body));
        // Bash runs a subshell with errexit active; a failing command exits
        // the subshell with that status but the parent script continues.
        // Catch ExitCode errors at the subshell boundary. `return N` inside a
        // subshell likewise only ends the subshell with status N: the forked
        // child longjmps to its own copy of execute_function's return_catch
        // (return.def return_builtin) and exits N, so the function continues
        // with $? = N (func.tests: `( return 5 ); status=$?` prints 5, 5).
        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::ExitCode(code))
            | Err(ExecuteError::ExpansionFailure(code))
            | Err(ExecuteError::FatalFunctionError(code))
            | Err(ExecuteError::Return(code)) => code,
            Err(error) => {
                self.restore_shell_env(saved_env);
                self.shell_state.variables = saved_variables;
                self.set_positional_params(saved_positional_params);
                self.pipestatus = saved_pipestatus;
                self.subshell_depth.set(saved_depth);
                self.loop_depth = saved_loop_depth;
                return Err(error);
            }
        };

        // GNU execute_cmd.c runs the subshell's own EXIT trap (trap.c
        // run_exit_trap) while the subshell state is still live, before the
        // parent environment returns: `( trap "echo T" EXIT; echo body )`
        // prints body then T (niubash#70). reset_for_subshell cleared the
        // inherited traps at entry, so a pending EXIT here can only have
        // been registered by the body itself; a trap action that calls
        // exit N replaces the subshell status (bash exit_shell semantics).
        let status = match self.run_exit_trap_for_status(status) {
            Ok(trap_status) => trap_status,
            Err(error) => {
                self.restore_shell_env(saved_env);
                self.shell_state.variables = saved_variables;
                self.set_positional_params(saved_positional_params);
                self.pipestatus = saved_pipestatus;
                self.subshell_depth.set(saved_depth);
                self.loop_depth = saved_loop_depth;
                return Err(error);
            }
        };

        self.restore_shell_env(saved_env);
        self.shell_state.variables = saved_variables;
        self.set_positional_params(saved_positional_params);
        self.pipestatus = saved_pipestatus;
        self.subshell_depth.set(saved_depth);
        self.loop_depth = saved_loop_depth;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn execute_loop_command(&mut self, loop_command: &LoopCommand) -> Result<(), ExecuteError> {
        let mut ran_body = false;
        let mut last_body_status = 0;
        let condition = Ast {
            commands: loop_command.condition.clone(),
        };
        let body = Ast {
            commands: crate::builtins::source::normalize_inline_compound_commands(
                loop_command.body.clone(),
            ),
        };

        loop {
            self.with_errexit_suppressed(|executor| executor.execute_ast(&condition))?;
            self.run_pending_signal_traps()?;
            let condition_matched = self.exit_code == 0;
            if condition_matched == loop_command.until {
                break;
            }

            ran_body = true;
            self.loop_depth += 1;
            let result = self.execute_ast(&body);
            self.loop_depth -= 1;
            self.run_pending_signal_traps()?;
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
            // A loop's numbered heredoc is still an ordinary unquoted
            // heredoc. Keep its expansion rules identical to the command's
            // stdin heredoc, including parameter and command substitutions.
            let body = self.expand_heredoc_body_mut(&body);
            saved_fd_inputs.push((fd, self.fd_table.entries.get(&fd).cloned()));
            self.fd_table
                .open_input(fd, FdReadEndpoint::text(&body), true);
        }

        let result = f(self);
        for (fd, old_entry) in saved_fd_inputs {
            match old_entry {
                Some(entry) => {
                    self.fd_table.entries.insert(fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&fd);
                }
            }
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
        // GNU Bash 5.2 (probe f4, 2026-08-24): a word-expansion failure in
        // an if/elif condition abandons the whole compound command instead
        // of selecting the else branch.
        let matched = match self.if_condition_matches(&if_command.condition)? {
            Some(matched) => matched,
            None => return Ok(()),
        };
        if matched {
            return self.execute_ast(&Ast {
                commands: crate::builtins::source::normalize_inline_compound_commands(
                    if_command.then_body.clone(),
                ),
            });
        }

        for branch in &if_command.elif_branches {
            let matched = match self.if_condition_matches(&branch.condition)? {
                Some(matched) => matched,
                None => return Ok(()),
            };
            if matched {
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

    /// Returns `None` when a word-expansion failure abandoned the whole
    /// enclosing if command; `Some(true)` when the condition held.
    fn if_condition_matches(
        &mut self,
        condition: &[CommandNode],
    ) -> Result<Option<bool>, ExecuteError> {
        let ast = Ast {
            commands: condition.to_vec(),
        };
        let saved_condition = self.inside_compound_condition.replace(true);
        let result = self.with_errexit_suppressed(|executor| executor.execute_ast(&ast));
        self.inside_compound_condition.set(saved_condition);
        match result {
            Err(ExecuteError::ExpansionFailure(code)) => {
                self.exit_code = code;
                Ok(None)
            }
            other => {
                other?;
                Ok(Some(self.exit_code == 0))
            }
        }
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
            child.arg("-c").arg(&body_text);
            child
        } else if !coproc_cmd.words.is_empty() {
            // Simple command: coproc [NAME] command [args...]
            let mut child = Command::new(&exe);
            if coproc_cmd
                .words
                .first()
                .is_some_and(|word| word.starts_with('-'))
            {
                for word in &coproc_cmd.words {
                    child.arg(word);
                }
            } else {
                child.arg("-c").arg(command_words_source_text(
                    &coproc_cmd.words,
                    &coproc_cmd.word_metadata,
                ));
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
        // Preserve the parent script location for diagnostics emitted by the
        // coprocess shell, while keeping internal executor state isolated.
        if let Some(script) = self.env_vars.get("__RUBASH_SCRIPT_NAME") {
            child.env("__RUBASH_SCRIPT_NAME", script);
            let line = cmd
                .line
                .map(|line| line.to_string())
                .or_else(|| self.env_vars.get("__RUBASH_CURRENT_LINE").cloned())
                .unwrap_or_else(|| "1".to_string());
            child.env("__RUBASH_CURRENT_LINE", line.clone());
            // The child re-parses `-c` source from line 1. Preserve the
            // parent's physical call-site line when assigning child AST lines.
            let line_offset = line.parse::<usize>().unwrap_or(1).saturating_sub(1);
            child.env("__RUBASH_LINE_OFFSET", line_offset.to_string());
        }
        // Coprocess children own a shell-created stdin pipe. Keep builtin
        // readers attached to it and let TERM use native termination when a
        // blocked read cannot consume the shell signal mailbox.
        child.env(INHERIT_PROCESS_STDIN, "1");
        child.env("__RUBASH_COPROC_CHILD", "1");

        // Create pipes for bidirectional communication
        let stdin_result = std::io::pipe();
        let stdout_result = std::io::pipe();

        if let (Ok((stdin_reader, stdin_writer)), Ok((stdout_reader, stdout_writer))) =
            (stdin_result, stdout_result)
        {
            child.stdin(stdin_reader);
            child.stdout(stdout_writer);
            let coproc_stderr_target = self.coproc_stderr_forward_target();
            if coproc_stderr_target.is_some() {
                child.stderr(Stdio::piped());
            } else {
                child.stderr(Stdio::inherit());
            }
            self.apply_coproc_redirects(cmd, &mut child)?;

            match child.spawn() {
                Ok(mut child_proc) => {
                    // Rubash does not expose real coproc file descriptors yet,
                    // but keep the parent ends until after spawn so stdio uses
                    // the correct pipe direction on all hosts.
                    let pid = child_proc.id();
                    if let Some(target) = coproc_stderr_target {
                        if let Some(stderr) = child_proc.stderr.take() {
                            let forwarder =
                                std::thread::spawn(|| forward_coproc_stderr(stderr, target));
                            self.coproc_stderr_forwarders.insert(pid, forwarder);
                        }
                    }
                    self.background_children.insert(pid, child_proc);
                    let job_id =
                        self.job_table
                            .register_process(pid, bash_command_source_text(cmd), true);
                    self.background_jobs
                        .insert(pid, bash_command_source_text(cmd));
                    self.background_job_order.push(pid);
                    self.coproc_stdin_writers.insert(pid, stdin_writer);
                    self.coproc_stdout_readers.insert(pid, stdout_reader);
                    // GNU sh_openpipe moves the pipe ends to the highest free
                    // fds below 64 (move_to_high_fd with maxfd 64): rpipe
                    // 63/62 and wpipe 61/60, of which the parent keeps 63 and
                    // 60. All three coprocs in coproc.tests reuse that same
                    // pair, so `${COPROC[@]}` is literally "63 60".
                    let coproc_read_fd = self.allocate_coproc_fd(0);
                    self.fd_table.open_input(
                        coproc_read_fd,
                        FdReadEndpoint::CoprocStdout(pid),
                        true,
                    );
                    let coproc_write_fd = self.allocate_coproc_fd(1);
                    self.fd_table.open_output(
                        coproc_write_fd,
                        FdWriteEndpoint::CoprocStdin(pid),
                        true,
                    );
                    self.job_table.attach_coproc_endpoint(job_id, pid);
                    // Store the file descriptors in env for COPROC array
                    let stdin_key = format!("__RUBASH_COPROC_STDIN_{}", pid);
                    let stdout_key = format!("__RUBASH_COPROC_STDOUT_{}", pid);
                    self.env_vars.insert(stdin_key, "pipe".to_string());
                    self.env_vars.insert(stdout_key, "pipe".to_string());

                    // Windows has no inherited POSIX fd for this pipe. Expose
                    // two shell-owned virtual descriptors instead; the PID
                    // remains job identity, not the observable fd value.
                    let array_value = format!("({coproc_read_fd} {coproc_write_fd})");
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

    fn coproc_stderr_forward_target(&mut self) -> Option<CoprocStderrForwardTarget> {
        if self.fd_table.is_closed(2) {
            return Some(CoprocStderrForwardTarget::Discard);
        }

        match self.fd_table.write_endpoint(2)? {
            FdWriteEndpoint::Stdout => Some(CoprocStderrForwardTarget::Stdout),
            FdWriteEndpoint::Stderr => Some(CoprocStderrForwardTarget::Stderr),
            FdWriteEndpoint::File(path) => Some(CoprocStderrForwardTarget::File(path)),
            FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                Some(CoprocStderrForwardTarget::File(path))
            }
            FdWriteEndpoint::CoprocStdin(pid) => self
                .coproc_stdin_writers
                .get(&pid)
                .and_then(|writer| writer.try_clone().ok())
                .map(CoprocStderrForwardTarget::CoprocStdin),
        }
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
                    child.stdin(Stdio::from(self.open_input_redirect(&target)?));
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
        let word = tilde_expand::strip_assignment_quote_marker(&word);
        self.abandon_on_arithmetic_expansion_error()?;
        let nocasematch = crate::builtins::shopt::option_enabled(&self.env_vars, "nocasematch");
        let mut fall_through = false;
        let mut matched_any = false;
        let mut index = 0;
        while let Some(clause) = case_command.clauses.get(index) {
            let matched = fall_through
                || clause.pattern_nodes.iter().any(|pattern| {
                    let stripped = self.expand_case_pattern(pattern);
                    if self.arithmetic_fatal_error.get() || self.arithmetic_nounset_error.get() {
                        return false;
                    }
                    if case_pattern_has_extglob(&stripped) {
                        if nocasematch {
                            crate::executor::conditional::extglob_case_pattern_matches_nocase(
                                &stripped, &word,
                            )
                        } else {
                            crate::executor::conditional::extglob_case_pattern_matches(
                                &stripped, &word,
                            )
                        }
                    } else if nocasematch {
                        case_pattern_matches_nocase(&stripped, &word)
                    } else {
                        case_pattern_matches(&stripped, &word)
                    }
                });
            self.abandon_on_arithmetic_expansion_error()?;
            if matched {
                matched_any = true;
                if clause.body.is_empty() {
                    self.exit_code = 0;
                } else {
                    let body = Ast {
                        commands: clause.body.clone(),
                    };
                    self.execute_ast(&body)?;
                }
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

        if !matched_any {
            self.exit_code = 0;
        }
        Ok(())
    }

    fn expand_case_pattern(&mut self, pattern: &crate::parser::CasePattern) -> String {
        const PROTECTED_CASE_PATTERN_BACKSLASH: char = '\x15';

        if !case_pattern_raw_has_quotes(&pattern.raw_text) {
            // Backslashes in a case pattern are escape characters, not quote
            // removal subjects (bash execute_cmd.c: patterns do not undergo
            // quote removal). Protect every `\` (and the legacy `\x18` marker)
            // through expansion and decode_parameter_pattern_quotes, then
            // restore the real backslash so case_pattern_matches can apply its
            // escape semantics (`\]` is a bracket member, `\"` matches `"`).
            let protected = pattern.text.replace(
                |c| c == '\\' || c == '\x18',
                &PROTECTED_CASE_PATTERN_BACKSLASH.to_string(),
            );
            // Use the mutable expander: case pattern expansion is not an
            // isolated sub-expression — `$((x=1))` in a pattern must keep its
            // assignment side effects (bash execute_cmd.c evaluates patterns
            // with the current variable state; case.tests `;&` fall-through).
            let expanded = self.expand_word_mut(&protected);
            let decoded = decode_parameter_pattern_quotes(&expanded);
            let restored = decoded.replace(PROTECTED_CASE_PATTERN_BACKSLASH, "\\");
            return strip_surrounding_quotes(&restored);
        }

        quote_aware_case_pattern(&pattern.raw_text, |word| self.expand_word_mut(word))
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
    posix_format: bool,
}

fn time_prefix_parts(words: &[String]) -> Option<TimePrefixParts> {
    if words.first().map(String::as_str) != Some("time") {
        return None;
    }

    let mut index = 1;
    let mut inverted = false;
    let mut posix_format = false;
    while let Some(word) = words.get(index).map(String::as_str) {
        match word {
            "-p" => {
                posix_format = true;
                index += 1;
            }
            "--" => index += 1,
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
        posix_format,
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

fn case_pattern_has_extglob(pattern: &str) -> bool {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index + 1 < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if matches!(chars[index], '@' | '*' | '+' | '?' | '!') && chars[index + 1] == '(' {
            return true;
        }
        index += 1;
    }
    false
}

fn case_pattern_raw_has_quotes(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '\'' | '"' => return true,
            '$' if matches!(chars.get(index + 1), Some('\'' | '"')) => return true,
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    false
}

fn quote_aware_case_pattern(raw: &str, mut expand_word: impl FnMut(&str) -> String) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')) {
            let quote = chars[index + 1];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 2, quote) {
                let body = chars[index + 2..end].iter().collect::<String>();
                let literal = if quote == '\'' {
                    decode_ansi_c_escapes(&body)
                } else {
                    expand_word(&body)
                };
                output.push_str(&escape_case_pattern_literal(&literal));
                index = end + 1;
                continue;
            }
        }

        if matches!(chars[index], '\'' | '"') {
            let quote = chars[index];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 1, quote) {
                let body = chars[index + 1..end].iter().collect::<String>();
                let literal = if quote == '\'' {
                    body
                } else {
                    expand_word(&body)
                };
                output.push_str(&escape_case_pattern_literal(&literal));
                index = end + 1;
                continue;
            }
        }

        let start = index;
        while index < chars.len()
            && chars[index] != '\''
            && chars[index] != '"'
            && !(chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')))
        {
            if chars[index] == '\\' && index + 1 < chars.len() {
                index += 2;
            } else {
                index += 1;
            }
        }
        let segment = chars[start..index].iter().collect::<String>();
        let expanded = expand_word(&segment);
        output.push_str(&strip_surrounding_quotes(&decode_parameter_pattern_quotes(
            &expanded,
        )));
    }

    output
}

fn quoted_case_pattern_end(chars: &[char], start: usize, quote: char) -> Option<usize> {
    let mut index = start;
    let mut escaped = false;
    while index < chars.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if quote == '"' && chars[index] == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if chars[index] == quote {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn escape_case_pattern_literal(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        if matches!(ch, '*' | '?' | '[' | '\\' | '@' | '!' | '+' | '(') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}
