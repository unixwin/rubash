use super::*;

impl Executor {
    pub(in crate::executor) fn execute_pwd(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(self.execute_pwd_with_io(
                &cmd.words[1..],
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(self.execute_pwd_with_io(
                &cmd.words[1..],
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(self.execute_pwd_with_io(
                    &cmd.words[1..],
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(self.execute_pwd_with_io(
                &cmd.words[1..],
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(self.execute_pwd_with_io(
                &cmd.words[1..],
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        let mut stdout = std::io::stdout().lock();
        Ok(self.execute_pwd_with_io(&cmd.words[1..], &mut stdout, &mut std::io::stderr().lock())?)
    }

    pub(in crate::executor) fn execute_pwd_with_io<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> io::Result<i32>
    where
        W: Write,
        E: Write,
    {
        if args.is_empty() || args.first().map(String::as_str) == Some("-L") {
            if let Some(pwd) = self.env_vars.get("PWD") {
                if pwd.starts_with('/') {
                    writeln!(stdout, "{pwd}")?;
                    return Ok(0);
                }
            }
        }

        crate::builtins::pwd::execute_with_io(args.iter().map(String::as_str), stdout, stderr)
    }

    pub(in crate::executor) fn execute_loop_control(
        &mut self,
        cmd: &CommandNode,
        kind: LoopControlKind,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        if self.loop_depth == 0 {
            writeln!(
                &mut stderr,
                "{}{}: only meaningful in a `for', `while', or `until' loop",
                self.diagnostic_prefix(),
                kind.name()
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 0;
            return Ok(());
        }

        match loop_control_level(&cmd.words[1..]) {
            Ok(level) => match kind {
                LoopControlKind::Break => Err(ExecuteError::Break(level)),
                LoopControlKind::Continue => Err(ExecuteError::Continue(level)),
            },
            Err(LoopControlError::TooManyArguments) => {
                writeln!(
                    &mut stderr,
                    "{}{}: too many arguments",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 1;
                Ok(())
            }
            Err(LoopControlError::OutOfRange(value)) => {
                writeln!(
                    &mut stderr,
                    "{}{}: {value}: loop count out of range",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 1;
                Ok(())
            }
            Err(LoopControlError::NotNumeric(value)) => {
                writeln!(
                    &mut stderr,
                    "{}{}: {value}: numeric argument required",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 1;
                Ok(())
            }
        }
    }

    pub(in crate::executor) fn execute_return(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let args = &cmd.words[1..];
        let mut stderr = Vec::new();
        let status = if let Some(value) = args.first() {
            match value.parse::<i128>() {
                Ok(value) => crate::builtins::exit::normalize_status(value),
                Err(_) => {
                    writeln!(
                        &mut stderr,
                        "{}return: {value}: numeric argument required",
                        self.diagnostic_prefix()
                    )?;
                    2
                }
            }
        } else {
            self.exit_code
        };

        let in_function = self.function_depth > 0;
        let in_source = self.env_vars.get("__RUBASH_IN_SOURCE").map(String::as_str) == Some("1");
        if in_function || in_source {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            return Err(ExecuteError::Return(status));
        }

        writeln!(
            &mut stderr,
            "{}return: can only `return' from a function or sourced script",
            self.diagnostic_prefix()
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        self.exit_code = 2;
        Ok(())
    }
}
