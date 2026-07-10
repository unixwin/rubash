use super::*;

impl Executor {
    pub(in crate::executor) fn execute_times(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
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
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::times::execute_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
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
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::times::execute(&cmd.words[1..])?)
    }

    pub(in crate::executor) fn execute_caller(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let funcname = self.funcname_stack();
        let lineno = self.indexed_array_stack("BASH_LINENO");
        let source = self.indexed_array_stack("BASH_SOURCE");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::caller::execute_with_io(
            &cmd.words[1..],
            &funcname,
            &lineno,
            &source,
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_jobs(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let action = crate::builtins::jobs::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        match action {
            crate::builtins::jobs::JobsAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                Ok(status)
            }
            crate::builtins::jobs::JobsAction::Execute(words) => {
                if !stderr.is_empty() {
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(1);
                }
                let mut command = cmd.clone();
                command.words = words;
                self.execute_command(&command)?;
                Ok(self.exit_code)
            }
        }
    }

    pub(in crate::executor) fn execute_wait(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if cmd.words.len() == 2
            && self
                .last_background_pid
                .is_some_and(|pid| cmd.words[1] == pid.to_string())
        {
            self.write_buffered_builtin_output(cmd, &[], &[])?;
            return Ok(0);
        }

        let mut stderr = Vec::new();
        let status = crate::builtins::wait::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_disown(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = crate::builtins::disown::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_fg_bg(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::fg_bg::JobControlBuiltin,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = crate::builtins::fg_bg::execute_with_io(
            builtin,
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_suspend(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = crate::builtins::suspend::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_history(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = crate::builtins::history::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_bind(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = crate::builtins::bind::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_fc(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = crate::builtins::fc::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_completion_builtin(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::complete::CompletionBuiltin,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::complete::execute_with_io(
            builtin,
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }
}
