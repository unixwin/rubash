use super::*;

impl Executor {
    pub(in crate::executor) fn apply_external_redirects(
        &self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        self.apply_external_stdin_redirect(cmd, process)?;
        self.apply_external_stdout_redirect(cmd, process)?;
        self.apply_external_stderr_redirect(cmd, process)?;
        Ok(())
    }

    fn apply_external_stdout_redirect(
        &self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        if let Some(ref redirect) = cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            self.apply_external_stdout_target(process, &target, redirect.clobber)?;
        }

        if let Some(ref redirect) = cmd.append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                process.stdout(Stdio::null());
            } else if redirect_target_fd(&target) == Some(2) {
                process.stdout(Stdio::piped());
            } else if redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.seek(SeekFrom::End(0))?;
                process.stdout(Stdio::from(file));
            }
        }

        Ok(())
    }

    fn apply_external_stdout_target(
        &self,
        process: &mut Command,
        target: &str,
        clobber: bool,
    ) -> Result<(), ExecuteError> {
        if is_closed_redirect_target(target) {
            process.stdout(Stdio::null());
        } else if redirect_target_fd(target) == Some(2) {
            process.stdout(Stdio::piped());
        } else if redirect_target_fd(target).is_none() {
            let file = self.create_redirect_output(target, clobber)?;
            process.stdout(Stdio::from(file));
        }
        Ok(())
    }

    fn apply_external_stderr_redirect(
        &self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        if let Some(ref redirect) = cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                process.stderr(Stdio::null());
            } else if redirect_target_fd(&target) == Some(1) {
                process.stderr(Stdio::piped());
            } else if redirect_target_fd(&target).is_none() {
                let file = self.create_redirect_output(&target, redirect.clobber)?;
                process.stderr(Stdio::from(file));
            }
        }

        if let Some(ref redirect) = cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                process.stderr(Stdio::null());
            } else if redirect_target_fd(&target) == Some(1) {
                process.stderr(Stdio::piped());
            } else if redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.seek(SeekFrom::End(0))?;
                process.stderr(Stdio::from(file));
            }
        }

        Ok(())
    }

    pub(in crate::executor) fn write_external_fd_copy_output(
        &self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        if self.external_stdout_copies_to_stderr(cmd) {
            self.write_external_stdout_to_stderr(cmd, stdout)?;
        }
        if self.external_stderr_copies_to_stdout(cmd) {
            self.write_external_stderr_to_stdout(cmd, stderr)?;
        }
        Ok(())
    }

    fn write_external_stderr_to_stdout(
        &self,
        cmd: &CommandNode,
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        if stderr.is_empty() {
            return Ok(());
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(stderr)?;
                return Ok(());
            }
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(stderr)?;
                return Ok(());
            }
        }

        std::io::stdout().lock().write_all(stderr)?;
        Ok(())
    }

    fn write_external_stdout_to_stderr(
        &self,
        cmd: &CommandNode,
        stdout: &[u8],
    ) -> Result<(), ExecuteError> {
        if stdout.is_empty() {
            return Ok(());
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(stdout)?;
                return Ok(());
            }
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(stdout)?;
                return Ok(());
            }
        }

        std::io::stderr().lock().write_all(stdout)?;
        Ok(())
    }

    pub(in crate::executor) fn external_needs_fd_copy_capture(&self, cmd: &CommandNode) -> bool {
        self.external_stdout_copies_to_stderr(cmd) || self.external_stderr_copies_to_stdout(cmd)
    }

    fn external_stdout_copies_to_stderr(&self, cmd: &CommandNode) -> bool {
        cmd.redirect_out
            .as_ref()
            .or(cmd.append.as_ref())
            .map(|redirect| self.expand_word(&redirect.target))
            .is_some_and(|target| redirect_target_fd(&target) == Some(2))
    }

    fn external_stderr_copies_to_stdout(&self, cmd: &CommandNode) -> bool {
        cmd.redirect_err
            .as_ref()
            .or(cmd.redirect_err_append.as_ref())
            .map(|redirect| self.expand_word(&redirect.target))
            .is_some_and(|target| redirect_target_fd(&target) == Some(1))
    }
}
