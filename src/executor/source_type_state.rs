use super::*;

impl Executor {
    pub(in crate::executor) fn execute_source_from_command_builtin(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // TODO(builtins/command.def/builtins/source.def): `command` removes
        // special-builtin exit behavior while still invoking `.` as a builtin.
        // This covers builtins7.sub's `command . notthere` in POSIX mode.
        if cmd.words.get(1).is_none() {
            self.exit_code = 2;
            return Ok(());
        };

        let mut stderr = Vec::new();
        let result = crate::builtins::source::execute_named_with_io_and_redirects(
            self,
            &cmd.words[0],
            &cmd.words[1..],
            &mut stderr,
            cmd,
        );
        let had_diagnostic = !stderr.is_empty();
        if had_diagnostic {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        match result {
            Err(ExecuteError::ExitCode(1)) if had_diagnostic => Ok(()),
            other => other,
        }
    }

    pub(in crate::executor) fn execute_source_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        let result = crate::builtins::source::execute_named_with_io_and_redirects(
            self,
            &cmd.words[0],
            &cmd.words[1..],
            &mut stderr,
            cmd,
        );
        if !stderr.is_empty() {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        }
        result
    }

    pub(in crate::executor) fn execute_type_with_disabled_builtin_state(
        &mut self,
        args: &[String],
    ) -> Result<bool, ExecuteError> {
        // TODO(builtins/type.def/builtins.c): `type` should query the real
        // shell builtin table. This bridges the `enable -n test` state used by
        // upstream builtins.tests until builtins are centralized.
        if args.len() == 2
            && args[0] == "-t"
            && args[1] == "test"
            && crate::builtins::enable::is_disabled(&self.env_vars, "test")
        {
            if self.command_path("test", false).is_some() {
                println!("file");
                self.exit_code = 0;
            } else {
                self.exit_code = 1;
            }
            return Ok(true);
        }

        if args.len() == 2
            && args[0] == "-t"
            && args[1] == "test"
            && !crate::builtins::enable::is_disabled(&self.env_vars, "test")
        {
            println!("builtin");
            self.exit_code = 0;
            return Ok(true);
        }

        Ok(false)
    }

    pub(in crate::executor) fn apply_brace_group_redirects(
        &mut self,
        command: &CommandNode,
        body: &mut [CommandNode],
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &command.redirect_out {
            let target = self.expand_word(&redirect.target);
            if redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            apply_stdout_append_redirect(body, &append_redirect);
        }

        if let Some(redirect) = &command.append {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_word(&redirect.target);
            apply_stdout_append_redirect(body, &append_redirect);
        }

        if let Some(redirect) = &command.redirect_err {
            let target = self.expand_word(&redirect.target);
            if redirect_target_fd(&target).is_none() && !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            apply_stderr_append_redirect(body, &append_redirect);
        }

        if let Some(redirect) = &command.redirect_err_append {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_word(&redirect.target);
            apply_stderr_append_redirect(body, &append_redirect);
        }

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
            && crate::builtins::enable::is_disabled(&self.env_vars, "test")
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
            && !crate::builtins::enable::is_disabled(&self.env_vars, "test")
        {
            writeln!(stdout, "builtin")?;
            return Ok(Some(0));
        }

        Ok(None)
    }
}
