use super::*;

impl Executor {
    pub(in crate::executor) fn set_fd_input_text(&mut self, fd: u32, input: String, dynamic: bool) {
        self.set_fd_input_bytes(fd, input.into_bytes(), dynamic);
    }

    pub(in crate::executor) fn set_fd_input_bytes(
        &mut self,
        fd: u32,
        input: Vec<u8>,
        dynamic: bool,
    ) {
        self.fd_table
            .open_input(fd, FdReadEndpoint::bytes(input.clone()), dynamic);
        self.env_vars.insert(
            fd_stdin_key(fd),
            crate::executor::substitution_metadata::bytes_to_shell_text(&input),
        );
        self.env_vars
            .insert(fd_stdin_offset_key(fd), "0".to_string());
        if dynamic {
            self.env_vars
                .insert(fd_dynamic_input_key(fd), "1".to_string());
        } else {
            self.env_vars.remove(&fd_dynamic_input_key(fd));
        }
        self.env_vars.remove(&fd_closed_key(fd));
    }

    pub(in crate::executor) fn set_fd_output_file(&mut self, fd: u32, target: String, dynamic: bool) {
        let path = shell_path_to_windows(&target, &self.env_vars);
        self.fd_table
            .open_output(fd, FdWriteEndpoint::File(path), dynamic);
        self.env_vars.remove(&fd_closed_key(fd));
        self.env_vars
            .remove(&fd_output_process_substitution_key(fd));
        self.env_vars.insert(fd_output_key(fd), target);
    }

