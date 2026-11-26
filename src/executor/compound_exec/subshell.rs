use super::control_flow::compound_command_has_redirects;
use super::BackgroundStdio;
use crate::executor::*;

/// The subshell command node with genuinely NUMBERED (fd > 2) stdio
/// mirror fields stripped. Compound redirections on fd 0-2 keep their
/// stdio mirror fields; only a numbered redirect is stripped, because
/// those propagate exclusively through the ordered `redirects` list
/// (the mirror fields are fd-blind — a `3>&1` can land in
/// `redirect_out`). Treating the explicit fd digit of `2>`/`1>&2` as
/// "numbered" stripped fd-1/fd-2 redirections too, defeating both the
/// entry preparation and `cd`'s redirected-io binding: a subshell
/// `( cd /no_such ) 2>e` leaked the diagnostic to the real stderr
/// (rubash#699 R1.4). Building this node deep-clones the command
/// (body subtree included), so callers build it only on paths that
/// consume it (rubash#2246).
fn subshell_stdio_redirect_cmd(redirect_cmd: &CommandNode) -> CommandNode {
    let mut stdio_redirect_cmd = redirect_cmd.clone();
    let is_numbered = |redirect: &Redirect| {
        let fd = redirect.fd.unwrap_or_else(|| {
            redirect
                .operator
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        });
        fd > 2
    };
    if stdio_redirect_cmd
        .redirect_in
        .as_deref()
        .is_some_and(is_numbered)
    {
        stdio_redirect_cmd.redirect_in = None;
    }
    if stdio_redirect_cmd
        .redirect_out
        .as_deref()
        .is_some_and(is_numbered)
    {
        stdio_redirect_cmd.redirect_out = None;
    }
    if stdio_redirect_cmd
        .append
        .as_deref()
        .is_some_and(is_numbered)
    {
        stdio_redirect_cmd.append = None;
    }
    if stdio_redirect_cmd
        .redirect_err
        .as_deref()
        .is_some_and(is_numbered)
    {
        stdio_redirect_cmd.redirect_err = None;
    }
    if stdio_redirect_cmd
        .redirect_err_append
        .as_deref()
        .is_some_and(is_numbered)
    {
        stdio_redirect_cmd.redirect_err_append = None;
    }
    stdio_redirect_cmd
}

