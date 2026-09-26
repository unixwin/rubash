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
        self.shell_state.env_vars
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

        // A fresh reader-level run: an evalerror abort still pending from a
        // finished run is stale and must not discard commands here.
        self.evalerror_pending.set(false);
        self.evalerror_line.set(None);
        self.reader_command_line.set(None);

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
        super::exec_profile::print_summary();
        result
    }

    pub(in crate::executor) fn execute_ast_inner(&mut self, ast: &Ast) -> Result<(), ExecuteError> {
        self.evalerror_exec_depth
            .set(self.evalerror_exec_depth.get() + 1);
        let result = self.execute_ast_inner_body(ast);
        self.evalerror_exec_depth
            .set(self.evalerror_exec_depth.get().saturating_sub(1));
        result
    }

    fn execute_ast_inner_body(&mut self, ast: &Ast) -> Result<(), ExecuteError> {
        {
            let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_UPSTREAM);
            if self.try_upstream_scripts() {
                return Ok(());
            }
        }

        let mut index = 0;
        // GNU execute_cmd.c:1576 execute_in_subshell: the forked child's
        // whole mutable state is a copy of the parent's. The flat `( )`
        // region runs in place on this executor, so the boundary is the
        // saved ShellState clone — aliases, functions, scopes, positional
        // params, traps (env-carried), job bookkeeping and history all
        // restore wholesale. The process cwd stays a shared in-place
        // resource (niubash#100) and is restored separately.
        let mut subshell_state: Option<crate::shell::ShellState> = None;
        let mut subshell_cwd: Option<PathBuf> = None;
        // The fd table is executor state, outside ShellState: a forked
        // child's descriptor table is a copy, so `( exec 3<&- )` must not
        // close the parent's fd 3. Rc-shared endpoints still alias the same
        // open file description — reads in the subshell advance the shared
        // offset, matching fork().
        let mut subshell_fd_table: Option<crate::executor::fd_table::FdTable> = None;
        // GNU execute_cmd.c: `exit` and an errexit trigger unwind the shell
        // via jump_to_top_level (exit.def:152 EXITBLTIN, execute_cmd.c:1174
        // ERREXIT) — they are never a plain command status. The jump stops
        // only at a shell boundary. A flat `( )` region (f() ( list ) bodies
        // mark commands with `subshell`/`subshell_end`) is a boundary this
        // frame owns: absorb the status there, restore the parent state
        // saved at region entry, and resume past the region. Everywhere
        // else the jump keeps unwinding to the caller — an enclosing `( )`
        // command node (execute_subshell_command_with_redirects), pipeline
        // stage, command substitution, or the process top level.
        // The region-close check at the loop bottom is unreachable from the
        // ~20 early `continue` dispatch paths (background `&`, `time`,
        // `!`, alias-introduced compounds, ...), so a `subshell_end` command
        // executed through any of them left the saved parent state dropped
        // unrestored — leaking jobs, variables, aliases and every other
        // ShellState field into the parent (GNU: the forked child simply
        // exits; nothing crosses the boundary). Run the same close before
        // each early continue/return so the boundary is path-independent.
        macro_rules! close_subshell_region_if_ended {
            ($command:expr) => {
                if $command.subshell_end {
                    self.evalerror_pending.set(false);
                    self.evalerror_line.set(None);
                    if let Some(saved_state) = subshell_state.take() {
                        self.restore_flat_subshell(saved_state, subshell_cwd.take());
                        if let Some(saved) = subshell_fd_table.take() {
                            self.fd_table = saved;
                        }
                    }
                }
            };
        }
        macro_rules! handle_exit_code {
            ($code:expr, $command:expr) => {{
                let code = $code;
                if self.parse_error_occurred || subshell_state.is_none() {
                    return Err(ExecuteError::ExitCode(code));
                }
                self.exit_code = code;
                // The region is dead: fast-forward to its closing command.
                // If none exists the region is malformed — keep unwinding.
                while index + 1 < ast.commands.len() && !ast.commands[index + 1].subshell_end {
                    index += 1;
                }
                if index + 1 < ast.commands.len() {
                    index += 1;
                } else if !$command.subshell_end {
                    return Err(ExecuteError::ExitCode(code));
                }
                if let Some(saved_state) = subshell_state.take() {
                    self.restore_flat_subshell(saved_state, subshell_cwd.take());
                    if let Some(saved) = subshell_fd_table.take() {
                        self.fd_table = saved;
                    }
                }
                // The dead subshell's status is a failing command status in
                // the parent: under `set -e` it exits the script unless the
                // command sits in a suppressing `!`/&&/|| context
                // (execute_cmd.c:1170-1175, set-e1.sub `(exit 17)`).
                if self.exit_code != 0
                    && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "errexit")
                    && self.suppress_errexit == 0
                    && !$command.inverted
                    && $command.and_or().is_none()
                {
                    return Err(ExecuteError::ExitCode(self.exit_code));
                }
                // Resume past the region's closing command — the subshell
                // died at the exit, so its tail must not run.
                index += 1;
                continue;
            }};
        }

        while index < ast.commands.len() {
            let command = &ast.commands[index];
            // GNU execute_cmd.c:652-656: `!` adds CMD_IGNORE_RETURN to the
            // command under exit_immediately_on_error, so its status never
            // satisfies errexit. The grouped drivers re-check
            // `status != 0 && errexit` per complete command without seeing
            // node flags, so record here whether the last top-level command
            // was inverted. Only depth-1 nodes outside a flat `( )` region
            // count — a `!` inside `(...)` or a nested list inverts its own
            // command, not the group's (GNU: `( ! true )` still exits).
            if self.evalerror_exec_depth.get() == 1 && subshell_state.is_none() {
                self.last_command_inverted.set(
                    (command.inverted || command.inverted_command.is_some())
                        && command.and_or().is_none(),
                );
            }
            // GNU parse.y/eval.c: a syntax error inside a command
            // substitution is a read-time failure of the enclosing command —
            // the parser recovered at the `)`, so the command itself still
            // ran (comsub-posix6.sub `$( esac ...)` prints the `*)` branch's
            // `ok 2`), but no further commands execute (`echo we should not
            // see this` is skipped and the shell exits 2). The substitution
            // already emitted its diagnostic at expansion time; the list
            // just stops here, including paths the simple-command check in
            // execute_command never reached (case patterns, compound
            // commands).
            if self.last_command_substitution_parse_error.get() {
                self.mark_parse_error();
                return Err(ExecuteError::ExitCode(2));
            }
            // GNU expr.c evalerror -> jump_to_top_level (DISCARD): while an
            // evalerror abort is pending, a nested command list unwinds
            // silently and the reader-level loop discards the rest of the
            // failing command's list — the commands sharing its source
            // line (`a[$x]=v; echo after` never prints `after`).
            if self.evalerror_pending.get() {
                // An evalerror raised INSIDE a `( )` subshell reaches only
                // that subshell's own top level: GNU forked it, so
                // jump_to_top_level(DISCARD) discards the subshell's
                // remaining commands and the parent list continues
                // (`( a[$bad]=v; echo skipped ); echo after` prints
                // `after` — verified GNU 5.3). The subshell's commands share
                // this flat list, so discard forward and let the boundary
                // marker restore the saved parent state.
                if subshell_state.is_some() {
                    if command.subshell_end {
                        if let Some(saved_state) = subshell_state.take() {
                            self.restore_flat_subshell(saved_state, subshell_cwd.take());
                            if let Some(saved) = subshell_fd_table.take() {
                                self.fd_table = saved;
                            }
                        }
                        self.evalerror_pending.set(false);
                        self.evalerror_line.set(None);
                    }
                    index += 1;
                    continue;
                }
                // The subshell-boundary error arm already tore the subshell
                // down (subshell_state is None) but left this list's closing
                // marker behind: skip the marker and end the abort — GNU's
                // jump_to_top_level(DISCARD) cannot cross the forked
                // subshell boundary, so same-line parent commands still run
                // (`( a[\" \"]=v ); echo after` prints `after`).
                if command.subshell_end {
                    self.evalerror_pending.set(false);
                    self.evalerror_line.set(None);
                    index += 1;
                    continue;
                }
                if self.evalerror_exec_depth.get() > 1 {
                    return Ok(());
                }
                if self.evalerror_line.get().is_none() {
                    self.evalerror_line.set(self.reader_command_line.get());
                }
                let boundary = self.evalerror_line.get();
                if std::env::var("RUBASH_DEBUG_EVALERR").is_ok() {
                    eprintln!(
                        "EVALERR-SKIP? cmd.line={:?} boundary={:?} reader={:?}",
                        command.line,
                        boundary,
                        self.reader_command_line.get()
                    );
                }
                if boundary.is_some() && command.line == boundary {
                    index += 1;
                    continue;
                }
                self.evalerror_pending.set(false);
                self.evalerror_line.set(None);
            }
            if self.evalerror_exec_depth.get() == 1 {
                self.reader_command_line.set(command.line);
                // GNU eval.c:181 (reader_loop/parse_and_execute): stdin_redir
                // is cleared before each top-level command, so a redirected
                // control structure's flag never leaks into the next command.
                self.shell_state.stdin_redir.set(false);
            }
            {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_JOBS);
                let protected_coprocs = self.coprocs_referenced_by_command(command);
                self.refresh_background_jobs_with_protected_coprocs(&protected_coprocs)?;
                self.run_pending_signal_traps()?;
            }
            let _t_chain = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_CHAIN);
            // GNU `line_number` advances only while the reader parses each
            // top-level command — the ambient line a while/if/group command
            // runs under is its own parse-end line. Inside a pre-parsed
            // body (depth > 1) the ambient stays frozen at whatever the
            // enclosing command established.
            if self.evalerror_exec_depth.get() == 1 {
                self.ambient_line
                    .set(command.end_line.or(command.line));
            }
            self.set_current_line(command);
            if self.noexec_enabled() {
                // GNU shell.c/parse.y: `-n` (noexec) skips execution but the
                // reader still parses each command — parse-time diagnostics
                // (heredoc EOF warnings, syntax errors, unclosed compounds)
                // fire exactly as execute_command's preamble reports them.
                // Skipping them left `bash -n` silently accepting malformed
                // scripts (unclosed `if`/`case`/`{` reported nothing, rc 0).
                self.report_command_heredoc_errors(command)?;
                if !self.command_parse_diagnostics(command)? {
                    self.exit_code = 0;
                }
                if command.subshell_end {
                    if let Some(saved_state) = subshell_state.take() {
                        self.restore_flat_subshell(saved_state, subshell_cwd.take());
                        if let Some(saved) = subshell_fd_table.take() {
                            self.fd_table = saved;
                        }
                    }
                }
                index += 1;
                continue;
            }

            // A `subshell`-flagged command opens this frame's flat `( )`
            // region (f() ( list ) bodies): save the parent state BEFORE any
            // dispatch runs it, so compound commands inside the region get
            // the same isolation as the simple-command path below — GNU
            // executes the whole list in the forked subshell
            // (execute_cmd.c:1576 execute_in_subshell).
            if command.subshell && subshell_state.is_none() {
                subshell_state = Some(self.shell_state.clone_for_child_save());
                subshell_fd_table = Some(self.fd_table.clone());
                subshell_cwd = env::current_dir().ok();
                self.shell_state.loop_depth = 0;
                crate::builtins::trap::reset_for_subshell(&mut self.shell_state.env_vars);
                let old_depth = self.shell_state.subshell_depth.get();
                self.shell_state.subshell_depth.set(old_depth + 1);
                // Feed subshell group stdin redirect to all body commands;
                // FUNCTION_STDIN lives in env_vars, so the wholesale state
                // restore at the boundary reverts it.
                for fwd in index + 1..ast.commands.len() {
                    let c = &ast.commands[fwd];
                    if c.subshell_end {
                        if let Some(input) = self.command_input_redirect(c) {
                            self.shell_state.env_vars.insert(FUNCTION_STDIN.to_string(), input);
                            self.shell_state.env_vars
                                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
                        }
                        break;
                    }
                }
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
            // The remaining compound/wrapper node kinds have no
            // run_debug_trap call site in GNU execute_command_internal either:
            // the DEBUG fires belong to their inner commands — `!` bodies run
            // through execute_ast below, pipeline elements each fire inside
            // their element subshell (execute_cmd.c:4506, stage loop below),
            // case/select fire the printed command head inside their handlers
            // (execute_cmd.c:3668/3528), and brace-group, `time`, coproc, and
            // `&` bodies dispatch to inner command lists (or subshells whose
            // children reset the trap table, execute_cmd.c:1670).
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
                || command.and_or_list.is_some()
                || command.inverted_command.is_some()
                || command.pipeline_command.is_some()
                || command.pipe.is_some()
                || command.brace_group.is_some()
                || command.case_command.is_some()
                || command.select_command.is_some()
                || command.time_command.is_some()
                || command.coproc_command.is_some()
                || command.background_command.is_some()
                || command_is_time_prefixed_compound(command);
            let debug_trap_active = crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
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
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if let Some(next_index) = self.execute_time_prefixed_command_sequence(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_compound_source(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = crate::builtins::source::execute_simple_if(self, ast, index)?
            {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_simple_loop(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) =
                crate::builtins::source::execute_pipe_into_source(self, ast, index)?
            {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_escaped_pipe(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_inversion(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_time(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_function(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_brace_group(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_subshell(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_for(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_select(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_case(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_introduced_coproc(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_heredoc(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_inverted_pipeline(ast, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
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
                        handle_exit_code!(code, command)
                    }
                    Err(error) => return Err(error),
                }
                if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                close_subshell_region_if_ended!(command);
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
                        if self.shell_state.loop_depth == 0 =>
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
                        handle_exit_code!(code, command)
                    }
                    Err(error) => return Err(error),
                }
                if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                close_subshell_region_if_ended!(command);
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
                        handle_exit_code!(code, command)
                    }
                    Err(error) => return Err(error),
                }
                index += 1;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if command_is_time_prefixed_compound(command) {
                let execution_result = self.execute_time_prefixed_compound_command(command);
                match execution_result {
                    Ok(()) => {}
                    Err(ExecuteError::Break(_) | ExecuteError::Continue(_))
                        if self.shell_state.loop_depth == 0 =>
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
                        handle_exit_code!(code, command)
                    }
                    Err(error) => return Err(error),
                }
                if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                close_subshell_region_if_ended!(command);
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
                        if self.shell_state.loop_depth == 0 =>
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
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::LastpipeExit(code)) => {
                        // Issue #74 (G2): `exit N` in a lastpipe stage runs in
                        // the current shell (shopt -s lastpipe), so it must
                        // exit the current shell — not just set the pipeline's
                        // exit status.  Propagate to the top level.
                        self.exit_code = code;
                        return Err(ExecuteError::ExitCode(code));
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
                        if self.shell_state.loop_depth == 0 =>
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
                        handle_exit_code!(code, command)
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
                Err(ExecuteError::Break(_) | ExecuteError::Continue(_)) if self.shell_state.loop_depth == 0 => {
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
                    handle_exit_code!(code, command)
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
                Err(ExecuteError::Break(_) | ExecuteError::Continue(_)) if self.shell_state.loop_depth == 0 => {
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

            drop(_t_chain);
            let execution_result = if command.inverted || command.and_or().is_some() {
                self.with_errexit_suppressed(|executor| executor.execute_command(command))
            } else {
                self.execute_command(command)
            };
            match execution_result {
                Ok(()) => {}
                Err(ExecuteError::Break(_) | ExecuteError::Continue(_)) if self.shell_state.loop_depth == 0 => {
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
                    if self.shell_state.loop_depth == 0
                        && self.shell_state.function_depth == 0
                        && subshell_state.is_none()
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
                Err(ExecuteError::ExitCode(code)) => {
                    handle_exit_code!(code, command)
                }
                // Bash runs a subshell with errexit active; when a command
                // fails inside the subshell the subshell exits with that
                // status but the parent script continues. Catch the error at
                // the subshell boundary instead of propagating it.
                Err(ExecuteError::ExpansionFailure(code)) if subshell_state.is_some() => {
                    self.exit_code = code;
                    while index + 1 < ast.commands.len() && !ast.commands[index + 1].subshell_end {
                        index += 1;
                    }
                    if index + 1 < ast.commands.len() {
                        index += 1;
                    }
                    if let Some(saved_state) = subshell_state.take() {
                        self.restore_flat_subshell(saved_state, subshell_cwd.take());
                        if let Some(saved) = subshell_fd_table.take() {
                            self.fd_table = saved;
                        }
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
                        && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "errexit")
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

            // GNU execute_cmd.c:1004-1018 (simple command errexit): after a
            // simple command finishes, if errexit is active and the command's
            // exit status is non-zero, jump_to_top_level (ERREXIT) exits the
            // shell.
            if self.errexit_enabled()
                && self.errexit_is_active()
                && self.suppress_errexit == 0
                && self.exit_code != 0
                && !command.inverted
                && command.and_or().is_none()
            {
                return Err(ExecuteError::ExitCode(self.exit_code));
            }

            // Execute ERR trap if command failed and not in &&/||/! context
            // (trap_exec.rs maybe_run_error_trap; GNU execute_cmd.c).
            self.maybe_run_error_trap(command)?;

            if command.subshell_end {
                // A pending evalerror was raised inside the subshell that just
                // ended: GNU's jump_to_top_level(DISCARD) cannot cross the
                // forked process boundary, so the abort dies with the
                // subshell (`( a[$bad]=v ); echo after` runs `after`).
                self.evalerror_pending.set(false);
                self.evalerror_line.set(None);
                if let Some(saved_state) = subshell_state.take() {
                    self.restore_flat_subshell(saved_state, subshell_cwd.take());
                    if let Some(saved) = subshell_fd_table.take() {
                        self.fd_table = saved;
                    }
                }
            }

            if let Some(next_index) = self.skip_and_or_rhs(ast, index) {
                index = next_index;
            } else {
                index += 1;
            }
        }
        // Same abort as the loop-top check, for a substitution parse error
        // raised by the final command in the list.
        if self.last_command_substitution_parse_error.get() {
            self.mark_parse_error();
            return Err(ExecuteError::ExitCode(2));
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

pub(in crate::executor) fn is_closed_output_io_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe || error.raw_os_error() == Some(232)
}
