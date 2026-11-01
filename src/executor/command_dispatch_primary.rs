use super::command_registry::{primary_dispatch_slot, PrimarySlot};
use super::*;

impl Executor {
    pub(in crate::executor) fn execute_primary_builtin_command(
        &mut self,
        cmd: &CommandNode,
        word: &str,
    ) -> Result<Option<Result<(), ExecuteError>>, ExecuteError> {
        // GNU Bash has no `sudo` builtin (issue #759 item 4): without an
        // embedding-host elevation handler the word takes the PATH route
        // like any external command; with one, the host-capability
        // builtin intercepts it (introspection keeps reporting external,
        // the docs/builtins.md §3 hidden-builtin contract). Not a
        // registry claim (rubash#2036 S3): the cfg(windows) handler
        // pre-check stays ahead of the table, with the historical
        // fall-through to `Ok(None)` when no handler is registered.
        #[cfg(windows)]
        if word == "sudo" && self.elevation_handler.is_some() {
            let result = if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "sudo")
            {
                self.execute_external(cmd)
            } else {
                self.exit_code = self.execute_sudo(cmd)?;
                Ok(())
            };
            return Ok(Some(result));
        }
        // rubash#2397: execute_cmd.c:4810-4813 roll — a confirmed
        // primary slot is a builtin dispatch (the sudo host-capability
        // arm above is not a GNU builtin and never rolls; an unclaimed
        // name falls through at the None arm and does not roll either).
        if primary_dispatch_slot(word).is_some() {
            self.note_builtin_dispatch(Self::builtin_dispatch_class(word));
        }
        // rubash#2036 S3: the name set is the registry's SimpPrimary
        // projection (command_registry::PRIMARY_DISPATCH); the slot
        // match carries the historical per-name bodies verbatim.
        let result = match primary_dispatch_slot(word) {
            Some(PrimarySlot::Exit) => self.execute_exit_command_word(cmd)?,
            Some(PrimarySlot::Echo) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "echo") {
                    self.execute_external(cmd)
                } else {
                    self.execute_echo(cmd)?;
                    Ok(())
                }
            }
            Some(PrimarySlot::Eval) => self.execute_eval(cmd),
            Some(PrimarySlot::Enable) => {
                self.exit_code = self.execute_enable(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Exec) => self.execute_exec_command(cmd),
            Some(PrimarySlot::Logout) => {
                self.exit_code = self.execute_logout(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Return) => self.execute_return(cmd),
            Some(PrimarySlot::Break) => self.execute_loop_control(cmd, LoopControlKind::Break),
            Some(PrimarySlot::Continue) => {
                self.execute_loop_control(cmd, LoopControlKind::Continue)
            }
            Some(PrimarySlot::Pwd) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "pwd") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_pwd(cmd)?;
                    Ok(())
                }
            }
            Some(PrimarySlot::Source) => self.execute_source_command(cmd),
            Some(PrimarySlot::Printf) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "printf") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_printf(cmd)?;
                    Ok(())
                }
            }
            Some(PrimarySlot::Command) => self.execute_command_builtin_command(cmd)?,
            Some(PrimarySlot::Builtin) => self.execute_builtin_direct_command(cmd),
            Some(PrimarySlot::Cd) => {
                self.exit_code = self.execute_cd(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Pushd) => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Pushd)?;
                Ok(())
            }
            Some(PrimarySlot::Popd) => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Popd)?;
                Ok(())
            }
            Some(PrimarySlot::Dirs) => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Dirs)?;
                Ok(())
            }
            Some(PrimarySlot::Alias) => {
                self.exit_code = self.execute_alias(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Declare) => self.execute_declare_command(cmd),
            Some(PrimarySlot::Local) => {
                self.exit_code = self.execute_local(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Unalias) => {
                self.exit_code = self.execute_unalias(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Export) => {
                self.exit_code = self.execute_export(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Readonly) => {
                self.exit_code = self.execute_readonly(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Colon) => {
                let redirect_failed = self.apply_no_output_builtin_redirects_with_status(cmd)?;
                self.exit_code = if redirect_failed {
                    1
                } else {
                    crate::builtins::colon::colon()
                };
                Ok(())
            }
            Some(PrimarySlot::True) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "true") {
                    self.execute_external(cmd)
                } else {
                    let redirect_failed =
                        self.apply_no_output_builtin_redirects_with_status(cmd)?;
                    self.exit_code = if redirect_failed {
                        1
                    } else {
                        crate::builtins::colon::true_builtin()
                    };
                    Ok(())
                }
            }
            Some(PrimarySlot::False) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "false") {
                    self.execute_external(cmd)
                } else {
                    let redirect_failed =
                        self.apply_no_output_builtin_redirects_with_status(cmd)?;
                    self.exit_code = if redirect_failed {
                        1
                    } else {
                        crate::builtins::colon::false_builtin()
                    };
                    Ok(())
                }
            }
            Some(PrimarySlot::Sleep) => {
                if !self.emulated_external_stands_down("sleep")
                    // rubash#2398: with the job-control channel live,
                    // GNU's sleep is a real job-controlled child
                    // (jobs.c waitchld's WUNTRACED face) — the
                    // in-process emulation has no process group to
                    // stop, hand the terminal to, or resume with
                    // fg/bg, so it stands down to the real spawn for
                    // exactly the channel's lifetime.
                    && !(cfg!(unix) && self.job_control_channel_active())
                    && crate::builtins::sleep::can_execute_fast_path(&cmd.words[1..])
                    // rubash#1497: a stamped -c tail command must reach
                    // the spawn site so it can exec-replace the process
                    // (GNU builtins/evalstring.c:126 should_suppress_
                    // fork); the in-process fast path would keep the
                    // shell's own process image. Windows has no process
                    // replacement and keeps the fast path.
                    && !(cfg!(unix) && self.exec_replace_dispatch_stamp.get() != 0)
                {
                    // GNU sleep is an external child process, so the waiting
                    // shell is interruptible: a caught signal is recorded
                    // (trap.c:545 trap_handler -> trap.c:537 set_trap_state)
                    // and its trap runs when the sleep completes; an untrapped
                    // terminating signal kills the shell mid-wait (SIG_DFL).
                    // sleep_interruptibly slices the wait so the Windows
                    // mailbox is observed with the same deferral semantics.
                    if let Some(seconds) = crate::builtins::sleep::total_duration(&cmd.words[1..]) {
                        // rubash#1034: Some(sig) means the virtual child
                        // was killed by a trapped terminating signal and
                        // its trap already dispatched — the command's
                        // status is the signal death 128+sig, like GNU
                        // reaping a real sleep killed the same way.
                        match self.sleep_interruptibly(seconds)? {
                            Some(signal) => {
                                self.exit_code = 128 + signal;
                                // rubash#1639: an UNTRAPPED SIGINT abort
                                // of the virtual child is GNU's foreground
                                // job dying by SIGINT with the shell
                                // receiving the same terminal delivery —
                                // wait_for throws and the rest of the line
                                // is discarded. A trapped SIGINT already
                                // ran its action inside sleep_interruptibly
                                // and the list continues, so only the
                                // no-trap case marks the discard.
                                //
                                // rubash#1811: the trap-table lookup must
                                // use the canonical key form. The table is
                                // keyed verbatim by trap_key() over the
                                // names normalize_signal returns — the
                                // SIG-prefixed SIGNALS entries for real
                                // signals ("SIGINT"), bare names only for
                                // the EXIT/ERR/DEBUG/RETURN pseudo-signals
                                // — and every other lookup site honors
                                // that (signal_trap_name(), the "SIGCHLD"
                                // literal). This site's bare "INT" could
                                // never hit a stored trap, so EVERY abort
                                // read as untrapped and a trapped SIGINT
                                // discarded the line exactly like an
                                // untrapped one. GNU's adjudication is
                                // by signal number against the trap table
                                // (jobs.c:3315 waitchld: ADDINTERRUPT only
                                // when signal_is_trapped(SIGINT) == 0;
                                // a trapped delivery is only recorded,
                                // trap.c:553 trap_handler -> trap.c:545
                                // set_trap_state, and its action runs at
                                // the next boundary, execute_cmd.c:643
                                // run_pending_traps, after which the
                                // list continues with the wait's 130).
                                #[cfg(unix)]
                                if signal == libc::SIGINT
                                    && self.interactive_reader_session()
                                    && crate::builtins::trap::get_trap_action(
                                        &self.shell_state.env_vars,
                                        "SIGINT",
                                    )
                                    .is_none()
                                {
                                    self.fg_sigint_unwind = true;
                                }
                            }
                            None => self.exit_code = 0,
                        }
                    } else {
                        let (status, stderr) = crate::builtins::sleep::execute(&cmd.words[1..]);
                        if let Some(stderr) = stderr {
                            eprint!("{stderr}");
                        }
                        self.exit_code = status;
                    }
                    // GNU sleep is a real child process; its completion
                    // delivers SIGCHLD and a set trap runs at this boundary
                    // (trap8.sub counts the foreground sleep among the four
                    // CHLD firings). The fast path performs no spawn, so the
                    // notification is raised here instead. The emulated
                    // wait is also a reap observation point (part_036:
                    // GNU's wait_for reaps background children while it
                    // blocks for the real sleep).
                    self.note_foreground_wait();
                    self.run_sigchld_trap_for_reaped_child()?;
                    Ok(())
                } else {
                    self.execute_external(cmd)
                }
            }
            Some(PrimarySlot::Env) => {
                // #2036 S2 (KD-1): env is in BUILTIN_NAMES and, now
                // that enable's list projects from the same registry,
                // it is enable-managed like every other builtin. The
                // per-arm `enable -n` check the sibling arms carry was
                // never wired here (enable refused the name outright,
                // so the disabled set could never contain it); without
                // it, `enable -n env` would report success while
                // dispatch ignored the disable.
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "env") {
                    self.execute_external(cmd)
                } else {
                    self.execute_env_command(cmd)?;
                    Ok(())
                }
            }
            Some(PrimarySlot::Set) => self.execute_set_command(cmd),
            Some(PrimarySlot::Getopts) => {
                self.exit_code = self.execute_getopts_command(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Shopt) => {
                self.exit_code = self.execute_shopt(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Hash) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "hash") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_hash(cmd)?;
                    Ok(())
                }
            }
            Some(PrimarySlot::Help) => {
                self.exit_code = self.execute_help(cmd)?;
                Ok(())
            }
            Some(PrimarySlot::Kill) => {
                self.exit_code = self.execute_kill(cmd)?;
                Ok(())
            }
            None => return Ok(None),
        };
        Ok(Some(result))
    }

    pub(in crate::executor) fn execute_exit_command_word(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Result<(), ExecuteError>, ExecuteError> {
        // GNU builtins/common.c get_exitstat (bash 5.3) argument shape:
        // ISOPTION(word, '-') matches "--" exactly, and a leading `--` is
        // skipped before the status word — `exit --` exits with $?,
        // `exit -- 1` exits 1, `exit -- 1 2` is the too-many error
        // (2026-10-18 probes X4-X7). A raw first word of `--help` keeps
        // the CHECK_HELPOPT help path (builtins/exit.def:59). `exit -` is
        // NOT skipped: valid_number("-") fails, so it is the numeric
        // argument error (probe X3).
        let raw: Vec<&str> = cmd.words[1..].iter().map(String::as_str).collect();
        // CHECK_HELPOPT (builtins/exit.def:59) inspects the RAW first word
        // before anything else: `exit --help` prints the help text, and
        // `exit -- --help` is the numeric error on `--help` (the `--` skip
        // happens inside get_exitstat, after CHECK_HELPOPT).
        if raw.first() != Some(&"--help") {
            // builtins/exit.def `exit_or_logout`: the interactive job
            // check runs BEFORE get_exitstat parses the argument — a
            // blocked exit reports the jobs, sets status 1, and the
            // shell continues (rubash#2397). The banner text mirrors
            // execute_exit's echo (exit.def:59-62 prints it before
            // exit_or_logout runs).
            let banner = if self.invocation_login() {
                "logout"
            } else {
                "exit"
            };
            if self.defer_exit_for_jobs(
                cmd,
                crate::shell::BuiltinDispatchClass::Exit,
                Some(banner),
            )? {
                self.exit_code = 1;
                return Ok(Ok(()));
            }
            let arg_index = if raw.first() == Some(&"--") { 1 } else { 0 };
            let rest = &raw[arg_index.min(raw.len())..];
            // get_exitstat order: the FIRST status word's numeric validity is
            // checked before no_args, so `exit abc def` reports the numeric
            // error, not too-many-arguments. The numeric error path (below)
            // keeps the 5.3 semantics: the command FAILS with status 2
            // (EX_USAGE 258 -> builtin_status -> EX_BADUSAGE) and the shell
            // CONTINUES — 5.2 exited here, but docs/architecture.md pins the
            // 5.3.0 baseline (probe D: `-c 'exit abc; echo AFTER'` prints
            // AFTER, $?=2).
            if let Some(status) = rest.first() {
                // get_exitstat's acceptance rule is general.c:248
                // valid_number (intmax_t): whitespace-padded numerics
                // pass (rubash#1339), out-of-range fails (rubash#1403).
                if crate::builtins::exit::gnu_valid_number(status).is_none() {
                    let mut stderr = Vec::new();
                    writeln!(
                        &mut stderr,
                        "{}exit: {}: numeric argument required",
                        self.diagnostic_prefix(),
                        status
                    )?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    self.exit_code = 2;
                    // get_exitstat's failure return is EX_USAGE (258 >
                    // EX_SHERRBASE=256): execute_simple_command marks
                    // special_builtin_failed (execute_cmd.c:4886-4888), so
                    // in POSIX non-interactive mode this failure is fatal
                    // at this frame unless the context is tested — the
                    // gate in execute_materialized_command converts
                    // (execute_cmd.c:1004-1017). RC audit phase-1 finding
                    // ④ / errors10: the committed errors.snap golden had
                    // absorbed the non-fatal behavior.
                    self.special_builtin_failed.set(true);
                    return Ok(Ok(()));
                }
            }
            // no_args (builtins/common.c, fatal == 0): the diagnostic carries
            // the reader prolog (`bash: line N:` / `<script>: line N:` — the
            // old builtin-level `std::env::var` helper never saw the engine's
            // internal env, so the line segment was silently dropped, issue
            // #705 R7.4), then `set_exit_status (EX_BADUSAGE)` puts 2 into $?,
            // and `top_level_cleanup` + `jump_to_top_level (DISCARD)` abandon
            // the rest of the current command list — nested eval/source string
            // levels popped whole — before the reader continues with the next
            // command ($?=2). Under `-c` the whole string is abandoned and
            // shell.c:1477 run_one_command's DISCARD catch `return
            // last_command_exit_value = 1` ends the shell (probes P1-P9,
            // S1-S9, W1-W6: 2026-10-18 GNU 5.3.0). The discard rides the
            // evalerror vehicle with the exit-too-many class marker.
            if rest.len() > 1 {
                let mut stderr = Vec::new();
                writeln!(
                    &mut stderr,
                    "{}exit: too many arguments",
                    self.diagnostic_prefix()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 2;
                self.raise_evalerror_abort();
                self.evalerror_exit_toomany_pending.set(true);
                return Ok(Ok(()));
            }
        }
        Ok(match self.execute_exit(cmd)? {
            crate::builtins::exit::ExitAction::Exit(code) => {
                self.exit_code = code;
                let code = self.run_exit_trap_for_status(code)?;
                // DE-09 (design WP-1 G-13): the dispatch finish
                // side reports the final status to the session —
                // never the EXIT funnel itself.
                self.debugger_note_exit(code);
                Err(ExecuteError::ExitCode(code))
            }
            crate::builtins::exit::ExitAction::Continue(status) => {
                self.exit_code = status;
                // The only Continue outcome reachable past the intercept
                // above is CHECK_HELPOPT's `--help` (builtins/exit.def:59),
                // which RETURNS EX_USAGE from the builtin itself — the
                // same special-builtin failure class as the numeric error
                // (execute_cmd.c:4886-4888), fatal in POSIX
                // non-interactive mode via the dispatch gate.
                if status == crate::builtins::exit::EX_USAGE {
                    self.special_builtin_failed.set(true);
                }
                Ok(())
            }
        })
    }

    fn execute_command_builtin_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Result<(), ExecuteError>, ExecuteError> {
        // rubash#289d: `command -v/-V` describe output must ALWAYS take the
        // buffered single-truth path (execute_command_describe_with_io →
        // write_buffered_builtin_output → fd table → comsub capture). The
        // old split ran a raw `println!` describe whenever the command had
        // no redirects of its own — under `$(FOO=1 command -v sed)` or
        // `$(PATH=/usr/bin; command -v sed)` that bypassed STDOUT_CAPTURE,
        // leaking the path to the terminal and leaving the substitution
        // empty. GNU has no such bypass: subst.c:7320 command_substitute()
        // dup2s the pipe onto fd 1, and describe_command prints via printf
        // (builtins/command.def:113 → type.def describe_command), so every
        // fd-1 write is captured.
        if self.execute_command_describe_redirected(cmd)? {
            return Ok(Ok(()));
        }
        // GNU execute_cmd.c: the builtin's own diagnostics go to the
        // shell's CURRENTLY BOUND fd 1/2 (redir.c) — an `exec 2>/dev/null`
        // must contain them (rubash#218/#222-era leak: the raw
        // `builtins::command::execute` wrote straight to process stdio).
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action = crate::builtins::command::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(match action {
            crate::builtins::command::CommandAction::Complete(status) => {
                self.exit_code = status;
                Ok(())
            }
            crate::builtins::command::CommandAction::Execute {
                words,
                use_standard_path,
            } => {
                let mut command = cmd.clone();
                command.words = words;
                self.execute_command_without_aliases_with_path(&command, use_standard_path)
            }
        })
    }
}
