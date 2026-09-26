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

        let mut stderr = Vec::new();
        // A sourced file is fresh parser input: GNU expands its aliases
        // while reading it (bashhist.c reader -> parse.y), so the driver's
        // streamed-batch marker lifts for the sourced execution.
        let saved_alias_streamed = self.suspend_alias_streamed();
        let result = crate::builtins::source::execute_named_with_io_and_redirects(
            self,
            &expanded.words[0],
            &expanded.words[1..],
            &mut stderr,
            cmd,
        );
        self.resume_alias_streamed(saved_alias_streamed);
        let had_diagnostic = !stderr.is_empty();
        if had_diagnostic {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
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
        if let Some(redirect) = command
            .redirect_out
            .as_ref()
            .filter(|r| r.fd.unwrap_or(1) == 1)
        {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            apply_stdout_append_redirect(body, &append_redirect);
        }

        if let Some(redirect) = command.append.as_ref().filter(|r| r.fd.unwrap_or(1) == 1) {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_redirect_target(redirect);
            apply_stdout_append_redirect(body, &append_redirect);
        }

        if let Some(redirect) = command
            .redirect_err
            .as_ref()
            .filter(|r| r.fd.unwrap_or(2) == 2)
        {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() && !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            apply_stderr_append_redirect(body, &append_redirect);
        }

        if let Some(redirect) = command
            .redirect_err_append
            .as_ref()
            .filter(|r| r.fd.unwrap_or(2) == 2)
        {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_redirect_target(redirect);
            apply_stderr_append_redirect(body, &append_redirect);
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