impl Executor {
    pub(in crate::executor) fn execute_subshell_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        subshell_command: &SubshellCommand,
    ) -> Result<(), ExecuteError> {
        // GNU execute_cmd.c:1576 execute_in_subshell: the forked child's
        // mutable state is a whole-copy of the parent's. The in-place body
        // gets the same boundary from a ShellState clone — assignments,
        // aliases, functions, set --, IFS, env-carried traps, positional
        // params, and job bookkeeping all restore wholesale at the end.
        let saved_state = self.shell_state.clone_for_child_save();
        // rubash#1551: the fork bracket also covers the process-global
        // hash-table mirror (see executor::path::hashed_cache_fork_guard).
        let _hashed_fork =
            crate::executor::path::hashed_cache_fork_guard(&self.shell_state.env_vars);
        // The fd table is executor state, not ShellState: a forked child's
        // descriptor table is a copy (execute_in_subshell after make_child),
        // so `( exec 3<&- )` cannot close the parent's fd 3. Rc-shared
        // entries still alias the same open file description — reads in the
        // subshell advance the shared offset, matching fork().
        let saved_fd_table = self.fd_table.saved_table();
        // rubash#4181: GNU execute_cmd.c:1713-1715 — the forked child
        // closes every coproc endpoint (coproc_closeall) before its
        // own redirects bind, so the region's live table drops them
        // here; the saved table above restores the parent's endpoints
        // at the boundary.
        self.fd_table.close_coproc_endpoints_for_subshell();
        let saved_depth = self.shell_state.subshell_depth.get();
        // GNU execute_cmd.c runs `( list )` via execute_in_subshell ->
        // make_child: the forked child owns its own cwd, so a `cd` in the
        // body never reaches the parent. The body here runs in place on
        // this executor; since G-02 the cwd is a ShellState field, so the
        // saved_state clone below carries the boundary (niubash#100).
        // rubash#439: the body's word-expansion error mark dies with the
        // forked child (GNU's DISCARD jump cannot cross the fork boundary) —
        // the parent judges the subshell only by its exit STATUS, so
        // `(x=${!bad})` still fires the parent's ERR trap.
        let saved_expansion_failed = self.command_expansion_failed.get();
        self.command_expansion_failed.set(false);
        // rubash#1012: the body runs in place on this executor, but GNU
        // forked it — an `exec` in the body replaces only the child's image
        // (setting the child's exec_replaced latch), while the parent keeps
        // its own EXIT trap. Save/restore the latch like the fd table so a
        // subshell exec cannot disarm the parent's trap.
        let saved_exec_replaced = self.exec_replaced;
        // The exit-trap latch is executor state, not ShellState: the
        // body's own EXIT trap runs below (run_exit_trap sets the
        // latch, trap_exec.rs run_exit_trap_action) and without a
        // save/restore the parent's EXIT trap would read as already
        // fired and never run at the real shell exit. GNU's forked
        // child owns a private copy of the "exit trap already ran"
        // fact (trap.c run_exit_trap runs per-process), so the latch
        // restores with the rest of the boundary state.
        let saved_exit_trap_ran = self.exit_trap_ran.get();
        crate::builtins::trap::reset_for_subshell_child(&mut self.shell_state.env_vars);
        self.shell_state.subshell_depth.set(saved_depth + 1);
        // rubash#3605: GNU execute_cmd.c:1731-1739 (execute_in_subshell)
        // ASSIGNS stdin_redir for a user subshell — `stdin_redir =
        // stdin_redirects(redirects) || pipe_in != NO_PIPE` — an assign,
        // not an OR: a nested `( )` with no fd-0 redirect CLEARS the
        // enclosing structure's flag (`{ ( cat & wait ); } <file` feeds
        // the async cat /dev/null), while the ( )'s own `< file` or its
        // being a pipe-fed element's own dispatch sets it even when the
        // parent's flag was 0. `redirects` here is the ( )'s own list —
        // the same list do_redirections is about to consume (GNU reads
        // it before the :1759-1764 consumption). element_pipe_stdin
        // carries the element child's `pipe_in != NO_PIPE` for exactly
        // this dispatch; consume it so nested ( )s don't inherit the
        // outer element's pipe.
        let element_pipe_stdin = self.shell_state.element_pipe_stdin.replace(false);
        self.shell_state
            .stdin_redir
            .set(cmd.redirects.iter().any(redirect_updates_stdin_redir) || element_pipe_stdin);
        // execute_in_subshell (execute_cmd.c:1636-1637): the region is
        // "neither login nor interactive" — record the login reset
        // alongside the depth bump; saved_state's wholesale restore
        // reverts it at the boundary (rubash#2438).
        self.shell_state.subshell_login_cleared.set(true);
        // jobs.c:5356-5366 without_job_control (via
        // execute_in_subshell): the job_control variable is erased
        // with the login state; the monitor flag letter stays in `$-`
        // and a `set -m` inside re-arms the variable through
        // flags.c:257 (rubash#2487). saved_state's wholesale restore
        // reverts it at the boundary.
        self.shell_state.job_control.set(false);
        // rubash#2897: without_job_control's other half — it also runs
        // delete_all_jobs(0) (jobs.c:5363), so the forked child starts
        // with an EMPTY job list and no claim on the parent's
        // children. This in-place region shares the executor, so set
        // the parent's child-process state aside (restored by
        // restore_flat_subshell) and clear the job table; the saved
        // state's wholesale restore reverts the table at the boundary.
        self.push_subshell_process_save(saved_depth);
        self.shell_state.job_table.clear_for_subshell();
        // rubash#1493: GNU's signal-death notice for a job killed inside
        // the parens names the WHOLE subshell command (jobs.c:4625
        // pretty_print_job on the dead job). Record the node's print_cmd
        // text for the exec-death queue site in trap_exec.rs; the entry
        // clone saved the parent's value, so the wholesale boundary
        // restore reverts this automatically.
        self.shell_state.subshell_source_text =
            Some(crate::executor::command_text::bash_command_source_text(cmd));
        // rubash#1115: the forked child reseeds RANDOM
        // (variables.c:1418-1423 get_random_number); saved_state keeps
        // the parent's sequence for the wholesale restore.
        self.shell_state.random_state.reseed_fresh();
        // rubash#2851: bank the enclosing loop depth across the
        // boundary — see the flat-region entry (ast_exec.rs) and the
        // field doc on ShellState. Accumulated AFTER the saved clone
        // above (the wholesale restore reverts it) and BEFORE the
        // compat-gated reset below; NOT compat-gated.
        self.shell_state.subshell_saved_loop_depth += self.shell_state.loop_depth + 1;
        // GNU execute_cmd.c:1641-1642 (execute_in_subshell): the
        // loop_level reset is compat-gated — `if
        // (shell_compatibility_level > 44) loop_level = 0;`. At compat44
        // and below the child inherits the depth as
        // inherited_loop_depth (the command-substitution transfer), so
        // break/continue validate against it but unwind only the
        // child's own list (audit B4-F07).
        if crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) > 44 {
            self.shell_state.loop_depth = 0;
            self.shell_state.inherited_loop_depth = 0;
        } else {
            self.shell_state.inherited_loop_depth += self.shell_state.loop_depth;
            self.shell_state.loop_depth = 0;
        }

        // RESTRICTED_SHELL (#804): the subshell's redirects apply inside
        // the child before the body (do_redirections precedes the list), so
        // the refusal lands before any splice, anchor or target creation —
        // nothing is created and the body never runs, like GNU's child
        // dying at do_redirections with status 1.
        if compound_command_has_redirects(cmd) {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }

        // Assignment prefix (`x=1 (cmd)`, #452): GNU apply_shell_assignments
        // runs the prefix inside the forked child, so the values live only
        // for the body; the whole-state save above restores the parent
        // afterwards. Scalar assignments only — compound/array-element
        // prefixes have no GNU verdict in the audit corpus yet and never
        // reach the parser's subshell gate.
        for (name, value) in &cmd.assignments {
            let result = self.expand_assignment_value_result_with_raw(name, value, None);
            if !self.apply_shell_assignment(name, result.value, None) {
                self.exit_code = 1;
                return Err(ExecuteError::ExpansionFailure(1));
            }
        }

        // GNU executes the `( list )` body by POINTER (execute_in_subshell
        // walks the Subshell->command the parser allocated once; make_cmd.c
        // never copies). The clone below exists only to feed the one
        // remaining mutation: materialize_compound_output_process_substitutions
        // rewrites a target only when it starts with `>(` (both arms
        // require that prefix — the
        // compound_redirect_needs_procsub_rewrite gate). The per-leaf
        // splice that used to force a body copy is gone (rubash#2246):
        // prepare_compound_output_redirects performs the entry side
        // effects and with_command_input_redirects binds the fd table,
        // so the stored slice always runs as-is.
        let mut redirect_owned;
        let group_outputs;
        let redirect_cmd: &CommandNode =
            if crate::executor::pipeline_exec::compound_redirect_needs_procsub_rewrite(cmd) {
                redirect_owned = cmd.clone();
                group_outputs =
                    self.materialize_compound_output_process_substitutions(&mut redirect_owned)?;
                &redirect_owned
            } else {
                group_outputs = Vec::new();
                cmd
            };
        // Numbered redirects are duplicated in parser stdio fields. Keep
        // only true stdio fields in the entry preparation below; numbered
        // redirects propagate exclusively through the ordered `redirects`
        // list (bound by with_command_input_redirects). Any stdio slot
        // present means the preparation has arms to run.
        let prepare_needed = cmd.redirect_out.is_some()
            || cmd.append.is_some()
            || cmd.redirect_err.is_some()
            || cmd.redirect_err_append.is_some();
        if prepare_needed {
            let stdio_redirect_cmd = subshell_stdio_redirect_cmd(redirect_cmd);
            // rubash#846 (GNU execute_cmd.c:1759-1762): the subshell's own
            // redirect failure in the forked child exits inverted — the
            // parent wait-side (:763) flips it back, netting status 1 with
            // no flip. Mark it so an enclosing `!` suppresses its flip.
            if let Err(error) = self.prepare_compound_output_redirects(&stdio_redirect_cmd) {
                if matches!(error, ExecuteError::IoError(_)) {
                    self.note_compound_redirect_failure();
                }
                // rubash#940: the subshell boundary owns its saved state on
                // every exit path — GNU's forked child dies with its own
                // copy and nothing leaks back (execute_cmd.c:1576). The
                // early prepare-failure return must not skip the restore
                // below and leak subshell_depth + the child trap reset into
                // the parent.
                self.restore_flat_subshell(saved_state);
                self.fd_table = saved_fd_table;
                self.command_expansion_failed.set(saved_expansion_failed);
                self.exec_replaced = saved_exec_replaced;
                return Err(error);
            }
        }
        let body: &[CommandNode] = &subshell_command.body;
        // rubash#720 form 1: GNU execute_cmd.c:1799-1806 (execute_in_subshell)
        // marks a user subshell's WHOLE body CMD_NO_FORK when it is a single
        // simple or subshell command; execute_cmd.c:662-663 runs a subshell
        // body so marked IN PLACE inside the already-forked child — no
        // second fork, no wait_for, and therefore no :769 run_error_trap in
        // that frame. With `( (false) )` the middle frame never observes the
        // status, so only the innermost `false` and the outermost waiter
        // fire ERR (GNU 5.3.0: 2 fires, not 3). The flag scopes exactly the
        // body execution of a single-subshell-command body.
        let body_fork_collapsed = body.len() == 1 && body[0].subshell_command.is_some();
        let saved_fork_collapse = if body_fork_collapsed {
            Some(self.subshell_fork_collapse.replace(true))
        } else {
            None
        };
        // GNU execute_cmd.c resolves subshell redirections in the parent
        // context before the body runs: expand and anchor relative file
        // targets now, or a `cd` in the body relocates them (niubash#118).
        // Numbered output redirects bind fd-table slots for the body's
        // duration through open_compound_output_redirects inside
        // with_command_input_redirects (redir.c do_redirection_internal).
        // GNU execute_cmd.c:696 SET_LINE_NUMBER(command->value.Subshell->line):
        // the subshell body runs under the `)` line as its ambient
        // line_number — inner while/if/group diagnostics report it, while
        // inner simple commands stamp and restore their own.
        let subshell_line = cmd.end_line.or(cmd.line);
        // The subshell's EXIT trap runs inside the scoped fd bindings: GNU's
        // execute_in_subshell forks, applies do_redirections to the child's
        // real descriptors, and runs run_exit_trap before the child exits —
        // the redirections are never undone in the child, so the trap writes
        // through the same open file descriptions as the body
        // (execute_cmd.c:1576-1763, redir.c:767-955). Re-binding `>file`
        // after the scope would re-open and truncate it, clobbering the
        // body's output.
        let mut body_status: Option<i32> = None;
        let mut trap_result: Option<Result<i32, ExecuteError>> = None;
        // rubash#2246: storage for the EXIT trap's stdio node, built
        // lazily inside the body closure (see the trap call site).
        let mut trap_stdio_redirect_cmd: Option<CommandNode> = None;
        let result = self.with_loop_fd_heredocs(cmd, |executor| {
            executor.with_ambient_line(subshell_line, |executor| {
                executor.with_command_input_redirects(cmd, |executor| {
                    // rubash#815 profiler: the flat-subshell frame (GNU
                    // execute_cmd.c:1576 execute_in_subshell). The guard
                    // spans the EXIT trap too — GNU runs it inside the
                    // forked child.
                    let _profiler_subshell = crate::executor::profiler::enter_subshell();
                    // rubash#3932: GNU marks the subshell body's
                    // qualifying command before dispatching it
                    // (execute_cmd.c:1756 optimize_subshell_command ->
                    // :1803 CMD_NO_FORK), so the body's tail plain spawn
                    // exports live-1 while earlier body nodes keep the
                    // forked face. The walk consumes the candidate
                    // one-shot and arms the spawn stamp at the tail's
                    // dispatch; the seam re-checks the trap state at
                    // spawn time (the forked child reset the parent's
                    // traps, trap.c:1480, and traps armed inside the
                    // body block the mark). Redirects block the mark:
                    // optimize_subshell_command consults
                    // should_optimize_fork with subshell==0, whose
                    // redirects==0 arm holds (evalstring.c:106-117).
                    executor.nofork_spawn_face_allows_redirects.set(false);
                    let timed_free = executor.timed_dispatch_depth.get() == 0;
                    executor
                        .nofork_spawn_body_candidate_pending
                        .set(timed_free && Self::nofork_spawn_body_tail_ok(body, false));
                    let body_result = executor.execute_ast_inner(body);
                    // GNU execute_cmd.c execute_in_subshell: expr.c
                    // evalerror's jump_to_top_level(DISCARD) reaches only the
                    // forked subshell's own top level — the abort dies with
                    // the subshell (status 1) and cannot discard parent
                    // commands (`( a[$bad]=v ); echo after` prints `after` —
                    // verified GNU 5.3).
                    executor.evalerror_pending.set(false);
                    executor.nounset_force_eof.set(false);
                    executor.evalerror_subscript_pending.set(false);
                    executor.evalerror_failglob_pending.set(false);
                    executor.evalerror_line.set(None);
                    // rubash#1541: a no_args DISCARD raised inside the
                    // region, when the region itself sits inside a
                    // command-substitution body, folds to 1: the forked
                    // region inherits the body's parse_and_execute frame,
                    // whose DISCARD catch forces EXECUTION_FAILURE
                    // (builtins/evalstring.c:423-426). At the main level
                    // the same region keeps the pre-set EX_BADUSAGE 2
                    // (execute_in_subshell, execute_cmd.c:1821-1825).
                    // rubash#2439: a `-c` process is the same fold class
                    // one level up — run_one_command runs the string
                    // through parse_and_execute (shell.c:1477), and GNU
                    // folds the region's status to 1 there too
                    // (`bash -c '(suspend foo); echo after rc=$?'`
                    // prints rc=1, while the identical script file
                    // prints rc=2 — verified GNU 5.3.20). Consuming the
                    // pending here also keeps it from surviving the
                    // region into execute_ast's depth-0 tail conversion,
                    // which would otherwise convert the WHOLE `-c` run
                    // to ExitCode(1) after the parent list continued.
                    if executor.evalerror_exit_toomany_pending.get()
                        && (executor.discard_fold_frame()
                            || executor.shell_state.env_vars.contains_key("__RUBASH_IS_C"))
                    {
                        executor.evalerror_exit_toomany_pending.set(false);
                        executor.exit_code = 1;
                    }
                    // Bash runs a subshell with errexit active; a failing
                    // command exits the subshell with that status but the
                    // parent script continues. Catch ExitCode errors at the
                    // subshell boundary. `return N` inside a subshell
                    // likewise only ends the subshell with status N: the
                    // forked child longjmps to its own copy of
                    // execute_function's return_catch (return.def
                    // return_builtin) and exits N, so the function continues
                    // with $? = N (func.tests: `( return 5 ); status=$?`
                    // prints 5, 5).
                    let status = match body_result {
                        Ok(()) => executor.exit_code,
                        Err(ExecuteError::ExitCode(code))
                        | Err(ExecuteError::ExpansionFailure(code))
                        | Err(ExecuteError::FatalFunctionError(code))
                        | Err(ExecuteError::Return(code)) => code,
                        Err(error) => return Err(error),
                    };
                    body_status = Some(status);
                    // rubash#1461: this layer was killed by a signal (a
                    // `kill` to its fake BASHPID, intercepted in
                    // execute_kill). A GNU process killed by a signal runs
                    // no EXIT trap; consume the death mark here and let
                    // the code after the state restore queue the parent's
                    // jobs.c notice instead.
                    if executor.subshell_signal_death == Some((saved_depth + 1, status - 128))
                        && status >= 128
                    {
                        trap_result = None;
                        return Ok(());
                    }
                    // rubash#2836 slice 4 (count shape rubash#2851):
                    // the stop-unwind's subshell arm. A child stopped by
                    // SIGTSTP inside this subshell set
                    // `stopped_loop_breaking = saved + live` (the
                    // single-child and pipeline job-control waits); the
                    // body has abandoned its remaining commands, and the
                    // subshell itself has NOT exited — so its EXIT trap
                    // must not run (GNU: a stopped subshell has not
                    // exited). Consume exactly one level here — the
                    // entry banked +1 for this boundary — so the parent
                    // list is unaffected unless an enclosing loop owns
                    // the remaining levels (same-line commands run,
                    // oracle-verified; a stop inside a subshell in a
                    // loop abandons the loop, jobs.c:4341-4343).
                    if executor.consume_stopped_loop_breaking() {
                        trap_result = None;
                        return Ok(());
                    }
                    // GNU execute_cmd.c runs the subshell's own EXIT trap
                    // (trap.c run_exit_trap) while the subshell state is
                    // still live: `( trap "echo T" EXIT; echo body )` prints
                    // body then T (niubash#70). reset_for_subshell cleared
                    // the inherited traps at entry, so a pending EXIT here
                    // can only have been registered by the body itself; a
                    // trap action that calls exit N replaces the subshell
                    // status (bash exit_shell semantics).
                    // rubash#2246: the stdio node is consumed only when an
                    // EXIT trap action actually runs —
                    // run_exit_trap_for_status_with_output_redirects
                    // early-outs on exec_replaced and on a missing/empty
                    // EXIT action before touching it — so build it here,
                    // behind the same two verdicts (get_trap_action peeks
                    // with take_exit_trap's reset-disposition guard),
                    // instead of deep-cloning the whole command node for
                    // every subshell execution.
                    let trap_redirect_arg = if !executor.exec_replaced
                        && crate::builtins::trap::active_trap_action_ref(
                            &executor.shell_state.env_vars,
                            "EXIT",
                        )
                        .is_some()
                    {
                        trap_stdio_redirect_cmd = Some(subshell_stdio_redirect_cmd(redirect_cmd));
                        trap_stdio_redirect_cmd.as_ref()
                    } else {
                        None
                    };
                    trap_result =
                        Some(executor.run_exit_trap_for_status_with_output_redirects(
                            status,
                            trap_redirect_arg,
                        ));
                    Ok(())
                })
            })
        });
        let status = match result {
            Ok(()) => body_status.unwrap_or(self.exit_code),
            Err(error) => {
                if let Some(saved) = saved_fork_collapse {
                    self.subshell_fork_collapse.set(saved);
                }
                self.restore_flat_subshell(saved_state.clone());
                self.fd_table = saved_fd_table.clone();
                self.command_expansion_failed.set(saved_expansion_failed);
                self.exec_replaced = saved_exec_replaced;
                self.exit_trap_ran.set(saved_exit_trap_ran);
                return Err(error);
            }
        };
        let status = match trap_result {
            Some(Ok(trap_status)) => trap_status,
            Some(Err(error)) => {
                if let Some(saved) = saved_fork_collapse {
                    self.subshell_fork_collapse.set(saved);
                }
                self.restore_flat_subshell(saved_state.clone());
                self.fd_table = saved_fd_table.clone();
                self.command_expansion_failed.set(saved_expansion_failed);
                self.exec_replaced = saved_exec_replaced;
                self.exit_trap_ran.set(saved_exit_trap_ran);
                return Err(error);
            }
            None => status,
        };

        if let Some(saved) = saved_fork_collapse {
            self.subshell_fork_collapse.set(saved);
        }
        self.restore_flat_subshell(saved_state);
        self.fd_table = saved_fd_table;
        self.command_expansion_failed.set(saved_expansion_failed);
        self.exec_replaced = saved_exec_replaced;
        self.exit_trap_ran.set(saved_exit_trap_ran);
        // rubash#4181: the parent WAITS for the forked subshell child
        // (execute_cmd.c execute_in_subshell -> wait_for), and that
        // wait is a coproc disposal point (the notify_and_cleanup
        // inside wait_for, jobs.c:3494 -> coproc_reap) — a coproc
        // death recorded before the `( )` is retired before the next
        // command expands. The in-place region performs no real wait,
        // so record the emulated one, like the other whole-command
        // emulations (see note_emulated_foreground_wait).
        self.note_emulated_foreground_wait();
        // rubash#1461: this layer died by a signal (see the trap skip in
        // the body closure above) — queue the parent-side death notice
        // with the layer's fake BASHPID, like a reaped job's (jobs.c:4625
        // notify_of_job_status via note_child_signal_death).
        if let Some((depth, signal)) = self.subshell_signal_death {
            if depth == saved_depth + 1 {
                self.subshell_signal_death = None;
                let layer_pid = std::process::id().saturating_add(
                    u32::try_from(depth.saturating_sub(self.bashpid_origin_depth))
                        .unwrap_or(u32::MAX),
                );
                let text = self.signal_notice_command_text(cmd);
                self.note_child_signal_death(layer_pid, signal, text, true);
            }
        }
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        // rubash#846 (GNU execute_cmd.c:1808-1833 execute_in_subshell): the
        // forked child lifts CMD_INVERT_RETURN off the direct body and flips
        // the adjudicated result — `( ! C>bad )` is 0 while a top-level
        // `! C>bad` is 1. Only the single-inverted-command body qualifies
        // (tcom is the direct body; a `!` nested in a list keeps its flag
        // into execute_command_internal and hits the :854 early return).
        // Consume the mark when flipping; otherwise leave it for an
        // enclosing `!` (`! ( C>bad )` suppresses) or the flat-loop backstop.
        let mut status = status;
        if self.compound_redirect_failed.get()
            && body.len() == 1
            && (body[0].inverted_command.is_some() || body[0].inverted)
        {
            self.take_compound_redirect_failed();
            status = invert_exit_status(status);
        } else {
            // Body-set marks die with the forked child (GNU
            // execute_in_subshell): the parent judges the region by status
            // only. (The region's OWN redirect failure returns before this
            // point, leaving the mark for an enclosing `!`.)
            self.compound_redirect_failed.set(false);
        }
        self.exit_code = status;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    /// Mirror of GNU do_redirections (redir.c) restricted to fds 0-2 for the
    /// spawned background subshell: walk the command's redirections in parse
    /// order so `2>&1 >/dev/null` keeps stderr on the inherited pipe while
    /// `>/dev/null 2>&1` releases both, exactly as GNU's sequential dup2
    /// ordering does. Best-effort: a target that cannot be resolved here
    /// (virtual fds, failed opens, expansion side effects) is left in the
    /// returned `consumed` map as false so it stays in the child's `-c`
    /// source, where the child re-runs it and reports the same diagnostic
    /// GNU's subshell would.
    ///
    /// rubash#1862: one class is NOT resolved here even when its open
    /// would succeed — FIFO targets. GNU's async child applies the
    /// redirections on its own descriptors after the fork
    /// (execute_cmd.c:1761-1763 execute_in_subshell), so a FIFO open's
    /// rendezvous wait happens in the child; resolving it in this
    /// parent walk deadlocks the shell's own execution thread when the
    /// peer is a command that has not run yet (`( ... ) > fifo &` with
    /// the reader still to come). A FIFO redirect therefore defers its
    /// whole fd (background_stdio_defer_fifo_fds), and the child
    /// replays that fd's redirect subsequence in order.
    pub(super) fn resolve_background_stdio(
        &mut self,
        command: &CommandNode,
    ) -> ([BackgroundStdio; 3], Vec<bool>, bool) {
        use crate::parser::RedirectKind;
        // GNU execute_cmd.c:2837-2841 (execute_connection '&') + :595
        // async_redirect_stdin: CMD_STDIN_REDIR is set — and the spawned
        // child dups /dev/null onto fd 0 — only when the shell is in a
        // subshell or lacks job control AND the enclosing control structure
        // did not redirect stdin. The global stdin_redir (execute_cmd.c:828,
        // cleared per reader command at eval.c:181) decides: `{ read; } &`
        // inside `while ... done <<EOF` inherits the heredoc, while a bare
        // `sleep 5 &` gets /dev/null.
        let job_control =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor");
        let subshell_environment = self.shell_state.subshell_depth.get() > 0
            || self.shell_state.in_command_substitution.get();
        let mut stdio: [BackgroundStdio; 3] = [
            if (subshell_environment || !job_control) && !self.shell_state.stdin_redir.get() {
                BackgroundStdio::Null
            } else {
                self.inherited_async_stdin()
            },
            BackgroundStdio::Inherit,
            BackgroundStdio::Inherit,
        ];
        // rubash#169: GNU's substitution child holds a real capture pipe as
        // fd 1 (subst.c:7143 command_substitute), and an async child forked
        // inside (`{ ...; } & wait` — nvm `nvm ls`) inherits THAT pipe, so
        // its output is captured. In-process captures are Vec buffers an
        // OS-spawned child cannot write into, so `Inherit` (duplicating the
        // process stdout) leaks the output past the capture. When a capture
        // scope is live, replace the inherited fd 1 with a duplicate of the
        // scope's shared background pipe — every background child of the
        // scope shares one write end and the kernel merges their writes, and
        // the drain at reap/capture-end reads it back in that order. The
        // command's own fd-1 redirects below still override this, and
        // `2>&1`-style dups copy it, matching the sequential dup2 ordering a
        // forked GNU child would apply on top of the inherited pipe.
        // rubash#2660: the base stdio above starts from the PROCESS
        // descriptors, but an enclosing compound command's
        // redirections live in this executor's fd table — the flat
        // body never touches the process's real descriptors. GNU's
        // async child is a fork of the shell AS REDIRECTED
        // (execute_cmd.c:2837 execute_connection '&' forks after the
        // compound applied do_redirections to its own descriptors),
        // so it is born holding the compound's bindings. Seed slots
        // 1/2 from the live fd table — the same source a foreground
        // spawn inherits from — before the capture override below:
        // an explicit binding (e.g. `>/dev/null`) beats the capture
        // pipe, exactly like GNU's ordering (the subshell's redirect
        // rebinds the capture child's fd 1 before the body forks).
        for slot in 1..=2u32 {
            if let Some(seeded) = self.background_stdio_from_fd_table(slot) {
                stdio[slot as usize] = seeded;
            }
        }
        let mut captured_pipe_writer = false;
        if matches!(stdio[1], BackgroundStdio::Inherit) {
            if let Some(write) = crate::executor::shell_options::background_capture_write_end() {
                stdio[1] = BackgroundStdio::File(write);
                // Registration is conservative: a later fd-1 redirect may drop
                // the pipe for THIS child, but reaping it then only drains an
                // empty pipe — never another child's bytes (the drain fires when
                // the whole writer set is reaped).
                captured_pipe_writer = true;
            }
        }
        // rubash#1862 bookkeeping for the FIFO deferral further down:
        // the stdio the child spawns with before any of the command's
        // own redirects (base_stdio), plus per-slot stacks of the
        // redirects this walk has consumed — (redirect index, slot
        // value after the write, dup source fd, staged-heredoc flag) —
        // so a deferral can roll each slot back to exactly the value
        // its surviving writes imply.
        let base_stdio = stdio.clone();
        let mut slot_writes: [Vec<(usize, BackgroundStdio, Option<u32>, bool)>; 3] =
            [Vec::new(), Vec::new(), Vec::new()];
        let mut deferred_fd = [false; 3];
        let mut consumed = vec![false; command.redirects.len()];

        for (index, redirect) in command.redirects.iter().enumerate() {
            match redirect.kind {
                // GNU here_document_to_fd (redir.c): a forked async subshell
                // finds the heredoc already staged on a real fd — the body
                // text cannot ride the child's `-c` source, so expand and
                // materialize it into a temp file here and hand the child
                // that descriptor, exactly like do_redirections does in the
                // subshell.
                RedirectKind::HereDoc | RedirectKind::HereString => {
                    let fd = redirect.fd.unwrap_or(0);
                    if fd > 2 || redirect.fd_var.is_some() {
                        continue;
                    }
                    // rubash#1862: a heredoc on an already-deferred fd
                    // stays in the child's source with the rest of that
                    // fd's redirect subsequence.
                    if deferred_fd[fd as usize] {
                        continue;
                    }
                    match self.materialize_async_stdin_body(command, index) {
                        Some((file, _path)) => {
                            let value = BackgroundStdio::File(file);
                            stdio[fd as usize] = value.clone();
                            slot_writes[fd as usize].push((index, value, None, true));
                            consumed[index] = true;
                        }
                        None => continue,
                    }
                    continue;
                }
                _ => {}
            }
            // `{fd}>file` dynamic fd bindings and `<(cmd)`/`>(cmd)` process
            // substitutions are owned by the child's executor, which knows
            // how to bind them; do not mistake them for plain fd 0-2 files.
            if redirect.fd_var.is_some()
                || redirect.target.starts_with("<(")
                || redirect.target.starts_with(">(")
            {
                continue;
            }
            let fd = redirect.fd.unwrap_or(match redirect.kind {
                RedirectKind::Input
                | RedirectKind::DuplicateInput
                | RedirectKind::CloseInput
                | RedirectKind::ReadWrite => 0,
                _ => 1,
            });
            if fd > 2 {
                continue;
            }
            // rubash#1862: once an fd is deferred to the child (a FIFO
            // redirect below), every later redirect on that fd stays
            // unconsumed as well — the child replays the fd's whole
            // redirect subsequence in order on top of the spawn stdio,
            // which is the only ordering that stays GNU-faithful.
            if deferred_fd[fd as usize] {
                continue;
            }
            // rubash#4099: a redirect word carrying a parameter-
            // expansion error (nounset hit, `${x:?}`, bad
            // substitution) is never resolved here — the silent
            // expander returns a bogus target for an erroring word
            // (the `:?` message text becomes a created file, a
            // nounset word becomes an unopenable empty string), the
            // redirect is consumed, and the diagnostic GNU's async
            // child reports (execute_cmd.c:1761-1763
            // execute_in_subshell applies do_redirections after the
            // fork) is lost or displaced by a later word's error.
            // The word joins this walk's cannot-resolve class (see
            // the doc comment above): leave it unconsumed, and the
            // respawned child re-runs it through the ordinary reject
            // gate, which reports the first error in list order.
            // The child's diagnostic routes through fd 2 as bound
            // AT this list position, so fd 2's subsequence defers
            // with it (the rubash#1862 rollback): a later `2>file`
            // or `2>/dev/null` this walk already resolved into the
            // spawn stdio would otherwise hand the child a binding
            // GNU's child has not applied yet at the error point —
            // and a deferred `2>file` also keeps the parent from
            // creating a file GNU's child dies before reaching.
            if self.redirect_word_expansion_error(redirect).is_some() {
                Self::background_stdio_defer_fifo_fds(
                    &mut stdio,
                    &base_stdio,
                    &mut slot_writes,
                    &mut consumed,
                    &mut deferred_fd,
                    &[2],
                );
                continue;
            }
            let mut dup_source: Option<u32> = None;
            let resolved = match redirect.kind {
                RedirectKind::Input => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if redirect_target_fd(&target).is_some() {
                        continue;
                    } else if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        // rubash#1862: the FIFO open belongs to the
                        // child; this redirect stays in its source.
                        continue;
                    } else {
                        match self.open_input_redirect(&target) {
                            Ok(file) => BackgroundStdio::File(file),
                            Err(_) => continue,
                        }
                    }
                }
                RedirectKind::DuplicateInput | RedirectKind::DuplicateOutput => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if let Some(source_fd) = redirect_target_fd(&target) {
                        // `N>&M`: fd N ends up wherever fd M currently
                        // points — GNU's dup2 ordering semantics. Source fds
                        // above 2 are the child's own virtual fd table, so
                        // leave them for the child to resolve.
                        if source_fd > 2 {
                            continue;
                        }
                        // rubash#1862: a dup OF a deferred fd is
                        // replayed by the child too — this walk's slot
                        // for the source still holds the spawn value,
                        // not the FIFO the child will open onto it.
                        if deferred_fd[source_fd as usize] {
                            continue;
                        }
                        dup_source = Some(source_fd);
                        stdio[source_fd as usize].clone()
                    } else {
                        continue;
                    }
                }
                RedirectKind::CloseInput | RedirectKind::CloseOutput => BackgroundStdio::Null,
                RedirectKind::Output | RedirectKind::ClobberOutput => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if redirect_target_fd(&target).is_some() {
                        continue;
                    } else if is_null_device(&target) {
                        BackgroundStdio::Null
                    } else if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        // rubash#1862: the FIFO open belongs to the
                        // child; this redirect stays in its source.
                        continue;
                    } else {
                        match self.create_redirect_output_for(
                            Some(redirect),
                            &target,
                            redirect.clobber,
                        ) {
                            Ok(file) => BackgroundStdio::File(file),
                            Err(_) => continue,
                        }
                    }
                }
                RedirectKind::Append => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if redirect_target_fd(&target).is_some() {
                        continue;
                    } else if is_null_device(&target) {
                        BackgroundStdio::Null
                    } else if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        // rubash#1862: the FIFO open belongs to the
                        // child; this redirect stays in its source.
                        continue;
                    } else {
                        match open_redirect_append(
                            &self.session_path(self.redirect_target_filesystem_path(&target)),
                        ) {
                            Ok(file) => BackgroundStdio::File(file),
                            Err(_) => continue,
                        }
                    }
                }
                RedirectKind::ReadWrite => {
                    let target = self.expand_redirect_target(redirect);
                    let path = self.session_path(self.redirect_target_filesystem_path(&target));
                    // rubash#1862: keep the parent's hands off FIFO
                    // opens uniformly, even though `<>` itself would
                    // not block — the open belongs to the child.
                    if crate::executor::shell_options::redirect_path_is_fifo(&path)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        continue;
                    }
                    // niubash#519: CON rejects a read+write handle; the
                    // stdin side needs only its CONIN$ half.
                    let opened = if is_con_device_path(&path) {
                        File::open("CONIN$")
                    } else {
                        OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .open(&path)
                    };
                    match opened {
                        Ok(file) => BackgroundStdio::File(file),
                        Err(_) => continue,
                    }
                }
                RedirectKind::CombinedOutput | RedirectKind::CombinedAppend => {
                    // `&>file` / `&>>file`: one open, both fds share it.
                    // rubash#1862: fd 2 already deferred (fd 1 is gated
                    // by the loop-top check) defers the pair as well.
                    if deferred_fd[2] {
                        continue;
                    }
                    let target = self.expand_redirect_target(redirect);
                    if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[1, 2],
                        )
                    {
                        // The FIFO open belongs to the child; this
                        // redirect stays in its source.
                        continue;
                    }
                    let file = if is_null_device(&target) {
                        stdio[1] = BackgroundStdio::Null;
                        stdio[2] = BackgroundStdio::Null;
                        slot_writes[1].push((index, BackgroundStdio::Null, None, false));
                        slot_writes[2].push((index, BackgroundStdio::Null, None, false));
                        consumed[index] = true;
                        continue;
                    } else {
                        let opened = if redirect.kind == RedirectKind::CombinedAppend {
                            open_redirect_append(&self.session_path(shell_path_to_windows(
                                &target,
                                &self.shell_state.env_vars,
                            )))
                        } else {
                            self.create_redirect_output_for(Some(redirect), &target, false)
                        };
                        match opened {
                            Ok(file) => file,
                            Err(_) => continue,
                        }
                    };
                    stdio[1] = match file.try_clone() {
                        Ok(clone) => BackgroundStdio::File(clone),
                        Err(_) => continue,
                    };
                    stdio[2] = BackgroundStdio::File(file);
                    slot_writes[1].push((index, stdio[1].clone(), None, false));
                    slot_writes[2].push((index, stdio[2].clone(), None, false));
                    consumed[index] = true;
                    continue;
                }
                _ => continue,
            };
            stdio[fd as usize] = resolved.clone();
            slot_writes[fd as usize].push((index, resolved, dup_source, false));
            consumed[index] = true;
        }

        (stdio, consumed, captured_pipe_writer)
    }

    /// rubash#2660: the async spawn's base for one stdio slot, read
    /// from the executor's LIVE fd table — the bindings an enclosing
    /// compound command's redirections installed (`( ... ) >/dev/null`,
    /// `{ ...; } >file`, `exec >file`, a function's redirects). GNU's
    /// async child forks from the shell holding exactly these
    /// descriptors (execute_cmd.c:2837), so a file-backed binding
    /// hands the child a duplicate of the same open file description
    /// (shared offset, like dup2). Returns None to keep the caller's
    /// default (process inherit) when the slot carries no spawnable
    /// binding: unbound slots, the process-stdio markers naming the
    /// slot itself, and the in-process channel endpoints
    /// (process-substitution / pipe-channel writers), which a spawned
    /// child cannot hold — the same shapes the foreground faces serve
    /// in-process instead. A CLOSED slot maps to Null, matching this
    /// resolver's own treatment of the command's `>&-` redirects.
    fn background_stdio_from_fd_table(&self, fd: u32) -> Option<BackgroundStdio> {
        use crate::executor::fd_table::FdWriteEndpoint;
        if self.fd_table.is_closed(fd) {
            return Some(BackgroundStdio::Null);
        }
        let dup_file = |handle: crate::fd::HANDLE| -> Option<BackgroundStdio> {
            crate::fd::duplicate_handle(handle)
                .ok()
                .map(|dup| BackgroundStdio::File(crate::fd::handle_to_file(dup)))
        };
        match self.fd_table.write_endpoint(fd) {
            Some(FdWriteEndpoint::File(file_fd)) => dup_file(file_fd.handle),
            Some(FdWriteEndpoint::CoprocStdin { fd: file_fd, .. }) => dup_file(file_fd.handle),
            // The slot was dup'd from the sibling stdio slot (e.g.
            // `2>&1` at compound level): the marker names the process
            // descriptor as bound at entry, so hand the child a
            // duplicate of that live process descriptor.
            Some(FdWriteEndpoint::Stdout) if fd != 1 => dup_file(1),
            Some(FdWriteEndpoint::Stderr) if fd != 2 => dup_file(2),
            _ => None,
        }
    }

    /// True when a background redirect target resolves to an existing
    /// FIFO (rubash#1862): the one real open of such a target carries
    /// rendezvous semantics, so it must happen in the async child,
    /// never in the parent walk of resolve_background_stdio.
    fn background_redirect_target_is_fifo(&self, target: &str) -> bool {
        let path = self.session_path(shell_path_to_windows(target, &self.shell_state.env_vars));
        if crate::executor::shell_options::redirect_path_is_fifo(&path) {
            return true;
        }
        // rubash#3286: the joined-path check above misses from a
        // deleted cwd — the anchor's fstatat classifies there, and a
        // FIFO still defers its open to the child exactly as above
        // (rubash#1862: the parent must not take the rendezvous).
        #[cfg(unix)]
        {
            let fs_operand = self.redirect_target_filesystem_path(target);
            if fs_operand.is_relative() && self.anchor_operand_is_fifo(&fs_operand) == Some(true) {
                return true;
            }
        }
        false
    }

    /// Defer fds to the background child (rubash#1862): mark them
    /// deferred and roll back every redirect this walk already
    /// consumed for them — plus every consumed dup whose SOURCE is one
    /// of them — so the child replays each deferred fd's whole
    /// redirect subsequence, in order, on top of the spawn stdio, and
    /// performs the FIFO open itself. Slots keep the value their
    /// surviving writes imply (base_stdio when none survive). Returns
    /// false — deferring nothing — when a target fd already carries a
    /// staged heredoc: its parent-materialized temp file cannot be
    /// replayed from the child's source, so that fd mix keeps today's
    /// parent resolution.
    fn background_stdio_defer_fifo_fds(
        stdio: &mut [BackgroundStdio; 3],
        base_stdio: &[BackgroundStdio; 3],
        slot_writes: &mut [Vec<(usize, BackgroundStdio, Option<u32>, bool)>; 3],
        consumed: &mut [bool],
        deferred_fd: &mut [bool; 3],
        fds: &[usize],
    ) -> bool {
        for &fd in fds {
            if slot_writes[fd].iter().any(|entry| entry.3) {
                return false;
            }
        }
        for &fd in fds {
            deferred_fd[fd] = true;
        }
        for slot in 0..3 {
            let writes = &mut slot_writes[slot];
            let mut i = 0;
            while i < writes.len() {
                let entry_index = writes[i].0;
                let entry_source = writes[i].2;
                if fds.contains(&slot)
                    || entry_source.is_some_and(|source| fds.contains(&(source as usize)))
                {
                    consumed[entry_index] = false;
                    writes.remove(i);
                } else {
                    i += 1;
                }
            }
            stdio[slot] = writes
                .last()
                .map(|entry| entry.1.clone())
                .unwrap_or_else(|| base_stdio[slot].clone());
        }
        true
    }

    /// GNU make_child fork semantics for an async command that inherits
    /// fd 0 (stdin_redir set, or job-control shell outside a subshell):
    /// the child shares the parent's open file description, file offset
    /// included. Handle-backed endpoints map to DuplicateHandle dups, which
    /// share the file pointer exactly like a fork. Text endpoints and the
    /// FUNCTION_STDIN channel have no kernel object, so the unread tail is
    /// written to a temp file and fd 0 is moved onto it — the same trick
    /// GNU gets for free because here-documents already live in an
    /// unlinked temp file (redir.c here_document_to_fd).
    fn inherited_async_stdin(&mut self) -> BackgroundStdio {
        // FUNCTION_STDIN is the newest fd-0 authority when present: a
        // compound `done <<EOF` stages the body on the text channel while a
        // previous `exec 0<real` may still hold a File endpoint on slot 0 —
        // the virtual text wins, exactly like GNU's dup2 of the heredoc
        // temp file over the earlier descriptor.
        if let Some(remaining) = self.function_stdin_remaining() {
            let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(&remaining);
            match self.materialize_virtual_stdin_fd0(&bytes) {
                Some(file) => {
                    // fd 0 now owns a real endpoint; drop the text channel
                    // so no consumer double-sources it. The compound-command
                    // restore puts the parent's FUNCTION_STDIN back when the
                    // scope ends.
                    self.shell_state.env_vars.remove(FUNCTION_STDIN);
                    self.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
                    return BackgroundStdio::File(file);
                }
                None => return BackgroundStdio::Null,
            }
        }
        match self.fd_table.read_endpoint(0) {
            Some(FdReadEndpoint::File(file))
            | Some(FdReadEndpoint::CoprocStdout { fd: file, .. }) => {
                match crate::fd::duplicate_handle_inheritable(file.handle) {
                    Ok(dup) => BackgroundStdio::File(crate::fd::handle_to_file(dup)),
                    Err(_) => BackgroundStdio::Null,
                }
            }
            Some(FdReadEndpoint::Text(_)) | Some(FdReadEndpoint::ProcessSubstitution(_)) => {
                let bytes = self
                    .fd_table
                    .input_snapshot_bytes(0)
                    .map(|(data, offset)| data[offset.min(data.len())..].to_vec())
                    .unwrap_or_default();
                match self.materialize_virtual_stdin_fd0(&bytes) {
                    Some(file) => BackgroundStdio::File(file),
                    None => BackgroundStdio::Null,
                }
            }
            Some(FdReadEndpoint::InheritedProcessStdin) => BackgroundStdio::Inherit,
            // rubash#1937: a directory endpoint has no spawnable
            // handle; the null device is the closest state (the
            // child's reads see EOF instead of EISDIR — the
            // cross-boundary residual class).
            Some(FdReadEndpoint::Directory) => BackgroundStdio::Null,
            // fd 0 closed or absent: the closest spawnable state to GNU's
            // inherited-but-unreadable descriptor.
            None => BackgroundStdio::Null,
        }
    }

    /// Move fd 0 onto a real temp file holding `bytes`: the FileFd on the
    /// slot keeps the parent's read position in lockstep with any child's
    /// dup (shared file object), which is exactly what GNU's inherited fd
    /// gives a forked async subshell.
    fn materialize_virtual_stdin_fd0(&mut self, bytes: &[u8]) -> Option<std::fs::File> {
        let path = self.process_substitution_temp_path().ok()?;
        // rubash#2409: GNU's here_document_to_fd (redir.c:423) makes the
        // document "really temporary" (redir.c:548-551): the scratch
        // file is written through one handle, re-opened by name for
        // reading, and the name is unlinked before the descriptor is
        // ever used — the temp file has no directory entry for its
        // whole working life. The previous body wrote the file and
        // opened it but never unlinked, so every async subshell that
        // inherited a virtual fd 0 (FUNCTION_STDIN text channel or a
        // Text/ProcessSubstitution fd-0 endpoint) leaked a named
        // rubash-process-subst*.tmp into the temp dir — the #2395 leak
        // class on this face (the exec face's fix in
        // materialize_exec_stdin_bytes is the sibling). Mirror GNU:
        // scratch write handle, read re-open while it is alive,
        // immediate unlink on Unix; FILE_FLAG_DELETE_ON_CLOSE carries
        // the same lifetime on Windows (the re-opened handle keeps the
        // delete pending until fd 0 itself closes).
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_FLAG_DELETE_ON_CLOSE.
            options.custom_flags(0x0400_0000);
        }
        let mut scratch = options.open(&path).ok()?;
        {
            use std::io::Write as _;
            scratch.write_all(bytes).ok()?;
        }
        let file_fd = FileFd::open_read(path).ok()?;
        #[cfg(unix)]
        {
            // The descriptor stays valid after the name is unlinked.
            let _ = std::fs::remove_file(&file_fd.path);
        }
        drop(scratch);
        let dup = crate::fd::duplicate_handle_inheritable(file_fd.handle).ok()?;
        self.set_fd_input_file(0, file_fd, false);
        Some(crate::fd::handle_to_file(dup))
    }

    /// rubash#2681: both callers stage the body for a SPAWNED child
    /// (the async stdio builder and the spawn redirect replay), which
    /// is GNU's forked-child do_redirections context
    /// (execute_cmd.c:5522) — the body's expansion effects belong to
    /// the child and must not leak into the parent, so the expansion
    /// runs inside the fork boundary. A body already expanded at the
    /// do_redirections point rides as a Preexpanded carrier and reads
    /// back verbatim, so the boundary is inert for it.
    pub(in crate::executor) fn materialize_async_stdin_body(
        &mut self,
        command: &CommandNode,
        index: usize,
    ) -> Option<(std::fs::File, std::path::PathBuf)> {
        self.with_forked_heredoc_expansion(|executor| {
            executor.materialize_async_stdin_body_inner(command, index)
        })
    }

    /// Expand the body behind `command.redirects[index]` (a HereDoc or
    /// HereString entry) and stage it on a real temp file, mirroring GNU's
    /// here_document_to_fd so the spawned `-c` child reads an actual
    /// descriptor. `heredoc_redirects` is the parallel store for bodies; an
    /// entry correlates by fd, here-string flag, and ordinal among the
    /// matching `redirects` entries, with the fd-0 legacy fields
    /// (`heredoc`/`here_string` and their carriers) as fallback.
    fn materialize_async_stdin_body_inner(
        &mut self,
        command: &CommandNode,
        index: usize,
    ) -> Option<(std::fs::File, std::path::PathBuf)> {
        use crate::parser::RedirectKind;
        let redirect = &command.redirects[index];
        let want_string = matches!(redirect.kind, RedirectKind::HereString);
        let ordinal = command.redirects[..index]
            .iter()
            .filter(|entry| {
                matches!(entry.kind, RedirectKind::HereDoc | RedirectKind::HereString)
                    && matches!(entry.kind, RedirectKind::HereString) == want_string
                    && entry.fd == redirect.fd
            })
            .count();
        let expanded = command
            .heredoc_redirects
            .iter()
            .filter(|entry| entry.fd == redirect.fd && entry.here_string == want_string)
            .nth(ordinal)
            .map(|entry| self.expand_heredoc_redirect_text(entry))
            .or_else(|| {
                if redirect.fd.is_some() {
                    return None;
                }
                if want_string {
                    // rubash#2409 (same-wave residual): a here-string's
                    // bytes are the expanded word PLUS a trailing
                    // newline (GNU heredoc_expand for r_reading_string,
                    // redir.c:396-397; the correlated path above appends
                    // it in expand_heredoc_redirect_text). The legacy
                    // fallback returned the bare word, so every async
                    // here-string (`cat <<<word &`) lost its newline.
                    let mut expanded = if command.here_string_carrier.is_some() {
                        self.expand_here_string_mut_from_carrier(&command.here_string_carrier)
                    } else {
                        let body = command.here_string.clone()?;
                        self.expand_here_string_mut(&body)
                    };
                    expanded.push('\n');
                    Some(expanded)
                } else if command.heredoc_body.is_some() {
                    Some(self.expand_heredoc_body_mut_from_carrier(&command.heredoc_body))
                } else {
                    let body = command.heredoc.clone()?;
                    Some(self.expand_heredoc_body_mut(&body))
                }
            })?;
        let path = self.process_substitution_temp_path().ok()?;
        // rubash#2409: same leak class as materialize_virtual_stdin_fd0
        // above — the carrier was created by name and never unlinked,
        // so every background command carrying its own heredoc /
        // here-string redirect left a rubash-process-subst*.tmp in the
        // temp dir. GNU's here_document_to_fd (redir.c:423) unlinks
        // the name before the descriptor is used (redir.c:548-551);
        // mirror it here so both callers (the background-stdio face,
        // which discarded the path, and the redirect-replay face,
        // which keeps it only as the FileFd name record) inherit the
        // anonymous lifetime. The returned path is a name record only
        // from here on.
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_FLAG_DELETE_ON_CLOSE.
            options.custom_flags(0x0400_0000);
        }
        let mut scratch = options.open(&path).ok()?;
        {
            use std::io::Write as _;
            scratch.write_all(expanded.as_bytes()).ok()?;
        }
        let file = std::fs::File::open(&path).ok()?;
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&path);
        }
        drop(scratch);
        Some((file, path))
    }
}
