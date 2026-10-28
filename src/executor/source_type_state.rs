use super::*;

impl Executor {
    pub(in crate::executor) fn execute_source_from_command_builtin(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // TODO(builtins/command.def/builtins/source.def): `command` removes
        // special-builtin exit behavior while still invoking `.` as a builtin.
        // This covers builtins7.sub's `command . notthere` in POSIX mode.
        self.execute_source_command_with_expanded_args(cmd, true)
    }

    pub(in crate::executor) fn execute_source_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        self.execute_source_command_with_expanded_args(cmd, false)
    }

    fn execute_source_command_with_expanded_args(
        &mut self,
        cmd: &CommandNode,
        command_builtin: bool,
    ) -> Result<(), ExecuteError> {
        let expanded = self.expand_command_words(cmd)?;
        if expanded.words.get(1).is_none() {
            // GNU builtins/source.def:143-144: `source`/`.` with no filename
            // reports "filename argument required" + usage via builtin_error/
            // builtin_usage, and returns EX_USAGE (2).
            let mut stderr = Vec::new();
            let command_name = &expanded.words[0];
            writeln!(
                stderr,
                "{}{command_name}: filename argument required",
                self.diagnostic_prefix()
            )?;
            writeln!(
                stderr,
                "{command_name}: usage: {command_name} [-p path] filename [arguments]"
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 2;
            return Ok(());
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // A sourced file is fresh parser input: GNU expands its aliases
        // while reading it (bashhist.c reader -> parse.y), so the driver's
        // streamed-batch marker lifts for the sourced execution.
        let saved_alias_streamed = self.suspend_alias_streamed();
        let result = crate::builtins::source::execute_named_with_io_and_redirects(
            self,
            &expanded.words[0],
            &expanded.words[1..],
            &mut stdout,
            &mut stderr,
            cmd,
        );
        self.resume_alias_streamed(saved_alias_streamed);
        let had_diagnostic = !stderr.is_empty();
        if had_diagnostic || !stdout.is_empty() {
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        }
        match result {
            Err(ExecuteError::ExitCode(1)) if command_builtin && had_diagnostic => Ok(()),
            other => other,
        }
    }

    pub(in crate::executor) fn apply_brace_group_redirects(
        &mut self,
        command: &CommandNode,
        body: &mut [CommandNode],
    ) -> Result<(), ExecuteError> {
        // The stdio mirror fields are fd-blind: `3>&1`/`2>&1` also land in
        // `redirect_out`. Only fd-matching entries are stdio redirects;
        // numbered ones propagate via splice_numbered_output_redirects_in_order
        // below (GNU do_redirection_internal applies them in parse order).
        //
        // rubash#826: the arms run in PARSE ORDER (their relative position in
        // `command.redirects`), not field order — GNU do_redirection_internal
        // (redir.c:767-955) walks the redirect list left to right, so
        // `{ :; } 2>err2.txt > /nonexistent/f` binds fd 2 to err2.txt BEFORE
        // the fd-1 open is attempted and the failure report lands there.
        // An fd-2 file target applied by an earlier arm is where a later
        // arm's failure report must land. It is established at report time
        // rather than bound into the fd table: a standing fd_table[2]
        // binding here would be seen by the walker's downstream `1>&2` dup,
        // which GNU resolves against the fd 2 in force at the DUP's own
        // parse position — before the err target ever rebinds it
        // (issue699 R1.1).
        let mut established_err_target: Option<String> = None;
        for arm in brace_stdio_arms_in_parse_order(command) {
            let result = match arm {
                BraceStdioArm::Out => self.apply_brace_group_out_arm(command, body),
                BraceStdioArm::Append => self.apply_brace_group_append_arm(command, body),
                BraceStdioArm::Err => {
                    self.apply_brace_group_err_arm(command, body, &mut established_err_target)
                }
                BraceStdioArm::ErrAppend => self.apply_brace_group_err_append_arm(
                    command,
                    body,
                    &mut established_err_target,
                ),
            };
            if let Err(error) = result {
                // GNU redir.c:135 redirection_error reports the failed open
                // through the fd 2 binding the redirect list's earlier
                // entries already applied — err2.txt for
                // `{ :; } 2>err2.txt > /nonexistent/f` — exactly once (#826
                // faces 1-2). The report destination is opened fresh in
                // append mode: the arm's create already did the single
                // O_TRUNC GNU does, and append keeps the message at EOF
                // (GNU models the same with one shared open file
                // description).
                if let ExecuteError::IoError(io_error) = &error {
                    let mut message = Vec::new();
                    let rendered = writeln!(
                        &mut message,
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(io_error)
                    );
                    if rendered.is_ok() {
                        let routed = match &established_err_target {
                            Some(target) => {
                                let path =
                                    shell_path_to_windows(target, &self.shell_state.env_vars);
                                crate::executor::fd_table::FileFd::open_write(path, true, false)
                                    .map(|file| {
                                        let _ = crate::fd::write_all(file.handle, &message);
                                    })
                                    .is_ok()
                            }
                            None => false,
                        };
                        if !routed {
                            let _ = self.write_default_stderr(&message);
                        }
                    }
                }
                return Err(error);
            }
        }
        // Numbered (fd-prefixed) output redirects — `3>&1`, `10>f` — bind
        // real fd-table entries for the body's duration through
        // open_compound_output_redirects (with_command_input_redirects),
        // matching GNU do_redirection_internal's left-to-right descriptor
        // setup (redir.c:767-955). Leaves resolve `1>&3` against the
        // group's binding; a text-level splice would re-dup the leaf's own
        // fd 1 and mis-bind inside pipelines.
        Ok(())
    }

    fn apply_brace_group_out_arm(
        &mut self,
        command: &CommandNode,
        body: &mut [CommandNode],
    ) -> Result<(), ExecuteError> {
        {
            if let Some(redirect) = command
                .redirect_out
                .as_deref()
                .filter(|r| is_stdout_splice_redirect(command, r))
            {
                let target = self.expand_redirect_target(redirect);
                // subst.c:12663-12668 failglob: abort before creating or
                // splicing the failed target into the body.
                if self.redirect_failglob_aborted() {
                    return Err(ExecuteError::ExpansionFailure(1));
                }
                if redirect_target_fd(&target).is_none() {
                    self.create_redirect_output_prepared(&target, redirect.clobber)?;
                }
                let mut append_redirect = redirect.clone();
                append_redirect.target = target;
                append_redirect.append = true;
                append_redirect.clobber = false;
                apply_stdout_append_redirect(body, &append_redirect);
            }
        }
        Ok(())
    }

    fn apply_brace_group_append_arm(
        &mut self,
        command: &CommandNode,
        body: &mut [CommandNode],
    ) -> Result<(), ExecuteError> {
        {
            if let Some(redirect) = command
                .append
                .as_deref()
                .filter(|r| is_stdout_write_redirect(r))
            {
                let target = self.expand_redirect_target(redirect);
                // subst.c:12663-12668 failglob: abort before splicing.
                if self.redirect_failglob_aborted() {
                    return Err(ExecuteError::ExpansionFailure(1));
                }
                let mut append_redirect = redirect.clone();
                append_redirect.target = target;
                append_redirect.append = true;
                apply_stdout_append_redirect(body, &append_redirect);
            }
        }
        Ok(())
    }

    fn apply_brace_group_err_arm(
        &mut self,
        command: &CommandNode,
        body: &mut [CommandNode],
        established_err_target: &mut Option<String>,
    ) -> Result<(), ExecuteError> {
        {
            if let Some(redirect) = command
                .redirect_err
                .as_deref()
                .filter(|r| is_stderr_splice_redirect(command, r))
            {
                let target = self.expand_redirect_target(redirect);
                // subst.c:12663-12668 failglob: abort before creating or
                // splicing (see the redirect_out arm above).
                if self.redirect_failglob_aborted() {
                    return Err(ExecuteError::ExpansionFailure(1));
                }
                if redirect_target_fd(&target).is_none() && !is_null_device(&target) {
                    self.create_redirect_output_prepared(&target, redirect.clobber)?;
                }
                let mut append_redirect = redirect.clone();
                append_redirect.target = target.clone();
                append_redirect.append = true;
                append_redirect.clobber = false;
                apply_stderr_append_redirect(body, &append_redirect);
                // rubash#826: GNU do_redirection_internal (redir.c:767-955)
                // rebinds fd 2 to the opened target at application time, so
                // the group's own later diagnostics — a later arm's failed
                // open, a nested compound's redirect failure — land in that
                // file. Track it as the failure-report destination; the
                // BODY's per-leaf writes keep moving through the splice
                // above. A standing fd_table[2] binding here would instead
                // be seen by the walker's downstream `1>&2` dup (issue699
                // R1.1), which GNU resolves against the fd 2 in force at
                // the DUP's own parse position.
                if redirect_target_fd(&target).is_none() {
                    *established_err_target = Some(target);
                }
            }
        }
        Ok(())
    }

    fn apply_brace_group_err_append_arm(
        &mut self,
        command: &CommandNode,
        body: &mut [CommandNode],
        established_err_target: &mut Option<String>,
    ) -> Result<(), ExecuteError> {
        {
            if let Some(redirect) = command
                .redirect_err_append
                .as_deref()
                .filter(|r| is_stderr_splice_redirect(command, r))
            {
                let target = self.expand_redirect_target(redirect);
                // subst.c:12663-12668 failglob: abort before splicing.
                if self.redirect_failglob_aborted() {
                    return Err(ExecuteError::ExpansionFailure(1));
                }
                let mut append_redirect = redirect.clone();
                append_redirect.target = target.clone();
                apply_stderr_append_redirect(body, &append_redirect);
                // rubash#826: the `2>>` arm never opened the target at group
                // level, so `{ :; } 2>>err.log > /nonexistent/f` lost the
                // diagnostic (GNU reports it into the append-opened file,
                // redir.c:135). Track it like the redirect_err arm above.
                if redirect_target_fd(&target).is_none() {
                    *established_err_target = Some(target);
                }
            }
        }
        Ok(())
    }
}

/// The four stdio-slot redirect arms of a brace group, ordered by their
/// position in the command's parse-order redirect list (rubash#826). An arm
/// whose mirror entry cannot be found in the list (a synthesized desugar)
/// keeps its field order after every matched arm, so the sort is total and
/// stable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BraceStdioArm {
    Out,
    Append,
    Err,
    ErrAppend,
}

