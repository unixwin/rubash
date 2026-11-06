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
        let mut diag = super::builtin_diag::Diag::new(self.diagnostic_prolog(), row, &mut stderr);
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
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
            self.exit_code = 2;
            return Ok(());
        }
        if diag.status() != 0 {
            diag.usage();
            let status = diag.status();
            drop(diag);
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = status;
            return Ok(());
        }
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
        match name {
            ":" => {
                let redirect_failed =
                    self.apply_no_output_builtin_redirects_with_status(&builtin_cmd)?;
                self.exit_code = if redirect_failed {
                    1
                } else {
                    crate::builtins::colon::colon()
                };
                Ok(())
            }
            "true" => {
                let redirect_failed =
                    self.apply_no_output_builtin_redirects_with_status(&builtin_cmd)?;
                self.exit_code = if redirect_failed {
                    1
                } else {
                    crate::builtins::colon::true_builtin()
                };
                Ok(())
            }
            "false" => {
                let redirect_failed =
                    self.apply_no_output_builtin_redirects_with_status(&builtin_cmd)?;
                self.exit_code = if redirect_failed {
                    1
                } else {
                    crate::builtins::colon::false_builtin()
                };
                Ok(())
            }
            "echo" => {
                self.execute_echo(&builtin_cmd)?;
                Ok(())
            }
            "printf" => {
                self.exit_code = self.execute_printf(&builtin_cmd)?;
                Ok(())
            }
            "pwd" => {
                self.exit_code = self.execute_pwd(&builtin_cmd)?;
                Ok(())
            }
            "cd" => {
                self.exit_code = self.execute_cd(&builtin_cmd)?;
                Ok(())
            }
            "hash" => {
                self.exit_code = self.execute_hash(&builtin_cmd)?;
                Ok(())
            }
            "help" => {
                self.exit_code = self.execute_help(&builtin_cmd)?;
                Ok(())
            }
            "alias" => {
                self.exit_code = self.execute_alias(&builtin_cmd)?;
                Ok(())
            }
            "unalias" => {
                self.exit_code = self.execute_unalias(&builtin_cmd)?;
                Ok(())
            }
            "export" => {
                self.exit_code = self.execute_export(&builtin_cmd)?;
                Ok(())
            }
            "readonly" => {
                self.exit_code = self.execute_readonly(&builtin_cmd)?;
                Ok(())
            }
            "declare" | "typeset" => self.execute_declare_command(&builtin_cmd),
            "local" => {
                self.exit_code = self.execute_local(&builtin_cmd)?;
                Ok(())
            }
            "unset" => {
                self.exit_code = self.execute_unset(&builtin_cmd)?;
                Ok(())
            }
            "pushd" => {
                self.exit_code = self.execute_stack_builtin(
                    &builtin_cmd,
                    crate::builtins::pushd::StackBuiltin::Pushd,
                )?;
                Ok(())
            }
            "popd" => {
                self.exit_code = self.execute_stack_builtin(
                    &builtin_cmd,
                    crate::builtins::pushd::StackBuiltin::Popd,
                )?;
                Ok(())
            }
            "dirs" => {
                self.exit_code = self.execute_stack_builtin(
                    &builtin_cmd,
                    crate::builtins::pushd::StackBuiltin::Dirs,
                )?;
                Ok(())
            }
            "set" => self.execute_set_command(&builtin_cmd),
            "getopts" => {
                self.exit_code = self.execute_getopts_command(&builtin_cmd)?;
                Ok(())
            }
            "shopt" => {
                self.exit_code = self.execute_shopt(&builtin_cmd)?;
                Ok(())
            }
            "enable" => {
                self.exit_code = self.execute_enable(&builtin_cmd)?;
                Ok(())
            }
            // rubash#2581: `env` is in BUILTIN_NAMES, has a registry row
            // (command_registry.rs, dispatching to execute_env_command),
            // is enable-managed, and compgen -b lists it — but this
            // table had no arm, so `builtin env` reported "not a shell
            // builtin". GNU has no env builtin at all (no builtins/
            // env.def; env is coreutils), and rubash deliberately
            // diverges there: the internal env is the only env on
            // platforms without /usr/bin/env, and every other face
            // already treats it as a builtin. The consistent choice is
            // the arm, not stripping env from the faces (which would
            // break `enable -n env`, `compgen -b` and `type env`).
            // The generic is_disabled check above already covers
            // `enable -n env` for this route.
            "env" => {
                self.execute_env_command(&builtin_cmd)?;
                Ok(())
            }
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
                self.exit_code = self.execute_logout(&builtin_cmd)?;
                Ok(())
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
            "kill" => {
                self.exit_code = self.execute_kill(&builtin_cmd)?;
                Ok(())
            }
            "let" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code = self.with_compound_output_redirects(&builtin_cmd, |executor| {
                    Ok(executor.execute_let(&builtin_cmd.words[1..]))
                })?;
                Ok(())
            }
            "umask" => {
                self.exit_code = self.execute_umask(&builtin_cmd)?;
                Ok(())
            }
            "ulimit" => {
                self.exit_code = self.execute_ulimit(&builtin_cmd)?;
                Ok(())
            }
            "read" => {
                self.exit_code = self.execute_read(&builtin_cmd);
                Ok(())
            }
            "mapfile" | "readarray" => {
                // Redirect preflight like the `let` arm above (GNU
                // execute_builtin applies redirections before the builtin;
                // rubash#324: `< missing` must report and return 1 with
                // the array untouched, before option validation).
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code = self.execute_mapfile(&builtin_cmd);
                Ok(())
            }
            "times" => {
                self.exit_code = self.execute_times(&builtin_cmd)?;
                Ok(())
            }
            "caller" => {
                self.exit_code = self.execute_caller(&builtin_cmd)?;
                Ok(())
            }
            "jobs" => {
                // rubash#2397: inner roll, as at the "exit" arm above.
                self.note_builtin_dispatch(crate::shell::BuiltinDispatchClass::Jobs);
                self.exit_code = self.execute_jobs(&builtin_cmd)?;
                Ok(())
            }
            "disown" => {
                self.exit_code = self.execute_disown(&builtin_cmd)?;
                Ok(())
            }
            "wait" => {
                self.exit_code = self.execute_wait(&builtin_cmd)?;
                Ok(())
            }
            "fg" => {
                self.exit_code = self
                    .execute_fg_bg(&builtin_cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                Ok(())
            }
            "bg" => {
                self.exit_code = self
                    .execute_fg_bg(&builtin_cmd, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                Ok(())
            }
            "suspend" => {
                self.exit_code = self.execute_suspend(&builtin_cmd)?;
                Ok(())
            }
            "history" => {
                self.exit_code = self.execute_history(&builtin_cmd)?;
                Ok(())
            }
            "bind" => {
                self.exit_code = self.execute_bind(&builtin_cmd)?;
                Ok(())
            }
            "fc" => {
                self.exit_code = self.execute_fc(&builtin_cmd)?;
                Ok(())
            }
            "complete" => {
                self.exit_code = self.execute_completion_builtin(
                    &builtin_cmd,
                    crate::builtins::complete::CompletionBuiltin::Complete,
                )?;
                Ok(())
            }
            "compgen" => {
                self.exit_code = self.execute_completion_builtin(
                    &builtin_cmd,
                    crate::builtins::complete::CompletionBuiltin::Compgen,
                )?;
                Ok(())
            }
            "compopt" => {
                self.exit_code = self.execute_completion_builtin(
                    &builtin_cmd,
                    crate::builtins::complete::CompletionBuiltin::Compopt,
                )?;
                Ok(())
            }
            // No "time" arm (rubash#1369): `time` is a reserved word,
            // not a shell builtin, so `builtin time ...` takes the
            // default arm's `builtin: time: not a shell builtin`, rc=1.
            "trap" => {
                self.exit_code = self.execute_trap(&builtin_cmd)?;
                Ok(())
            }
            "type" => {
                self.exit_code = self.execute_type_redirected(&builtin_cmd)?;
                Ok(())
            }
            "test" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code =
                    self.execute_test_words(&builtin_cmd.words[1..], false, &builtin_cmd)?;
                Ok(())
            }
            "[" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code =
                    self.execute_test_words(&builtin_cmd.words[1..], true, &builtin_cmd)?;
                Ok(())
            }
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
