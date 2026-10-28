use super::*;

impl Executor {
    /// rubash#1240: is the list currently executing a DISCARD boundary —
    /// the reader's top list, or an eval'd string's own top list
    /// (evalstring.c installs a local top_level catch, so the discard
    /// ends that string's list and no more)? GNU's jump_to_top_level
    /// (DISCARD) unwinds every frame in between: brace-group bodies,
    /// &&/|| element lists, loop bodies and function bodies are the
    /// SAME list as their enclosing line for this purpose.
    pub(in crate::executor) fn at_discard_boundary(&self) -> bool {
        self.evalerror_exec_depth.get() <= 1
            || self.eval_list_depth.get() == Some(self.evalerror_exec_depth.get())
    }

    /// Mark the list a following execute_ast call will run as a DISCARD
    /// boundary list, returning the saved marker for restoration.
    /// evalstring.c's parse_and_execute is the execution unit for reader
    /// input, eval strings AND sourced files (evalfile.c source_file):
    /// each installs its own top_level catch, so a DISCARD ends only
    /// that unit's list. execute_eval_source_inner marks this inline for
    /// eval; the source driver (builtins::source) uses this pair around
    /// its per-group runs (rubash#753 case 17).
    pub(crate) fn enter_discard_boundary_list(&self) -> Option<usize> {
        self.eval_list_depth
            .replace(Some(self.evalerror_exec_depth.get() + 1))
    }

    pub(crate) fn leave_discard_boundary_list(&self, saved: Option<usize>) {
        self.eval_list_depth.set(saved);
    }

