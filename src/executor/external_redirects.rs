use super::*;

impl Executor {
    pub(in crate::executor) fn apply_external_redirects(
        &self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        self.apply_external_stdin_redirect(cmd, process)?;
        if self.apply_external_combined_output_redirect(cmd, process)? {
            return Ok(());
        }
        self.apply_external_stdout_redirect(cmd, process)?;
        self.apply_external_stderr_redirect(cmd, process)?;
        Ok(())
    }

    fn apply_external_combined_output_redirect(
        &self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<bool, ExecuteError> {
        if let (Some(stdout_redirect), Some(stderr_redirect)) =
            (&cmd.redirect_out, &cmd.redirect_err_append)
        {
            let stdout_target = self.expand_word(&stdout_redirect.target);
            let stderr_target = self.expand_word(&stderr_redirect.target);
            if stdout_target == stderr_target {
                let file = self.create_redirect_output(&stdout_target, stdout_redirect.clobber)?;
                process.stderr(Stdio::from(file.try_clone()?));
                process.stdout(Stdio::from(file));
                return Ok(true);
            }
        }

        if let (Some(stdout_redirect), Some(stderr_redirect)) =
            (&cmd.append, &cmd.redirect_err_append)
        {
            let stdout_target = self.expand_word(&stdout_redirect.target);
            let stderr_target = self.expand_word(&stderr_redirect.target);
            if stdout_target == stderr_target {
                let mut file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(shell_path_to_windows(&stdout_target, &self.env_vars))?;
                file.seek(SeekFrom::End(0))?;
                process.stderr(Stdio::from(file.try_clone()?));
                process.stdout(Stdio::from(file));
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn apply_external_stdout_redirect(
        &self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        let mut redirected = false;
        if let Some(ref redirect) = cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            self.apply_external_stdout_target(process, &target, redirect.clobber)?;
            redirected = true;
        }

        if let Some(ref redirect) = cmd.append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                process.stdout(Stdio::null());
            } else if redirect_target_fd(&target) == Some(2)
                || self.output_fd_redirects_to_stderr(&target)
            {
                process.stdout(Stdio::piped());
            } else if self.output_fd_redirects_to_stdout(&target) {
            } else if self.has_output_fd_target(&target) {
                process.stdout(Stdio::from(self.open_output_fd_append(&target)?));
            } else if redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.seek(SeekFrom::End(0))?;
                process.stdout(Stdio::from(file));
            }
            redirected = true;
        }

        if !redirected {
            if self.env_vars.contains_key(&fd_closed_key(1)) {
                process.stdout(Stdio::null());
            } else if let Some(target) = self.env_vars.get(&fd_output_key(1)) {
                if is_null_device(target) {
                    process.stdout(Stdio::null());
                } else {
                    let file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(target, &self.env_vars))?;
                    process.stdout(Stdio::from(file));
                }
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
        } else if redirect_target_fd(target) == Some(2)
            || self.output_fd_redirects_to_stderr(target)
        {
            process.stdout(Stdio::piped());
        } else if self.output_fd_redirects_to_stdout(target) {
        } else if self.has_output_fd_target(target) {
            process.stdout(Stdio::from(self.open_output_fd_append(target)?));
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
        let mut redirected = false;
        if let Some(ref redirect) = cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                process.stderr(Stdio::null());
            } else if redirect_target_fd(&target) == Some(1)
                || self.output_fd_redirects_to_stdout(&target)
            {
                process.stderr(Stdio::piped());
            } else if self.output_fd_redirects_to_stderr(&target) {
            } else if self.has_output_fd_target(&target) {
                process.stderr(Stdio::from(self.open_output_fd_append(&target)?));
            } else if redirect_target_fd(&target).is_none() {
                let file = self.create_redirect_output(&target, redirect.clobber)?;
                process.stderr(Stdio::from(file));
            }
            redirected = true;
        }

        if let Some(ref redirect) = cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                process.stderr(Stdio::null());
            } else if redirect_target_fd(&target) == Some(1)
                || self.output_fd_redirects_to_stdout(&target)
            {
                process.stderr(Stdio::piped());
            } else if self.output_fd_redirects_to_stderr(&target) {
            } else if self.has_output_fd_target(&target) {
                process.stderr(Stdio::from(self.open_output_fd_append(&target)?));
            } else if redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.seek(SeekFrom::End(0))?;
                process.stderr(Stdio::from(file));
            }
            redirected = true;
        }

        if !redirected {
            if self.env_vars.contains_key(&fd_closed_key(2)) {
                process.stderr(Stdio::null());
            } else if let Some(target) = self.env_vars.get(&fd_output_key(2)) {
                if is_null_device(target) {
                    process.stderr(Stdio::null());
                } else {
                    let file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(target, &self.env_vars))?;
                    process.stderr(Stdio::from(file));
                }
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
            self.write_external_stderr_to_stdout(stderr)?;
        }
        Ok(())
    }

    fn write_external_stderr_to_stdout(&self, stderr: &[u8]) -> Result<(), ExecuteError> {
        if stderr.is_empty() {
            return Ok(());
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
            .is_some_and(|target| {
                redirect_target_fd(&target) == Some(2)
                    || self.output_fd_redirects_to_stderr(&target)
            })
    }

    fn external_stderr_copies_to_stdout(&self, cmd: &CommandNode) -> bool {
        cmd.redirect_err
            .as_ref()
            .or(cmd.redirect_err_append.as_ref())
            .map(|redirect| self.expand_word(&redirect.target))
            .is_some_and(|target| {
                redirect_target_fd(&target) == Some(1)
                    || self.output_fd_redirects_to_stdout(&target)
            })
    }
}