fn brace_stdio_arms_in_parse_order(command: &CommandNode) -> Vec<BraceStdioArm> {
    let candidates = [
        (
            command
                .redirect_out
                .as_deref()
                .filter(|r| is_stdout_splice_redirect(command, r)),
            BraceStdioArm::Out,
        ),
        (
            command
                .append
                .as_deref()
                .filter(|r| is_stdout_write_redirect(r)),
            BraceStdioArm::Append,
        ),
        (
            command
                .redirect_err
                .as_deref()
                .filter(|r| is_stderr_splice_redirect(command, r)),
            BraceStdioArm::Err,
        ),
        (
            command
                .redirect_err_append
                .as_deref()
                .filter(|r| is_stderr_splice_redirect(command, r)),
            BraceStdioArm::ErrAppend,
        ),
    ];
    let fallback = command.redirects.len();
    let mut order: Vec<(usize, BraceStdioArm)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(field_index, (redirect, arm))| {
            let redirect = (*redirect)?;
            let position = command
                .redirects
                .iter()
                .position(|entry| entry == redirect)
                .unwrap_or(fallback + field_index);
            Some((position, *arm))
        })
        .collect();
    order.sort_by_key(|(position, _)| *position);
    order.into_iter().map(|(_, arm)| arm).collect()
}

impl Executor {
    pub(in crate::executor) fn execute_type_with_disabled_builtin_state_with_io<W>(
        &mut self,
        args: &[String],
        stdout: &mut W,
    ) -> Result<Option<i32>, ExecuteError>
    where
        W: Write,
    {
        if args.len() == 2
            && args[0] == "-t"
            && args[1] == "test"
            && crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "test")
        {
            if self.command_path("test", false).is_some() {
                writeln!(stdout, "file")?;
                return Ok(Some(0));
            }
            return Ok(Some(1));
        }

        if args.len() == 2
            && args[0] == "-t"
            && args[1] == "test"
            && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "test")
        {
            writeln!(stdout, "builtin")?;
            return Ok(Some(0));
        }

        Ok(None)
    }
}