    pub fn execute_ast(&mut self, ast: &Ast) -> Result<(), ExecuteError> {
        if EXECUTION_LOCK_DEPTH.with(|depth| depth.get() > 0) {
            return self.execute_ast_inner(&ast.commands);
        }

        // A fresh reader-level run: an evalerror abort still pending from a
        // finished run is stale and must not discard commands here.
        self.evalerror_pending.set(false);
        self.nounset_force_eof.set(false);
        self.evalerror_subscript_pending.set(false);
        self.evalerror_failglob_pending.set(false);
        self.evalerror_arith_pending.set(false);
        self.evalerror_exit_toomany_pending.set(false);
        self.evalerror_line.set(None);
        self.reader_command_line.set(None);
        // A fresh reader-level run is a GNU observation point for
        // deferred background-job completions (job_builtins.rs
        // refresh_background_jobs): the reap visibility mark advances.
        self.execute_generation
            .set(self.execute_generation.get() + 1);
        // rubash#353: this run's unit (file / -c string) was upfront-lexed
        // under the CURRENT locale; words carrying that lex locale are
        // eligible for the read-time re-derivation (words lexed later, e.g.
        // inside eval, already decoded under their own read-time locale).
        self.unit_lex_locale
            .replace(Some(crate::locale::locale_name()));
        self.line_lex_locales.borrow_mut().clear();

        // G-01: EXECUTION_LOCK is no longer held for the whole run.
        // EXECUTION_LOCK_DEPTH is now only the run-nesting marker (it
        // still gates the latch resets above); the lock itself lives
        // at the process-global touch points — see
        // local_helpers::acquire_process_state.
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(1));
        // G-03: attribute this run's process-env mirror writes to this
        // executor's journal (nested runs on the same executor share the
        // push; child executors running under this run journal into it as
        // the same lineage — see local_helpers::push_env_journal).
        let _env_journal_guard = push_env_journal(&self.env_write_journal);
        let result = self.execute_ast_inner(&ast.commands);
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(0));
        // rubash#1639: a marked SIGINT discard whose foreground command
        // was the LAST of this unit has no next command boundary to
        // convert it — flush it here so the mark can never leak into
        // the next input unit (whose first line must run normally,
        // seeing only the recorded $?=130).
        let result = match result {
            Ok(()) if self.fg_sigint_unwind => {
                self.fg_sigint_unwind = false;
                Err(ExecuteError::SigintUnwind)
            }
            other => other,
        };
        // rubash#423: GNU has no per-run cwd undo. `cd` runs chdir(2) in the
        // shell process itself (builtins/cd.def cd_builtin -> chdir), and the
        // process cwd IS the session state every later command inherits —
        // the reader never restores it. The restore this port paid per
        // execute_ast call (ced287c4 "isolate executor cwd during runs")
        // modeled an embedding-host boundary, but every real driver —
        // rubash.exe's own script/interactive readers AND the winuxsh $0
        // shim — drives whole shell sessions through this entry, so the
        // undo fired per interactive LINE and every child spawned by the
        // NEXT line (external ls, sed, ...) saw the stale startup cwd while
        // the shell's own PWD already moved (`cd dir && ls` on one line was
        // correct, two lines were not).
        //
        // G-02 (multi-session stopbleed): the #423 invariant survives, but
        // its carrier changes. The session cwd is now the ShellState.cwd
        // FIELD: `cd` moves only the field (never the process cwd), every
        // spawn is stamped current_dir(field), and every relative open
        // anchors at the field — so the cwd stays live across execute_ast
        // calls exactly like GNU, while two sessions hosted in one process
        // no longer share one process cwd. The process cwd is no longer a
        // session-semantics source at all.
        super::exec_profile::print_summary();
        result
    }

    /// Execute a borrowed command slice. GNU executes every compound body by
    /// POINTER: execute_cmd.c:624 execute_command_internal dispatches on the
    /// COMMAND tag of the node make_cmd.c allocated once at parse, and no
    /// compound executor (execute_while_or_until execute_cmd.c:3801,
    /// execute_for_command, execute_case_command, execute_function
    /// execute_cmd.c:5200) copies its body — `while`/`for` iterations and
    /// nested `{ { ...; } }` groups re-walk the same commands. The Vec
    /// wrapper this port used forced a deep `Vec<CommandNode>` clone per
    /// compound execution (per `for` ITERATION, and O(depth^2) for nested
    /// brace groups); this entry takes the borrowed slice so those sites can
    /// match GNU's zero-copy walk.
    pub(in crate::executor) fn execute_ast_inner(
        &mut self,
        commands: &[CommandNode],
    ) -> Result<(), ExecuteError> {
        // rubash#1801: every nested command list (compound bodies, function
        // bodies, loop bodies, pipeline stages) funnels through here, so
        // this is the executor's recursion hub — grow the host thread's
        // stack on demand instead of overflowing it (src/stack_growth.rs).
        crate::stack_growth::ensure_stack(|| self.execute_ast_inner_walk(commands))
    }

    fn execute_ast_inner_walk(&mut self, commands: &[CommandNode]) -> Result<(), ExecuteError> {
        self.evalerror_exec_depth
            .set(self.evalerror_exec_depth.get() + 1);
        // rubash#1164: a nested list (function body, compound body) samples
        // was_error_trap per its own commands; the enclosing command's
        // sample must be intact when its adjudication runs after the
        // nested list returns.
        let saved_err_trap_sample = self.err_trap_armed_sample.get();
        // rubash#1497: the -c tail exec-replacement stamp survives only
        // the final unit's own top frame (and, via the one-shot hold, an
        // and_or_list tail element's wrapper frame — GNU
        // execute_cmd.c:2886 execute_connection walks &&/|| operands in
        // the caller's frame; this port wraps each operand in its own
        // execute_ast). Every other nested list walk — function bodies,
        // trap actions, sourced/eval units, compound bodies — clears it
        // for its duration, so a spawn inside can never mistake itself
        // for the -c string's last top-level command.
        let saved_exec_replace_stamp = self.exec_replace_dispatch_stamp.get();
        let saved_exec_replace_shlvl = self.exec_replace_shlvl_snapshot.get();
        if self.exec_replace_stamp_hold.replace(0) == 0
            && !(self.exec_replace_final_frame.get() && self.evalerror_exec_depth.get() == 1)
        {
            self.exec_replace_dispatch_stamp.set(0);
            self.exec_replace_shlvl_snapshot.set(None);
        }
        let result = self.execute_ast_inner_body(commands);
        self.exec_replace_dispatch_stamp
            .set(saved_exec_replace_stamp);
        self.exec_replace_shlvl_snapshot
            .set(saved_exec_replace_shlvl);
        self.err_trap_armed_sample.set(saved_err_trap_sample);
        self.evalerror_exec_depth
            .set(self.evalerror_exec_depth.get().saturating_sub(1));
        // A trapped list tail with no following commands never reaches the
        // in-loop abandon check (rubash#639): convert a surviving nounset
        // trap here. Only the true reader top (entered at depth 0) converts;
        // nested lists (if/loop/function/eval bodies enter above 0)
        // silent-unwind with the flags intact for their owner, which
        // abandons and converts.
        if self.evalerror_exec_depth.get() == 0 && self.nounset_force_eof.replace(false) {
            self.exit_code = self.expansion_fatal_status();
            self.evalerror_pending.set(false);
            self.evalerror_line.set(None);
        }
        // rubash#705: an exit too-many discard armed by this run's LAST
        // command never reaches the in-loop abandon check — the loop is
        // already over. Mirror the nounset tail conversion: at the true
        // reader top under `-c`, no_args' top_level_cleanup unwound every
        // parse_and_execute level and shell.c:1477 run_one_command's
        // DISCARD catch ends the shell with status 1 (`bash -c 'exit 1 2'`
        // exits 1 — probe W4). The evalerror_pending stays armed so a
        // nested driver (a sourced file's run_source_impl, which returns
        // the status to the source builtin) still abandons its caller's
        // remaining commands through the in-loop check. In script mode the
        // pending is simply stale now: the reader run is over, $? already
        // holds 2 (probe W5: a script ending in `exit 1 2` exits 2), and
        // the next execute_ast clears it at entry.
        if self.evalerror_exec_depth.get() == 0 && self.evalerror_exit_toomany_pending.get() {
            // rubash#1541: the same conversion at a command-substitution
            // body frame — the child's top frame is parse_and_execute, not
            // reader_loop, and its DISCARD catch (builtins/evalstring.c:
            // 423-426) forces EXECUTION_FAILURE and stops the body, so a
            // body ending in `exit 1 2` yields substitution status 1, not
            // the pre-set 2 (probe: `x=$(exit 1 2)` in a script file).
            // command_substitution_result_status maps the Err to the
            // substitution status exactly like the `-c` driver's exit.
            if self.shell_state.env_vars.contains_key("__RUBASH_IS_C") || self.discard_fold_frame()
            {
                return Err(ExecuteError::ExitCode(1));
            }
            self.evalerror_exit_toomany_pending.set(false);
        }
        result
    }

    /// Report an IoError escaping a command's execution. When it is the
    /// command's own redirect OPEN failure — the `{target}: ` head names a
    /// redirect-list entry, possibly through `!`/`time`/`&` wrappers — GNU
    /// redir.c:135 redirection_error routes the message through the fd-2
    /// binding the list's earlier entries already applied
    /// (`( :; ) 2>cap.txt > /missing` lands in cap.txt, rubash#940 R1).
    /// Any other IoError (body write errors, rubash#440 closed-output)
    /// keeps the direct stderr write so the reported bytes are identical.
    fn report_io_error_routed(
        &mut self,
        command: &CommandNode,
        error: &std::io::Error,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        // GNU redir.c:135 redirection_error (internal_error family) honors
        // gnu_errfmt (rubash#1038).
        writeln!(
            &mut stderr,
            "{}{}",
            self.error_prefix(),
            crate::posix_errors::message(error)
        )?;
        let mut owner = command;
        loop {
            if let Some(inner) = &owner.inverted_command {
                owner = &inner.command;
            } else if let Some(inner) = &owner.time_command {
                owner = &inner.command;
            } else if let Some(inner) = &owner.background_command {
                owner = &inner.command;
            } else {
                break;
            }
        }
        if self.failing_open_redirect_index(owner, &stderr).is_some() {
            // A routed write that itself fails (closed fd 2) is lost like
            // GNU's builtin_error — never a second diagnostic (rubash#699).
            let _ = self.write_redirect_diagnostic_routed(owner, &stderr);
        } else {
            let _ = self.write_default_stderr(&stderr);
        }
        self.exit_code = 1;
        Ok(())
    }

    /// rubash#1497: the node shapes GNU's fork optimization accepts
    /// for the last command of a -c string (builtins/evalstring.c:106
    /// should_optimize_fork: a cm_simple command, not a pipeline
    /// element, carrying no redirections; parse_and_execute also
    /// refuses CMD_TIME_PIPELINE / CMD_INVERT_RETURN carriers). Prefix
    /// assignments are fine — they fold into the child's environment
    /// exactly as in the fork path. A folded &&/|| list qualifies
    /// through its last element, mirroring optimize_connection_fork
    /// (builtins/evalstring.c:148).
    fn exec_replace_shape_ok(command: &CommandNode) -> bool {
        if let Some(list) = &command.and_or_list {
            return !command.background
                && command.redirects.is_empty()
                && command.redirect_in.is_none()
                && command.redirect_out.is_none()
                && command.redirect_err.is_none()
                && command.append.is_none()
                && command.redirect_err_append.is_none()
                && command.heredoc.is_none()
                && command.here_string.is_none()
                && command.heredoc_redirects.is_empty()
                && list
                    .commands
                    .last()
                    .is_some_and(Self::exec_replace_simple_shape_ok);
        }
        Self::exec_replace_simple_shape_ok(command)
    }

    fn exec_replace_simple_shape_ok(command: &CommandNode) -> bool {
        !command.words.is_empty()
            && command.pipeline_command.is_none()
            && command.and_or_list.is_none()
            && command.time_command.is_none()
            && command.background_command.is_none()
            && command.inverted_command.is_none()
            && command.coproc_command.is_none()
            && command.for_command.is_none()
            && command.arithmetic_command.is_none()
            && command.if_command.is_none()
            && command.loop_command.is_none()
            && command.conditional_command.is_none()
            && command.subshell_command.is_none()
            && command.case_command.is_none()
            && command.select_command.is_none()
            && command.function_command.is_none()
            && command.brace_group.is_none()
            && command.pipe.is_none()
            && command.and_or.is_none()
            && !command.background
            && !command.inverted
            && !command.subshell
            && !command.subshell_end
            && command.redirects.is_empty()
            && command.redirect_in.is_none()
            && command.redirect_out.is_none()
            && command.redirect_err.is_none()
            && command.append.is_none()
            && command.redirect_err_append.is_none()
            && command.heredoc.is_none()
            && command.heredoc_body.is_none()
            && command.here_string.is_none()
            && command.heredoc_redirects.is_empty()
    }

    fn execute_ast_inner_body(&mut self, commands: &[CommandNode]) -> Result<(), ExecuteError> {
        let mut index = 0;
        // GNU execute_cmd.c:1576 execute_in_subshell: the forked child's
        // whole mutable state is a copy of the parent's. The flat `( )`
        // region runs in place on this executor, so the boundary is the
        // saved ShellState clone — aliases, functions, scopes, positional
        // params, traps (env-carried), job bookkeeping and history all
        // restore wholesale. Since G-02 the cwd is a ShellState field, so
        // it restores with the clone too (it was a shared in-place process
        // resource before, niubash#100).
        let mut subshell_state: Option<crate::shell::ShellState> = None;
        // The fd table is executor state, outside ShellState: a forked
        // child's descriptor table is a copy, so `( exec 3<&- )` must not
        // close the parent's fd 3. Rc-shared endpoints still alias the same
        // open file description — reads in the subshell advance the shared
        // offset, matching fork().
        let mut subshell_fd_table: Option<crate::executor::fd_table::FdTable> = None;
        // rubash#1141: the FUNCTION_STDIN production schedule is executor
        // state too — saved with the region (a nested group's redirect
        // feed replaces FUNCTION_STDIN mid-region and clears it) and
        // restored at the boundary like the fd table.
        let mut subshell_timed_stdin: Option<Option<super::timed_input::SharedTimedInput>> = None;
        // rubash#720 form 1: the enclosing fork-collapse value to restore
        // when the flat region closes — Some only when the region opened
        // with a single-subshell-command body (execute_cmd.c:1799-1806 +
        // execute_cmd.c:662-663 erase this frame's :769 ERR wait).
        let mut saved_fork_collapse: Option<bool> = None;
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
                    self.nounset_force_eof.set(false);
                    self.evalerror_subscript_pending.set(false);
                    self.evalerror_failglob_pending.set(false);
                    self.evalerror_arith_pending.set(false);
                    self.evalerror_exit_toomany_pending.set(false);
                    self.evalerror_line.set(None);
                    if let Some(saved_state) = subshell_state.take() {
                        self.restore_flat_subshell(saved_state);
                        if let Some(saved) = subshell_fd_table.take() {
                            self.fd_table = saved;
                        }
                        if let Some(saved) = subshell_timed_stdin.take() {
                            self.timed_stdin = saved;
                        }
                        if let Some(saved) = saved_fork_collapse.take() {
                            self.subshell_fork_collapse.set(saved);
                        }
                        // rubash#720 form 2: GNU adjudicates the user
                        // subshell's completion ERR in the frame that waited
                        // (execute_cmd.c:769-778 run_error_trap after
                        // wait_for) — the forkless region closes here, so the
                        // wait-side fire belongs at every close path.
                        self.close_flat_region_err_adjudication(commands, index)?;
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
                while index + 1 < commands.len() && !commands[index + 1].subshell_end {
                    index += 1;
                }
                if index + 1 < commands.len() {
                    index += 1;
                } else if !$command.subshell_end {
                    return Err(ExecuteError::ExitCode(code));
                }
                if let Some(saved_state) = subshell_state.take() {
                    self.restore_flat_subshell(saved_state);
                    if let Some(saved) = subshell_fd_table.take() {
                        self.fd_table = saved;
                    }
                    if let Some(saved) = subshell_timed_stdin.take() {
                        self.timed_stdin = saved;
                    }
                    if let Some(saved) = saved_fork_collapse.take() {
                        self.subshell_fork_collapse.set(saved);
                    }
                    // rubash#720 form 2: the wait-side ERR fire belongs after
                    // the region restore and before the parent-side errexit
                    // adjudication — GNU's :769 run_error_trap precedes the
                    // :777 ERREXIT jump (execute_cmd.c:769-795).
                    self.close_flat_region_err_adjudication(commands, index)?;
                }
                // The dead subshell's status is a failing command status in
                // the parent: under `set -e` it exits the script unless the
                // command sits in a suppressing `!`/&&/|| context
                // (execute_cmd.c:1170-1175, set-e1.sub `(exit 17)`).
                if self.exit_code != 0
                    && crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "errexit",
                    )
                    && self.suppress_errexit == 0
                    && !$command.inverted
                    && $command.and_or().is_none()
                {
                    // DE-06 (design 9.3; PLAN row 12): the parent
                    // frame's errexit adjudication of the dead
                    // region's status — park a selected `set -e`
                    // filter here, in the restored parent state,
                    // before the exit path runs. (The wait-side
                    // ERR fire above has already run, so the GNU
                    // trap-before-exit order holds.)
                    self.debugger_note_exception_site(
                        crate::debugger::ExceptionKind::Errexit,
                        self.exit_code,
                        None,
                    )?;
                    return Err(ExecuteError::ExitCode(self.exit_code));
                }
                // Resume past the region's closing command — the subshell
                // died at the exit, so its tail must not run.
                index += 1;
                continue;
            }};
        }

        while index < commands.len() {
            // rubash#1639: a foreground wait marked a SIGINT line
            // discard (fg_sigint_unwind): GNU wait_for throws to the
            // top-level reader, so the NEXT command of this list — at
            // any frame depth — never dispatches. Inside a flat `( )`
            // region the region first closes as a signal death
            // (fast-forward + restore, the handle_exit_code discipline)
            // because the region's saved parent state is this frame's
            // to restore; the throw then unwinds this frame too.
            if self.fg_sigint_unwind {
                self.fg_sigint_unwind = false;
                if subshell_state.is_some() {
                    while index + 1 < commands.len() && !commands[index + 1].subshell_end {
                        index += 1;
                    }
                    if index + 1 < commands.len() {
                        index += 1;
                    }
                    if let Some(saved_state) = subshell_state.take() {
                        self.restore_flat_subshell(saved_state);
                        if let Some(saved) = subshell_fd_table.take() {
                            self.fd_table = saved;
                        }
                        if let Some(saved) = subshell_timed_stdin.take() {
                            self.timed_stdin = saved;
                        }
                        if let Some(saved) = saved_fork_collapse.take() {
                            self.subshell_fork_collapse.set(saved);
                        }
                    }
                    self.exit_code = 130;
                }
                return Err(ExecuteError::SigintUnwind);
            }
            let command = &commands[index];
            // rubash#439: the expansion-failure mark belongs to ONE command.
            // Reset before each iteration so a previous command's expansion
            // error never suppresses the ERR trap this command earns by
            // failing (GNU run_error_trap is per execute_simple_command).
            self.command_expansion_failed.set(false);
            // rubash#1164: sample was_error_trap for THIS command before it
            // executes (execute_cmd.c:914) — adjudication below and inside
            // dispatch consults the sample, so a trap this command installs
            // (e.g. in a function body) cannot fire for the command itself.
            self.err_trap_armed_sample.set(Some(matches!(
                crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "ERR"),
                Some(ref action) if !action.is_empty()
            )));
            // issue #485: the comsub $? overlay (word_expansion_comsub_exit)
            // belongs to ONE command's word expansion, exactly like GNU's
            // last_command_exit_value write in subst.c command_substitute —
            // the next command commits its own status, so clear the overlay
            // here or `echo $(false) $?; true; echo $?` would still see the
            // stale substitution status.
            self.word_expansion_comsub_exit.set(None);
            // eval.c:178 + shell.c:183: current_command_number increments
            // once per reader-loop command list, BEFORE it executes; `\#`
            // in ${var@P} reads the counter (parse.y:6568-6574). This
            // depth-1 walk IS the reader's list sequence for every driver
            // (script-file lines/groups, interactive accepted lines, stdin
            // groups); nested walks (source/eval/comsub bodies — entered
            // while the execution lock was already held) run INSIDE an
            // executing list and stay frozen at the inherited value (WSL
            // GNU 5.3.0: sourced lines all print the source line's number).
            // `bash -c` never enters reader_loop (shell.c run_one_command),
            // hence the command_string_mode suppression. One source line is
            // ONE GNU list (read_command gathers `a; b` together), so the
            // counter steps only when the node's line changes.
            if self.evalerror_exec_depth.get() == 1
                && subshell_state.is_none()
                && !self.command_string_mode.get()
            {
                let node_line = command.line;
                let is_new_list = match (node_line, self.reader_last_list_line.get()) {
                    (Some(line), Some(previous)) => line != previous,
                    _ => true,
                };
                if is_new_list {
                    self.shell_state.command_number += 1;
                }
                if let Some(line) = node_line {
                    self.reader_last_list_line.set(Some(line));
                }
            }
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
            // rubash#1497: stamp the final -c unit's last top-level
            // command dispatch (GNU builtins/evalstring.c:126
            // should_suppress_fork + execute_cmd.c:5813: the last
            // command of a -c string replaces the shell process when it
            // is a plain external command). The stamp is consumed at
            // the spawn site; nested walks clear it (execute_ast_inner),
            // so only this node's own spawn can observe it.
            if self.exec_replace_final_frame.get() && self.evalerror_exec_depth.get() == 1 {
                let armed = index + 1 == commands.len()
                    && subshell_state.is_none()
                    && Self::exec_replace_shape_ok(command);
                if armed {
                    let stamp = self.exec_replace_stamp_counter.get() + 1;
                    self.exec_replace_stamp_counter.set(stamp);
                    self.exec_replace_dispatch_stamp.set(stamp);
                    // rubash#1497: freeze the SHLVL the spawn-site
                    // adjustment must read (see
                    // exec_replace_shlvl_snapshot) before this command's
                    // own prefix assignments are committed.
                    self.exec_replace_shlvl_snapshot
                        .set(Some(self.current_shlvl_value()));
                } else {
                    self.exec_replace_dispatch_stamp.set(0);
                    self.exec_replace_shlvl_snapshot.set(None);
                }
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
                // A BACKTICK body's parse error is contained in the
                // substituted child (GNU subst.c parse_and_execute; issue
                // #515) — clear the latch and keep reading; the `$(`-body
                // read-time failure still stops the list.
                if self
                    .last_command_substitution_parse_error_backtick
                    .replace(false)
                {
                    self.last_command_substitution_parse_error.set(false);
                } else {
                    self.mark_parse_error();
                    return Err(ExecuteError::ExitCode(2));
                }
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
                            self.restore_flat_subshell(saved_state);
                            if let Some(saved) = subshell_fd_table.take() {
                                self.fd_table = saved;
                            }
                            if let Some(saved) = subshell_timed_stdin.take() {
                                self.timed_stdin = saved;
                            }
                            if let Some(saved) = saved_fork_collapse.take() {
                                self.subshell_fork_collapse.set(saved);
                            }
                            // rubash#1541: a `( )` region closed by the
                            // no_args DISCARD inside a command-substitution
                            // body takes the folded status 1, not the pre-set
                            // EX_BADUSAGE 2 — the region's forked child
                            // inherits the body's parse_and_execute frame
                            // (see discard_fold_frame). At the main level
                            // the same region keeps 2 (execute_in_subshell,
                            // execute_cmd.c:1821-1825). Only the too-many
                            // class pre-sets 2; every other DISCARD class
                            // here already carries 1.
                            if self.evalerror_exit_toomany_pending.get()
                                && self.discard_fold_frame()
                            {
                                self.exit_code = 1;
                            }
                            // rubash#720 form 2: the evalerror DISCARD aborts
                            // the forked subshell with status 1 and the
                            // parent's :769 wait-side ERR adjudication still
                            // runs (rubash#439: the parent judges by status).
                            self.close_flat_region_err_adjudication(commands, index)?;
                        }
                        self.evalerror_pending.set(false);
                        self.nounset_force_eof.set(false);
                        self.evalerror_subscript_pending.set(false);
                        self.evalerror_failglob_pending.set(false);
                        self.evalerror_arith_pending.set(false);
                        self.evalerror_exit_toomany_pending.set(false);
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
                    self.nounset_force_eof.set(false);
                    self.evalerror_subscript_pending.set(false);
                    self.evalerror_failglob_pending.set(false);
                    self.evalerror_arith_pending.set(false);
                    self.evalerror_exit_toomany_pending.set(false);
                    self.evalerror_line.set(None);
                    index += 1;
                    continue;
                }
                if self.evalerror_exec_depth.get() > 1
                    && (self.eval_list_depth.get() != Some(self.evalerror_exec_depth.get())
                        || self.evalerror_exit_toomany_pending.get())
                {
                    // Nested function/loop/subshell bodies unwind silently on
                    // abort — except the eval'd top list itself (rubash#637):
                    // GNU's parse_and_execute loops per line inside eval, so
                    // falling through lets the normal line-skip logic run
                    // later lines and eval return their status. The exit
                    // too-many class is the exception to that exception
                    // (rubash#705): no_args' top_level_cleanup pops every
                    // parse_and_execute level including the eval'd one, so
                    // the WHOLE eval'd string is abandoned (`eval "exit 1 2
                    // \necho IN"` never prints IN — probe V2) and the
                    // pending discard keeps unwinding to the reader.
                    return Ok(());
                }
                if self.evalerror_line.get().is_none() {
                    self.evalerror_line.set(self.reader_command_line.get());
                    // GNU shell.c:1477 (jump catch, case DISCARD):
                    // `return last_command_exit_value = 1;` — the reader
                    // observes the unwound abort before anything else runs,
                    // so $? is 1 no matter which compound shape carried the
                    // failure (an `if` whose condition died never completes
                    // and cannot leave its own 0). A nounset subscript trap
                    // is FORCE_EOF, not DISCARD (rubash#639, shell.c:1471):
                    // recompute the context status here (127 at the `-c`
                    // top level, 1 in script mode) so no intermediate
                    // exit-code write between the trap and the abandon can
                    // downgrade it.
                    if self.nounset_force_eof.replace(false) {
                        self.exit_code = self.expansion_fatal_status();
                    } else {
                        self.exit_code = 1;
                    }
                    // rubash#705: the exit too-many class leaves 2, not the
                    // plain DISCARD 1 — no_args ran set_exit_status
                    // (EX_BADUSAGE) before its jump, and the reader keeps a
                    // non-zero value (eval.c:115-117 only rewrites a zero).
                    // Consumed here: the eval'd list already unwound whole
                    // (the nested check above), so no eval boundary owns
                    // this class.
                    // rubash#705: the exit too-many class leaves 2, not the
                    // plain DISCARD 1 — no_args ran set_exit_status
                    // (EX_BADUSAGE) before its jump, and the reader keeps a
                    // non-zero value (eval.c:115-117 only rewrites a zero).
                    // Consumed here: the eval'd list already unwound whole
                    // (the nested check above), so no eval boundary owns
                    // this class. The `-c` stop below rides THIS class
                    // only — the other discard classes keep GNU's
                    // line-contained shape under `-c` (probe G1:
                    // `bash -c 'shopt -s failglob; set -- nope*\necho
                    // after'` prints after).
                    if self.evalerror_exit_toomany_pending.replace(false) {
                        self.exit_code = 2;
                        // rubash#705: under `-c` the discard never returns to
                        // this reader — no_args' top_level_cleanup unwound
                        // every parse_and_execute level and shell.c:1477
                        // run_one_command's DISCARD catch ends the shell with
                        // status 1, dropping every remaining command of the
                        // -c string regardless of line (probe P2: `bash -c
                        // 'exit 1 2\necho AFTER'` prints nothing more).
                        // Contained inside a `( )` region — GNU's forked
                        // subshell never sees the -c catch (probe P6). The
                        // Err converts through script_driver's execute_ast
                        // arm: exit_jump_pending stops any grouped read and
                        // finish_shell runs the EXIT trap with $?=1 (probe
                        // T1).
                        if self.evalerror_exec_depth.get() == 1
                            && subshell_state.is_none()
                            && (self.shell_state.env_vars.contains_key("__RUBASH_IS_C")
                                || self.discard_fold_frame())
                        {
                            self.evalerror_pending.set(false);
                            self.evalerror_line.set(None);
                            return Err(ExecuteError::ExitCode(1));
                        }
                    }
                    // The subscript class served its purpose once the abandon
                    // fires (rubash#637) — except anywhere inside an eval
                    // string, whose boundary (not this block) owns
                    // absorption: consuming here would starve it, and the
                    // same-line skip below still runs. Functions called
                    // inside eval keep the mark for the eval boundary for
                    // the same reason; nested silent unwinds return before
                    // this block either way.
                    if self.eval_list_depth.get().is_none() {
                        self.evalerror_subscript_pending.set(false);
                    }
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
                // The discard boundary is the LOGICAL line (GNU abandons
                // the reader's whole current list): commands joined by
                // backslash-newline continuations share it even though
                // their physical lines differ (rubash#411).
                let command_list_line = command.logical_line.or(command.line);
                if boundary.is_some() && command_list_line == boundary {
                    index += 1;
                    continue;
                }
                self.evalerror_pending.set(false);
                self.nounset_force_eof.set(false);
                self.evalerror_subscript_pending.set(false);
                self.evalerror_failglob_pending.set(false);
                self.evalerror_arith_pending.set(false);
                self.evalerror_exit_toomany_pending.set(false);
                self.evalerror_line.set(None);
            }
            if self.evalerror_exec_depth.get() == 1 {
                // rubash#1494: one tick of the notification-lag clock per
                // top-level reader command; the refresh below applies
                // deferred job-state marks once they are a full command
                // old (GNU waitchld lag).
                self.shell_state.job_table.note_reader_command();
                self.reader_command_line
                    .set(command.logical_line.or(command.line));
                // rubash#353: record the locale in effect when this SOURCE
                // LINE is first reached at reader level — the locale GNU's
                // parser would decode the line's `$'...'` backslash-u
                // escapes under (GNU reads newline-terminated lists one at
                // a time; the first command of a line fixes the read-time
                // locale for every word the reader pulls in that list).
                if let Some(line) = command.line {
                    self.line_lex_locales
                        .borrow_mut()
                        .entry(line)
                        .or_insert_with(crate::locale::locale_name);
                }
                // GNU eval.c:181 (reader_loop/parse_and_execute): stdin_redir
                // is cleared before each top-level command, so a redirected
                // control structure's flag never leaks into the next command.
                self.shell_state.stdin_redir.set(false);
                // rubash#846: same per-command hygiene for the
                // compound-redirect-failure mark — a stale mark (set by a
                // previous command whose `!` consult never ran) must never
                // suppress the next command's flip. Nested scopes clear at
                // their own fork/call boundaries.
                self.compound_redirect_failed.set(false);
            }
            {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_JOBS);
                // GNU has no "referenced by the next command" protection for
                // dead coprocs: cleanup_dead_jobs -> coproc_reap (jobs.c:
                // 1342) disposes every COPROC_DEAD entry through
                // coproc_dispose -> coproc_unsetvars (execute_cmd.c:2166,
                // :2450) — <name> and <name>_PID unbind unconditionally, so
                // `cat <&${A[0]}` for a reaped coproc reports `ambiguous
                // redirect` (rubash#1023).
                self.refresh_background_jobs()?;
                self.run_pending_signal_traps()?;
                // GNU eval.c:355 runs notify_and_cleanup after each top-level
                // command: children killed by a signal get their stderr
                // notice here (rubash#229, jobs.c:4625 notify_of_job_status).
                // At this point __RUBASH_CURRENT_LINE is still the previous
                // command's line, matching GNU's line_number at notify time.
                self.report_pending_signal_notices()?;
            }
            let _t_chain = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_CHAIN);
            // GNU `line_number` advances only while the reader parses each
            // top-level command — the ambient line a while/if/group command
            // runs under is its own parse-end line. Inside a pre-parsed
            // body (depth > 1) the ambient stays frozen at whatever the
            // enclosing command established.
            if self.evalerror_exec_depth.get() == 1 {
                self.ambient_line.set(command.end_line.or(command.line));
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
                        self.restore_flat_subshell(saved_state);
                        if let Some(saved) = subshell_fd_table.take() {
                            self.fd_table = saved;
                        }
                        if let Some(saved) = subshell_timed_stdin.take() {
                            self.timed_stdin = saved;
                        }
                        // No ERR adjudication here: GNU -n executes nothing,
                        // so no subshell completion status can exist to
                        // adjudicate (run_error_trap gates on exec_result).
                        if let Some(saved) = saved_fork_collapse.take() {
                            self.subshell_fork_collapse.set(saved);
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
                // rubash#720 form 2: GNU's fork preamble (execute_cmd.c:703-710)
                // fixes the_printed_command_except_trap to make_command_string
                // of the subshell when a user subshell forks with the ERR trap
                // armed — both parent-side fires (:769 for the region, :1000
                // for the function call returning its status) then report that
                // text as BASH_COMMAND. Stamp it BEFORE the wholesale clone so
                // the boundary restore keeps it; the body's in-process
                // mutations die with the restore the way the forked child's
                // copy died with the child.
                if crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "ERR")
                    .is_some()
                {
                    let close_index = (index..commands.len())
                        .find(|candidate| commands[*candidate].subshell_end)
                        .unwrap_or(index);
                    let region_text =
                        crate::executor::command_text::flat_subshell_region_source_text(
                            &commands[index..=close_index],
                        );
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_LAST_COMMAND".to_string(), region_text.clone());
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_CURRENT_COMMAND".to_string(), region_text);
                }
                subshell_state = Some(self.shell_state.clone_for_child_save());
                subshell_fd_table = Some(self.fd_table.clone());
                subshell_timed_stdin = Some(self.timed_stdin.clone());
                self.shell_state.loop_depth = 0;
                // execute_in_subshell resets loop_level (execute_cmd.c:1642):
                // fork-inherited depth does not cross a `( )` region either.
                self.shell_state.inherited_loop_depth = 0;
                crate::builtins::trap::reset_for_subshell_child(&mut self.shell_state.env_vars);
                let old_depth = self.shell_state.subshell_depth.get();
                self.shell_state.subshell_depth.set(old_depth + 1);
                // rubash#1493: record the region's whole `( ... )` text
                // for the exec signal-death notice (jobs.c:4625 names
                // the whole job command) — a node-form region renders
                // its own node, a flat region renders its command span
                // the way flat_subshell_region_source_text does for the
                // ERR-trap stamp above. The region's saved state clone
                // (taken just above) holds the parent's value, so the
                // boundary restore reverts this automatically.
                self.shell_state.subshell_source_text =
                    Some(if command.subshell_command.is_some() {
                        crate::executor::command_text::bash_command_source_text(command)
                    } else {
                        let close_index = (index..commands.len())
                            .find(|candidate| commands[*candidate].subshell_end)
                            .unwrap_or(index);
                        crate::executor::command_text::flat_subshell_region_source_text(
                            &commands[index..=close_index],
                        )
                    });
                // rubash#1115: the forked child reseeds RANDOM
                // (variables.c:1418-1423 get_random_number); the saved
                // parent state above keeps the parent's sequence intact.
                self.shell_state.random_state.reseed_fresh();
                // rubash#720 form 1: a single-subshell-command region body is
                // the body GNU's execute_in_subshell marks CMD_NO_FORK
                // (execute_cmd.c:1799-1806) and runs in place via
                // execute_cmd.c:662-663 — the region frame performs no wait
                // and therefore no :769 ERR adjudication for it.
                saved_fork_collapse = if command.subshell_end && command.subshell_command.is_some()
                {
                    Some(self.subshell_fork_collapse.replace(true))
                } else {
                    None
                };
                // Feed subshell group stdin redirect to all body commands;
                // FUNCTION_STDIN lives in env_vars, so the wholesale state
                // restore at the boundary reverts it.
                for fwd in index + 1..commands.len() {
                    let c = &commands[fwd];
                    if c.subshell_end {
                        if let Some(input) = self.command_input_redirect(c) {
                            self.shell_state
                                .env_vars
                                .insert(FUNCTION_STDIN.to_string(), input);
                            self.shell_state
                                .env_vars
                                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
                            // rubash#1141: the fed text is locally
                            // produced redirect input — the enclosing
                            // stream's production schedule does not
                            // describe it (restored at the boundary).
                            self.timed_stdin = None;
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
            let debug_trap_active = crate::builtins::trap::has_active_trap_action(
                &self.shell_state.env_vars,
                "DEBUG",
                "__RUBASH_TRAP_DEBUG",
            );
            // Do not fire for commands inside the trap action itself: Bash
            // does not re-enter the DEBUG trap while an action runs, and
            // firing would let the action's commands overwrite LINENO with
            // their own (synthetic) line before run_debug_trap's guard sees
            // the flag (dbg-support2.tests `print_trap $LINENO`).
            // The DEBUG trap is inherited by functions only under functrace
            // (execute_cmd.c:5270); extdebug reaches that state solely through
            // the functrace it enables (shopt.def:621), so a later set +T
            // disables inheritance even with extdebug active.
            // rubash#186: debug_trap_in_scope() is folded into the condition
            // after debug_trap_active so the common no-trap run never pays it
            // (both are pure reads, so the reordering is behavior-neutral).
            // WP-3 (design 4.4): an attached session widens this
            // seam's admission past the trap-action test — the
            // session's scope is the attachment itself, not the
            // functrace inheritance the trap path checks.
            let session_here = self.debugger_session_here();
            if !skips_debug_trap
                && ((debug_trap_active && self.debug_trap_in_scope()) || session_here)
                && !self.debug_trap_running
            {
                // Bash exposes the about-to-run command's line via LINENO
                // inside the DEBUG trap action (dbg-support2.tests).
                self.set_current_line(command);
                let command_text = crate::executor::command_text::bash_command_source_text(command);
                // The main seam's kind comes from the node's own
                // fields (150-c REPORT section 5, note 8).
                let seam = if command.conditional_command.is_some() {
                    crate::debugger::SeamKind::Conditional
                } else if command.arithmetic_command.is_some() {
                    crate::debugger::SeamKind::Arithmetic
                } else {
                    crate::debugger::SeamKind::SimpleCommand
                };
                if self.run_debug_trap(seam, &command_text)?.is_some() {
                    // execute_cmd.c:4510: execute_simple_command returns
                    // EXECUTION_SUCCESS for the skipped unit, and
                    // execute_command_internal commits that as
                    // last_command_exit_value (probe m1: after `false`, a
                    // skipped `echo hi` leaves $? = 0).
                    self.exit_code = 0;
                    index += 1;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            // rubash#186: combined admission for the whole alias-introduced
            // probe family below (compound_source, inversion, time, function,
            // brace_group, subshell, for, select, case, coproc, heredoc).
            // GNU has no such probes — they exist because rubash expands
            // aliases at execution instead of in the parser stream
            // (parse.y:3013 alias_expand_token). Each probe's admission is
            // exactly: expand_aliases(words) must turn the first word into a
            // compound marker, or alias_parser_source must resolve the first
            // word through the alias table. With ZERO defined aliases,
            // expand_aliases is provably the identity (expand_alias_word:
            // aliases.get(word) miss returns the word unchanged) and
            // alias_parser_source_inner returns None at its aliases.get(word)?
            // — so every probe's outcome collapses to a raw-first-word
            // marker test. Evaluate those markers in one pass here and skip
            // the 11 per-command probe bodies (each re-scanning the shopt
            // state and the alias table) when none can fire. `for`/`select`
            // may advance past empty-word commands before their marker test,
            // so an empty-words command keeps the chain.
            let alias_chain_applicable = if self.shell_state.aliases.is_empty() {
                command.words.is_empty()
                    || matches!(
                        command.words.first().map(String::as_str),
                        Some(
                            "!" | "("
                                | "{"
                                | "function"
                                | "time"
                                | "case"
                                | "coproc"
                                | "select"
                                | "for"
                        )
                    )
            } else {
                true
            };

            if let Some(next_index) =
                self.execute_time_prefixed_command_sequence(commands, index)?
            {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if alias_chain_applicable {
                if let Some(next_index) =
                    self.execute_alias_introduced_compound_source(commands, index)?
                {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if let Some(next_index) =
                crate::builtins::source::execute_simple_if(self, commands, index)?
            {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_simple_loop(commands, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) =
                crate::builtins::source::execute_pipe_into_source(self, commands, index)?
            {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(next_index) = self.execute_alias_escaped_pipe(commands, index)? {
                index = next_index;
                close_subshell_region_if_ended!(command);
                continue;
            }

            if alias_chain_applicable {
                if let Some(next_index) =
                    self.execute_alias_introduced_inversion(commands, index)?
                {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_introduced_time(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_introduced_function(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) =
                    self.execute_alias_introduced_brace_group(commands, index)?
                {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_introduced_subshell(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_introduced_for(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_introduced_select(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_introduced_case(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_introduced_coproc(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if alias_chain_applicable {
                if let Some(next_index) = self.execute_alias_heredoc(commands, index)? {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
            }

            if let Some(next_index) = self.execute_inverted_pipeline(commands, index)? {
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
                        self.report_io_error_routed(command, &error)?;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                        // GNU: `return N` inside a `( )` subshell ends the
                        // child with status N (return.def:57-76
                        // return_builtin — the child's own return_catch), the
                        // parent observes a plain status-N completion and the
                        // function continues after the region. Absorb at the
                        // boundary (rubash#720 CR round 2).
                        handle_exit_code!(code, command)
                    }
                    Err(error) => {
                        // CR round 2 (rubash#720): an unwind crossing a flat
                        // region boundary must not leak the saved parent
                        // state — close first (a no-op outside a region),
                        // then let the unwind continue.
                        close_subshell_region_if_ended!(command);
                        return Err(error);
                    }
                }
                if let Some(next_index) = self.skip_and_or_rhs(commands, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                close_subshell_region_if_ended!(command);
                continue;
            }

            if let Some(time_command) = &command.time_command {
                // rubash#1277: queue the timing report inside
                // execute_time_ast_command; it is flushed below, after
                // the ERR trap adjudication (GNU's order).
                self.defer_list_time_report = true;
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
                    Err(ExecuteError::IoError(error)) => {
                        // rubash#440: a closed output (SIGPIPE analogue) is a
                        // write error for this command — GNU's parent shell
                        // ignores SIGPIPE, reports the failed write and keeps
                        // running; it never aborts the surrounding list.
                        self.report_io_error_routed(command, &error)?;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        // An unwind: GNU's jump leaves execute_time_command
                        // before its print — drop the queued report.
                        self.defer_list_time_report = false;
                        self.pending_list_time_reports.clear();
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                        // GNU: `return N` inside a `( )` subshell ends the
                        // child with status N (return.def:57-76
                        // return_builtin — the child's own return_catch), the
                        // parent observes a plain status-N completion and the
                        // function continues after the region. Absorb at the
                        // boundary (rubash#720 CR round 2).
                        handle_exit_code!(code, command)
                    }
                    Err(error) => {
                        // CR round 2 (rubash#720): an unwind crossing a flat
                        // region boundary must not leak the saved parent
                        // state — close first (a no-op outside a region),
                        // then let the unwind continue. The queued time
                        // report dies with the unwind, as in GNU.
                        self.defer_list_time_report = false;
                        self.pending_list_time_reports.clear();
                        close_subshell_region_if_ended!(command);
                        return Err(error);
                    }
                }
                // GNU execute_cmd.c: the ERR trap and the errexit jump run
                // for a completed `time` command exactly as for a pipeline
                // (the branch below) — this branch used to skip both, so
                // `set -e; time f` with f failing, and a pipefail-failing
                // pipeline under `time`, never exited (rubash#1277). The
                // timed command's own `!` (time_command.inverted, already
                // folded into exit_code by execute_time_ast_command)
                // exempts the command like an outer inversion.
                self.maybe_run_error_trap(command)?;
                // rubash#1277: flush the queued timing report(s) AFTER the
                // ERR trap (GNU: the trap fires inside the inner
                // execute_command, execute_time_command prints after it
                // returns) and BEFORE the errexit jump below (GNU still
                // prints the report for a `set -e`-fatal timed command).
                self.defer_list_time_report = false;
                let queued_time_reports = std::mem::take(&mut self.pending_list_time_reports);
                for (started, posix_format) in queued_time_reports {
                    self.print_time_shell(posix_format, started);
                }
                if self.errexit_enabled()
                    && self.errexit_is_active()
                    && self.suppress_errexit == 0
                    && self.exit_code != 0
                    && !command.inverted
                    && !time_command.inverted
                    && command.and_or().is_none()
                {
                    // DE-06 (design 9.3; PLAN row 12): the errexit
                    // decision for a timed command is made here —
                    // park a selected `set -e` filter before the
                    // exit path runs.
                    self.debugger_note_exception_site(
                        crate::debugger::ExceptionKind::Errexit,
                        self.exit_code,
                        None,
                    )?;
                    return Err(ExecuteError::ExitCode(self.exit_code));
                }
                if let Some(next_index) = self.skip_and_or_rhs(commands, index) {
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
                        self.report_io_error_routed(command, &error)?;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                        // GNU: `return N` inside a `( )` subshell ends the
                        // child with status N (return.def:57-76
                        // return_builtin — the child's own return_catch), the
                        // parent observes a plain status-N completion and the
                        // function continues after the region. Absorb at the
                        // boundary (rubash#720 CR round 2).
                        handle_exit_code!(code, command)
                    }
                    Err(error) => {
                        // CR round 2 (rubash#720): an unwind crossing a flat
                        // region boundary must not leak the saved parent
                        // state — close first (a no-op outside a region),
                        // then let the unwind continue.
                        close_subshell_region_if_ended!(command);
                        return Err(error);
                    }
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
                    Err(ExecuteError::IoError(error)) => {
                        // rubash#440: a closed output (SIGPIPE analogue) is a
                        // write error for this command — GNU's parent shell
                        // ignores SIGPIPE, reports the failed write and keeps
                        // running; it never aborts the surrounding list.
                        self.report_io_error_routed(command, &error)?;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                        // GNU: `return N` inside a `( )` subshell ends the
                        // child with status N (return.def:57-76
                        // return_builtin — the child's own return_catch), the
                        // parent observes a plain status-N completion and the
                        // function continues after the region. Absorb at the
                        // boundary (rubash#720 CR round 2).
                        handle_exit_code!(code, command)
                    }
                    Err(error) => {
                        // CR round 2 (rubash#720): an unwind crossing a flat
                        // region boundary must not leak the saved parent
                        // state — close first (a no-op outside a region),
                        // then let the unwind continue.
                        close_subshell_region_if_ended!(command);
                        return Err(error);
                    }
                }
                if let Some(next_index) = self.skip_and_or_rhs(commands, index) {
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
                    Err(ExecuteError::IoError(error)) => {
                        // rubash#440: a closed output (SIGPIPE analogue) is a
                        // write error for this command — GNU's parent shell
                        // ignores SIGPIPE, reports the failed write and keeps
                        // running; it never aborts the surrounding list.
                        self.report_io_error_routed(command, &error)?;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::LastpipeExit(code)) if subshell_state.is_some() => {
                        // Issue #74 (G2): `exit N` in a lastpipe stage runs in
                        // the current shell (shopt -s lastpipe), so it must
                        // exit the current shell — not just set the pipeline's
                        // exit status. Propagate to the top level. Inside a
                        // flat `( )` region the "current shell" is the region's
                        // forked stand-in: GNU's child dies with status N and
                        // the parent observes a plain completion
                        // (execute_in_subshell), so absorb at the boundary.
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::LastpipeExit(code)) => {
                        self.exit_code = code;
                        return Err(ExecuteError::ExitCode(code));
                    }
                    Err(error) => {
                        // CR round 2 (rubash#720): an unwind crossing the
                        // region boundary must not leak the saved parent
                        // state — close first, then let the unwind continue.
                        close_subshell_region_if_ended!(command);
                        return Err(error);
                    }
                }
                if command.inverted {
                    // rubash#846: pipeline-level `!` always flips (GNU
                    // execute_cmd.c:763 wait-side — `! true | cat > bad` is
                    // 0). A stage's own redirect failure may have marked
                    // compound_redirect_failed; the pipeline owns the
                    // adjudicated status from here, so consume the mark.
                    self.take_compound_redirect_failed();
                    self.exit_code = invert_exit_status(self.exit_code);
                }
                // GNU order: the ERR trap for a failing pipeline
                // (run_error_trap callers, execute_cmd.c:773/874/1000/1166)
                // fires BEFORE the errexit jump in execute_command_internal
                // (execute_cmd.c:624) — `set -e; trap 'echo E' ERR; false`
                // prints E and exits 1, never dying silently (rubash#344).
                self.maybe_run_error_trap(command)?;
                if self.errexit_enabled()
                    && self.errexit_is_active()
                    && self.suppress_errexit == 0
                    && self.exit_code != 0
                    && !command.inverted
                    && command.and_or().is_none()
                {
                    // DE-06 (design 9.3; PLAN row 12): the errexit
                    // decision — park a selected `set -e` filter
                    // before the exit path runs.
                    self.debugger_note_exception_site(
                        crate::debugger::ExceptionKind::Errexit,
                        self.exit_code,
                        None,
                    )?;
                    return Err(ExecuteError::ExitCode(self.exit_code));
                }
                if let Some(next_index) = self.skip_and_or_rhs(commands, index) {
                    index = next_index;
                } else {
                    index += 1;
                }
                close_subshell_region_if_ended!(command);
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
                        // GNU expr.c evalerror -> jump_to_top_level
                        // (DISCARD): a fatal expansion failure inside an
                        // &&/|| chain discards the rest of the current
                        // list, not just the chain (rubash#1240). In a
                        // nested list the error keeps unwinding to the
                        // discard boundary; only there is the remainder
                        // of the line skipped (mirroring the
                        // plain-command DISCARD skip below).
                        if !self.at_discard_boundary() {
                            return Err(ExecuteError::ExpansionFailure(code));
                        }
                        let failed_line = command
                            .logical_line
                            .or(command.line)
                            .filter(|line| *line != 0);
                        if failed_line.is_some() {
                            while let Some(next) = commands.get(index + 1) {
                                let next_logical = next.logical_line.or(next.line);
                                if next_logical == failed_line && !next.subshell_end {
                                    index += 1;
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    Err(ExecuteError::IoError(error)) => {
                        // rubash#440: a closed output (SIGPIPE analogue) is a
                        // write error for this command — GNU's parent shell
                        // ignores SIGPIPE, reports the failed write and keeps
                        // running; it never aborts the surrounding list.
                        self.report_io_error_routed(command, &error)?;
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        handle_exit_code!(code, command)
                    }
                    Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                        // GNU: `return N` inside a `( )` subshell longjmps to
                        // the child's own return_catch and the child EXITS N
                        // (return.def:57-76 return_builtin; the parent sees a
                        // plain status-N completion and the function continues
                        // after the region — the node-form boundary documents
                        // the same absorption). Absorb here so the unwind
                        // neither crosses the boundary nor leaks the saved
                        // parent state (rubash#720 CR round 2, form 3 probe).
                        handle_exit_code!(code, command)
                    }
                    Err(error) => {
                        // CR round 2 (rubash#720): an unwind crossing the
                        // region boundary must not leak the saved parent
                        // state — close first, then let the unwind continue.
                        close_subshell_region_if_ended!(command);
                        return Err(error);
                    }
                }
                close_subshell_region_if_ended!(command);
                index += 1;
                continue;
            }

            let brace_result = self.execute_brace_group_pipeline(command);
            match brace_result {
                Ok(true) => {
                    if let Some(next_index) = self.skip_and_or_rhs(commands, index) {
                        index = next_index;
                    } else {
                        index += 1;
                    }
                    close_subshell_region_if_ended!(command);
                    continue;
                }
                Ok(false) => {}
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
                    // GNU expr.c evalerror -> jump_to_top_level
                    // (DISCARD): a fatal expansion failure inside a brace
                    // group discards the rest of the current list, not
                    // just the group (rubash#1240). In a nested list the
                    // error keeps unwinding to the discard boundary; only
                    // there is the remainder of the line skipped
                    // (mirroring the plain-command DISCARD skip below).
                    if !self.at_discard_boundary() {
                        return Err(ExecuteError::ExpansionFailure(code));
                    }
                    let failed_line = command
                        .logical_line
                        .or(command.line)
                        .filter(|line| *line != 0);
                    if failed_line.is_some() {
                        while let Some(next) = commands.get(index + 1) {
                            let next_logical = next.logical_line.or(next.line);
                            if next_logical == failed_line && !next.subshell_end {
                                index += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    // The absorb must END this command's dispatch.
                    // Falling through to the simple/execution dispatches
                    // re-runs the group through execute_command; with the
                    // DISCARD armed (rubash#1318, and the evalerror arming
                    // generally), the re-run's body list silently skips
                    // the failing line and completes with status 0,
                    // clobbering the failure status recorded above
                    // (`{ z=1; }` left $? = 0 for the next line; GNU 1).
                    // Mirror the plain-command arm's errexit promotion,
                    // then consume the command like the Ok(true) arm.
                    // The arithmetic evalerror class is exempt from the
                    // promotion there and here (rubash#1518).
                    if self.errexit_enabled()
                        && self.errexit_is_active()
                        && self.suppress_errexit == 0
                        && self.exit_code != 0
                        && !self.evalerror_arith_pending.get()
                        && !command.inverted
                        && command.and_or().is_none()
                    {
                        // DE-06 (design 9.3; PLAN row 12): the
                        // errexit decision — park a selected
                        // `set -e` filter before the exit path runs.
                        self.debugger_note_exception_site(
                            crate::debugger::ExceptionKind::Errexit,
                            self.exit_code,
                            None,
                        )?;
                        return Err(ExecuteError::ExitCode(self.exit_code));
                    }
                    close_subshell_region_if_ended!(command);
                    index += 1;
                    continue;
                }
                Err(ExecuteError::IoError(error)) => {
                    // rubash#440: a closed output (SIGPIPE analogue) is this
                    // command's write failure — GNU's parent shell ignores
                    // SIGPIPE, reports the failed write with status 1 and
                    // keeps executing the list.
                    self.report_io_error_routed(command, &error)?;
                }
                Err(ExecuteError::ExitCode(code)) => {
                    handle_exit_code!(code, command)
                }
                Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                    // GNU: `return N` inside a `( )` subshell ends the child
                    // with status N (return.def:57-76; the child's own
                    // return_catch), the parent sees a plain completion and
                    // the function continues after the region. Absorb at the
                    // boundary — see the and_or_list twin above (rubash#720
                    // CR round 2).
                    handle_exit_code!(code, command)
                }
                Err(error) => {
                    // CR round 2 (rubash#720): close the region before the
                    // unwind crosses it — the saved parent state must not
                    // leak (brace bodies close flat regions too:
                    // `f() ( { false; } )`).
                    close_subshell_region_if_ended!(command);
                    return Err(error);
                }
            }

            let simple_result = self.execute_simple_pipeline(commands, index);
            match simple_result {
                Ok(Some(next_index)) => {
                    index = next_index;
                    close_subshell_region_if_ended!(command);
                    continue;
                }
                Ok(None) => {}
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
                Err(ExecuteError::IoError(error)) => {
                    // rubash#440: a closed output (SIGPIPE analogue) is this
                    // command's write failure — GNU's parent shell ignores
                    // SIGPIPE, reports the failed write with status 1 and
                    // keeps executing the list.
                    self.report_io_error_routed(command, &error)?;
                }
                Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                    // GNU: `return N` inside a `( )` subshell ends the child
                    // with status N (return.def:57-76 return_builtin — the
                    // child's own return_catch), the parent observes a plain
                    // status-N completion and the function continues after
                    // the region. Absorb at the boundary (rubash#720 CR
                    // round 2).
                    handle_exit_code!(code, command)
                }
                Err(error) => {
                    // CR round 2 (rubash#720): an unwind crossing a flat
                    // region boundary must not leak the saved parent state —
                    // close first (a no-op outside a region), then let the
                    // unwind continue.
                    close_subshell_region_if_ended!(command);
                    return Err(error);
                }
            }

            drop(_t_chain);
            // rubash#1525: the absorbed-write marker belongs to one
            // command execution; a flag left by a command whose own
            // write site was skipped (a pipeline stage) must not
            // suppress this command's completion write.
            self.pipestatus_write_absorbed.set(false);
            let execution_result = if command.inverted || command.and_or().is_some() {
                self.with_errexit_suppressed(|executor| executor.execute_command(command))
            } else {
                self.execute_command(command)
            };
            match execution_result {
                Ok(()) => {}
                Err(ExecuteError::Break(_) | ExecuteError::Continue(_))
                    if self.shell_state.loop_depth == 0 =>
                {
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
                // rubash#1240: the absorption must happen only in a
                // DISCARD-boundary list — the reader top, or an eval'd
                // string's own top list (eval_list_depth). A brace-group
                // body or an &&/|| element list is the SAME flat list in
                // GNU (execute_connection / cm_group walk one LIST), so
                // absorbing there — as this arm used to — let the
                // enclosing line's remaining commands run
                // (`{ x=$((1/0)); }; echo after` printed `after`).
                // Nested lists now fall to the catch-all and unwind.
                Err(ExecuteError::ExpansionFailure(code))
                    if self.shell_state.loop_depth == 0
                        && self.shell_state.function_depth == 0
                        && subshell_state.is_none()
                        && !self.inside_compound_condition.get()
                        && self.at_discard_boundary() =>
                {
                    // rubash#1542: inside a command-substitution body the
                    // DISCARD catch of parse_and_execute (builtins/
                    // evalstring.c:423-426) folds the pre-set EX_BADUSAGE 2
                    // of a failed `${var:=word}` bind to EXECUTION_FAILURE
                    // — `x=$(echo ${v:=x})` on a readonly `v` substitutes
                    // with status 1, not 2 (see discard_fold_frame).
                    // rubash#1563: an eval/source body's catch is the same
                    // parse_and_execute catch — fold there too
                    // (in_parse_and_execute_frame), e.g. a DISCARD inside
                    // a brace group or function called from the string.
                    let code = if code == 2
                        && (self.discard_fold_frame() || self.in_parse_and_execute_frame())
                    {
                        1
                    } else {
                        code
                    };
                    self.exit_code = code;
                    // rubash#1563: at an eval/source boundary inside a
                    // subshell environment, GNU's parse_and_execute
                    // DISCARD arm folds and RE-JUMPS instead of
                    // returning (evalstring.c:427-450
                    // should_jump_to_top_level): the enclosing body is
                    // abandoned with the folded status. Absorbing here
                    // would let a command-substitution body continue
                    // past a failed eval/source (`x=$(eval ': ${v:=x}'
                    // || true; echo IN)` prints IN in the old code, GNU
                    // substitutes nothing and the body dies at 1).
                    if self.eval_list_depth.get() == Some(self.evalerror_exec_depth.get())
                        && self.in_subshell_environment()
                    {
                        return Err(ExecuteError::ExpansionFailure(code));
                    }
                    // GNU 5.3.20: under `set -e` a bad-substitution or
                    // failglob word-expansion failure exits the script
                    // (probe 2026-09-02: `set -e; echo ${#:}; echo after`
                    // stops after the bad substitution; same for
                    // `set -e; echo nope*` with failglob). An ARITHMETIC
                    // evalerror is a different class: expr.c's evalerror
                    // jumps DISCARD directly, bypassing the errexit
                    // epilogue in every context — `set -e` assignment,
                    // argument, subscript, for-list and case-word forms
                    // all discard the command and continue (rubash#1518,
                    // probes 2026-10-25). The 2026-09-02 comment here
                    // lumped arithmetic into the exiting classes from a
                    // single-line probe, where line-abandon and exit are
                    // indistinguishable; that was the #1518 regression.
                    if self.errexit_enabled()
                        && self.errexit_is_active()
                        && self.suppress_errexit == 0
                        && self.exit_code != 0
                        && !self.evalerror_arith_pending.get()
                        && !command.inverted
                        && command.and_or().is_none()
                    {
                        // DE-06 (design 9.3; PLAN row 12): the
                        // errexit decision — park a selected
                        // `set -e` filter before the exit path runs.
                        self.debugger_note_exception_site(
                            crate::debugger::ExceptionKind::Errexit,
                            self.exit_code,
                            None,
                        )?;
                        return Err(ExecuteError::ExitCode(self.exit_code));
                    }
                    // GNU eval.c:111 (reader_loop case DISCARD): the rest of
                    // the current command LIST — the whole LOGICAL line,
                    // including commands joined onto it by backslash-newline
                    // continuations — is abandoned. `logical_line` carries
                    // that list boundary; comparing physical `line` would
                    // stop the skip at the first continuation join and run
                    // commands GNU discarded (rubash#411).
                    let failed_line = command
                        .logical_line
                        .or(command.line)
                        .filter(|line| *line != 0);
                    if failed_line.is_some() {
                        while let Some(next) = commands.get(index + 1) {
                            let next_logical = next.logical_line.or(next.line);
                            if next_logical == failed_line && !next.subshell_end {
                                index += 1;
                            } else {
                                break;
                            }
                        }
                    }
                }
                Err(ExecuteError::IoError(error)) => {
                    // rubash#440: a closed output (SIGPIPE analogue) is this
                    // command's write failure — GNU's parent shell ignores
                    // SIGPIPE, reports the failed write with status 1 and
                    // keeps executing the list; only errexit promotes it.
                    self.report_io_error_routed(command, &error)?;
                    // A compound's own failed redirection (the wrapper's
                    // open() -> ExecuteError::IoError) is THIS level's
                    // failure, not a body-inherited aggregate: GNU fires
                    // the ERR trap for it (redir12.sub:
                    // `trap 'echo got error ERR' ERR;
                    // while ...; done </dev/null > unwritable-file`
                    // prints "got error ERR", redir.right). The #507
                    // pass-through exemption only covers the body's
                    // inherited status — without this fire the gated
                    // ERR site below skipped the trap and the redir
                    // snapshot lost its "got error ERR" byte (PR #553
                    // follow-up). maybe_run_error_trap's own gates keep
                    // the &&/||, ! and errtrace exclusions.
                    if command_status_produced_by_body(command) {
                        self.maybe_run_error_trap(command)?;
                    }
                    if self.errexit_enabled()
                        && self.errexit_is_active()
                        && self.suppress_errexit == 0
                        && !command.inverted
                        && command.and_or().is_none()
                    {
                        // DE-06 (design 9.3; PLAN row 12): the
                        // errexit decision — park a selected
                        // `set -e` filter before the exit path runs.
                        self.debugger_note_exception_site(
                            crate::debugger::ExceptionKind::Errexit,
                            1,
                            None,
                        )?;
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
                    // rubash#1542: same parse_and_execute fold as the
                    // discard-boundary arm — a `( )` region inside a
                    // command-substitution body that dies on a failed
                    // `${var:=word}` bind reports 1, not the pre-set 2.
                    let code = if code == 2 && self.discard_fold_frame() {
                        1
                    } else {
                        code
                    };
                    self.exit_code = code;
                    while index + 1 < commands.len() && !commands[index + 1].subshell_end {
                        index += 1;
                    }
                    if index + 1 < commands.len() {
                        index += 1;
                    }
                    if let Some(saved_state) = subshell_state.take() {
                        self.restore_flat_subshell(saved_state);
                        if let Some(saved) = subshell_fd_table.take() {
                            self.fd_table = saved;
                        }
                        if let Some(saved) = subshell_timed_stdin.take() {
                            self.timed_stdin = saved;
                        }
                        if let Some(saved) = saved_fork_collapse.take() {
                            self.subshell_fork_collapse.set(saved);
                        }
                        // rubash#720 form 2: the expansion-failure abort dies
                        // with the forked subshell (status `code`) and the
                        // parent's :769 wait-side ERR adjudication judges the
                        // status (rubash#439).
                        self.close_flat_region_err_adjudication(commands, index)?;
                    }
                    // A malformed subshell can leave the command list with
                    // no closing marker.  In that case there is no boundary
                    // to advance to; continuing would execute the same
                    // failing command forever (and repeatedly print the
                    // heredoc EOF diagnostic).
                    let has_subshell_end = commands[index + 1..]
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
                        && crate::builtins::set::shell_option_enabled(
                            &self.shell_state.env_vars,
                            "errexit",
                        )
                        && self.suppress_errexit == 0
                        && !command.inverted
                        && command.and_or().is_none()
                    {
                        return Err(ExecuteError::ExitCode(self.exit_code));
                    }
                    continue;
                }
                Err(ExecuteError::Return(code)) if subshell_state.is_some() => {
                    // GNU: `return N` inside a `( )` subshell ends the child
                    // with status N (return.def:57-76 return_builtin — the
                    // child's own return_catch), the parent observes a plain
                    // status-N completion and the function continues after
                    // the region. Absorb at the boundary (rubash#720 CR
                    // round 2).
                    handle_exit_code!(code, command)
                }
                Err(error) => {
                    // CR round 2 (rubash#720): an unwind crossing a flat
                    // region boundary must not leak the saved parent state —
                    // close first (a no-op outside a region), then let the
                    // unwind continue.
                    close_subshell_region_if_ended!(command);
                    return Err(error);
                }
            }
            if command.inverted {
                // rubash#846 (GNU execute_cmd.c:854-883 vs :1204): a
                // compound's OWN redirect failure suppresses the flip (mark
                // left set for an outer `! !` wrapper; boundaries consume).
                if !self.compound_redirect_failed.get() {
                    self.exit_code = invert_exit_status(self.exit_code);
                }
            }
            // GNU execute_cmd.c:1210-1224: PIPESTATUS is set from the exit
            // status only for simple commands and the `((...))`/`[[...]]`
            // compounds (cm_arith/cm_cond) — `if`/`while`/`until`/`for`/
            // `case`/etc. leave the last pipeline's PIPESTATUS untouched
            // (rubash#1383 #1384).
            let is_pipestatus_compound =
                command.arithmetic_command.is_some() || command.conditional_command.is_some();
            let is_non_pipestatus_compound = command.if_command.is_some()
                || command.loop_command.is_some()
                || command.for_command.is_some()
                || command.case_command.is_some()
                || command.select_command.is_some()
                || command.subshell_command.is_some()
                || command.function_command.is_some()
                || command.brace_group.is_some()
                || command.coproc_command.is_some();
            if !is_non_pipestatus_compound || is_pipestatus_compound {
                // rubash#1525: a command whose tempenv PIPESTATUS
                // absorbed the completion write inside the dispatch
                // funnel has already spent this write.
                if !self.pipestatus_write_absorbed.replace(false) {
                    self.set_pipestatus([self.exit_code]);
                }
            }

            // GNU order (rubash#344): the ERR trap fires for the failed
            // simple command (execute_cmd.c:1000/1166 run_error_trap)
            // BEFORE the errexit jump at execute_command_internal
            // (execute_cmd.c:624): `set -e; trap 'echo E' ERR; false`
            // prints E and exits 1.
            //
            // A pass-through compound (issue #507) is skipped: the body's
            // inner commands already had their ERR look in the nested walk,
            // and an exempt inner failure (`false && true` arm) must not
            // fire ERR a second time at the aggregate (GNU 5.3.0:
            // `trap 'echo E' ERR; if true; then false && true; fi; echo R`
            // prints only R).
            //
            // A subshell command running under the rubash#720 fork-collapse
            // is skipped too: GNU dispatched it in place inside the
            // already-forked parent subshell (execute_cmd.c:1799-1806 marks
            // the single-subshell-command body CMD_NO_FORK; execute_cmd.c:
            // 662-663 skips make_child/wait_for), so this frame performs no
            // :769 wait-side ERR adjudication for it — the failure is
            // observed one frame up (`( (false) )` fires 2, not 3).
            if !command_status_produced_by_body(command)
                && !(command.subshell_command.is_some() && self.subshell_fork_collapse.get())
            {
                self.maybe_run_error_trap(command)?;
            }

            // GNU execute_cmd.c:1004-1018 (simple command errexit): after a
            // simple command finishes, if errexit is active and the command's
            // exit status is non-zero, jump_to_top_level (ERREXIT) exits the
            // shell.
            //
            // The same pass-through compounds (issue #507) are exempt here:
            // `set -e; if true; then false && true; fi; echo R` must print R
            // with rc 0 — the if's nonzero aggregate is inherited from the
            // exempt arm, not this command's own failure.
            if !command_status_produced_by_body(command)
                && self.errexit_enabled()
                && self.errexit_is_active()
                && self.suppress_errexit == 0
                && self.exit_code != 0
                && !self.evalerror_arith_pending.get()
                && !command.inverted
                && command.and_or().is_none()
            {
                // DE-06 (design 9.3; PLAN row 12): the errexit
                // decision — park a selected `set -e` filter
                // before the exit path runs.
                self.debugger_note_exception_site(
                    crate::debugger::ExceptionKind::Errexit,
                    self.exit_code,
                    None,
                )?;
                return Err(ExecuteError::ExitCode(self.exit_code));
            }

            if command.subshell_end {
                // A pending evalerror was raised inside the subshell that just
                // ended: GNU's jump_to_top_level(DISCARD) cannot cross the
                // forked process boundary, so the abort dies with the
                // subshell (`( a[$bad]=v ); echo after` runs `after`).
                self.evalerror_pending.set(false);
                self.nounset_force_eof.set(false);
                self.evalerror_subscript_pending.set(false);
                self.evalerror_failglob_pending.set(false);
                self.evalerror_arith_pending.set(false);
                self.evalerror_exit_toomany_pending.set(false);
                self.evalerror_line.set(None);
                if let Some(saved_state) = subshell_state.take() {
                    self.restore_flat_subshell(saved_state);
                    if let Some(saved) = subshell_fd_table.take() {
                        self.fd_table = saved;
                    }
                    if let Some(saved) = subshell_timed_stdin.take() {
                        self.timed_stdin = saved;
                    }
                    if let Some(saved) = saved_fork_collapse.take() {
                        self.subshell_fork_collapse.set(saved);
                    }
                    // rubash#720 form 2: the normal-path region close — GNU's
                    // parent waits for the forked subshell here and runs the
                    // :769 ERR adjudication on the waited status.
                    self.close_flat_region_err_adjudication(commands, index)?;
                }
            }

            if let Some(next_index) = self.skip_and_or_rhs(commands, index) {
                index = next_index;
            } else {
                index += 1;
            }
        }
        // Same abort as the loop-top check, for a substitution parse error
        // raised by the final command in the list. Backtick-origin errors
        // stay contained (issue #515).
        if self.last_command_substitution_parse_error.get() {
            if self
                .last_command_substitution_parse_error_backtick
                .replace(false)
            {
                self.last_command_substitution_parse_error.set(false);
            } else {
                self.mark_parse_error();
                return Err(ExecuteError::ExitCode(2));
            }
        }
        self.run_pending_signal_traps()?;
        Ok(())
    }

    pub(in crate::executor) fn execute_inverted_pipeline(
        &mut self,
        commands: &[CommandNode],
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        // TODO(parse.y/execute_cmd.c/execute_pipeline): Bash attaches `!` to a
        // pipeline command node and executes the whole pipeline before status
        // inversion. Rubash still flattens pipelines into simple commands, so
        // cover the small status-only cases used by upstream invert.tests.
        let Some(command) = commands.get(index) else {
            return Ok(None);
        };

        if !command.inverted || command.pipe.is_none() {
            return Ok(None);
        }

        let mut pipeline = vec![command];
        let mut end = index;
        while commands
            .get(end)
            .is_some_and(|command| command.pipe.is_some())
        {
            end += 1;
            let Some(next) = commands.get(end) else {
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
            // rubash#846: pipeline-level `!` always flips (GNU
            // execute_cmd.c:763 wait-side); consume any stage-set mark — the
            // pipeline owns the adjudicated status from here.
            self.take_compound_redirect_failed();
            self.exit_code = invert_exit_status(self.exit_code);
            return Ok(Some(end + 1));
        }

        // Keep the flattened fallback for parser forms that do not yet expose a
        // PipelineCommand, but do not use it when the real pipeline is available.
        for command in pipeline {
            self.execute_command(command)?;
        }
        // rubash#846: same pipeline-level always-flip rule as above.
        self.take_compound_redirect_failed();
        self.exit_code = invert_exit_status(self.exit_code);
        Ok(Some(end + 1))
    }

    /// rubash#720 form 2: the wait-side ERR adjudication for a closing flat
    /// `f() ( list )` region. GNU runs it in the frame that waited for the
    /// subshell fork — wait_for then run_error_trap keyed on the subshell
    /// command itself (execute_cmd.c:756-778; the site checks user_subshell,
    /// was_error_trap, ignore_return, invert and the status, with no
    /// &&/|| exemption). The forkless region has no subshell node: the
    /// opener command stands in for the gates (its invert/and_or context is
    /// the region's own parse context), the text is the region's print_cmd
    /// rendering, and the line is the opener's close-paren end_line
    /// (execute_cmd.c:696). Callers run it after restore_flat_subshell, so
    /// the trap table is the parent's — GNU's child died with its own.
    fn close_flat_region_err_adjudication(
        &mut self,
        commands: &[CommandNode],
        closer_index: usize,
    ) -> Result<(), ExecuteError> {
        let last = commands.len().saturating_sub(1);
        let Some(open_index) = (0..=closer_index.min(last))
            .rev()
            .find(|i| commands[*i].subshell)
        else {
            return Ok(());
        };
        let opener = &commands[open_index];
        let body = &commands[open_index..=closer_index.min(last)];
        let text = crate::executor::command_text::flat_subshell_region_source_text(body);
        let line = opener.end_line;
        self.maybe_run_error_trap_for_flat_region(opener, text, line)
    }
}

pub(in crate::executor) fn is_closed_output_io_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe || error.raw_os_error() == Some(232)
}

/// True for a compound whose body runs IN THIS PROCESS through the same
/// walker — `if`/`elif`/`else`, `while`/`until`, `for` (word and arithmetic),
/// `case`, `select` — so the body's aggregate status passes through it.
///
/// Issue #507: the walker below is also the nested walk a body runs under,
/// and every inner command already had its own errexit/ERR adjudication
/// there. A non-exempt inner failure unwinds as `ExecuteError::ExitCode`
/// before the compound ever returns (`set -e; if true; then false; fi` exits
/// via the inner `false`), so a nonzero aggregate that survives to the
/// compound level was produced by an EXEMPT inner failure — a `&&`/`||`
/// short-circuit arm or a `!` pipeline (POSIX XCU 2.14: -e is ignored for
/// "any command of an AND-OR list other than the last" and for `!`
/// pipelines). Re-adjudicating that inherited status here killed the shell
/// after `set -e; if true; then false && true; fi` where GNU prints `R` and
/// exits 0 — GNU's execute_if_command (execute_cmd.c:5200 family) hands the
/// body's status up without an errexit check of its own: one adjudication
/// per failure, at the site that produced it.
///
/// Deliberately NOT pass-through — the aggregate is a fresh status this
/// level owns, and the errexit/ERR checks stay live for them:
/// - `subshell_command` — GNU forks it; nothing adjudicated inside the
///   child, so the parent sees the wait status fresh (`set -e; (false &&
///   true)` exits 1; execute_command's post-compound errexit check owns it).
/// - `conditional_command` (`[[ ]]`) and `arithmetic_command` (`(( ))`) —
///   these ARE the innermost failing command (`set -e; [[ 1 = 2 ]]` exits 1).
/// - a function call — the call's status belongs to the call (`f() { false
///   && true; }; f` exits 1).
/// - a compound's own failed redirection — the wrapper's failure, not an
///   inherited one (`set -e; { :; } > /nonexistent/x` exits 1).
fn command_status_produced_by_body(command: &CommandNode) -> bool {
    command.if_command.is_some()
        || command.loop_command.is_some()
        || command.for_command.is_some()
        || command.case_command.is_some()
        || command.select_command.is_some()
}
