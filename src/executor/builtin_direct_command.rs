use super::*;

impl Executor {
    pub(in crate::executor) fn execute_builtin_direct_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let args = &cmd.words[1..];
        // Stage-2 ④ Phase 2: the option scan is the (table row +
        // `BuiltinOpt` + `Diag`) triple. GNU builtins/builtin.def:
        // `no_options()` (empty option string) with the leading
        // `--help` mapped to CASE_HELPOPT by internal_getopt
        // (rubash#1229); a bare `--` ends option parsing and is
        // skipped, so `builtin -- CMD` runs CMD (rubash#1376) and —
        // critically — a word AFTER `--` is a NAME, never an option:
        // `builtin -- --help` is `builtin: --help: not a shell
        // builtin`, rc 1 (rubash#2939), not an invalid-option error.
        // `BuiltinOpt` on the row's empty spec produces exactly that
        // shape: `--help` in option position → Help, `--` → End with
        // the rest as operands, `-x` → InvalidOpt, `--badflag` →
        // InvalidOpt("--") (bashgetopt.c:99, rubash#1259).
        let row = super::builtin_table::builtin_table_row("builtin")
            .expect("builtin row in BUILTIN_TABLE");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut diag = super::builtin_diag::Diag::new(row, &mut stderr);
        let (help, operands) = {
            let mut opt = super::builtin_opt::BuiltinOpt::new(row, args, &mut diag, false);
            let mut help = false;
            loop {
                match opt.next() {
                    super::builtin_opt::OptEvent::Help => {
                        help = true;
                        break;
                    }
                    super::builtin_opt::OptEvent::End => break,
                    // The row's spec is `""`: no option character
                    // (and no argument-taking option) can ever arrive.
                    super::builtin_opt::OptEvent::Opt(_)
                    | super::builtin_opt::OptEvent::OptArg(_, _) => {
                        unreachable!("builtin spec is \"\"")
                    }
                }
            }
            let operands: Vec<String> = opt.operands().iter().map(|s| s.to_string()).collect();
            (help, operands)
        };
        if help {
            crate::builtins::help::print_builtin_help(row.name, &mut stdout)?;
            // The scan records nothing on the help path, but flush
            // anyway: a `Diag` dropped with pending diagnostics
            // loses them, and the cost here is one `is_empty`.
            diag.flush_with(|| self.diagnostic_prolog());
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
            self.exit_code = 2;
            return Ok(());
        }
        if diag.status() != 0 {
            diag.usage();
            diag.flush_with(|| self.diagnostic_prolog());
            let status = diag.status();
            drop(diag);
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = status;
            return Ok(());
        }
        // Flush before the dispatch drops the handle: the scan
        // records nothing on the clean path, but the flush is the
        // only render point — dropping first would lose a message
        // the scan ever recorded without setting a status.
        diag.flush_with(|| self.diagnostic_prolog());
        drop(diag);
        let args: &[String] = &operands;
        let Some(name) = args.first().map(String::as_str) else {
            // rubash#1643: a bare `builtin` runs no builtin, but GNU still
            // performs the invocation's own redirections first (redir.c
            // do_redirections runs for the command node, not for the named
            // builtin): `builtin > f` creates/truncates f, `builtin 2> e`
            // creates e, and a failing redirection fails the invocation
            // with status 1. Same helper the `:`/`true`/`false` arms use.
            let redirect_failed = self.apply_no_output_builtin_redirects_with_status(cmd)?;
            self.exit_code = i32::from(redirect_failed);
            return Ok(());
        };

