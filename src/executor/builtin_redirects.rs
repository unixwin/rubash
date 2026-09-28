use super::*;

impl Executor {
    /// GNU redir.c do_redirection_internal, r_duplicating_input /
    /// r_duplicating_output arms (redir.c:1129-1210): the dup2 of the
    /// source fd happens for builtins too, and a closed source fails with
    /// EBADF ("N: Bad file descriptor", redirect status 1). fd 0/1/2 have
    /// no explicit fd-table entry when they hold the process's own stdio —
    /// those are always open; every other untracked or explicitly-closed
    /// fd is a bad descriptor (rubash's virtual fd table is the single
    /// source of truth, replacing the OS handle slot Windows exposes
    /// nondeterministically).
    fn validate_builtin_dup_source(&self, source_fd: u32) -> Result<(), ExecuteError> {
        if source_fd <= 2 {
            return Ok(());
        }
        let open = matches!(
            self.fd_table.entries.get(&source_fd),
            Some(entry) if !entry.closed
        );
        if open {
            Ok(())
        } else {
            Err(ExecuteError::IoError(std::io::Error::other(format!(
                "{source_fd}: Bad file descriptor"
            ))))
        }
    }

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
            } else if let Some((source_fd, _)) = redirect_target_fd_and_move(&target) {
                // `<&N`, the move form `<&N-` (make_cmd.c:704-718) and the
                // fd-alias paths (/dev/stdin, /dev/fd/0, /proc/self/fd/0):
                // the builtin keeps reading its current stdin channel — the
                // dup is a no-op for the virtual input model, and the
                // reject_invalid_redirects gate already validated fd N is
                // open (redir.c:1115 dup2 EBADF). GNU r_duplicating_input
                // (redir.c:1152-1160) still runs the dup2 and fails EBADF
                // when fd N is closed, so a closed or untracked source must
                // fail the builtin with `N: Bad file descriptor' instead of
                // silently succeeding (modernish BUG_SCLOSEDFD probe
                // `command : <&8' — rubash#258).
                self.validate_builtin_dup_source(source_fd)?;
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
            } else if let Some((source_fd, _)) = redirect_target_fd_and_move(&target) {
                // `>&N` / `>&N-` dup (and move) forms: validated by the
                // reject_invalid_redirects gate; nothing to open here. GNU
                // redir.c:1152-1160 dup2 EBADF — closed source fails with
                // `N: Bad file descriptor' (rubash#258 SCLOSEDFD).
                self.validate_builtin_dup_source(source_fd)?;
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
