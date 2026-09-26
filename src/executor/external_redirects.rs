use super::*;

impl Executor {
    pub(in crate::executor) fn apply_external_redirects(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        self.apply_external_stdin_redirect(cmd, process)?;
        if self.command_needs_ordered_output_capture(cmd) {
            process.stdout(Stdio::piped());
            process.stderr(Stdio::piped());
            return Ok(());
        }
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
            let stdout_target = self.expand_redirect_target(stdout_redirect);
            let stderr_target = self.expand_redirect_target(stderr_redirect);
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
            let stdout_target = self.expand_redirect_target(stdout_redirect);
            let stderr_target = self.expand_redirect_target(stderr_redirect);
            if stdout_target == stderr_target {
                let mut file =
                    OpenOptions::new()
                        .create(true)
                        .write(true)
                        .open(shell_path_to_windows(
                            &stdout_target,
                            &self.shell_state.env_vars,
                        ))?;
                file.seek(SeekFrom::End(0))?;
                process.stderr(Stdio::from(file.try_clone()?));
                process.stdout(Stdio::from(file));
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn apply_external_stdout_redirect(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        let mut redirected = false;
        if let Some(ref redirect) = cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if let Some(fd) = redirect_target_fd(&target) {
                if self.apply_external_coproc_output(process, fd, true)? {
                    redirected = true;
                }
            }
            if redirected {
                return Ok(());
            }
            self.apply_external_stdout_target(process, &target, redirect.clobber)?;
            redirected = true;
        }

        if let Some(ref redirect) = cmd.append {
            // Injected group redirect already bound on fd 1: fall through to
            // the fd-table dup below so the child inherits the group's shared
            // open file description (see injected_redirect_fd_is_bound).
            if !self.injected_redirect_fd_is_bound(redirect, 1) {
                let target = self.expand_redirect_target(redirect);
                if let Some(fd) = redirect_target_fd(&target) {
                    if self.apply_external_coproc_output(process, fd, true)? {
                        redirected = true;
                    }
                }
                if redirected {
                    return Ok(());
                }
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
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
                    file.seek(SeekFrom::End(0))?;
                    process.stdout(Stdio::from(file));
                }
                redirected = true;
            }
        }

        if !redirected {
            if self.fd_table.has_entry(1) && !self.fd_table.is_open_for_write(1) {
                process.stdout(Stdio::null());
            } else if matches!(
                self.fd_table.write_endpoint(1),
                Some(FdWriteEndpoint::Stderr)
            ) {
                process.stdout(Stdio::piped());
            } else if let Some(FdWriteEndpoint::File(file_fd)) = self.fd_table.write_endpoint(1) {
                let dup = crate::fd::duplicate_handle(file_fd.handle)?;
                process.stdout(Stdio::from(crate::fd::handle_to_file(dup)));
            } else if self.stdout_capture.is_some()
                || crate::executor::shell_options::stdout_capture_active()
            {
                // The thread-local capture covers builtin pipeline stages
                // (execute_builtin_pipeline_stage): a builtin that spawns an
                // external command transitively (`command find`, `eval
                // "find ..."`, `env find`) must pipe that child's stdout into
                // the stage capture instead of inheriting the process
                // stdout, where it would leak past the downstream pipe
                // element (unixwin/niubash#93).
                process.stdout(Stdio::piped());
            }
        }

        Ok(())
    }