        let mut builtin_cmd = cmd.clone();
        builtin_cmd.words = args.to_vec();

        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, name) {
            self.write_builtin_not_found(cmd, name)?;
            self.exit_code = 1;
            return Ok(());
        }

        // rubash#2126: `builtin NAME` is itself a dispatch of NAME
        // (builtin.def calls the builtin's function directly), so the
        // tracker note follows the re-dispatched name — `builtin eval`
        // arms it exactly like a plain `eval` dispatch (GNU-verified),
        // anything else disarms. The catch-all arm below is the
        // not-a-shell-builtin error: no dispatch happened there, so it
        // restores the pre-note value.
        let tracker_before_builtin_redispatch = self.shell_state.last_dispatch_was_eval_or_source;
        self.note_simple_command_dispatch(name);
        // GNU builtins/builtin.def calls the named builtin's function
        // directly; loadable entries are part of shell_builtins, so
        // `builtin NAME` reaches them through the same table.
        if self.execute_dynamic_builtin_command(&builtin_cmd, name)? {
            return Ok(());
        }
        // rubash#2753: the named builtin's own CHECK_HELPOPT /
        // CASE_HELPOPT — `builtin cd --help` is cd's long help with
        // status 2, exactly as a plain `cd --help` through the funnel
        // gate (this re-dispatch calls the handlers directly and
        // never re-enters the funnel, so the gate must be applied
        // here too). The decision is the shared `help_opt_requested`
        // scan; the text rides `builtin_cmd`, which carries the
        // outer command's redirections. The dispatch itself is the
        // shared `apply_builtin_help`, so the pre-option-scan
        // prologue (rubash#3340: `builtin bind --help` still warns,
        // `set +h; builtin hash --help` still refuses) is replayed
        // here exactly as in the funnel.
        if self.builtin_help_gate(&builtin_cmd, name)? {
            return Ok(());
        }
        match name {
            ":" => self.execute_constant_no_output_builtin_with_redirect_status(
                &builtin_cmd,
                crate::builtins::colon::colon,
            ),
            "true" => self.execute_constant_no_output_builtin_with_redirect_status(
                &builtin_cmd,
                crate::builtins::colon::true_builtin,
            ),
            "false" => self.execute_constant_no_output_builtin_with_redirect_status(
                &builtin_cmd,
                crate::builtins::colon::false_builtin,
            ),
            "echo" => self.execute_echo(&builtin_cmd),
            "printf" => self.execute_status_builtin(&builtin_cmd, Executor::execute_printf),
            "pwd" => self.execute_status_builtin(&builtin_cmd, Executor::execute_pwd),
            "cd" => self.execute_status_builtin(&builtin_cmd, Executor::execute_cd),
            "hash" => self.execute_status_builtin(&builtin_cmd, Executor::execute_hash),
            "help" => self.execute_status_builtin(&builtin_cmd, Executor::execute_help),
            "alias" => self.execute_status_builtin(&builtin_cmd, Executor::execute_alias),
            "unalias" => self.execute_status_builtin(&builtin_cmd, Executor::execute_unalias),
            "export" => self.execute_status_builtin(&builtin_cmd, Executor::execute_export),
            "readonly" => self.execute_status_builtin(&builtin_cmd, Executor::execute_readonly),
            "declare" | "typeset" => self.execute_declare_command(&builtin_cmd),
            "local" => self.execute_status_builtin(&builtin_cmd, Executor::execute_local),
            "unset" => self.execute_status_builtin(&builtin_cmd, Executor::execute_unset),
            "pushd" => self.execute_stack_builtin_status(
                &builtin_cmd,
                crate::builtins::pushd::StackBuiltin::Pushd,
            ),
            "popd" => self.execute_stack_builtin_status(
                &builtin_cmd,
                crate::builtins::pushd::StackBuiltin::Popd,
            ),
            "dirs" => self.execute_stack_builtin_status(
                &builtin_cmd,
                crate::builtins::pushd::StackBuiltin::Dirs,
            ),
            "set" => self.execute_set_command(&builtin_cmd),
            "getopts" => {
                self.execute_status_builtin(&builtin_cmd, Executor::execute_getopts_command)
            }
            "shopt" => self.execute_status_builtin(&builtin_cmd, Executor::execute_shopt),
            "enable" => self.execute_status_builtin(&builtin_cmd, Executor::execute_enable),
            "exec" => {
                self.builtin_wrapper_depth += 1;
                let result = self.execute_exec_command(&builtin_cmd);
                self.builtin_wrapper_depth -= 1;
                result
            }
            // rubash#1722: `exit` and `builtin` itself were missing
            // from this table, so `builtin exit` (which ble.sh's exit
            // wrapper issues when the session ends) reported
            // `builtin: exit: not a shell builtin` and did not exit.
            // GNU's builtin table (builtins/*.def) contains both:
            // builtin.def calls the named builtin's function directly,
            // so `builtin exit` IS exit_builtin (same argument shape,
            // same jump), and `builtin builtin ...` recurses.
            "exit" => {
                // rubash#2397: the inner dispatch rolls like GNU's
                // direct builtin call (execute_cmd.c:4810-4813); the
                // outer `builtin` dispatch already rolled `Other`.
                self.note_builtin_dispatch(crate::shell::BuiltinDispatchClass::Exit);
                self.execute_exit_command_word(&builtin_cmd)?
            }
            "builtin" => {
                self.builtin_wrapper_depth += 1;
                let result = self.execute_builtin_direct_command(&builtin_cmd);
                self.builtin_wrapper_depth -= 1;
                result
            }
            "logout" => {
                // rubash#2397: inner roll, as at the "exit" arm above.
                self.note_builtin_dispatch(crate::shell::BuiltinDispatchClass::Logout);
                self.execute_status_builtin(&builtin_cmd, Executor::execute_logout)
            }
            "eval" => self.execute_eval(&builtin_cmd),
            "command" => {
                self.builtin_wrapper_depth += 1;
                let result = self.execute_command_without_aliases(&builtin_cmd);
                self.builtin_wrapper_depth -= 1;
                result
            }
            // GNU Bash has no `sudo` builtin (issue #759 item 4): the
            // host-capability builtin only exists while an embedding host
            // registered an elevation handler; otherwise `builtin sudo`
            // falls through to the not-a-builtin error below, matching
            // GNU's `builtin` on a non-builtin name.
            #[cfg(windows)]
            "sudo" if self.elevation_handler.is_some() => {
                self.exit_code = self.execute_sudo(&builtin_cmd)?;
                Ok(())
            }
            "source" | "." => self.execute_source_command(&builtin_cmd),
            "return" => self.execute_return(&builtin_cmd),
            "break" => self.execute_loop_control(&builtin_cmd, LoopControlKind::Break),
            "continue" => self.execute_loop_control(&builtin_cmd, LoopControlKind::Continue),
            "kill" => self.execute_status_builtin(&builtin_cmd, Executor::execute_kill),
            "let" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code = self.with_compound_output_redirects(&builtin_cmd, |executor| {
                    Ok(executor.execute_let(&builtin_cmd.words[1..]))
                })?;
                Ok(())
            }
            "umask" => self.execute_status_builtin(&builtin_cmd, Executor::execute_umask),
            "ulimit" => self.execute_status_builtin(&builtin_cmd, Executor::execute_ulimit),
            "read" => self.execute_read_builtin(&builtin_cmd),
            "mapfile" | "readarray" => {
                // Redirect preflight like the `let` arm above (GNU
                // execute_builtin applies redirections before the builtin;
                // rubash#324: `< missing` must report and return 1 with
                // the array untouched, before option validation).
                self.execute_mapfile_builtin(&builtin_cmd)
            }
            "times" => self.execute_status_builtin(&builtin_cmd, Executor::execute_times),
            "caller" => self.execute_status_builtin(&builtin_cmd, Executor::execute_caller),
            "jobs" => {
                // rubash#2397: inner roll, as at the "exit" arm above.
                self.note_builtin_dispatch(crate::shell::BuiltinDispatchClass::Jobs);
                self.execute_status_builtin(&builtin_cmd, Executor::execute_jobs)
            }
            "disown" => self.execute_status_builtin(&builtin_cmd, Executor::execute_disown),
            "wait" => self.execute_status_builtin(&builtin_cmd, Executor::execute_wait),
            "fg" => self.execute_job_control_builtin(
                &builtin_cmd,
                crate::builtins::fg_bg::JobControlBuiltin::Fg,
            ),
            "bg" => self.execute_job_control_builtin(
                &builtin_cmd,
                crate::builtins::fg_bg::JobControlBuiltin::Bg,
            ),
            "suspend" => self.execute_status_builtin(&builtin_cmd, Executor::execute_suspend),
            "history" => self.execute_status_builtin(&builtin_cmd, Executor::execute_history),
            "bind" => self.execute_status_builtin(&builtin_cmd, Executor::execute_bind),
            "fc" => self.execute_status_builtin(&builtin_cmd, Executor::execute_fc),
            "complete" => self.execute_completion_builtin_status(
                &builtin_cmd,
                crate::builtins::complete::CompletionBuiltin::Complete,
            ),
            "compgen" => self.execute_completion_builtin_status(
                &builtin_cmd,
                crate::builtins::complete::CompletionBuiltin::Compgen,
            ),
            "compopt" => self.execute_completion_builtin_status(
                &builtin_cmd,
                crate::builtins::complete::CompletionBuiltin::Compopt,
            ),
            // No "time" arm (rubash#1369): `time` is a reserved word,
            // not a shell builtin, so `builtin time ...` takes the
            // default arm's `builtin: time: not a shell builtin`, rc=1.
            "trap" => self.execute_status_builtin(&builtin_cmd, Executor::execute_trap),
            "type" => self.execute_status_builtin(&builtin_cmd, Executor::execute_type_redirected),
            "test" => self.execute_test_builtin(&builtin_cmd, false),
            "[" => self.execute_test_builtin(&builtin_cmd, true),
            "shift" => self.execute_shift_command(&builtin_cmd),
            _ => {
                // rubash#2126: no dispatch — undo the note above.
                self.shell_state.last_dispatch_was_eval_or_source =
                    tracker_before_builtin_redispatch;
                self.write_builtin_not_found(cmd, name)?;
                self.exit_code = 1;
                Ok(())
            }
        }
    }
}
