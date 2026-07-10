use super::*;

impl Executor {
    pub(in crate::executor) fn write_buffered_builtin_output(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if redirect_target_fd(&target) == Some(2) {
                std::io::stderr().lock().write_all(stdout)?;
            } else if redirect_target_fd(&target) == Some(1) {
                std::io::stdout().lock().write_all(stdout)?;
            } else {
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(stdout)?;
            }
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if redirect_target_fd(&target) == Some(2) {
                std::io::stderr().lock().write_all(stdout)?;
            } else if redirect_target_fd(&target) == Some(1) {
                std::io::stdout().lock().write_all(stdout)?;
            } else {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(stdout)?;
            }
        } else if let Some(capture) = &mut self.stdout_capture {
            capture.write_all(stdout)?;
        } else {
            std::io::stdout().lock().write_all(stdout)?;
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if redirect_target_fd(&target) == Some(1) {
                if let Some(capture) = &mut self.stdout_capture {
                    capture.write_all(stderr)?;
                } else {
                    std::io::stdout().lock().write_all(stderr)?;
                }
            } else if !is_null_device(&target) {
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(stderr)?;
            }
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if redirect_target_fd(&target) == Some(1) {
                if let Some(capture) = &mut self.stdout_capture {
                    capture.write_all(stderr)?;
                } else {
                    std::io::stdout().lock().write_all(stderr)?;
                }
            } else {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(stderr)?;
            }
        } else {
            std::io::stderr().lock().write_all(stderr)?;
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_trap(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::trap::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
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
            return Ok(crate::builtins::trap::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::trap::execute_with_io(
                    &cmd.words[1..],
                    &mut self.env_vars,
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::trap::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
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
            return Ok(crate::builtins::trap::execute_with_io(
                &cmd.words[1..],
                &mut self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::trap::execute_with_io(
            &cmd.words[1..],
            &mut self.env_vars,
            &mut std::io::stdout().lock(),
            &mut std::io::stderr().lock(),
        )?)
    }

    pub(in crate::executor) fn execute_help(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::help::execute_with_io(
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
            return Ok(crate::builtins::help::execute_with_io(
                &cmd.words[1..],
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::help::execute_with_io(
                    &cmd.words[1..],
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::help::execute_with_io(
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
            return Ok(crate::builtins::help::execute_with_io(
                &cmd.words[1..],
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::help::execute(&cmd.words[1..])?)
    }

    pub(in crate::executor) fn execute_stack_builtin(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::pushd::StackBuiltin,
    ) -> Result<i32, ExecuteError> {
        let diagnostic_prefix = self.diagnostic_prefix();
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::pushd::execute_with_io(
                builtin,
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &diagnostic_prefix,
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
            return Ok(crate::builtins::pushd::execute_with_io(
                builtin,
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &diagnostic_prefix,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::pushd::execute_with_io(
                    builtin,
                    cmd.words[1..].iter().map(String::as_str),
                    &mut self.env_vars,
                    &diagnostic_prefix,
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::pushd::execute_with_io(
                builtin,
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &diagnostic_prefix,
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
            return Ok(crate::builtins::pushd::execute_with_io(
                builtin,
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &diagnostic_prefix,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::pushd::execute(
            builtin,
            &cmd.words[1..],
            &mut self.env_vars,
            &diagnostic_prefix,
        )?)
    }
}