    pub(in crate::executor) fn execute_eval(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        let args = cmd.words[1..].to_vec();
        match crate::builtins::eval::execute_with_io(args.iter().map(String::as_str), &mut stderr)?
        {
            crate::builtins::eval::EvalAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = status;
                Ok(())
            }
            crate::builtins::eval::EvalAction::Execute(source) => {
                if std::env::var("RUBASH_DBG_EVAL").is_ok() {
                    eprintln!("DBG eval words={:?} src={:?}", cmd.words, source);
                }
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                let source = eval_source_for_reparse(&source);
                // GNU parse.y re-reads the eval string as parser input, so
                // alias expansion applies at command position
                // (parse.y alias_expand_token / push_string; comsub21.sub
                // `eval my_alias` inside a substitution body expands here).
                let source = self.comsub_body_alias_splice(&source);
                let mut tokens = crate::lexer::tokenize(&source);
                // GNU eval reports errors with the caller line numbering:
                // the string lines continue the script line counter
                // (posix2.tests: "eval: line 199: syntax error ...").
                let caller_line: usize = self
                    .env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1);
                if caller_line > 1 {
                    for token in tokens.iter_mut() {
                        token.position += caller_line - 1;
                    }
                }
                let mut ast = crate::parser::parse_with_options(
                    &tokens,
                    crate::parser::ParseLoopOptions {
                        stray_close_is_error: true,
                        source_text: Some(source.clone()),
                        source_line_offset: caller_line.saturating_sub(1),
                    },
                );
                self.apply_command_output_redirects(cmd, &mut ast)?;
                // A syntax error inside the eval string makes eval return 2;
                // it does not abort the calling script the way a top-level
                // parse error does. An inner exit still exits the shell, so
                // only the parse-error marker (status 2) becomes a status.
                let has_parse_error = ast
                    .commands
                    .iter()
                    .any(|command| command.has_assignment("__RUBASH_PARSE_ERROR__"));
                let saved_eval_context = self
                    .env_vars
                    .insert("__RUBASH_EVAL_CONTEXT".to_string(), "1".to_string());
                let result = self.execute_ast(&ast);
                match saved_eval_context {
                    Some(previous) => {
                        self.env_vars.insert("__RUBASH_EVAL_CONTEXT".to_string(), previous);
                    }
                    None => {
                        self.env_vars.remove("__RUBASH_EVAL_CONTEXT");
                    }
                }
                match result {
                    Err(ExecuteError::ExitCode(code)) if has_parse_error && code == 2 => {
                        self.exit_code = code;
                        Ok(())
                    }
                    other => other,
                }
            }
        }
    }

    pub fn run_exit_trap(&mut self) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(self.exit_code)
    }

    pub fn run_exit_trap_with_status(&mut self, exit_status: i32) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(exit_status)
    }

    pub(in crate::executor) fn run_exit_trap_for_status(
        &mut self,
        exit_status: i32,
    ) -> Result<i32, ExecuteError> {
        let Some(action) = crate::builtins::trap::take_exit_trap(&mut self.env_vars) else {
            return Ok(exit_status);
        };
        if action.is_empty() {
            return Ok(exit_status);
        }

        self.exit_code = exit_status;
        let tokens = crate::lexer::tokenize(&action);
        let ast = crate::parser::parse(&tokens);
        let saved_trap_command = self.debug_trap_command.borrow().clone();
        let has_command = self.debug_trap_command.borrow().is_none();
        if has_command {
            *self.debug_trap_command.borrow_mut() = self
                .env_vars
                .get("__RUBASH_LAST_COMMAND")
                .or_else(|| self.env_vars.get("__RUBASH_CURRENT_COMMAND"))
                .cloned();
        }
        let result = self.execute_ast(&ast);
        *self.debug_trap_command.borrow_mut() = saved_trap_command;
        match result {
            Ok(()) => {
                self.exit_code = exit_status;
                Ok(exit_status)
            }
            Err(ExecuteError::ExitCode(code)) => {
                self.exit_code = code;
                Ok(code)
            }
            Err(error) => Err(error),
        }
    }

    /// Runs the DEBUG trap action before a command, mirroring Bash's
    /// per-command debug hook. Nested executions of the trap action are
    /// suppressed (Bash does not re-enter the DEBUG trap while an action
    /// is running). `command_text` is the text of the command about to run,
    /// exposed to the trap action through BASH_COMMAND like Bash does.
    pub(crate) fn run_debug_trap(&mut self, command_text: &str) -> Result<bool, ExecuteError> {
        if self.debug_trap_running {
            return Ok(false);
        }
        let Some(action) = crate::builtins::trap::get_trap_action(&self.env_vars, "DEBUG") else {
            return Ok(false);
        };
        if action.is_empty() {
            return Ok(false);
        }
        self.debug_trap_running = true;
        *self.debug_trap_command.borrow_mut() = Some(command_text.to_string());
        let call_line = self
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok());
        let tokens = crate::lexer::tokenize(&action);
        let mut ast = crate::parser::parse(&tokens);
        if let Some(call_line) = call_line {
            for command in &mut ast.commands {
                command.line = Some(call_line);
            }
        }
        let result = self.execute_ast(&ast);
        *self.debug_trap_command.borrow_mut() = None;
        self.debug_trap_running = false;
        result?;
        let skip_command = self.exit_code == 2;
        if skip_command {
            self.exit_code = 0;
        }
        Ok(skip_command)
    }

    /// Runs the RETURN trap action when a function (or sourced script)
    /// returns. Mirrors Bash's `trap ... RETURN` hook used by debuggers.
    pub(crate) fn run_return_trap(&mut self) -> Result<(), ExecuteError> {
        if self.return_trap_running {
            return Ok(());
        }
        let Some(action) = crate::builtins::trap::get_trap_action(&self.env_vars, "RETURN") else {
            return Ok(());
        };
        if action.is_empty() {
            return Ok(());
        }
        self.return_trap_running = true;
        let tokens = crate::lexer::tokenize(&action);
        let mut ast = crate::parser::parse(&tokens);
        // GNU trap.c:_run_trap_internal:1196 only sets SEVAL_RESETLINE for
        // ordinary signal traps; the RETURN trap keeps the caller's current
        // line_number, so $LINENO inside the action (and the DEBUG fire for
        // the trap command itself, execute_simple_command:4506) reports the
        // caller's line. Stamp the parsed action with that line, mirroring
        // run_debug_trap above.
        let call_line = self
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok());
        if let Some(call_line) = call_line {
            for command in &mut ast.commands {
                command.line = Some(call_line);
            }
        }
        let result = self.execute_ast(&ast);
        self.return_trap_running = false;
        result
    }

    /// Whether the DEBUG trap is in execution scope at the current context.
    /// GNU removes the DEBUG trap inside functions that do not inherit it
    /// (execute_cmd.c:5270: no trace attribute and functrace off), so fires
    /// for compound-command sub-parts (for / arith-for expressions) only
    /// happen at the top level or inside inheriting functions.
    /// GNU builtins/source.def:208-216 additionally unsets the DEBUG trap
    /// for the whole duration of a sourced file when functrace is off.
    /// Whether the DEBUG trap is in execution scope at the current context.
    /// Function-level suppression is table-driven: execute_function removes
    /// the inherited DEBUG trap at entry unless the function carries the
    /// trace attribute or functrace is on (execute_cmd.c:5270), and a trap
    /// set inside the body fires like any other command (trap.tests
    /// "func[29] funcdebug"). Subshell environments (command substitutions,
    /// pipeline members) inherit the DEBUG trap only under functrace, which
    /// the cli trap contract encodes: without functrace the
    /// substitution-internal commands do not fire, with functrace they do
    /// (dbg-support.tests caller echoes carry the debug lines).
    pub(crate) fn debug_trap_in_scope(&self) -> bool {
        if self.source_debug_suppressed {
            return false;
        }
        if self.subshell_depth.get() > 0 {
            return crate::builtins::set::shell_option_enabled(&self.env_vars, "functrace");
        }
        true
    }

    /// Whether a function carries the trace attribute (declare -ft name;
    /// trap.def/execute_cmd.c trace_p(var)) making it inherit the DEBUG and
    /// RETURN traps even with the global functrace option off.
    pub(crate) fn function_has_trace_attribute(&self, name: &str) -> bool {
        marked_env_names(&self.env_vars, FUNC_TRACE_FUNCTIONS)
            .iter()
            .any(|entry| entry == name)
    }

    /// Whether the RETURN trap can fire at a sourced-file exit here.
    /// GNU evalfile.c:395 (source_file) runs the RETURN trap unconditionally,
    /// but inside a function that does not inherit it the trap was already
    /// restored to default (execute_cmd.c:5295: `signal_in_progress
    /// (DEBUG_TRAP) || ((trace_p (var) == 0) && function_trace_mode == 0)`),
    /// so nothing fires (dbg-support.tests:96/97: no `return lineno` when
    /// functrace is off, but `return lineno: 98 main` at the top level).
    pub(crate) fn return_trap_in_scope(&self) -> bool {
        if self.function_depth == 0 {
            return true;
        }
        if self.debug_trap_running {
            return false;
        }
        crate::builtins::set::shell_option_enabled(&self.env_vars, "functrace")
    }

    pub(crate) fn set_source_debug_suppressed(&mut self, value: bool) {
        self.source_debug_suppressed = value;
    }

    pub(crate) fn source_debug_suppressed(&self) -> bool {
        self.source_debug_suppressed
    }

    pub(crate) fn run_pending_signal_traps(&mut self) -> Result<(), ExecuteError> {
        if self.signal_trap_running || self.subshell_depth.get() > 0 {
            // Pending signals belong to the shell process. A subshell can target
            // the parent with $$, but must not consume its mailbox or dispatch
            // the parent's traps after resetting caught dispositions.
            return Ok(());
        }

        let signals = crate::builtins::kill::take_pending_signals(std::process::id())?;
        for signal in signals {
            let Some(signal_name) = signal_trap_name(signal) else {
                continue;
            };
            let action = crate::builtins::trap::get_trap_action(&self.env_vars, &signal_name);
            let Some(action) = action else {
                // Bash's default disposition for SIGCHLD is to ignore it.
                // Child completion/reaping notifications must not turn into
                // a synthetic 128+SIGCHLD shell exit when no CHLD trap is
                // installed (busybox ash `reap*.tests`).
                if signal == 20 {
                    continue;
                }
                return Err(ExecuteError::ExitCode(128 + signal));
            };
            if action.is_empty() {
                continue;
            }

            let saved_exit = self.exit_code;
            self.exit_code = saved_exit;
            self.signal_trap_running = true;
            let old_signal_status = self.env_vars.insert(
                "__RUBASH_SIGNAL_TRAP_STATUS".to_string(),
                saved_exit.to_string(),
            );
            // The pre-trap status override for a bare "return" applies only
            // to the return that terminates the trap action itself (posix
            // interp 1602). A "return" in a function CALLED by the action
            // uses the current $?, so record the action's entry
            // function_depth for the return builtin to compare.
            let old_signal_depth = self.env_vars.insert(
                "__RUBASH_SIGNAL_TRAP_DEPTH".to_string(),
                self.function_depth.to_string(),
            );
            // GNU trap.c _run_trap_internal:373-385 binds BASH_TRAPSIG to the
            // signal number of the trap being executed for the duration of
            // the action, restoring the previous value afterwards
            // (save_bash_trapsig/set_bash_trapsig/restore_bash_trapsig).
            let old_bash_trapsig = self.env_vars.get("BASH_TRAPSIG").cloned();
            self.env_vars
                .insert("BASH_TRAPSIG".to_string(), signal.to_string());
            // BASH_TRAPSIG is bound unexported (bind_var_to_int attrs 0), but
            // command substitutions run as child processes that only receive
            // exported variables, so the trap action's $(kill -l
            // $BASH_TRAPSIG) would otherwise see nothing. Export it for the
            // duration of the action; this is invisible to the script.
            let bash_trapsig_was_exported = marked_env_names(&self.env_vars, EXPORTED_VARS)
                .iter()
                .any(|name| name == "BASH_TRAPSIG");
            if !bash_trapsig_was_exported {
                mark_env_name(&mut self.env_vars, EXPORTED_VARS, "BASH_TRAPSIG");
            }
            let tokens = crate::lexer::tokenize(&action);
            let ast = crate::parser::parse(&tokens);
            let result = self.execute_ast(&ast);
            if !bash_trapsig_was_exported {
                unmark_env_name(&mut self.env_vars, EXPORTED_VARS, "BASH_TRAPSIG");
            }
            match old_bash_trapsig {
                Some(value) => {
                    self.env_vars.insert("BASH_TRAPSIG".to_string(), value);
                }
                None => {
                    self.env_vars.remove("BASH_TRAPSIG");
                }
            }
            match old_signal_depth {
                Some(value) => {
                    self.env_vars
                        .insert("__RUBASH_SIGNAL_TRAP_DEPTH".to_string(), value);
                }
                None => {
                    self.env_vars.remove("__RUBASH_SIGNAL_TRAP_DEPTH");
                }
            }
            match old_signal_status {
                Some(value) => {
                    self.env_vars
                        .insert("__RUBASH_SIGNAL_TRAP_STATUS".to_string(), value);
                }
                None => {
                    self.env_vars.remove("__RUBASH_SIGNAL_TRAP_STATUS");
                }
            }
            self.signal_trap_running = false;
            match result {
                Ok(()) => self.exit_code = saved_exit,
                Err(error @ ExecuteError::Return(_)) => return Err(error),
                Err(error @ ExecuteError::ExitCode(_)) => return Err(error),
                Err(error) => return Err(error),
            }
        }

        Ok(())
    }

    /// Run the ERR trap after a command (or pipeline) completes with a
    /// non-zero status. GNU execute_cmd.c fires the error trap for every
    /// failing command that is not part of a &&/|| list, not inverted with
    /// the ! keyword, and not suppressed by errexit handling; functions
    /// inherit it only under "set -o errtrace" (trap.c error_trace_mode).
    pub(crate) fn maybe_run_error_trap(&mut self, command: &CommandNode) -> Result<(), ExecuteError> {
        if self.exit_code == 0
            || command.inverted
            || command.and_or().is_some()
            || self.suppress_errexit != 0
            || (self.function_depth > 0
                && !crate::builtins::set::shell_option_enabled(&self.env_vars, "errtrace"))
        {
            return Ok(());
        }
        let Some(action) = crate::builtins::trap::get_trap_action(&self.env_vars, "ERR") else {
            return Ok(());
        };
        if action.is_empty() {
            return Ok(());
        }
        let saved_exit = self.exit_code;
        let saved_trap_command = self.debug_trap_command.borrow().clone();
        *self.debug_trap_command.borrow_mut() =
            Some(crate::executor::command_text::bash_command_text(command));
        let tokens = crate::lexer::tokenize(&action);
        let ast = crate::parser::parse(&tokens);
        let _ = self.execute_ast(&ast);
        *self.debug_trap_command.borrow_mut() = saved_trap_command;
        self.exit_code = saved_exit;
        Ok(())
    }

    /// Run the SIGCHLD trap once for one reaped child. GNU bash processes
    /// each child-death notification after waitpid and runs a set SIGCHLD
    /// trap at the next boundary (trap8.sub: four CHLD firings for the
    /// reaped background subshell, two background sleeps, and the
    /// foreground sleep).
    pub(crate) fn run_sigchld_trap_for_reaped_child(&mut self) -> Result<(), ExecuteError> {
        if self.signal_trap_running || self.subshell_depth.get() > 0 {
            return Ok(());
        }
        let Some(action) = crate::builtins::trap::get_trap_action(&self.env_vars, "SIGCHLD")
        else {
            return Ok(());
        };
        if action.is_empty() {
            return Ok(());
        }
        let saved_exit = self.exit_code;
        self.signal_trap_running = true;
        let tokens = crate::lexer::tokenize(&action);
        let ast = crate::parser::parse(&tokens);
        let result = self.execute_ast(&ast);
        self.signal_trap_running = false;
        match result {
            Ok(()) => self.exit_code = saved_exit,
            Err(error) => return Err(error),
        }
        Ok(())
    }

    pub(crate) fn run_function_return_trap(&mut self) -> Result<(), ExecuteError> {
        // GNU execute_cmd.c:5295 (execute_function): while the DEBUG trap is
        // executing (signal_in_progress (DEBUG_TRAP)), a function does not
        // inherit the RETURN trap at all (restore_default_signal(RETURN_TRAP)
        // plus the unwind-protect restore), so no RETURN trap fires when the
        // DEBUG trap handler function itself returns
        // (dbg-support.tests: no "return lineno" lines for print_debug_trap).
        if self.debug_trap_running {
            return Ok(());
        }
        // Bash only fires a RETURN trap for function returns when the
        // function inherits the DEBUG/RETURN traps, which tracks the
        // functrace flag alone (execute_cmd.c:5295 checks
        // function_trace_mode, not debugging_mode; shopt extdebug reaches it
        // only through the functrace it enables, shopt.def:621).
        let traced = crate::builtins::set::shell_option_enabled(&self.env_vars, "functrace");
        let function_scoped = self
            .env_vars
            .get("__RUBASH_RETURN_TRAP_FUNCTION")
            .zip(self.function_name_stack.first())
            .is_some_and(|(registered, current)| registered == current);
        if !traced && !function_scoped {
            return Ok(());
        }
        self.run_return_trap()
    }

    pub(in crate::executor) fn note_return_trap_scope(&mut self, args: &[String]) {
        let mut index = usize::from(args.first().map(String::as_str) == Some("--"));
        let Some(action) = args.get(index) else {
            return;
        };
        if action != "-" && action.starts_with('-') {
            return;
        }
        index += 1;
        let has_return = args[index..].iter().any(|signal| {
            signal
                .strip_prefix("SIG")
                .unwrap_or(signal)
                .eq_ignore_ascii_case("RETURN")
        });
        if !has_return {
            return;
        }
        if action == "-" {
            self.env_vars.remove("__RUBASH_RETURN_TRAP_FUNCTION");
        } else if let Some(function) = self.function_name_stack.first() {
            self.env_vars.insert(
                "__RUBASH_RETURN_TRAP_FUNCTION".to_string(),
                function.clone(),
            );
        }
    }

    pub(crate) fn apply_command_output_redirects(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: ">>".to_string(),
                operator_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    ">>".to_string(),
                    ">>".to_string(),
                )),
                kind: crate::parser::RedirectKind::Append,
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: redirect.operator.clone(),
                operator_metadata: redirect.operator_metadata.clone(),
                kind: redirect.kind.clone(),
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target)
                && redirect_target_fd(&target).is_none()
                && !is_null_device(&target)
            {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: "2>>".to_string(),
                operator_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    "2>>".to_string(),
                    "2>>".to_string(),
                )),
                kind: crate::parser::RedirectKind::Append,
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: redirect.operator.clone(),
                operator_metadata: redirect.operator_metadata.clone(),
                kind: redirect.kind.clone(),
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_exec(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(status) = self.execute_dynamic_fd_exec_redirect(cmd)? {
            return Ok(status);
        }

        if exec_has_only_redirects(cmd) {
            if let Some(status) = self.execute_stdio_only_exec_redirect(cmd)? {
                return Ok(status);
            }
        }

        // Bash applies redirections before `exec` parses its options.  This
        // matters when an expanded option is invalid: the diagnostic is still
        // emitted, but a stdout redirection remains in effect afterwards.
        if self.exec_has_no_command_operand_after_expansion(cmd) {
            self.execute_stdio_only_exec_redirect(cmd)?;
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status = crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
                &mut stdout,
                &mut stderr,
            )?;
            // GNU exec.def reports option errors through builtin_error,
            // which carries the script:line context prefix
            // (redir.tests:53: "./redir.tests: line 53: exec: -1: invalid
            // option"). The builtin emits the bare shell-name form; swap
            // the prefix when running inside a script.
            let prefix = self.diagnostic_prefix();
            if prefix != "rubash: " {
                let text = String::from_utf8_lossy(&stderr).into_owned();
                if text.starts_with("rubash: ") {
                    stderr.clear();
                    stderr.extend_from_slice(
                        format!("{}{}", prefix, &text["rubash: ".len()..]).as_bytes(),
                    );
                }
            }
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            // GNU redir.c:832-838: `exec cmd >&WORD` with a non-numeric
            // WORD translates to r_err_and_out - the child's stdout AND
            // stderr go to WORD. The raw target keeps the `&` dup marker,
            // so strip it before opening (redir4.sub: exec >&$fd).
            if redirect.fd.is_none() && target.starts_with('&') {
                let path = target.strip_prefix('&').unwrap_or(&target).to_string();
                let mut file = self.create_redirect_output(&path, redirect.clobber)?;
                // r_err_and_out: the child's stdout and stderr both go to
                // WORD, so the builtin's own diagnostics follow the file too.
                let child_stdout = Stdio::from(file.try_clone()?);
                let child_stderr = Stdio::from(file.try_clone()?);
                let mut child_diag = file.try_clone()?;
                return Ok(crate::builtins::exec::execute_with_child_stdio(
                    &cmd.words[1..],
                    &self.env_vars,
                    &mut file,
                    &mut child_diag,
                    child_stdout,
                    child_stderr,
                )?);
            }
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            let child_stdout = Stdio::from(file.try_clone()?);
            return Ok(crate::builtins::exec::execute_with_child_stdio(
                &cmd.words[1..],
                &self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
                child_stdout,
                Stdio::inherit(),
            )?);
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            let child_stdout = Stdio::from(file.try_clone()?);
            return Ok(crate::builtins::exec::execute_with_child_stdio(
                &cmd.words[1..],
                &self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
                child_stdout,
                Stdio::inherit(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            let child_stderr = Stdio::from(file.try_clone()?);
            return Ok(crate::builtins::exec::execute_with_child_stdio(
                &cmd.words[1..],
                &self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
                Stdio::inherit(),
                child_stderr,
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            let child_stderr = Stdio::from(file.try_clone()?);
            return Ok(crate::builtins::exec::execute_with_child_stdio(
                &cmd.words[1..],
                &self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
                Stdio::inherit(),
                child_stderr,
            )?);
        }

        self.apply_no_output_builtin_redirects(cmd)?;
        Ok(crate::builtins::exec::execute(
            &cmd.words[1..],
            &self.env_vars,
        )?)
    }

    fn execute_stdio_only_exec_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<i32>, ExecuteError> {
        // GNU redir.c: `exec` with only redirections applies every
        // redirection left to right (do_redirection_internal over the whole
        // redirect list, redir.c:767+), keeping each descriptor open
        // persistently and undoing nothing (RX_ACTIVE without undo). The
        // parser mirrors every stdio shortcut field into cmd.redirects, so
        // iterating that list in source order covers `exec >file 2>&1`,
        // `exec 1>&3 2>&4` (redir4.sub:54), `exec 4>&1 >&3 3>&-`
        // (redir7.sub:23), `exec 4>&- 5>&-` (redir.tests:88) and
        // single-redirect forms alike.
        let mut handled = false;
        for redirect in &cmd.redirects {
            let target = self.expand_word(&redirect.target);
            // Mark the redirect as handled before dispatching: every arm
            // below ends in a `continue`, which would otherwise bypass a
            // bottom-of-loop flag and make `exec >&file` fall through to
            // the legacy redirect_out shortcut with the raw `&word` target
            // (GNU redir.c do_redirection_internal applies the whole list;
            // exec with only redirections always ends with status 0 unless
            // a redirection itself fails).
            handled = true;
            match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput
                | crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    let fd = redirect.fd.unwrap_or(1);
                    if is_closed_redirect_target(&target) {
                        if let Some(name) = redirect.fd_var.as_deref() {
                            self.close_dynamic_fd(name)?;
                        } else {
                            self.close_persistent_output_fd(fd)?;
                            self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                        }
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        self.copy_persistent_output_fd(fd, source_fd);
                        if move_source {
                            self.close_persistent_output_fd(source_fd)?;
                        }
                        continue;
                    }
                    // GNU redir.c:832-838: exec >&WORD with a non-numeric
                    // WORD and redirector 1 translates to r_err_and_out -
                    // both stdout and stderr go to WORD
                    // (redir4.sub: exec >&${TMPDIR}/err-and-out).
                    if redirect.fd.unwrap_or(1) == 1
                        && redirect.fd_var.is_none()
                        && target.starts_with('&')
                        && redirect_target_fd(&target).is_none()
                    {
                        let path = target.strip_prefix('&').unwrap_or(&target).to_string();
                        if !is_null_device(&path) {
                            self.create_redirect_output(&path, redirect.clobber)?;
                        }
                        self.set_fd_output_file(1, path.clone(), false);
                        self.set_fd_output_file(2, path, false);
                        continue;
                    }
                    if self.open_persistent_output_process_substitution(fd, &target)? {
                        continue;
                    }
                    if !is_null_device(&target) {
                        if matches!(
                            redirect.kind,
                            crate::parser::RedirectKind::Append
                                | crate::parser::RedirectKind::CombinedAppend
                        ) {
                            OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(shell_path_to_windows(&target, &self.env_vars))?;
                        } else {
                            self.create_redirect_output(&target, redirect.clobber)?;
                        }
                    }
                    self.set_fd_output_file(fd, target.clone(), fd >= 10);
                    if matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::CombinedOutput
                            | crate::parser::RedirectKind::CombinedAppend
                    ) {
                        // &>file / &>>file: stderr follows stdout
                        // (redir.c r_err_and_out / r_append_err_and_out).
                        self.set_fd_output_file(2, target, fd >= 10);
                    }
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    let fd = redirect.fd.unwrap_or(1);
                    if is_closed_redirect_target(&target) {
                        self.close_persistent_output_fd(fd)?;
                        self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        self.copy_persistent_output_fd(fd, source_fd);
                        if move_source {
                            self.close_persistent_output_fd(source_fd)?;
                        }
                        continue;
                    }
                    if redirect.fd.unwrap_or(1) == 1
                        && redirect.fd_var.is_none()
                        && target.starts_with('&')
                    {
                        // GNU redir.c:832-838 r_duplicating_output_word
                        // translation (see above).
                        let path = target.strip_prefix('&').unwrap_or(&target).to_string();
                        if !is_null_device(&path) {
                            self.create_redirect_output(&path, redirect.clobber)?;
                        }
                        self.set_fd_output_file(1, path.clone(), false);
                        self.set_fd_output_file(2, path, false);
                        continue;
                    }
                    // Other non-numeric dup targets stay AMBIGUOUS_REDIRECT
                    // (redir.c:839-843); reject_ambiguous_redirects already
                    // reported them before exec ran.
                }
                crate::parser::RedirectKind::CloseOutput => {
                    let fd = redirect.fd.unwrap_or(1);
                    self.close_persistent_output_fd(fd)?;
                    self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                }
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite => {
                    let fd = redirect.fd.unwrap_or(0);
                    if is_closed_redirect_target(&target) {
                        self.close_persistent_input_fd(fd);
                        self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        self.copy_persistent_input_fd(fd, source_fd);
                        if move_source {
                            self.close_persistent_input_fd(source_fd);
                        }
                        continue;
                    }
                    if let Some(source_fd) = redirect_target_fd(&target) {
                        self.copy_persistent_input_fd(fd, source_fd);
                        continue;
                    }

                    if let Some(source) = target
                        .strip_prefix("<(")
                        .and_then(|target| target.strip_suffix(')'))
                    {
                        if let Some(input) = self.process_substitution_output(source) {
                            self.fd_table.open_input(
                                fd,
                                FdReadEndpoint::process_substitution(&input),
                                fd != 0,
                            );
                            self.set_fd_input_text(fd, input, fd != 0);
                            continue;
                        }
                    }

                    if matches!(
                        target.as_str(),
                        "/dev/stdin" | "/proc/self/fd/0" | "/dev/fd/0"
                    ) {
                        self.fd_table
                            .open_input(fd, FdReadEndpoint::InheritedProcessStdin, fd != 0);
                        continue;
                    }

                    let path = shell_path_to_windows(&target, &self.env_vars);
                    if redirect.append {
                        // [N]<> opens the file for reading and writing
                        // (redir.c r_input_output, O_RDWR).
                        let _ = OpenOptions::new()
                            .create(true)
                            .read(true)
                            .write(true)
                            .open(&path)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    }
                    let input = std::fs::read(&path)
                        .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    self.set_fd_input_bytes(fd, input, fd != 0);
                    // The operator token keeps any numeric redirector prefix
                    // (`exec 6<>file` lexes as `6<>`), so match on the
                    // suffix (redir.tests: `exec 6<>$TMPDIR/bash-c` then
                    // `echo to c 1>&6`).
                    if redirect.operator.ends_with("<>") {
                        self.fd_table
                            .open_output(fd, FdWriteEndpoint::File(path), fd >= 10);
                    }
                }
                crate::parser::RedirectKind::CloseInput => {
                    let fd = redirect.fd.unwrap_or(0);
                    self.close_persistent_input_fd(fd);
                    self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                }
                crate::parser::RedirectKind::DuplicateInput => {
                    let fd = redirect.fd.unwrap_or(0);
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        if self.fd_table.is_open_for_read(source_fd) {
                            self.copy_persistent_input_fd(fd, source_fd);
                            if move_source {
                                self.close_persistent_input_fd(source_fd);
                            }
                        }
                    }
                }
                _ => {}
            }
            handled = true;
        }

        if let Some((fd, input)) = self.exec_heredoc_fd_input(cmd) {
            self.set_fd_input_text(fd, input, fd != 0);
            return Ok(Some(0));
        }

        if handled {
            // GNU applys the redirections and keeps `exec` itself as a
            // no-op with status 0 (redir.c / builtins/exec.def: `exec`
            // with only redirections).
            return Ok(Some(0));
        }
        Ok(None)
    }

    fn exec_heredoc_fd_input(&self, cmd: &CommandNode) -> Option<(u32, String)> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd.is_some() && redirect.body.is_some())?;
        let fd = redirect.fd?;
        let body = redirect.body.as_deref()?;
        let input = if let Some(word) = body.strip_prefix('\x1d') {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            input
        } else {
            strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
        };
        Some((fd, input))
    }

    fn copy_persistent_output_fd(&mut self, target_fd: u32, source_fd: u32) {
        if self.fd_table.is_open_for_write(source_fd) {
            let source_endpoint = self.fd_table.write_endpoint(source_fd);
            let target_endpoint = self.fd_table.write_endpoint(target_fd);
            if target_fd != source_fd
                && target_endpoint.as_ref().is_some_and(|endpoint| {
                    matches!(endpoint, FdWriteEndpoint::ProcessSubstitution { .. })
                })
                && target_endpoint != source_endpoint
            {
                let _ = self.close_persistent_output_fd(target_fd);
            }
            if self.fd_table.dup_output(target_fd, source_fd).is_ok() {
                match self.fd_table.output_endpoint(target_fd) {
                    Some(FdWriteEndpoint::Stdout) => {
                        self.env_vars.remove(&fd_closed_key(target_fd));
                        self.env_vars
                            .insert(fd_output_key(target_fd), FD_STDOUT_TARGET.to_string());
                        self.env_vars
                            .remove(&fd_output_process_substitution_key(target_fd));
                    }
                    Some(FdWriteEndpoint::Stderr) => {
                        self.env_vars.remove(&fd_closed_key(target_fd));
                        self.env_vars
                            .insert(fd_output_key(target_fd), FD_STDERR_TARGET.to_string());
                        self.env_vars
                            .remove(&fd_output_process_substitution_key(target_fd));
                    }
                    Some(FdWriteEndpoint::File(path)) => {
                        self.env_vars.remove(&fd_closed_key(target_fd));
                        self.env_vars.insert(
                            fd_output_key(target_fd),
                            shell_display_path(&path.to_string_lossy()),
                        );
                        self.env_vars
                            .remove(&fd_output_process_substitution_key(target_fd));
                    }
                    Some(FdWriteEndpoint::CoprocStdin(pid)) => {
                        self.env_vars.remove(&fd_closed_key(target_fd));
                        self.env_vars.insert(
                            fd_output_key(target_fd),
                            format!("{FD_COPROC_STDIN_TARGET_PREFIX}{pid}"),
                        );
                        self.env_vars
                            .remove(&fd_output_process_substitution_key(target_fd));
                    }
                    Some(FdWriteEndpoint::ProcessSubstitution { path, command }) => {
                        self.env_vars.remove(&fd_closed_key(target_fd));
                        self.env_vars.insert(
                            fd_output_key(target_fd),
                            shell_display_path(&path.to_string_lossy()),
                        );
                        self.env_vars
                            .insert(fd_output_process_substitution_key(target_fd), command);
                    }
                    None => {}
                }
            } else {
                let _ = self.close_persistent_output_fd(target_fd);
                self.env_vars
                    .insert(fd_closed_key(target_fd), "1".to_string());
            }
            return;
        }
        if self.env_vars.contains_key(&fd_closed_key(source_fd)) {
            let _ = self.close_persistent_output_fd(target_fd);
            self.env_vars
                .insert(fd_closed_key(target_fd), "1".to_string());
        } else if self.coproc_stdin_writers.contains_key(&source_fd) {
            self.env_vars.remove(&fd_closed_key(target_fd));
            self.env_vars.insert(
                fd_output_key(target_fd),
                format!("{FD_COPROC_STDIN_TARGET_PREFIX}{source_fd}"),
            );
            self.env_vars
                .remove(&fd_output_process_substitution_key(target_fd));
        } else if let Some(target) = self.env_vars.get(&fd_output_key(source_fd)).cloned() {
            self.env_vars.remove(&fd_closed_key(target_fd));
            self.env_vars.insert(fd_output_key(target_fd), target);
            if let Some(source) = self
                .env_vars
                .get(&fd_output_process_substitution_key(source_fd))
                .cloned()
            {
                self.env_vars
                    .insert(fd_output_process_substitution_key(target_fd), source);
            } else {
                self.env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
        } else if let Some(target) = stdio_output_target(source_fd) {
            self.env_vars.remove(&fd_closed_key(target_fd));
            self.env_vars
                .insert(fd_output_key(target_fd), target.to_string());
            self.env_vars
                .remove(&fd_output_process_substitution_key(target_fd));
        } else {
            let _ = self.close_persistent_output_fd(target_fd);
            self.env_vars.remove(&fd_closed_key(target_fd));
        }
    }

    fn open_persistent_output_process_substitution(
        &mut self,
        fd: u32,
        target: &str,
    ) -> Result<bool, ExecuteError> {
        let Some(source) = target
            .strip_prefix(">(")
            .and_then(|target| target.strip_suffix(')'))
        else {
            return Ok(false);
        };

        let path = self.empty_process_substitution_temp()?;
        self.fd_table.open_output(
            fd,
            FdWriteEndpoint::ProcessSubstitution {
                path: path.clone(),
                command: source.to_string(),
            },
            fd >= 10,
        );
        self.env_vars.remove(&fd_closed_key(fd));
        self.env_vars
            .insert(fd_output_key(fd), path.to_string_lossy().into_owned());
        self.env_vars
            .insert(fd_output_process_substitution_key(fd), source.to_string());
        Ok(true)
    }

    fn close_persistent_output_fd(&mut self, fd: u32) -> Result<(), ExecuteError> {
        let coproc_pid = self
            .fd_table
            .entries
            .get(&fd)
            .filter(|entry| !entry.closed)
            .and_then(|entry| match entry.write.as_ref() {
                Some(FdWriteEndpoint::CoprocStdin(pid)) => Some(*pid),
                _ => None,
            });
        self.fd_table.close_output(fd);
        if let Some(pid) = coproc_pid {
            self.mark_coproc_array_endpoint_closed(pid, fd);
            let has_alias = self.fd_table.entries.values().any(|entry| {
                !entry.closed
                    && matches!(
                        entry.write.as_ref(),
                        Some(FdWriteEndpoint::CoprocStdin(alias_pid))
                            if *alias_pid == pid
                    )
            });
            if !has_alias {
                self.coproc_stdin_writers.remove(&pid);
            }
        }
        let target = self.env_vars.remove(&fd_output_key(fd));
        let source = self
            .env_vars
            .remove(&fd_output_process_substitution_key(fd));
        if let (Some(target), Some(source)) = (target, source) {
            let path = shell_path_to_windows(&target, &self.env_vars);
            let input = fs::read_to_string(&path).unwrap_or_default();
            self.execute_persistent_output_process_substitution(&source, input)?;
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    fn close_persistent_input_fd(&mut self, fd: u32) {
        let coproc_pid = self
            .fd_table
            .read_endpoint(fd)
            .and_then(|endpoint| match endpoint {
                FdReadEndpoint::CoprocStdout(pid) => Some(pid),
                _ => None,
            });
        self.fd_table.close_input(fd);
        if let Some(pid) = coproc_pid {
            self.mark_coproc_array_endpoint_closed(pid, fd);
            let has_alias = self.fd_table.entries.values().any(|entry| {
                !entry.closed
                    && matches!(
                        entry.read.as_ref(),
                        Some(FdReadEndpoint::CoprocStdout(alias_pid))
                            if *alias_pid == pid
                    )
            });
            if !has_alias {
                self.coproc_stdout_readers.remove(&pid);
            }
        }
        self.env_vars.remove(&fd_stdin_key(fd));
        self.env_vars.remove(&fd_stdin_offset_key(fd));
        self.env_vars.remove(&fd_dynamic_input_key(fd));
    }

    fn mark_coproc_array_endpoint_closed(&mut self, pid: u32, fd: u32) {
        let names: Vec<String> = self
            .env_vars
            .iter()
            .filter_map(|(name, value)| {
                (name.ends_with("_PID") && value.parse::<u32>().ok() == Some(pid))
                    .then(|| name.trim_end_matches("_PID").to_string())
            })
            .collect();
        for name in names {
            let Some(storage) = self.env_vars.get(&name).cloned() else { continue };
            let mut entries = indexed_array_entries(&storage);
            let mut changed = false;
            for value in entries.values_mut() {
                if value == &fd.to_string() {
                    *value = "-1".to_string();
                    changed = true;
                }
            }
            if changed {
                self.env_vars.insert(name, format_indexed_array_storage(entries));
            }
        }
    }

    fn close_persistent_fd(&mut self, fd: u32) -> Result<(), ExecuteError> {
        self.close_persistent_output_fd(fd)?;
        self.close_persistent_input_fd(fd);
        self.fd_table.close(fd);
        self.env_vars.insert(fd_closed_key(fd), "1".to_string());
        Ok(())
    }

    pub(in crate::executor) fn execute_persistent_output_process_substitution(
        &mut self,
        source: &str,
        input: String,
    ) -> Result<(), ExecuteError> {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        let old_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        let old_fd0 = self.fd_table.entries.get(&0).cloned();
        let fd0_key = fd_stdin_key(0);
        let fd0_offset_key = fd_stdin_offset_key(0);
        let fd0_dynamic_key = fd_dynamic_input_key(0);
        let fd0_closed_key = fd_closed_key(0);
        let old_fd0_stdin = self.env_vars.get(&fd0_key).cloned();
        let old_fd0_offset = self.env_vars.get(&fd0_offset_key).cloned();
        let old_fd0_dynamic = self.env_vars.get(&fd0_dynamic_key).cloned();
        let old_fd0_closed = self.env_vars.get(&fd0_closed_key).cloned();
        self.set_fd_input_text(0, input.clone(), false);
        self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
        self.env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        let result = self.execute_ast(&ast);
        match old_fd0 {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }
        restore_optional_env_var(&mut self.env_vars, &fd0_key, old_fd0_stdin);
        restore_optional_env_var(&mut self.env_vars, &fd0_offset_key, old_fd0_offset);
        restore_optional_env_var(&mut self.env_vars, &fd0_dynamic_key, old_fd0_dynamic);
        restore_optional_env_var(&mut self.env_vars, &fd0_closed_key, old_fd0_closed);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN_OFFSET, old_offset);
        result
    }

    fn execute_dynamic_fd_exec_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<i32>, ExecuteError> {
        let Some(name) = cmd.words.get(1).and_then(|word| dynamic_fd_var_name(word)) else {
            return Ok(None);
        };
        if cmd.words.len() != 2 {
            return Ok(None);
        }

        let closes_existing_fd = cmd
            .redirect_in
            .as_ref()
            .or(cmd.redirect_out.as_ref())
            .or(cmd.append.as_ref())
            .is_some_and(|redirect| is_closed_redirect_target(&self.expand_word(&redirect.target)));
        // GNU sets up the redirection itself (creating output targets, opening
        // input files) before failing the fd assignment to a readonly variable.
        let readonly_blocked = !closes_existing_fd && self.dynamic_fd_assignment_readonly(name);

        if cmd.here_string.is_some() || cmd.heredoc.is_some() {
            if readonly_blocked {
                self.report_readonly_fd_assignment(name);
                return Ok(Some(1));
            }
            let Some(input) = self.stdin_string_for_command_mut(cmd) else {
                return Ok(None);
            };
            let fd = self.allocate_dynamic_fd();
            self.set_dynamic_fd_variable(name, fd);
            self.set_fd_input_text(fd, input, true);
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.redirect_in {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                self.close_dynamic_fd(name)?;
                return Ok(Some(0));
            }

            if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                let fd = self.allocate_dynamic_fd();
                self.copy_persistent_input_fd(fd, source_fd);
                if readonly_blocked {
                    self.report_readonly_fd_assignment(name);
                    return Ok(Some(1));
                }
                self.set_dynamic_fd_variable(name, fd);
                if move_source {
                    self.close_persistent_fd(source_fd)?;
                }
                return Ok(Some(0));
            }

            if let Some(source) = target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                if let Some(input) = self.process_substitution_output(source) {
                    let fd = self.allocate_dynamic_fd();
                    self.fd_table.open_input(
                        fd,
                        FdReadEndpoint::process_substitution(&input),
                        true,
                    );
                    self.set_fd_input_text(fd, input, true);
                    if readonly_blocked {
                        self.report_readonly_fd_assignment(name);
                        return Ok(Some(1));
                    }
                    self.set_dynamic_fd_variable(name, fd);
                    return Ok(Some(0));
                }
            }

            if matches!(
                target.as_str(),
                "/dev/stdin" | "/proc/self/fd/0" | "/dev/fd/0"
            ) {
                let fd = self.allocate_dynamic_fd();
                self.fd_table
                    .open_input(fd, FdReadEndpoint::InheritedProcessStdin, true);
                if readonly_blocked {
                    self.report_readonly_fd_assignment(name);
                    return Ok(Some(1));
                }
                self.set_dynamic_fd_variable(name, fd);
                return Ok(Some(0));
            }

            let path = shell_path_to_windows(&target, &self.env_vars);
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path)?;
            }
            let input = crate::executor::substitution_metadata::read_shell_input_file(path)
                .map_err(|io| crate::posix_errors::path_error(&target, io))?;
            let fd = self.allocate_dynamic_fd();
            if readonly_blocked {
                self.report_readonly_fd_assignment(name);
                return Ok(Some(1));
            }
            self.set_dynamic_fd_variable(name, fd);
            self.set_fd_input_text(fd, input, true);
            // Same `6<>` suffix rule as the exec path above.
            if redirect.operator.ends_with("<>") {
                self.set_fd_output_file(fd, target, true);
            }
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                if self.dynamic_fd_variable_value(name).is_none()
                    && crate::builtins::set::shell_option_enabled(&self.env_vars, "nounset")
                {
                    eprintln!("{}{name}: ambiguous redirect", self.diagnostic_prefix());
                    return Ok(Some(1));
                }
                self.close_dynamic_output_fd(name)?;
                return Ok(Some(0));
            }

            let fd = self.allocate_dynamic_fd();
            if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                self.copy_persistent_output_fd(fd, source_fd);
                if readonly_blocked {
                    self.report_readonly_fd_assignment(name);
                    return Ok(Some(1));
                }
                self.set_dynamic_fd_variable(name, fd);
                if move_source {
                    self.close_persistent_fd(source_fd)?;
                }
                return Ok(Some(0));
            }
            if self.open_persistent_output_process_substitution(fd, &target)? {
                if readonly_blocked {
                    self.report_readonly_fd_assignment(name);
                    return Ok(Some(1));
                }
                self.set_dynamic_fd_variable(name, fd);
                return Ok(Some(0));
            }
            self.create_redirect_output(&target, redirect.clobber)?;
            if readonly_blocked {
                self.report_readonly_fd_assignment(name);
                return Ok(Some(1));
            }
            self.set_dynamic_fd_variable(name, fd);
            self.set_fd_output_file(fd, target, true);
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                self.close_dynamic_output_fd(name)?;
                return Ok(Some(0));
            }
            let fd = self.allocate_dynamic_fd();
            if self.open_persistent_output_process_substitution(fd, &target)? {
                if readonly_blocked {
                    self.report_readonly_fd_assignment(name);
                    return Ok(Some(1));
                }
                self.set_dynamic_fd_variable(name, fd);
                return Ok(Some(0));
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            if readonly_blocked {
                self.report_readonly_fd_assignment(name);
                return Ok(Some(1));
            }
            self.set_dynamic_fd_variable(name, fd);
            self.set_fd_output_file(fd, target, true);
            return Ok(Some(0));
        }

        Ok(None)
    }

    pub(in crate::executor) fn execute_dynamic_fd_var_redirect(
        &mut self,
        redirect: &Redirect,
        auto_close: bool,
    ) -> Result<bool, ExecuteError> {
        let Some(name) = redirect.fd_var.as_deref() else {
            return Ok(false);
        };
        let target = self.expand_word(&redirect.target);
        let is_close = matches!(
            redirect.kind,
            crate::parser::RedirectKind::CloseInput | crate::parser::RedirectKind::CloseOutput
        );
        if !is_close && self.dynamic_fd_assignment_readonly(name) {
            // GNU sets up the redirection (creating output targets, opening
            // input files) before failing the assignment, and the command
            // itself never runs.
            match redirect.kind {
                crate::parser::RedirectKind::Output => {
                    self.create_redirect_output(&target, redirect.clobber)?;
                }
                crate::parser::RedirectKind::Append => {
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.env_vars))?;
                }
                crate::parser::RedirectKind::ReadWrite => {
                    OpenOptions::new()
                        .create(true)
                        .read(true)
                        .write(true)
                        .open(shell_path_to_windows(&target, &self.env_vars))?;
                }
                crate::parser::RedirectKind::Input => {
                    let _ = crate::executor::substitution_metadata::read_shell_input_file(
                        shell_path_to_windows(&target, &self.env_vars),
                    )
                    .map_err(|io| crate::posix_errors::path_error(&target, io))?;
                }
                _ => {}
            }
            let prefix = self.diagnostic_prefix();
            let payload = format!(
                "{name}: readonly variable\n{prefix}{name}: cannot assign fd to variable"
            );
            return Err(ExecuteError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                payload,
            )));
        }
        let close_after_command =
            auto_close && crate::builtins::shopt::option_enabled(&self.env_vars, "varredir_close");

        let close_after_success = |executor: &mut Self, fd: u32| -> Result<(), ExecuteError> {
            if close_after_command {
                executor.close_persistent_fd(fd)?;
            }
            Ok(())
        };

        match redirect.kind {
            crate::parser::RedirectKind::CloseInput => {
                self.close_dynamic_input_fd(name);
                return Ok(true);
            }
            crate::parser::RedirectKind::CloseOutput => {
                self.close_dynamic_output_fd(name)?;
                return Ok(true);
            }
            crate::parser::RedirectKind::DuplicateInput => {
                if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                    if !self.fd_table.is_open_for_read(source_fd) {
                        return Ok(true);
                    }
                    let fd = self.allocate_dynamic_fd();
                    self.copy_persistent_input_fd(fd, source_fd);
                    self.set_dynamic_fd_variable(name, fd);
                    if move_source {
                        self.close_persistent_input_fd(source_fd);
                    }
                    close_after_success(self, fd)?;
                    return Ok(true);
                }
            }
            crate::parser::RedirectKind::DuplicateOutput => {
                if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                    if !self.fd_table.is_open_for_write(source_fd) {
                        return Ok(true);
                    }
                    let fd = self.allocate_dynamic_fd();
                    self.copy_persistent_output_fd(fd, source_fd);
                    self.set_dynamic_fd_variable(name, fd);
                    if move_source {
                        self.close_persistent_output_fd(source_fd)?;
                    }
                    close_after_success(self, fd)?;
                    return Ok(true);
                }
            }
            crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite => {
                if let Some(source) = target
                    .strip_prefix("<(")
                    .and_then(|target| target.strip_suffix(')'))
                {
                    if let Some(input) = self.process_substitution_output(source) {
                        let fd = self.allocate_dynamic_fd();
                        self.fd_table.open_input(
                            fd,
                            FdReadEndpoint::process_substitution(&input),
                            true,
                        );
                        self.set_fd_input_text(fd, input, true);
                        if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                            self.set_fd_output_file(fd, target.clone(), true);
                        }
                        self.set_dynamic_fd_variable(name, fd);
                        close_after_success(self, fd)?;
                        return Ok(true);
                    }
                }

                let path = shell_path_to_windows(&target, &self.env_vars);
                let input = if is_null_device(&target) {
                    String::new()
                } else {
                    crate::executor::substitution_metadata::read_shell_input_file(&path)?
                };
                let fd = self.allocate_dynamic_fd();
                self.set_fd_input_text(fd, input, true);
                if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                    self.set_fd_output_file(fd, target.clone(), true);
                }
                self.set_dynamic_fd_variable(name, fd);
                close_after_success(self, fd)?;
                return Ok(true);
            }
            crate::parser::RedirectKind::Output
            | crate::parser::RedirectKind::Append
            | crate::parser::RedirectKind::ClobberOutput => {
                let fd = self.allocate_dynamic_fd();
                if let Some(source) = target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                {
                    if self.open_persistent_output_process_substitution(fd, &target)? {
                        self.set_dynamic_fd_variable(name, fd);
                        close_after_success(self, fd)?;
                        return Ok(true);
                    }
                    let _ = source;
                }
                if redirect.kind == crate::parser::RedirectKind::Append {
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.env_vars))?;
                } else {
                    self.create_redirect_output(&target, redirect.clobber)?;
                }
                self.set_fd_output_file(fd, target, true);
                self.set_dynamic_fd_variable(name, fd);
                close_after_success(self, fd)?;
                return Ok(true);
            }
            _ => {}
        }

        Ok(true)
    }

    fn copy_persistent_input_fd(&mut self, target_fd: u32, source_fd: u32) {
        if self.fd_table.is_open_for_read(source_fd) {
            if self.fd_table.dup_input(target_fd, source_fd).is_ok() {
                match self
                    .fd_table
                    .entries
                    .get(&target_fd)
                    .and_then(|entry| entry.read.clone())
                {
                    Some(FdReadEndpoint::InheritedProcessStdin) => {
                        self.env_vars
                            .insert(fd_stdin_key(target_fd), FD_PROCESS_STDIN_TARGET.to_string());
                        self.env_vars.remove(&fd_stdin_offset_key(target_fd));
                        self.env_vars.remove(&fd_dynamic_input_key(target_fd));
                    }
                    Some(FdReadEndpoint::Text(_))
                    | Some(FdReadEndpoint::ProcessSubstitution(_)) => {
                        if let Some((input, offset)) = self.fd_table.input_snapshot(target_fd) {
                            self.env_vars.insert(fd_stdin_key(target_fd), input);
                            self.env_vars
                                .insert(fd_stdin_offset_key(target_fd), offset.to_string());
                            self.env_vars
                                .insert(fd_dynamic_input_key(target_fd), "1".to_string());
                        }
                    }
                    Some(FdReadEndpoint::File(path)) => {
                        self.env_vars.insert(
                            fd_stdin_key(target_fd),
                            shell_display_path(&path.to_string_lossy()),
                        );
                        self.env_vars.remove(&fd_stdin_offset_key(target_fd));
                        self.env_vars.remove(&fd_dynamic_input_key(target_fd));
                    }
                    Some(FdReadEndpoint::CoprocStdout(pid)) => {
                        self.env_vars.insert(
                            fd_stdin_key(target_fd),
                            format!("{FD_COPROC_STDIN_TARGET_PREFIX}{pid}"),
                        );
                        self.env_vars.remove(&fd_stdin_offset_key(target_fd));
                        self.env_vars.remove(&fd_dynamic_input_key(target_fd));
                    }
                    None => {}
                }
                self.env_vars.remove(&fd_closed_key(target_fd));
            } else {
                self.close_persistent_input_fd(target_fd);
                self.env_vars
                    .insert(fd_closed_key(target_fd), "1".to_string());
            }
            return;
        }
        // A source that is absent from FdTable is closed. Do not resurrect
        // shell input from the legacy environment mirror.
        self.close_persistent_input_fd(target_fd);
    }

    fn allocate_dynamic_fd(&mut self) -> u32 {
        self.fd_table.allocate_dynamic()
    }

    pub(in crate::executor) fn close_dynamic_fd(&mut self, name: &str) -> Result<(), ExecuteError> {
        if let Some(fd) = self.dynamic_fd_variable_value(name) {
            self.close_persistent_fd(fd)?;
        }
        Ok(())
    }

    fn close_dynamic_input_fd(&mut self, name: &str) {
        if let Some(fd) = self.dynamic_fd_variable_value(name) {
            self.close_persistent_input_fd(fd);
            if !self.fd_table.is_open_for_write(fd) {
                self.fd_table.close(fd);
                self.env_vars.insert(fd_closed_key(fd), "1".to_string());
            }
        }
    }

    fn close_dynamic_output_fd(&mut self, name: &str) -> Result<(), ExecuteError> {
        let Some(fd) = self.dynamic_fd_variable_value(name) else {
            return Ok(());
        };

        // A dynamic fd opened with `<>` is one shell descriptor.  Closing its
        // output side with `>&-` must release the descriptor completely;
        // otherwise its still-live input side prevents Bash's lowest-free fd
        // allocation from reusing the slot. Coprocess endpoints are modeled
        // as one-sided entries, so they retain the capability-specific path.
        if self.fd_table.read_endpoint(fd).is_some() {
            self.close_persistent_fd(fd)?;
            return Ok(());
        }

        // Coprocess input/output endpoints currently share the child PID as
        // their virtual descriptor. Close only the output capability so
        // `exec {COPROC[1]}>&-` does not invalidate `COPROC[0]` as well.
        self.close_persistent_output_fd(fd)?;
        if !self.fd_table.is_open_for_read(fd) {
            self.fd_table.close(fd);
            self.env_vars.insert(fd_closed_key(fd), "1".to_string());
        }
        Ok(())
    }

    fn dynamic_fd_variable_value(&self, name: &str) -> Option<u32> {
        if let Some((array_name, index)) = parse_array_numeric_subscript(name) {
            return self
                .array_element_parameter_value(&format!("{array_name}[{index}]"))
                .and_then(|value| value.parse::<u32>().ok());
        }

        let storage_name = self.resolved_variable_name(name)?;
        self.env_vars
            .get(&storage_name)
            .and_then(|value| value.parse::<u32>().ok())
    }

    fn dynamic_fd_assignment_readonly(&self, name: &str) -> bool {
        let base_name = parse_array_numeric_subscript(name)
            .map(|(array_name, _)| array_name)
            .unwrap_or(name);
        let resolved = self
            .resolved_variable_name(base_name)
            .unwrap_or_else(|| base_name.to_string());
        is_marked_var(&self.env_vars, READONLY_VARS, &resolved)
    }

    fn report_readonly_fd_assignment(&mut self, name: &str) {
        eprintln!("{}{}: readonly variable", self.diagnostic_prefix(), name);
        eprintln!(
            "{}{}: cannot assign fd to variable",
            self.diagnostic_prefix(),
            name
        );
        self.exit_code = 1;
    }

    fn set_dynamic_fd_variable(&mut self, name: &str, fd: u32) {
        if let Some((array_name, index)) = parse_array_numeric_subscript(name) {
            let storage_name = self
                .resolved_variable_name(array_name)
                .unwrap_or_else(|| array_name.to_string());
            let current = self
                .env_vars
                .get(&storage_name)
                .cloned()
                .unwrap_or_default();
            let mut entries = indexed_array_entries(&current);
            entries.insert(index, fd.to_string());
            self.env_vars
                .insert(storage_name.clone(), format_indexed_array_storage(entries));
            mark_env_name(&mut self.env_vars, ARRAY_VARS, &storage_name);
        } else {
            let storage_name = self
                .resolved_variable_name(name)
                .unwrap_or_else(|| name.to_string());
            self.env_vars.insert(storage_name, fd.to_string());
        }
    }

    pub(in crate::executor) fn execute_exec_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let status = self.execute_exec(cmd)?;
        let dynamic_fd_redirect = is_dynamic_fd_exec_redirect(cmd);
        self.exit_code = status;
        if !dynamic_fd_redirect && crate::builtins::exec::replaces_shell(&cmd.words[1..]) {
            return Err(ExecuteError::ExitCode(status));
        }
        Ok(())
    }

    fn exec_has_no_command_operand_after_expansion(&self, cmd: &CommandNode) -> bool {
        if cmd.words.len() <= 1 {
            return false;
        }

        let expanded_args: Vec<String> = cmd.words[1..]
            .iter()
            .map(|word| self.expand_word(word))
            .collect();
        !crate::builtins::exec::replaces_shell(&expanded_args)
            && (cmd.redirect_out.is_some()
                || cmd.append.is_some()
                || cmd.redirect_err.is_some()
                || cmd.redirect_err_append.is_some()
                || cmd.redirect_in.is_some())
    }
}

