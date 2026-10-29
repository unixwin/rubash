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

    /// GNU redir.c do_redirections (redir.c:260 do_redirection_internal)
    /// runs BEFORE the command body: every redirection in the list is
    /// opened in order, so a file target is created/truncated even when
    /// the command itself never writes to that descriptor, and a failed
    /// open aborts the command with the redirection_error diagnostic
    /// (redir.c:149-176) and status 1 — the setup is never skipped and
    /// its failure is never silent (rubash#1593). In-process execution
    /// funnels that never bind the command's redirects into the fd
    /// table — `execute_read` and the emulated external file builtins —
    /// call this walk for the open side effects alone: the command body
    /// keeps owning its fd 1/2 targets (it re-opens them when it
    /// actually writes), dup/close entries perform no open (their
    /// validation is the reject gate's), fd_var entries belong to
    /// apply_dynamic_fd_var_redirects, injected group redirects are
    /// already realized by the enclosing compound's bindings, and
    /// /dev/fd-style aliases resolve through the fd table instead of
    /// the filesystem. Returns Ok(true) when a failed open was
    /// diagnosed (routed like redir.c:135) and exit_code set to 1.
    pub(in crate::executor) fn apply_redirect_setup_side_effects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if let Some(error) = self.redirect_setup_open_error(cmd)? {
            let mut line = Vec::new();
            let _ = writeln!(
                &mut line,
                "{}{}",
                self.diagnostic_prefix(),
                crate::posix_errors::message(&error)
            );
            let _ = self.write_redirect_diagnostic_routed(cmd, &line);
            self.exit_code = 1;
            return Ok(true);
        }
        Ok(false)
    }

    /// The open walk behind apply_redirect_setup_side_effects, minus the
    /// diagnostic: returns the first failed open's error (already in the
    /// `<target>: <strerror>` path_error shape the printing caller
    /// renders) or None when every plain entry opened. Callers that own
    /// a different failure channel — the no-output builtin pass, whose
    /// fd 0/1/2 failures travel as ExecuteError::IoError — use this form
    /// so an fd>=3 failure stays indistinguishable from a mirror-field
    /// failure (rubash#1665).
    fn redirect_setup_open_error(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<io::Error>, ExecuteError> {
        use crate::parser::RedirectKind;
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some()
                || crate::executor::support_names::is_injected_group_redirect(redirect)
            {
                continue;
            }
            // Only the open-performing kinds consult a filesystem target
            // here; heredoc / herestring / dup kinds name no path, so
            // expanding their target word would be a side effect of its
            // own (GNU never expands a heredoc delimiter at redirect
            // time either).
            match redirect.kind {
                RedirectKind::Output
                | RedirectKind::ClobberOutput
                | RedirectKind::CombinedOutput
                | RedirectKind::Append
                | RedirectKind::CombinedAppend
                | RedirectKind::Input
                | RedirectKind::ReadWrite => {}
                _ => continue,
            }
            let target = self.expand_redirect_target(redirect);
            // A plain file target is the only shape this walk opens:
            // closed forms (`&-`), fd dups (`&N`), /dev/fd aliases, the
            // null device and process substitutions perform no
            // filesystem open here.
            let plain_file_target = !is_closed_redirect_target(&target)
                && redirect_target_fd(&target).is_none()
                && crate::executor::execution_misc::dev_stdio_redirect_fd(&target).is_none()
                && !is_null_device(&target)
                && !target.starts_with("<(")
                && !target.starts_with(">(");
            let opened: Result<(), io::Error> = match redirect.kind {
                RedirectKind::Output
                | RedirectKind::ClobberOutput
                | RedirectKind::CombinedOutput => {
                    if !plain_file_target {
                        continue;
                    }
                    // On a FIFO the create probe is a no-op whose
                    // open consumes the bind's rendezvous pairing
                    // (rubash#1820) — probe_output_redirect carves
                    // that out.
                    crate::executor::shell_options::probe_output_redirect(
                        self,
                        &target,
                        redirect.clobber,
                    )
                }
                RedirectKind::Append | RedirectKind::CombinedAppend => {
                    if !plain_file_target {
                        continue;
                    }
                    crate::executor::shell_options::probe_output_redirect_append(self, &target)
                }
                RedirectKind::Input => {
                    if !plain_file_target {
                        continue;
                    }
                    // probe_input_redirect's errors already carry the
                    // `<target>: <strerror>` path_error shape.
                    self.probe_input_redirect(&target).map(|_| ())
                }
                RedirectKind::ReadWrite => {
                    if !plain_file_target {
                        continue;
                    }
                    self.probe_input_redirect_readwrite(&target)
                        .map_err(|e| crate::posix_errors::path_error(&target, e))
                        .map(|_| ())
                }
                _ => continue,
            };
            if let Err(error) = opened {
                return Ok(Some(error));
            }
        }
        Ok(None)
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
        //
        // GNU redir.c do_redirections (redir.c:260
        // do_redirection_internal) performs every redirect of a simple
        // command before the body runs, but the mirror fields below only
        // carry fds 0/1/2 — the fd>=3 entries in cmd.redirects never got
        // their open (rubash#1665: `true 3>f` created nothing and a bad
        // target failed silently). Run the same open walk the read /
        // external-file paths use, FIRST, so list order decides which
        // failure is fatal: an earlier entry's side effect stands and a
        // later entry is never opened. The walk's opens of the fd 0/1/2
        // entries are repeated by the mirror code below; a repeated
        // truncate/probe is unobservable, and redirect targets are
        // memoized per command, so no expansion runs twice. The error is
        // already path_error-shaped, so it joins the IoError channel the
        // mirror failures use and every caller renders and marks it
        // exactly like an fd 0/1/2 failure.
        if let Some(error) = self.redirect_setup_open_error(cmd)? {
            return Err(ExecuteError::IoError(error));
        }
        let redirect_failed = false;

        if let Some(redirect) = &cmd.redirect_in {
            let target = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: the diagnostic already
            // printed at the expansion; the null-command redirect pass
            // must not open the failed target either.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            if redirect.fd_var.is_some() {
            } else if is_closed_redirect_target(&target) {
            } else if let Some(source_fd) = dup_spelling_source_fd(&target) {
                // `<&N`, the move form `<&N-` (make_cmd.c:704-718): the
                // builtin keeps reading its current stdin channel — the
                // dup is a no-op for the virtual input model, and the
                // reject_invalid_redirects gate already validated fd N is
                // open (redir.c:1115 dup2 EBADF). GNU r_duplicating_input
                // (redir.c:1152-1160) still runs the dup2 and fails EBADF
                // when fd N is closed, so a closed or untracked source must
                // fail the builtin with `N: Bad file descriptor' instead of
                // silently succeeding (modernish BUG_SCLOSEDFD probe
                // `command : <&8' — rubash#258). An fd-alias PATH spelling
                // (/dev/fd/N, /dev/stdin, /proc/self/fd/N) is NOT a dup
                // spelling: GNU opens it (redir_open), so it falls through
                // to the classified probes below, whose closed-alias answer
                // is the open's ENOENT (rubash#1963).
                self.validate_builtin_dup_source(source_fd)?;
            } else if redirect.append {
                // `<> word` on fd 0: the classified probe performs the
                // one O_RDWR open — registry classes first (net /
                // fd alias / synthetic /proc by write class), plain
                // files through the same raw open as before
                // (rubash#1935/#1963/#1964) — with the same
                // path_error shaping.
                self.probe_input_redirect_readwrite(&target)
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
            } else {
                self.probe_input_redirect(&target)?;
            }
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: the diagnostic already
            // printed at the expansion; the null-command redirect pass
            // must not open the failed target either.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            if redirect.fd_var.is_some() {
            } else if is_closed_redirect_target(&target) {
            } else if let Some(source_fd) = dup_spelling_source_fd(&target) {
                // `>&N` / `>&N-` dup (and move) forms: validated by the
                // reject_invalid_redirects gate; nothing to open here. GNU
                // redir.c:1152-1160 dup2 EBADF — closed source fails with
                // `N: Bad file descriptor' (rubash#258 SCLOSEDFD). An
                // fd-alias PATH spelling is an open, not a dup: it falls
                // through to create_redirect_output, whose registry arm
                // answers a closed alias with the open's ENOENT
                // (rubash#1963).
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
            // subst.c:12663-12668 failglob: the diagnostic already
            // printed at the expansion; the null-command redirect pass
            // must not open the failed target either.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            if redirect.fd_var.is_some() {
            } else if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                // GNU redir.c redirection_error (redir.c:149-176) carries the
                // failed open's target word in the diagnostic
                // (`$FIX/b: No such file or directory`) — the append open's
                // error needs the same path_error shaping as the truncating
                // open, or the renderer's message-only text drops the word
                // (issue #518 background/append repro). The open itself
                // classifies through the registry (rubash#1964).
                self.open_append_target(&target)
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
            }
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: the diagnostic already
            // printed at the expansion; the null-command redirect pass
            // must not open the failed target either.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
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
            // subst.c:12663-12668 failglob: the diagnostic already
            // printed at the expansion; the null-command redirect pass
            // must not open the failed target either.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            if redirect.fd_var.is_some() {
            } else if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                // Classified append open (rubash#1964), same
                // path_error shaping as the fd-1 arm above.
                self.open_append_target(&target)
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
            }
        }

        // rubash#1687: numbered redirects (`3>file`, `3<file`, `3>&1`)
        // live only in the ordered `cmd.redirects` list — the structured
        // fields above mirror just fds 0-2 (parser redirect_assign.rs).
        // GNU performs every redirection on the command node
        // (redir.c do_redirections), so a no-output builtin must still
        // create/truncate/validate its numbered targets even though it
        // writes nothing itself: `builtin 3>f` and `: 3>f` create f, a
        // bad target fails the builtin with status 1, and a dup from a
        // closed fd fails EBADF. `{var}` redirects were applied by
        // apply_dynamic_fd_var_redirects before this helper (see the
        // note at the top) and are skipped, as are close/heredoc forms
        // with no open to perform.
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some() {
                continue;
            }
            let Some(fd) = redirect.fd else {
                continue;
            };
            if fd < 3 {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::ClobberOutput
                | crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    if is_closed_redirect_target(&target) {
                    } else if let Some(source_fd) = dup_spelling_source_fd(&target) {
                        self.validate_builtin_dup_source(source_fd)?;
                    } else if !target.starts_with("<(") && !target.starts_with(">(") {
                        self.create_redirect_output(&target, redirect.clobber)?;
                    }
                }
                crate::parser::RedirectKind::Append => {
                    if !is_closed_redirect_target(&target)
                        && redirect_target_fd(&target).is_none()
                        && !target.starts_with("<(")
                        && !target.starts_with(">(")
                    {
                        // Classified append open (rubash#1964); the
                        // session_path anchoring the fd-1 arm always
                        // had applies here too — the previous raw
                        // fallback opened against the process cwd.
                        self.open_append_target(&target)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    }
                }
                crate::parser::RedirectKind::Input => {
                    if is_closed_redirect_target(&target) {
                    } else if let Some(source_fd) = dup_spelling_source_fd(&target) {
                        self.validate_builtin_dup_source(source_fd)?;
                    } else if !target.starts_with("<(") && !target.starts_with(">(") {
                        self.probe_input_redirect(&target)?;
                    }
                }
                crate::parser::RedirectKind::ReadWrite => {
                    if !is_closed_redirect_target(&target)
                        && (redirect_target_fd(&target).is_none()
                            || crate::special_paths::fd_alias_target(&target).is_some())
                        && !target.starts_with("<(")
                        && !target.starts_with(">(")
                    {
                        // The classified probe performs the one
                        // O_RDWR open — fd-alias spellings included (a
                        // closed alias fails ENOENT here instead of
                        // being skipped silently) and synthetic /proc
                        // entries by their write class
                        // (rubash#1935/#1963). `&N` dup spellings stay
                        // excluded: they name no file to open.
                        self.probe_input_redirect_readwrite(&target)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    }
                }
                crate::parser::RedirectKind::DuplicateInput
                | crate::parser::RedirectKind::DuplicateOutput => {
                    if let Some((source_fd, _)) = redirect_target_fd_and_move(&target) {
                        self.validate_builtin_dup_source(source_fd)?;
                    }
                }
                _ => {}
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
                            self.session_path(shell_path_to_windows(
                                path,
                                &self.shell_state.env_vars,
                            )),
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
                        self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )),
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