    fn apply_external_stdout_target(
        &mut self,
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
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        let mut redirected = false;
        if let Some(ref redirect) = cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            if let Some(fd) = redirect_target_fd(&target) {
                if self.apply_external_coproc_output(process, fd, false)? {
                    redirected = true;
                }
            }
            if redirected {
                return Ok(());
            }
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
            // Injected group redirect already bound on fd 2: fall through to
            // the fd-table dup below so the child inherits the group's shared
            // open file description.
            if !self.injected_redirect_fd_is_bound(redirect, 2) {
                let target = self.expand_redirect_target(redirect);
                if let Some(fd) = redirect_target_fd(&target) {
                    if self.apply_external_coproc_output(process, fd, false)? {
                        redirected = true;
                    }
                }
                if redirected {
                    return Ok(());
                }
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
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
                    file.seek(SeekFrom::End(0))?;
                    process.stderr(Stdio::from(file));
                }
                redirected = true;
            }
        }

        if !redirected {
            if self.fd_table.has_entry(2) && !self.fd_table.is_open_for_write(2) {
                process.stderr(Stdio::null());
            } else if matches!(
                self.fd_table.write_endpoint(2),
                Some(FdWriteEndpoint::Stdout)
            ) {
                process.stderr(Stdio::piped());
            } else if let Some(FdWriteEndpoint::File(file_fd)) = self.fd_table.write_endpoint(2) {
                let dup = crate::fd::duplicate_handle(file_fd.handle)?;
                process.stderr(Stdio::from(crate::fd::handle_to_file(dup)));
            }
        }

        Ok(())
    }

    fn apply_external_coproc_output(
        &mut self,
        process: &mut Command,
        fd: u32,
        stdout: bool,
    ) -> Result<bool, ExecuteError> {
        let Some(FdWriteEndpoint::CoprocStdin { fd: pipe, .. }) = self.fd_table.write_endpoint(fd)
        else {
            return Ok(false);
        };
        let dup = crate::fd::duplicate_handle_inheritable(pipe.handle)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "coprocess input is closed"))?;
        let writer = crate::fd::handle_to_file(dup);
        if stdout {
            process.stdout(Stdio::from(writer));
        } else {
            process.stderr(Stdio::from(writer));
        }
        Ok(true)
    }

    pub(in crate::executor) fn write_external_fd_copy_output(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        if self.command_needs_ordered_output_capture(cmd)
            && self.write_ordered_command_output(cmd, stdout, stderr)?
        {
            return Ok(());
        }
        if (self.stdout_capture.is_some()
            || crate::executor::shell_options::stdout_capture_active())
            && !self.external_stdout_copies_to_stderr(cmd)
        {
            self.write_default_stdout(stdout)?;
        }
        if self.external_stdout_copies_to_stderr(cmd) {
            self.write_external_stdout_to_stderr(cmd, stdout)?;
        }
        if self.external_stderr_copies_to_stdout(cmd) {
            self.write_external_stderr_to_stdout(stderr)?;
        }
        Ok(())
    }

    fn write_external_stderr_to_stdout(&mut self, stderr: &[u8]) -> Result<(), ExecuteError> {
        if stderr.is_empty() {
            return Ok(());
        }

        self.write_default_stdout(stderr)?;
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
            let target = self.expand_redirect_target(redirect);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
                file.write_all(stdout)?;
                return Ok(());
            }
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_redirect_target(redirect);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
                file.write_all(stdout)?;
                return Ok(());
            }
        }

        std::io::stderr().lock().write_all(stdout)?;
        Ok(())
    }

    pub(in crate::executor) fn external_needs_fd_copy_capture(&self, cmd: &CommandNode) -> bool {
        self.stdout_capture.is_some()
            || crate::executor::shell_options::stdout_capture_active()
            || self.command_needs_ordered_output_capture(cmd)
            || self.external_stdout_copies_to_stderr(cmd)
            || self.external_stderr_copies_to_stdout(cmd)
    }

    fn external_stdout_copies_to_stderr(&self, cmd: &CommandNode) -> bool {
        self.fd_table.write_endpoint(1) == Some(FdWriteEndpoint::Stderr)
            || cmd
                .redirect_out
                .as_ref()
                .or(cmd.append.as_ref())
                .map(|redirect| self.expand_redirect_target(redirect))
                .is_some_and(|target| {
                    redirect_target_fd(&target) == Some(2)
                        || self.output_fd_redirects_to_stderr(&target)
                })
    }

    fn external_stderr_copies_to_stdout(&self, cmd: &CommandNode) -> bool {
        self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout)
            || cmd
                .redirect_err
                .as_ref()
                .or(cmd.redirect_err_append.as_ref())
                .map(|redirect| self.expand_redirect_target(redirect))
                .is_some_and(|target| {
                    redirect_target_fd(&target) == Some(1)
                        || self.output_fd_redirects_to_stdout(&target)
                })
    }
}