fn signal_trap_name(signal: i32) -> Option<String> {
    if signal <= 0 {
        return None;
    }
    crate::builtins::kill::translate_signal(&signal.to_string()).map(|name| format!("SIG{name}"))
}

fn is_dynamic_fd_exec_redirect(cmd: &CommandNode) -> bool {
    cmd.words.len() == 2
        && cmd
            .words
            .get(1)
            .and_then(|word| dynamic_fd_var_name(word))
            .is_some()
        && (cmd.redirect_in.is_some()
            || cmd.redirect_out.is_some()
            || cmd.append.is_some()
            || cmd.here_string.is_some()
            || cmd.heredoc.is_some())
}

fn exec_has_only_redirects(cmd: &CommandNode) -> bool {
    if cmd.words.len() == 1 {
        return true;
    }

    matches!(
        cmd.words.as_slice(),
        [command, fd_word]
            if command == "exec"
                && fd_word.chars().all(|ch| ch.is_ascii_digit())
                && cmd
                    .redirects
                    .iter()
                    .any(|redirect| redirect.fd.is_some_and(|fd| fd.to_string() == *fd_word))
    )
}

fn dynamic_fd_var_name(word: &str) -> Option<&str> {
    let name = word.strip_prefix('{')?.strip_suffix('}')?;
    if let Some((array_name, index)) = parse_array_subscript(name) {
        if is_shell_name(array_name) && index.parse::<usize>().is_ok() {
            return Some(name);
        }
    }
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    chars
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        .then_some(name)
}
