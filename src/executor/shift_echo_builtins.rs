use super::*;

impl Executor {
    pub(in crate::executor) fn execute_shift_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action =
            crate::builtins::shift::execute_with_io(&cmd.words[1..], &mut stdout, &mut stderr)?;
        self.apply_shift_action_with_stderr(action, &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)
    }

    pub(in crate::executor) fn apply_shift_action_with_stderr<W: Write>(
        &mut self,
        action: crate::builtins::shift::ShiftAction,
        stderr: &mut W,
    ) -> Result<(), ExecuteError> {
        match action {
            crate::builtins::shift::ShiftAction::Complete(status) => {
                self.exit_code = status;
                // GNU builtins/shift.def: a non-numeric operand fails via
                // get_numeric_arg -> sh_neednumarg -> EX_USAGE, and
                // CHECK_HELPOPT returns EX_USAGE for `--help`; both are
                // 258 > EX_SHERRBASE, so execute_simple_command marks
                // special_builtin_failed (execute_cmd.c:4886-4888) and
                // the failure is fatal in POSIX non-interactive mode
                // unless the context is tested (execute_cmd.c:1004-1017,
                // converted at the gate in execute_materialized_command).
                // Negative / out-of-range counts return EXECUTION_FAILURE
                // instead and never mark it (see the Shift arm below).
                // RC audit phase-1 finding ④ / errors10.
                if status == crate::builtins::shift::EX_USAGE {
                    self.special_builtin_failed.set(true);
                }
            }
            // rubash#1650: the no_args DISCARD channel, shared with
            // `exit 1 2` (command_dispatch_primary.rs): set
            // EX_BADUSAGE, then arm the reader-level discard — the
            // list loop skips the rest of the failing logical line,
            // a function body unwinds with status 2 and its caller
            // continues, a subshell ends with 2, and under `-c` the
            // whole string is abandoned with status 1.
            crate::builtins::shift::ShiftAction::TooManyArguments => {
                self.exit_code = crate::builtins::shift::EX_USAGE;
                self.raise_evalerror_abort();
                self.evalerror_exit_toomany_pending.set(true);
            }
            crate::builtins::shift::ShiftAction::Shift(amount) => {
                if amount > self.shell_state.positional_len() {
                    // GNU general.c:112 (posix_initialize): `set -o posix'
                    // turns print_shift_error ON, so the out-of-range
                    // diagnostic prints in posix mode even without the
                    // shift_verbose shopt (builtins/shift.def:77-88
                    // sh_erange; errors8.sub's ok-4 stderr line). The
                    // failure itself stays EXECUTION_FAILURE (1, not >
                    // EX_SHERRBASE): shift never sets special_builtin_failed,
                    // so it is not posix-fatal even bare (execute_cmd.c:4886).
                    if crate::builtins::shopt::option_enabled(
                        &self.shell_state.env_vars,
                        "shift_verbose",
                    ) || self.posix_mode_enabled()
                    {
                        writeln!(
                            stderr,
                            "{}shift: {amount}: shift count out of range",
                            self.diagnostic_prefix()
                        )?;
                    }
                    self.exit_code = 1;
                    return Ok(());
                }
                // Offset window (variables.c `loptend` port): bumping the
                // base is O(1) — no per-shift clone of every remaining
                // param and no drain memmove. The shifted prefix stays in
                // the backing storage, invisible behind the window, until
                // the next reset/compaction reclaims it.
                self.shell_state.positional_offset += amount;
                let offset = self.shell_state.positional_offset;
                let len = self.shell_state.positional_params.len();
                if offset >= len {
                    // Empty window: drop the dead prefix outright so a
                    // shift-to-empty never retains memory (`set --`
                    // likewise starts from a clean storage).
                    self.shell_state.positional_params.clear();
                    self.shell_state.positional_offset = 0;
                } else if offset > 1024 && offset >= len / 2 {
                    // Long shift run without a reset: fold the dead prefix
                    // so the backing storage stays O(live) (amortized O(1)
                    // per shift; GNU frees the shifted list cells likewise).
                    self.shell_state.positional_params.drain(..offset);
                    self.shell_state.positional_offset = 0;
                }
                self.exit_code = 0;
            }
        }
        Ok(())
    }

    /// The `time` keyword's report: GNU print_time_info
    /// (execute_cmd.c:1552) writes the timing to the shell's CURRENTLY
    /// BOUND stderr in every mode — `{ time ...; } 2>file` captures it —
    /// so the report routes through the executor's fd-2 endpoint
    /// (xtrace_write) instead of the raw process stream (rubash#1128:
    /// the raw eprintln! in print_time leaked past an enclosing group's
    /// redirection).
    pub(in crate::executor) fn print_time_shell(
        &mut self,
        posix_format: bool,
        started: TimeCommandStart,
    ) {
        self.print_time_shell_at(posix_format, started, std::time::Instant::now());
    }

    /// print_time_shell with the interval ending at a captured
    /// instant (rubash#1528 deferred coproc reports: the end is the
    /// reap instant recorded when the job's death was observed).
    pub(in crate::executor) fn print_time_shell_at(
        &mut self,
        posix_format: bool,
        started: TimeCommandStart,
        end: std::time::Instant,
    ) {
        let metrics = TimeCommandMetrics::from_start_until(started, end);
        if let Some(report) = format_time_report(&self.shell_state.env_vars, posix_format, metrics)
        {
            self.xtrace_write(format!("{report}\n").as_bytes());
        }
    }

    pub(in crate::executor) fn execute_time_command_node(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // rubash#1355: an assignment prefix makes `time` an ordinary
        // command name (GNU parse.y simple_command prefix rule), never
        // the keyword — dispatch it as an external lookup (127 when
        // no `time` program is on PATH).
        if !cmd.assignments.is_empty()
            || !cmd.compound_assignments.is_empty()
            || !cmd.array_element_assignments.is_empty()
        {
            return self.execute_external(cmd);
        }
        // rubash#1218: in posix mode a dash word after `time` ends its
        // reserved-word status (GNU parse.y) — the whole command is an
        // ordinary external command named `time` (PATH-resolved, 127
        // when absent), never the timing keyword.
        // rubash#1520: GNU makes that call at parse time
        // (parse.y:3286 time_command_acceptable), so consult the frozen
        // decision recorded on the node — the live mode here may already
        // have been flipped by an earlier command in the same unit.
        if time_dash_word_not_keyword(cmd) {
            return self.execute_external(cmd);
        }
        // rubash#1548: operand window per parse.y special_case_tokens
        // (see crate::parser::scan_time_prefix_words) — a quoted '-p'
        // is the timed command's name, and the report format comes
        // from the operands actually consumed, never from a `-p`
        // appearing later among the timed command's own words.
        let aligned_metadata = if cmd.word_metadata.len() == cmd.words.len() {
            Some(cmd.word_metadata.as_slice())
        } else {
            None
        };
        let scan = crate::parser::scan_time_prefix_words(&cmd.words, aligned_metadata);
        let index = scan.next;
        let inverted = scan.inverted;
        if index >= cmd.words.len() {
            let started = time_command_started();
            self.print_time_shell(scan.posix_format, started);
            self.exit_code = 0;
            return Ok(());
        }

        let mut timed = cmd.clone();
        timed.words = cmd.words[index..].to_vec();
        if cmd.word_kinds.len() == cmd.words.len() {
            timed.word_kinds = cmd.word_kinds[index..].to_vec();
        }
        // Keep word_metadata index-aligned with the stripped words,
        // as the parser fold and the pipeline-prefix strip both do:
        // the restore below reads the recorded raw spellings through
        // it, and any other metadata consumer expects the alignment.
        if cmd.word_metadata.len() == cmd.words.len() {
            timed.word_metadata = cmd.word_metadata[index..].to_vec();
        }
        // rubash#1661: nodes that reach this name-dispatch without a
        // parse-time fold carry the same displaced classification the
        // parser fold restores — the assignment words sat behind the
        // `time` name word, so handle_token never classified them.
        crate::parser::reclassify_leading_assignment_words(&mut timed);
        let started = time_command_started();
        // GNU execute_cmd.c: the time report (print_time_info) is part of
        // executing the timed command — it is printed even when the
        // command fails and errexit then terminates the shell
        // (rubash#1278: `set -e; time false` prints the real/user/sys
        // block in GNU). Capture the execution result, print the report,
        // and only then propagate an errexit/exit unwind.
        let result = self.execute_command(&timed);
        // rubash#1611: a pure time chain prints exactly one report —
        // the innermost frame's (GNU `time time false` prints one).
        // When the stripped command is itself a `time` command it
        // re-enters this function through the name dispatch and
        // prints there, so this frame delegates and prints nothing.
        let delegated = timed.words.first().map(String::as_str) == Some("time");
        if !delegated {
            self.print_time_shell(scan.posix_format, started);
        }
        if result.is_ok() && inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        result
    }

    pub(in crate::executor) fn execute_echo(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // rubash#1488: echo's \u/\U locale gate must observe this shell's
        // variable-table locale (a plain `LC_ALL=C` assignment counts, per
        // GNU's sv_lc* watchers), not just the process environment.
        let _locale_guard = crate::locale::ShellLocaleGuard::new(&self.shell_state.env_vars);
        self.exit_code = 0;
        // TODO(redir.c/execute_cmd.c/builtins/echo.def): Generalize builtin
        // redirection. This covers upstream source tests that create sourced
        // files with `echo ... > file`.
        let echo_args = echo_args_without_background_marker(&cmd.words[1..]);
        let mut output = Vec::new();
        crate::builtins::echo::write_echo_decoded(
            echo_args.iter().map(String::as_str),
            &mut output,
        )?;
        if self.route_builtin_buffered_output(cmd, &output, &[])? {
            return Ok(());
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if self.has_output_fd_target(&target) {
                self.write_output_fd_redirect(&target, &output)?;
                return Ok(());
            }
            if target == "&2" {
                crate::builtins::echo::write_echo_decoded(
                    echo_args.iter().map(String::as_str),
                    &mut std::io::stderr().lock(),
                )?;
                return Ok(());
            }
            if is_closed_redirect_target(&target) || is_null_device(&target) {
                crate::builtins::echo::write_echo_decoded(
                    echo_args.iter().map(String::as_str),
                    &mut std::io::sink(),
                )?;
                return Ok(());
            }
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            // rubash#1496: sh_chkwrite (builtins/common.c:320) — a failed
            // write is `echo: write error: <strerror>` from the builtin,
            // not a bare io error at the command boundary.
            if let Err(error) = crate::builtins::echo::write_echo_decoded(
                echo_args.iter().map(String::as_str),
                &mut file,
            ) {
                let message = format!(
                    "{}echo: write error: {}\n",
                    self.diagnostic_prefix(),
                    crate::posix_errors::message(&error)
                );
                self.write_default_stderr(message.as_bytes())?;
                self.exit_code = 1;
            }
            return Ok(());
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_redirect_target(redirect);
            if self.has_output_fd_target(&target) {
                let mut output = Vec::new();
                crate::builtins::echo::write_echo_decoded(
                    echo_args.iter().map(String::as_str),
                    &mut output,
                )?;
                self.write_output_fd_redirect(&target, &output)?;
                return Ok(());
            }
            if target == "&2" {
                crate::builtins::echo::write_echo_decoded(
                    echo_args.iter().map(String::as_str),
                    &mut std::io::stderr().lock(),
                )?;
                return Ok(());
            }
            if target == "&1" {
                crate::builtins::echo::write_echo_decoded(
                    echo_args.iter().map(String::as_str),
                    &mut std::io::stdout().lock(),
                )?;
                return Ok(());
            }
            if is_closed_redirect_target(&target) {
                crate::builtins::echo::write_echo_decoded(
                    echo_args.iter().map(String::as_str),
                    &mut std::io::sink(),
                )?;
                return Ok(());
            }
            let mut file = open_redirect_append(
                &self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars)),
            )?;
            // rubash#1496: sh_chkwrite arm, as in the truncate path above.
            if let Err(error) = crate::builtins::echo::write_echo_decoded(
                echo_args.iter().map(String::as_str),
                &mut file,
            ) {
                let message = format!(
                    "{}echo: write error: {}\n",
                    self.diagnostic_prefix(),
                    crate::posix_errors::message(&error)
                );
                self.write_default_stderr(message.as_bytes())?;
                self.exit_code = 1;
            }
            return Ok(());
        }

        self.write_default_stdout(&output)?;
        self.exit_code = 0;
        Ok(())
    }
}
