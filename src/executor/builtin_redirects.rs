use super::*;

impl Executor {
    pub(in crate::executor) fn write_builtin_not_found(
        &mut self,
        cmd: &CommandNode,
        name: &str,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "{}builtin: {name}: not a shell builtin",
            self.diagnostic_prefix()
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)
    }

    pub(in crate::executor) fn apply_no_output_builtin_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        self.apply_no_output_builtin_redirects_with_status(cmd)
            .map(|_| ())
    }

    pub(in crate::executor) fn apply_no_output_builtin_redirects_with_status(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        // `{var}` redirects (fd_var) are applied once per command by
        // Executor::apply_dynamic_fd_var_redirects — for simple commands at
        // the execute_command dispatch point (GNU redir.c do_redirections →
        // redir_varassign); the conditional and empty-words paths invoke it
        // before calling here. This helper only sees commands whose fd_var
        // redirects are already applied, so it must not re-apply them.
        let redirect_failed = false;

        if let Some(redirect) = &cmd.redirect_in {
            let target = self.expand_redirect_target(redirect);
            if redirect.fd_var.is_some() {
            } else if is_closed_redirect_target(&target) {
            } else if redirect_target_fd(&target).is_some() {
                // `<&N` and fd-alias paths (/dev/stdin, /dev/fd/0,
                // /proc/self/fd/0): the builtin keeps reading its current
                // stdin channel — the fd dup is a no-op for the virtual
                // input model.
            } else if redirect.append {
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
            } else {
                self.probe_input_redirect(&target)?;
            }
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if redirect.fd_var.is_some() {
            } else if is_closed_redirect_target(&target) {
            } else if redirect_target_fd(&target).is_some() {
            } else if redirect.fd.unwrap_or(1) == 1 && target.starts_with('&') {
                // GNU redir.c:832-838: >&WORD with a non-numeric WORD and
                // redirector 1 translates to r_err_and_out (>&file ==
                // >file 2>&1) - open WORD for output instead of treating
                // it as an unusable dup source.
                let path = target.strip_prefix('&').unwrap_or(&target);
                self.create_redirect_output(path, redirect.clobber)?;
            } else {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_redirect_target(redirect);
            if redirect.fd_var.is_some() {
            } else if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                self.open_output_fd_append(&target).or_else(|_| {
                    if is_null_device(&target) {
                        self.create_redirect_output(&target, true)
                    } else {
                        OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                    }
                })?;
            }
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            if redirect.fd_var.is_some() {
            } else if !is_closed_redirect_target(&target)
                && !is_null_device(&target)
                && redirect_target_fd(&target).is_none()
            {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_redirect_target(redirect);
            if redirect.fd_var.is_some() {
            } else if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
            }
        }

        Ok(redirect_failed)
    }

    /// GNU redir.c: execute_cond_command applies the compound command's
    /// redirections to the real descriptors for the duration of the test
    /// and restores them afterwards (do_redirections/undo_redirections).
    /// fd 1 and fd 2 are the only descriptors `[[ ]]` can write; bind
    /// them in redirect order so `[[ ]] 2>f` captures the command's own
    /// diagnostics (invalid regex, syntax errors) and `2>&1`/`>&file`
    /// interact correctly with a following `>file`. File creation and
    /// validation already ran in apply_no_output_builtin_redirects; here
    /// each target becomes an fd-table endpoint.
    pub(in crate::executor) fn bind_conditional_stdio_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        use crate::parser::RedirectKind;
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some() {
                continue;
            }
            let fd = redirect.fd.unwrap_or(match redirect.kind {
                RedirectKind::Input
                | RedirectKind::ReadWrite
                | RedirectKind::DuplicateInput
                | RedirectKind::CloseInput
                | RedirectKind::HereDoc
                | RedirectKind::HereString => 0,
                _ => 1,
            });
            if fd != 1 && fd != 2 {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            match redirect.kind {
                RedirectKind::CloseInput | RedirectKind::CloseOutput => {
                    self.fd_table.close(fd);
                }
                RedirectKind::DuplicateInput | RedirectKind::DuplicateOutput => {
                    if let Some(source) = redirect_target_fd(&target) {
                        // GNU dup_redirects (redir.c:555-567) copies the
                        // descriptor wholesale; a closed/bad source was
                        // already diagnosed and leaves fd unchanged.
                        let _ = self.fd_table.dup_output(fd, source);
                    } else if fd == 1 && !is_closed_redirect_target(&target) {
                        // `>&word`/`&>word`-style dup with a non-numeric
                        // word on redirector 1 is r_err_and_out
                        // (redir.c:832-838): open once, bind both fds to
                        // the same open file description.
                        let path = target.strip_prefix('&').unwrap_or(&target);
                        let file = FileFd::open_write(
                            shell_path_to_windows(path, &self.shell_state.env_vars),
                            redirect.append,
                            false,
                        )
                        .map_err(|e| crate::posix_errors::path_error(path, e))?;
                        self.fd_table
                            .open_output(1, FdWriteEndpoint::File(file.clone()), false);
                        self.fd_table
                            .open_output(2, FdWriteEndpoint::File(file), false);
                    }
                }
                RedirectKind::Output
                | RedirectKind::Append
                | RedirectKind::ClobberOutput
                | RedirectKind::CombinedOutput
                | RedirectKind::CombinedAppend => {
                    if is_closed_redirect_target(&target) {
                        self.fd_table.close(fd);
                        continue;
                    }
                    if let Some(source) = redirect_target_fd(&target) {
                        let _ = self.fd_table.dup_output(fd, source);
                        continue;
                    }
                    let append = redirect.append
                        || matches!(
                            redirect.kind,
                            RedirectKind::Append | RedirectKind::CombinedAppend
                        );
                    let file = FileFd::open_write(
                        shell_path_to_windows(&target, &self.shell_state.env_vars),
                        append,
                        false,
                    )
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    self.fd_table
                        .open_output(fd, FdWriteEndpoint::File(file.clone()), false);
                    // `&>file`/`&>>file` (r_err_and_out): one open file
                    // description shared by fd 1 and fd 2 (redir.c:832-838).
                    if matches!(
                        redirect.kind,
                        RedirectKind::CombinedOutput | RedirectKind::CombinedAppend
                    ) {
                        let other = if fd == 1 { 2 } else { 1 };
                        self.fd_table
                            .open_output(other, FdWriteEndpoint::File(file), false);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
