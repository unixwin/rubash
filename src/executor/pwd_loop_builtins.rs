use super::*;

impl Executor {
    pub(in crate::executor) fn execute_pwd(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = self.execute_pwd_with_io(&cmd.words[1..], &mut stdout, &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
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

        crate::builtins::pwd::execute_with_env_and_io(
            args.iter().map(String::as_str),
            &self.env_vars,
            stdout,
            stderr,
        )
    }

    pub(in crate::executor) fn execute_loop_control(
        &mut self,
        cmd: &CommandNode,
        kind: LoopControlKind,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        if self.loop_depth == 0 {
            // GNU break.def check_loop_level (BREAK_COMPLAINS): the
            // out-of-loop diagnostic is suppressed in posix mode; the status
            // stays zero either way (func5.sub posix testfunc `break`).
            if !self.posix_mode_enabled() {
                writeln!(
                    &mut stderr,
                    "{}{}: only meaningful in a `for', `while', or `until' loop",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
            }
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            // Bash emits the diagnostic but leaves the command status at zero
            // when an out-of-loop break/continue is followed by another
            // command in the same list.
            self.exit_code = 0;
            return Ok(());
        }

        match loop_control_level(&cmd.words[1..]) {
            Ok(level) => {
                let level = level.min(self.loop_depth);
                match kind {
                    LoopControlKind::Break => Err(ExecuteError::Break(level)),
                    LoopControlKind::Continue => Err(ExecuteError::Continue(level)),
                }
            }
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
            // A bare return inside a running signal trap action gets the
            // pre-trap $? only when it terminates the trap action itself
            // (posix interp 1602); a return in a function called by the
            // action uses the current $? (trap9.sub: handler's return sees
            // setexit's 111, not the pre-trap status).
            let action_depth = self
                .env_vars
                .get("__RUBASH_SIGNAL_TRAP_DEPTH")
                .and_then(|value| value.parse::<usize>().ok());
            let in_trap_action = action_depth == Some(self.function_depth);
            if in_trap_action {
                self.env_vars
                    .get("__RUBASH_SIGNAL_TRAP_STATUS")
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(self.exit_code)
            } else {
                self.exit_code
            }
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
