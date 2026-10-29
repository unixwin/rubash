use super::redirection::{dup_bad_fd_name, dup_source_fd_open};
use super::*;
use crate::executor::fd_table::FdEntry;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

/// Saved descriptor state for a numbered fd opened by a compound command's
/// input redirection (see open_compound_numbered_input_redirects).
pub(in crate::executor) struct SavedNumberedFd {
    fd: u32,
    entry: Option<FdEntry>,
    fd_stdin: Option<String>,
    fd_stdin_offset: Option<String>,
    fd_dynamic: Option<String>,
    fd_closed: Option<String>,
}

/// Saved fd-0 state for a command-scoped input binding: the fd-table slot
/// snapshot plus the FUNCTION_STDIN text-channel pair it may have replaced
/// (see bind_command_fd0_input).
pub(in crate::executor) struct CommandFd0InputGuard {
    saved_fd0_entry: Option<FdEntry>,
    saved_function_stdin: Option<String>,
    saved_function_stdin_offset: Option<String>,
    /// rubash#1141: production schedule of the FUNCTION_STDIN being
    /// shadowed (the rebound text is heredoc/herestring/loop input with
    /// no production timing of its own).
    saved_timed_stdin: Option<super::timed_input::SharedTimedInput>,
    /// rubash#1141: when fd 0 was bound to a process-substitution
    /// stream's endpoint, the stream's registered path — restore writes
    /// the endpoint's consumed offset back into the stream.
    procsub_stream_path: Option<std::path::PathBuf>,
}

/// Saved descriptor state for an fd rebound by a compound command's
/// output redirection (see open_compound_output_redirects).
pub(in crate::executor) struct SavedOutputFd {
    fd: u32,
    entry: Option<FdEntry>,
    /// dup2 snapshot record (FdTable::stdout_alias_generation) that went
    /// with the saved entry; restored together so an inner scope's
    /// `2>/dev/null` cannot drop a `2>&1` binding's generation
    /// (rubash#223).
    alias_generation: Option<Option<usize>>,
    fd_output: Option<String>,
    fd_procsub: Option<String>,
    fd_closed: Option<String>,
}

impl Executor {
    pub(crate) fn with_command_input_redirects<T: Default>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        // GNU redir.c do_redirection_internal (redir.c:767-955) applies a
        // compound command's redirections before the command runs, and
        // execute_cmd.c undoes them when the command finishes (RX_UNDOABLE /
        // redir.c:949-955). A numbered input redirection such as
        // `while read -ru3 x; do :; done 3< <(echo x)` (redir10.sub,
        // procsub.tests bug()) must therefore keep fd 3 open for the whole
        // compound command - condition and body alike - and restore the
        // previous descriptor state afterwards.
        // RESTRICTED_SHELL (#804 N1): this binder reaches fd-var entries
        // through execute_dynamic_fd_var_redirect DIRECTLY and opens the
        // compound's output targets below, so the first restricted-write
        // entry refuses here — before anything opens or binds — and the
        // body never runs (status 1, GNU's in-child do_redirections
        // refusal).
        if self.reject_restricted_write_redirect_entry(cmd)? {
            let default_result = T::default();
            return Ok(default_result);
        }
        let dynamic_names = cmd
            .redirects
            .iter()
            .filter_map(|redirect| redirect.fd_var.clone())
            .collect::<Vec<_>>();
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some() {
                let _ = self.execute_dynamic_fd_var_redirect(redirect, false)?;
            }
        }
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        if let Some((index, name)) = self.compound_output_dup_bad_fd(cmd) {
            return self.abandon_compound_for_dup_bad_fd(cmd, index, &name);
        }
        let saved_output = self.open_compound_output_redirects(cmd)?;
        let result = self.with_command_input_redirects_inner(cmd, execute);
        self.restore_compound_output_redirects(saved_output);
        self.restore_compound_numbered_input_redirects(saved_numbered);
        for name in dynamic_names {
            self.close_dynamic_fd(&name)?;
        }
        result
    }

    fn with_command_input_redirects_inner<T: Default>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        // GNU redir.c do_redirection_internal (redir.c:767-955): a compound
        // command's fd-0 input redirection is opened BEFORE the command
        // runs; an open failure reports the diagnostic and terminates the
        // whole compound with the redirection's status (1) without
        // executing it — `while read -r l; do :; done < noexist` prints
        // "noexist: No such file or directory" and leaves $? = 1
        // (rubash#137; rubash used to run the loop against inherited stdin
        // silently and report 0).
        if let Some(message) = self.compound_stdin_open_failure(cmd) {
            let mut stderr = Vec::new();
            writeln!(&mut stderr, "{message}")?;
            self.write_redirected_command_stderr(cmd, &stderr)?;
            self.exit_code = 1;
            // The compound command is terminated without running: its
            // result is the default (unit for loop/group bodies, None for
            // the source-test `if` scanner, which then consumes the frame).
            return Ok(T::default());
        }
        // GNU do_redirection_internal opens `< file` as a REAL descriptor
        // on fd 0: the shell reads it record-wise off the shared file
        // offset (read.def zreadc + zsyncfd), and a child spawned in the
        // body inherits a dup of the same open file description — a child
        // that never reads leaves the parent's cursor untouched, so
        // `while read -r l; do expr ...; done < f` iterates over every
        // line (rubash#260: the FUNCTION_STDIN text mirror eagerly fed the
        // whole remaining buffer to every child and marked it consumed,
        // ending the loop after one iteration). For a stream source
        // (regular file, FIFO, device — compound_fd0_stream_source) the
        // fd table already has the whole GNU mechanism — byte-accurate
        // record reads (FdTable::read_file_bytes) and DuplicateHandle-based
        // child inheritance — so bind fd 0 to the handle instead of
        // slurping the text mirror. Text-production sources (procsubs,
        // heredocs, here-strings, /dev-aliases) keep the mirror below.
        let Some(guard) = self.bind_command_fd0_input(cmd)? else {
            return execute(self);
        };
        let result = execute(self);
        self.restore_command_fd0_input(guard);
        result
    }

    /// The binding half of with_command_input_redirects_inner: a
    /// stream-source `< file` (regular file, FIFO, character/block
    /// device) opens a real FileFd on fd slot 0 (GNU's shared-offset
    /// descriptor, redir.c do_redirection_internal), any other input
    /// source — here-string, heredoc, procsub text, /dev/fd alias —
    /// stages its expanded text on FUNCTION_STDIN with the slot
    /// snapshotted. Returns None when the command carries no fd-0 input
    /// binding at all (nothing to restore). Used by the compound wrapper
    /// and by the exec builtin, whose spawn must see the node's input
    /// redirections already dup2'd onto fd 0 (GNU redir.c applies them
    /// before execve — rubash#575).
    pub(in crate::executor) fn bind_command_fd0_input(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<CommandFd0InputGuard>, ExecuteError> {
        // rubash#1141: compound commands (brace group, select, loops)
        // dispatch BEFORE the simple-command materialization gate, so
        // their `< <(...)` redirect_in can still carry the raw
        // substitution form here and the registered-stream branch below
        // never sees it — the node fell through to command_input_redirect,
        // whose loop_redirect_input slurps the body UNTIMED into
        // FUNCTION_STDIN and clears timed_stdin, so TMOUT could never
        // fire for the compound's reads/selects. Materialize the body
        // with its production schedule (the same timed variant the
        // prepare path runs for simple commands) and bind fd 0 to the
        // timed endpoint directly; the endpoint is private to this
        // compound scope, so no stream registration or offset write-back
        // is needed. A body that fails to materialize falls through to
        // the existing paths unchanged.
        // NOTE (rubash#1931 residual): this arm — like the fd walk in
        // stdin_string_for_command and loop_redirect_input — matches
        // the target's quote-stripped value, so a QUOTED "<(...)"
        // compound input target is still spawned here even though GNU
        // treats it as a literal filename. The simple-command path
        // admits recorded spans only (external_setup's
        // redirect_target_span_is_recorded); this compound bind layer
        // cannot adopt that admission until it grows a literal-target
        // error channel — declining the spawn today leaves the
        // compound reading ambient stdin (a hang) instead of the
        // redirection error GNU reports, which is worse than the
        // current wrong answer. Recorded as a known adjacent
        // divergence in the #1931 cluster report.
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0
                && redirect.fd_var.is_none()
                && redirect.kind == crate::parser::RedirectKind::Input
            {
                if let Some(source) = redirect
                    .target
                    .strip_prefix("<(")
                    .and_then(|target| target.strip_suffix(')'))
                    .map(str::to_string)
                {
                    // rubash#1952: stream first — GNU process_substitute
                    // (subst.c:6362) binds the forked body's pipe, so
                    // the compound's reads observe real arrival times
                    // and a non-terminating producer no longer starves
                    // the compound forever. The timed collection below
                    // is the spawn-failure fallback (its recorded
                    // schedule served #1141 before a live pipe could).
                    if let Some(file_fd) =
                        self.spawn_input_process_substitution_child(&source, cmd.line)
                    {
                        let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
                        let saved_function_stdin =
                            self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
                        let saved_function_stdin_offset = self
                            .shell_state
                            .env_vars
                            .get(FUNCTION_STDIN_OFFSET)
                            .cloned();
                        self.fd_table
                            .open_input(0, FdReadEndpoint::File(file_fd), false);
                        return Ok(Some(CommandFd0InputGuard {
                            saved_fd0_entry,
                            saved_function_stdin,
                            saved_function_stdin_offset,
                            // The text channel is untouched in this
                            // branch, so its timing stays valid; restore
                            // writes the same value back.
                            saved_timed_stdin: self.timed_stdin.clone(),
                            procsub_stream_path: None,
                        }));
                    }
                    if let Some((output, body_timing)) =
                        self.process_substitution_output_bytes_timed(&source)
                    {
                        let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
                        let saved_function_stdin =
                            self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
                        let saved_function_stdin_offset = self
                            .shell_state
                            .env_vars
                            .get(FUNCTION_STDIN_OFFSET)
                            .cloned();
                        self.fd_table.open_input(
                            0,
                            FdReadEndpoint::process_substitution_bytes(output),
                            false,
                        );
                        self.fd_table.set_input_offset(0, 0);
                        if let Some(timing) = body_timing {
                            self.fd_table.set_input_timing(0, timing);
                        }
                        return Ok(Some(CommandFd0InputGuard {
                            saved_fd0_entry,
                            saved_function_stdin,
                            saved_function_stdin_offset,
                            // The text channel is untouched in this
                            // branch, so its timing stays valid; restore
                            // writes the same value back.
                            saved_timed_stdin: self.timed_stdin.clone(),
                            procsub_stream_path: None,
                        }));
                    }
                }
            }
        }
        // rubash#1141: a `< <(...)` redirect whose target materialized to
        // a registered process-substitution stream binds fd 0 to the
        // stream's fd-table endpoint — not to a FileFd on the temp
        // carrier — so the body's production schedule judges the
        // compound's reads/selects (GNU binds the pipe itself). The
        // endpoint starts at the stream's shared offset; restore writes
        // the consumed offset back.
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0
                && redirect.fd_var.is_none()
                && redirect.kind == crate::parser::RedirectKind::Input
                && !redirect.target.starts_with("<(")
            {
                let target = self.expand_redirect_target(redirect);
                let path = self.session_path(super::path::windows_operand_file_path(
                    super::path::shell_path_to_windows(&target, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                ));
                if let Some((data, offset, timing)) = self.procsub_stream_lookup(&path) {
                    let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
                    let saved_function_stdin =
                        self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
                    let saved_function_stdin_offset = self
                        .shell_state
                        .env_vars
                        .get(FUNCTION_STDIN_OFFSET)
                        .cloned();
                    self.fd_table.open_input(
                        0,
                        FdReadEndpoint::process_substitution_bytes(data),
                        false,
                    );
                    self.fd_table.set_input_offset(0, offset);
                    if let Some(timing) = timing {
                        self.fd_table.set_input_timing(0, timing);
                    }
                    return Ok(Some(CommandFd0InputGuard {
                        saved_fd0_entry,
                        saved_function_stdin,
                        saved_function_stdin_offset,
                        saved_timed_stdin: self.timed_stdin.clone(),
                        procsub_stream_path: Some(path),
                    }));
                }
            }
        }
        if let Some(file) = self.compound_fd0_stream_file(cmd) {
            // Snapshot the FUNCTION_STDIN pair too: restore_command_fd0_input
            // is unconditional, and this branch never rewrites the text
            // channel — the guard must put back exactly what it found.
            let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
            let saved_function_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
            let saved_function_stdin_offset = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .cloned();
            self.fd_table
                .open_input(0, FdReadEndpoint::File(file), false);
            return Ok(Some(CommandFd0InputGuard {
                saved_fd0_entry,
                saved_function_stdin,
                saved_function_stdin_offset,
                // The text channel is untouched in this branch, so its
                // timing stays valid; restore writes the same value back.
                saved_timed_stdin: self.timed_stdin.clone(),
                procsub_stream_path: None,
            }));
        }
        let Some(input) = self.command_input_redirect(cmd) else {
            return Ok(None);
        };
        let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
        let saved_function_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
        let saved_function_stdin_offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .cloned();
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input);
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        // rubash#1141: the rebound FUNCTION_STDIN is locally produced
        // text (heredoc / here-string / loop input) — the enclosing
        // stream's production schedule does not describe it.
        let saved_timed_stdin = self.timed_stdin.take();
        Ok(Some(CommandFd0InputGuard {
            saved_fd0_entry,
            saved_function_stdin,
            saved_function_stdin_offset,
            saved_timed_stdin,
            procsub_stream_path: None,
        }))
    }

    /// The unwind half of bind_command_fd0_input. GNU redir.c
    /// do_redirection_internal (RX_UNDOABLE) restores fd 0 at scope end —
    /// even undoing a permanent `exec 0<g` inside the body.
    pub(in crate::executor) fn restore_command_fd0_input(&mut self, guard: CommandFd0InputGuard) {
        // rubash#1141: fold the fd-0 endpoint's consumption back into
        // the process-substitution stream's shared offset before the
        // descriptor binding is unwound.
        if let Some(path) = &guard.procsub_stream_path {
            if let Some(offset) = self.fd_table.input_offset(0) {
                self.procsub_stream_set_offset(path, offset);
            }
        }
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN,
            guard.saved_function_stdin,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            guard.saved_function_stdin_offset,
        );
        self.timed_stdin = guard.saved_timed_stdin;
        match guard.saved_fd0_entry {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }
    }

    /// Opens the numbered input redirections of a compound command in the
    /// fd table for the command's whole duration. GNU applies compound
    /// redirections left to right before the command runs (redir.c
    /// do_redirection_internal:767); procsub targets feed
    /// FdReadEndpoint-backed text exactly like the persistent fd-var path
    /// (trap_exec.rs execute_dynamic_fd_var_redirect).
    pub(in crate::executor) fn open_compound_numbered_input_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Vec<SavedNumberedFd>, ExecuteError> {
        let mut saved: Vec<SavedNumberedFd> = Vec::new();
        for redirect in &cmd.redirects {
            // The fd before the operator defaults like GNU: input-side
            // redirects without an explicit number target fd 0
            // (`{ list; } <&3` dups fd 3 onto the group's stdin).
            let fd = redirect.fd.unwrap_or(0);
            if redirect.fd_var.is_some() {
                continue;
            }
            if !matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input
                    | crate::parser::RedirectKind::ReadWrite
                    | crate::parser::RedirectKind::DuplicateInput
                    | crate::parser::RedirectKind::CloseInput
            ) {
                continue;
            }
            // fd 0 `<file` stays on the FUNCTION_STDIN text channel for
            // external-children compatibility, but `0<&N`/`0<&-` are real
            // descriptor operations — dup_entry gives the group's fd 0 the
            // source's open file description (shared offset) for both
            // builtins and spawned children.
            if fd == 0
                && !matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::DuplicateInput
                        | crate::parser::RedirectKind::CloseInput
                )
            {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: the diagnostic already printed
            // at the expansion; abort the compound before any fd binding.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            if !saved.iter().any(|saved| saved.fd == fd) {
                saved.push(SavedNumberedFd {
                    fd,
                    entry: self.fd_table.entries.get(&fd).cloned(),
                    fd_stdin: self.shell_state.env_vars.get(&fd_stdin_key(fd)).cloned(),
                    fd_stdin_offset: self
                        .shell_state
                        .env_vars
                        .get(&fd_stdin_offset_key(fd))
                        .cloned(),
                    fd_dynamic: self
                        .shell_state
                        .env_vars
                        .get(&fd_dynamic_input_key(fd))
                        .cloned(),
                    fd_closed: self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned(),
                });
            }
            match redirect.kind {
                // `{ cmd; } N<&-` / `N>&-` — scoped close (redir.c
                // do_redirection_internal r_close_input).
                crate::parser::RedirectKind::CloseInput => {
                    self.fd_table.close_input(fd);
                    self.shell_state
                        .env_vars
                        .insert(fd_closed_key(fd), "1".to_string());
                    continue;
                }
                // `{ cmd; } N<&M` — dup2 the source descriptor into the
                // scoped slot (e.g. `4<&0` saves stdin for the group).
                crate::parser::RedirectKind::DuplicateInput => {
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        let _ = self.fd_table.dup_input(fd, source_fd);
                        self.shell_state.env_vars.remove(&fd_closed_key(fd));
                        // Self-move: GNU's `redir_fd != redirector`
                        // guard makes it a no-op (rubash#1236).
                        if move_source && source_fd != fd {
                            self.fd_table.close(source_fd);
                        }
                    }
                    continue;
                }
                _ => {}
            }
            if is_closed_redirect_target(&target) {
                continue;
            }
            if is_null_device(&target) {
                self.set_fd_input_bytes(fd, Vec::new(), true);
                if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                    self.set_fd_readwrite_file(fd, &target, true)
                        .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                }
                continue;
            }
            if let Some(source) = target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                // rubash#1952: stream first — GNU process_substitute
                // (subst.c:6362) binds the forked body's pipe read end
                // on the target fd; the collected form below is the
                // spawn-failure fallback.
                if let Some(file_fd) = self.spawn_input_process_substitution_child(source, cmd.line)
                {
                    self.set_fd_input_file(fd, file_fd, true);
                    if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                        self.set_fd_readwrite_file(fd, &target, true)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    }
                    continue;
                }
                let Some(output) = self.process_substitution_output(source) else {
                    continue;
                };
                self.set_fd_input_bytes(
                    fd,
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
                    true,
                );
                if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                    self.set_fd_readwrite_file(fd, &target, true)
                        .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                }
                continue;
            }
            // Real handle on the slot: [N]<> is a single O_RDWR open file
            // description (redir.c r_input_output), otherwise O_RDONLY.
            if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                self.set_fd_readwrite_file(fd, &target, true)
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
            } else if let Some(bytes) = crate::special_paths::synthetic_proc_content(&target) {
                // Registry first (rubash#1917, same ladder-skip family
                // as the exec-fd face): synthetic /proc content has no
                // filesystem file behind the mapped path on Windows;
                // serve the snapshot bytes through the fd byte channel.
                self.set_fd_input_bytes(fd, bytes, true);
            } else if let Some(link) = crate::special_paths::synthetic_link_target(
                &target,
                Some(&crate::proc_vfs::self_cwd_forms(self.session_cwd())),
            ) {
                // SyntheticLink (rubash#1900): open the resolved
                // native target (session cwd answers for self/cwd).
                let file = FileFd::open_read(std::path::PathBuf::from(&link.native))
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                self.set_fd_input_file(fd, file, true);
            } else {
                // niubash#177: same operand resolution as
                // open_input_redirect — a virtual-root input operand may
                // exist only under its `.exe` spelling on disk.
                let path = self.session_path(crate::executor::path::windows_operand_file_path(
                    shell_path_to_windows(&target, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                ));
                let file = FileFd::open_read(path)
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                self.set_fd_input_file(fd, file, true);
            }
        }
        Ok(saved)
    }

    pub(in crate::executor) fn restore_compound_numbered_input_redirects(
        &mut self,
        saved: Vec<SavedNumberedFd>,
    ) {
        for saved in saved {
            match saved.entry {
                Some(entry) => {
                    self.fd_table.entries.insert(saved.fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&saved.fd);
                }
            }
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_stdin_key(saved.fd),
                saved.fd_stdin,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_stdin_offset_key(saved.fd),
                saved.fd_stdin_offset,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_dynamic_input_key(saved.fd),
                saved.fd_dynamic,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_closed_key(saved.fd),
                saved.fd_closed,
            );
        }
    }

    /// GNU do_redirection_internal (redir.c:767-955) applies a compound
    /// command's output redirections to real descriptors — in parse order —
    /// before the body runs, and execute_cmd.c undoes them when the command
    /// finishes. Bind them on the fd table so a body's `1>&3` resolves fd 3
    /// to the *group's* binding (e.g. `>/dev/null 3>&1` leaves fd 3 on the
    /// null device) instead of re-dup'ing the leaf's own fd 1. Returns the
    /// touched slots for restore_compound_output_redirects; on a mid-list
    /// open failure the already-applied slots are unwound before the error
    /// propagates, like GNU abandoning the compound.
    pub(in crate::executor) fn open_compound_output_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Vec<SavedOutputFd>, ExecuteError> {
        let mut saved: Vec<SavedOutputFd> = Vec::new();
        if let Err(error) = self.open_compound_output_redirects_inner(cmd, &mut saved) {
            self.restore_compound_output_redirects(saved);
            return Err(error);
        }
        Ok(saved)
    }

    fn save_compound_output_fd(&mut self, saved: &mut Vec<SavedOutputFd>, fd: u32) {
        if saved.iter().any(|entry| entry.fd == fd) {
            return;
        }
        saved.push(SavedOutputFd {
            fd,
            entry: self.fd_table.entries.get(&fd).cloned(),
            alias_generation: self.fd_table.stdout_alias_generation.get(&fd).cloned(),
            fd_output: self.shell_state.env_vars.get(&fd_output_key(fd)).cloned(),
            fd_procsub: self
                .shell_state
                .env_vars
                .get(&fd_output_process_substitution_key(fd))
                .cloned(),
            fd_closed: self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned(),
        });
    }

    fn open_compound_output_redirects_inner(
        &mut self,
        cmd: &CommandNode,
        saved: &mut Vec<SavedOutputFd>,
    ) -> Result<(), ExecuteError> {
        for redirect in &cmd.redirects {
            // `{var}`-targeted redirections allocate a persistent dynamic fd
            // (redir.c: the variable keeps the descriptor after the command)
            // and are handled by execute_dynamic_fd_var_redirect above.
            if redirect.fd_var.is_some() {
                continue;
            }
            let fd = match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput
                | crate::parser::RedirectKind::DuplicateOutput
                | crate::parser::RedirectKind::CloseOutput => redirect.fd.unwrap_or(1),
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => 1,
                _ => continue,
            };
            let target = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: the diagnostic already printed
            // at the expansion; abort the compound before any fd binding.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }

            // See injected_redirect_fd_is_bound (redirection.rs): a redirect
            // injected from an enclosing compound is already realized by the
            // group's live fd-table binding — re-opening would re-truncate
            // the file and fork a second offset for the nested compound.
            if self.injected_redirect_fd_is_bound(redirect, fd) {
                continue;
            }

            match redirect.kind {
                crate::parser::RedirectKind::CloseOutput => {
                    self.save_compound_output_fd(saved, fd);
                    self.fd_table.close_output(fd);
                    self.shell_state
                        .env_vars
                        .insert(fd_closed_key(fd), "1".to_string());
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    self.save_compound_output_fd(saved, fd);
                    if is_closed_redirect_target(&target) {
                        self.fd_table.close_output(fd);
                        self.shell_state
                            .env_vars
                            .insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        if move_source {
                            self.save_compound_output_fd(saved, source_fd);
                        }
                        // GNU dup2 copies the whole descriptor — the group's
                        // fd takes the source's open file description, shared
                        // offset included (fd_table dup_entry).
                        if self.fd_table.dup_output(fd, source_fd).is_err()
                            && source_fd <= 2
                            && !self.fd_table.entries.contains_key(&source_fd)
                        {
                            // The source is an IMPLICIT std fd (no fd-table
                            // entry — the shell's own stdout/stderr). GNU
                            // redir.c dup2 still succeeds: the duplicate
                            // snapshots the process stdio AT ENTRY, so a
                            // later `exec > f` inside the body rebinds fd 1
                            // only and writes to fd 2 keep going to the
                            // original stdout (rubash#170 nest4:
                            // `( exec > f; echo x >&2 ) 2>&1` must print x,
                            // not write f). The FdWriteEndpoint::Stdout /
                            // ::Stderr markers carry exactly that meaning —
                            // they resolve to the process stdio (or the
                            // active capture), never to a later fd-1
                            // rebinding.
                            let endpoint = if source_fd == 1 {
                                FdWriteEndpoint::Stdout
                            } else {
                                FdWriteEndpoint::Stderr
                            };
                            self.fd_table.open_output(fd, endpoint, false);
                            self.shell_state.env_vars.remove(&fd_closed_key(fd));
                        }
                        self.record_output_fd_ledger(fd);
                        // Self-move: no-op, never closes (rubash#1236).
                        if move_source && source_fd != fd {
                            self.fd_table.close(source_fd);
                            self.shell_state
                                .env_vars
                                .insert(fd_closed_key(source_fd), "1".to_string());
                        }
                        continue;
                    }
                    // GNU redir.c:832-838: `>&word`/`1>&word` with a
                    // non-numeric word is r_err_and_out — one open, fd 2
                    // shares fd 1's description. Other redirectors report
                    // AMBIGUOUS_REDIRECT; the propagation/diagnostic paths
                    // already report it, so leave the fd table alone.
                    if fd == 1 {
                        let path = target.strip_prefix('&').unwrap_or(&target);
                        if redirect.append {
                            self.set_fd_output_file(1, path.to_string(), false, true)
                        } else {
                            // rubash#1868: probe form — on a FIFO the
                            // physical create open would consume the
                            // rendezvous pairing set_fd_output_file's
                            // bind open waits for (see
                            // probe_output_redirect).
                            crate::executor::shell_options::probe_output_redirect(
                                self,
                                path,
                                redirect.clobber,
                            )?;
                            self.set_fd_output_file(1, path.to_string(), false, false)
                        }
                        .map_err(|error| crate::posix_errors::path_error(path, error))?;
                        self.save_compound_output_fd(saved, 2);
                        self.share_fd_output_file(1, 2, path, false);
                    }
                }
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    if is_closed_redirect_target(&target) {
                        continue;
                    }
                    // `&> >(...)` / `&>> >(...)`: materialize_compound_output_
                    // process_substitutions (rubash#394) has already rewritten
                    // this list entry's `>(` text to the substitution's temp
                    // carrier, so the walk below binds fd 1 and fd 2 to the
                    // carrier exactly like GNU's r_err_and_out dup2 onto the
                    // pipe (redir.c:832-838) — the compound's own live
                    // writes (select prompt, loop diagnostics) follow it,
                    // and the body's leaf writes arrive through the per-leaf
                    // splice on the same carrier.
                    self.save_compound_output_fd(saved, 1);
                    self.save_compound_output_fd(saved, 2);
                    let append = redirect.kind == crate::parser::RedirectKind::CombinedAppend;
                    // rubash#1025: same prepare/bind pair as the Output
                    // arm below — consume the prepare phase's record.
                    // rubash#1868: probe form of the create (FIFO targets
                    // are access-checked, not opened — the bind open in
                    // set_fd_output_file is the single pairing open).
                    if !append && !self.take_noclobber_prepared_target(&target) {
                        crate::executor::shell_options::probe_output_redirect(
                            self,
                            &target,
                            redirect.clobber,
                        )?;
                    }
                    self.set_fd_output_file(1, target.clone(), false, append)
                        .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                    self.share_fd_output_file(1, 2, &target, false);
                }
                // Output | Append | ClobberOutput
                _ => {
                    self.save_compound_output_fd(saved, fd);
                    if is_closed_redirect_target(&target) {
                        self.fd_table.close_output(fd);
                        self.shell_state
                            .env_vars
                            .insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    // GNU redir.c:767-955 do_redirection_internal opens
                    // /dev/std*, /dev/fd/N and /proc/self/fd/N through the
                    // OS fd-alias layer: the kernel resolves the open() to a
                    // dup of fd N's CURRENT open file description
                    // (execution_misc::dev_stdio_redirect_fd documents the
                    // same for the input side, niubash#118). Opening the
                    // name as a literal Windows path instead bound the
                    // group's fd to the CONOUT$ console device, whose bytes
                    // bypass the real — possibly redirected or captured —
                    // stderr stream: a body's output vanished under pipes,
                    // and an external child re-opening the name failed with
                    // Permission denied (rubash#216). Dup the target fd's
                    // live binding here, with the implicit-std fallback the
                    // DuplicateOutput arm uses for a source with no
                    // fd-table entry (rubash#170 nest4).
                    if let Some(source_fd) = super::execution_misc::dev_stdio_redirect_fd(&target) {
                        if self.fd_table.dup_output(fd, source_fd).is_ok() {
                            self.record_output_fd_ledger(fd);
                            continue;
                        }
                        if source_fd <= 2 && !self.fd_table.entries.contains_key(&source_fd) {
                            let endpoint = if source_fd == 1 {
                                FdWriteEndpoint::Stdout
                            } else {
                                FdWriteEndpoint::Stderr
                            };
                            self.fd_table.open_output(fd, endpoint, false);
                            self.shell_state.env_vars.remove(&fd_closed_key(fd));
                            self.record_output_fd_ledger(fd);
                            continue;
                        }
                        // Source closed or unbound: fall through to the
                        // literal open, whose failure reports like GNU's
                        // open() on the alias would.
                    }
                    // rubash#1025: a noclobber target this compound's
                    // prepare phase already created (O_EXCL) must not be
                    // O_EXCL-opened again here — the record is consumed
                    // exactly once, so pre-existing files still fail in
                    // the prepare phase and bind-only scopes keep the
                    // plain check. rubash#1868: probe form of the create
                    // (FIFO targets are access-checked, not opened — the
                    // bind open in set_fd_output_file is the single
                    // pairing open).
                    if !redirect.append && !self.take_noclobber_prepared_target(&target) {
                        crate::executor::shell_options::probe_output_redirect(
                            self,
                            &target,
                            redirect.clobber,
                        )?;
                    }
                    // GNU redir.c do_redirection_internal: every `>file`
                    // redirection performs its OWN open() — `>f 2>f` binds
                    // two independent open file descriptions with
                    // independent offsets, so the fd-2 write overwrites the
                    // fd-1 write at offset 0 instead of concatenating
                    // (rubash#1068). A shared offset exists only where a dup
                    // created it (`2>&1`, `&>`, `>&word` — the dup arms
                    // above and share_fd_output_file). The earlier
                    // fd==2-forces-append model (rubash#826) made every
                    // compound fd-2 binding write at EOF, which is exactly
                    // the concatenation #1068 files; diagnostics routed
                    // through a plain `2>f` binding take the same
                    // independent-offset semantics as GNU's second open.
                    let append = redirect.append;
                    self.set_fd_output_file(fd, target.clone(), fd >= 10, append)
                        .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                }
            }
        }
        Ok(())
    }

    /// Scoped fd-table binding for a builtin that reparses its body in
    /// place (eval / fc / source / trap bodies): GNU applies the command's
    /// redirections to real descriptors for the command's whole duration
    /// (redir.c do_redirection_internal + execute_cmd.c undo on exit), so
    /// `eval 'echo x 1>&3' 3>&1` resolves fd 3 to the binding in force when
    /// eval ran.
    pub(crate) fn with_compound_output_redirects<T>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        let saved = self.open_compound_output_redirects(cmd)?;
        let result = execute(self);
        self.restore_compound_output_redirects(saved);
        result
    }

    pub(in crate::executor) fn restore_compound_output_redirects(
        &mut self,
        saved: Vec<SavedOutputFd>,
    ) {
        for saved in saved {
            match saved.entry {
                Some(entry) => {
                    self.fd_table.entries.insert(saved.fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&saved.fd);
                }
            }
            // Restore the dup2 snapshot record with the entry (rubash#223):
            // an inner scope may have cleared it (e.g. `2>/dev/null`
            // between the `2>&1` binding and the write).
            match saved.alias_generation {
                Some(record) => {
                    self.fd_table
                        .stdout_alias_generation
                        .insert(saved.fd, record);
                }
                None => {
                    self.fd_table.stdout_alias_generation.remove(&saved.fd);
                }
            }
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_output_key(saved.fd),
                saved.fd_output,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_output_process_substitution_key(saved.fd),
                saved.fd_procsub,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_closed_key(saved.fd),
                saved.fd_closed,
            );
        }
    }

    pub(in crate::executor) fn command_input_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        if let Some(input) = self.loop_redirect_input(cmd) {
            return Some(input);
        }

        if cmd.here_string_carrier.is_some() {
            // GNU redir.c here_document_to_fd: a here-string's word is
            // written to the pipe with a trailing newline — the compound's
            // staged stdin must end with it, like the direct-read path in
            // stdin_string_for_command_mut (rubash#270: without it,
            // `while read v; done <<< alpha` stages "alpha" and the read
            // ends at EOF-without-delimiter).
            let mut input = self.expand_here_string_mut_from_carrier(&cmd.here_string_carrier);
            input.push('\n');
            return Some(input);
        }
        if let Some(here_string) = cmd.here_string.clone() {
            // Here-string content already had quote removal applied by the
            // parser; expand only substitutions with quotes-as-data semantics.
            let mut input = self.expand_here_string_mut(&here_string);
            input.push('\n');
            return Some(input);
        }

        if cmd.heredoc_body.is_some() {
            return Some(self.expand_heredoc_body_mut_from_carrier(&cmd.heredoc_body));
        }
        if let Some(heredoc) = cmd.heredoc.clone() {
            return Some(self.expand_heredoc_body_mut(&heredoc));
        }

        cmd.heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd.is_none())
            .map(|redirect| {
                if redirect.body_carrier.is_some() {
                    if redirect.here_string {
                        self.expand_here_string_mut_from_carrier(&redirect.body_carrier)
                    } else {
                        self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier)
                    }
                } else {
                    redirect
                        .body
                        .as_deref()
                        .map(|body| self.expand_heredoc_body_mut(body))
                        .unwrap_or_default()
                }
            })
    }

    /// The canonical text of one heredoc/herestring redirect entry
    /// (redir.c here_document_to_fd). The form the parser stored decides
    /// the expansion ladder: a typed carrier expands through its
    /// snapshot; a here-string's STORAGE_WORD_PREFIX storage word is
    /// ANSI-C-decoded when it has that form, else word-expanded; a plain
    /// here-string body takes the here-string expansion; a heredoc body
    /// takes the heredoc expansion. A here-string's bytes always end
    /// with a newline — that terminator is what lets a final `read`
    /// still return the last line. Every staging consumer (compound fd
    /// binds, dynamic-fd allocation, /dev/fd materialization) must use
    /// this ladder: feeding the raw stored body to the heredoc
    /// expansion leaks the storage prefix into the delivered stream and
    /// drops the trailing newline (rubash#1867).
    /// Expand one STORAGE_WORD_PREFIX here-string word (the parser's
    /// stored token value) to its final data bytes: ANSI-C decode when
    /// the word kept its textual `$'...'` form, else word expansion.
    /// The result is a byte stream (redir.c here_document_to_fd), so
    /// the field-split transport guard on lexer-decoded `$'...'` blanks
    /// (ANSI_C_IFS_GUARD) must not survive into it — the same
    /// value-boundary strip the redirect-filename path applies
    /// (expand_word.rs, rubash#379); the strip is E400-aware, so a
    /// user-typed literal U+E401 stays data (rubash#735, rubash#1867).
    pub(in crate::executor) fn expand_stored_here_string_word(&mut self, word: &str) -> String {
        let input = decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
        crate::executor::markers::strip_ansi_c_ifs_guard(&input)
    }

    pub(in crate::executor) fn expand_heredoc_redirect_text(
        &mut self,
        redirect: &crate::parser::HereDocRedirect,
    ) -> String {
        if redirect.body_carrier.is_some() {
            let mut input = if redirect.here_string {
                self.expand_here_string_mut_from_carrier(&redirect.body_carrier)
            } else {
                self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier)
            };
            if redirect.here_string {
                input.push('\n');
            }
            return input;
        }
        let body = redirect.body.as_deref().unwrap_or("");
        if redirect.here_string {
            let mut input = if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
                self.expand_stored_here_string_word(word)
            } else {
                self.expand_here_string_mut(body)
            };
            input.push('\n');
            input
        } else {
            self.expand_heredoc_body_mut(body)
        }
    }

    /// Expands an unquoted heredoc body like Bash: parameter, command and
    /// arithmetic expansions run in the context of the receiving command.
    /// A dedicated quoted-heredoc marker keeps the body literal without sharing
    /// the compound-assignment transport protocol.
    pub(in crate::executor) fn expand_heredoc_body_mut(&mut self, body: &str) -> String {
        if let Some(pre) = preexpanded_stdin_body(body) {
            return decode_stdin_body_enq(pre);
        }
        let quoted = body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER);
        let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
        if quoted {
            return decode_stdin_body_enq(body);
        }
        // comsub-eof6: `read foo <<EOF` with body `$(seq 10` (missing `)`)
        // must not expand to `1`; GNU reports `command substitution:
        // unexpected EOF while looking for matching `)'` and leaves foo empty.
        // Detect unclosed `$(` in the raw body before expansion.
        if crate::lexer::has_unclosed_command_substitution(body) {
            // GNU error.c:300 parser_error with yy_input_name()=="command
            // substitution": `script: command substitution: line N:` — N is
            // the inherited script line when the comsub string runs out
            // (evalstring.c push_stream(0) keeps line_number), i.e. the
            // delimiter line: command line + body lines + 1.
            let eof_line = self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<usize>().ok())
                .unwrap_or(1)
                + body.lines().count()
                + 1;
            eprintln!(
                "{}unexpected EOF while looking for matching `)'",
                self.comsub_eof_error_diagnostic(eof_line)
            );
            return String::new();
        }
        let prepared = prepare_unquoted_heredoc_expansion(body);
        let expanded = self.expand_embedded_parameters_mut_with_context(
            &prepared,
            SubstitutionQuoteContext::HereDocument,
        );
        // Same decode boundary as the &self expand_heredoc_body below: the
        // mutable walker's substitution splices re-protect their output, so
        // C0 carriers and payload escapes must restore here or they leak as
        // raw bytes into the heredoc text.
        decode_stdin_body_enq(&decode_command_substitution_payload(
            &restore_command_substitution_output(&expanded),
        ))
    }

    /// Expands a heredoc body from a typed carrier. Returns the body verbatim
    /// if preexpanded; otherwise performs expansion.
    pub(in crate::executor) fn expand_heredoc_body_mut_from_carrier(
        &mut self,
        carrier: &Option<crate::parser::StdinBody>,
    ) -> String {
        match carrier {
            Some(crate::parser::StdinBody::Preexpanded(text)) => decode_stdin_body_enq(text),
            Some(crate::parser::StdinBody::NeedsExpansion(body)) => {
                self.expand_heredoc_body_mut(body)
            }
            None => String::new(),
        }
    }

    pub(in crate::executor) fn expand_heredoc_body_readback(
        &self,
        body: &str,
    ) -> SubstitutionOutput {
        let expanded = self.expand_heredoc_body(body);
        // expand_heredoc_body returns shell text; readback takes raw bytes,
        // so decode marker pairs once instead of re-encoding them.
        SubstitutionOutput::readback(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&expanded),
            0,
            SubstitutionQuoteContext::HereDocument,
        )
    }

    /// Readback variant of `expand_heredoc_body_mut_from_carrier`: a
    /// `Preexpanded` carrier is already expanded text (preexpand_command_stdin
    /// ran at the GNU do_redirections point), so it returns verbatim —
    /// expanding again would re-run embedded substitutions and collapse
    /// backslash sequences a second time (e.g. `c\\`<newline>`d` -> `cd`).
    pub(in crate::executor) fn expand_heredoc_body_readback_from_carrier(
        &self,
        carrier: &Option<crate::parser::StdinBody>,
        fallback_body: Option<&str>,
    ) -> SubstitutionOutput {
        match carrier {
            Some(crate::parser::StdinBody::Preexpanded(text)) => {
                return SubstitutionOutput::readback(
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                        &decode_stdin_body_enq(text),
                    ),
                    0,
                    SubstitutionQuoteContext::HereDocument,
                );
            }
            Some(crate::parser::StdinBody::NeedsExpansion(body)) => {
                return self.expand_heredoc_body_readback(body);
            }
            None => {}
        }
        self.expand_heredoc_body_readback(fallback_body.unwrap_or_default())
    }

    pub(in crate::executor) fn expand_heredoc_body(&self, body: &str) -> String {
        if let Some(pre) = preexpanded_stdin_body(body) {
            return decode_stdin_body_enq(pre);
        }
        let quoted = body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER);
        let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
        if quoted {
            return decode_stdin_body_enq(body);
        }
        if crate::lexer::has_unclosed_command_substitution(body) {
            // Same GNU parser_error shape as the mut variant above: the
            // reported line is the heredoc delimiter line (command line +
            // body lines + 1), since the comsub inherits the outer
            // line_number (evalstring.c push_stream(0)).
            let eof_line = self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<usize>().ok())
                .unwrap_or(1)
                + body.lines().count()
                + 1;
            eprintln!(
                "{}unexpected EOF while looking for matching `)'",
                self.comsub_eof_error_diagnostic(eof_line)
            );
            return String::new();
        }
        let expanded =
            self.expand_embedded_parameters_for_heredoc(&prepare_unquoted_heredoc_expansion(body));
        decode_stdin_body_enq(&decode_command_substitution_payload(
            &restore_command_substitution_output(&expanded),
        ))
    }
}

fn prepare_unquoted_heredoc_expansion(body: &str) -> String {
    let mut output = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    let mut command_depth = 0usize;
    let mut single = false;
    let mut double = false;
    while let Some(ch) = chars.next() {
        if ch == '$' && !single && chars.peek() == Some(&'(') {
            chars.next();
            output.push('$');
            output.push('(');
            command_depth += 1;
            continue;
        }

        if command_depth > 0 {
            match ch {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '(' if !single && !double => command_depth += 1,
                ')' if !single && !double => command_depth = command_depth.saturating_sub(1),
                _ => {}
            }
        }

        if ch != '\\' {
            output.push(ch);
            continue;
        }

        if command_depth > 0 && single {
            output.push('\\');
            continue;
        }

        let mut slash_count = 1usize;
        while chars.peek() == Some(&'\\') {
            chars.next();
            slash_count += 1;
        }

        if matches!(chars.peek(), Some('\n') | Some('\r')) {
            for _ in 0..(slash_count / 2) {
                output.push('\\');
            }
            if slash_count % 2 == 1 {
                if chars.peek() == Some(&'\r') {
                    chars.next();
                }
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            continue;
        }

        for _ in 0..(slash_count / 2) {
            output.push(crate::executor::markers::DATA_BACKSLASH);
        }
        if slash_count % 2 == 0 {
            continue;
        }

        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                output.push(DATA_DOLLAR);
            }
            Some('`') => {
                chars.next();
                output.push(crate::executor::markers::DATA_BACKTICK);
            }
            Some('\\') => unreachable!("backslash runs are consumed above"),
            _ => output.push('\\'),
        }
    }
    output
}

#[allow(dead_code)]
fn strip_heredoc_body(body: &str) -> String {
    strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
}

impl Executor {
    /// The compound's fd-0 `< file` source when it is a real stream
    /// object on disk — a regular file, a FIFO, or a character/block
    /// device: the sources whose kernel reads are an on-demand stream
    /// over one shared open file description, exactly GNU's
    /// shared-offset descriptor semantics (rubash#260; see
    /// with_command_input_redirects_inner). Everything else — process
    /// substitutions, /dev-fd aliases, /proc synthetic files,
    /// /dev/null, missing files, directories, `<>` read-write opens —
    /// returns None and keeps the FUNCTION_STDIN text-mirror transport.
    /// The distinction is load-bearing for FIFOs (rubash#1861): the
    /// mirror's fs::read_to_string (loop_redirect_input) reads a FIFO
    /// to EOF before the body runs — the bind stalls until the writer
    /// closes, the over-read bytes die with the mirror, and a later
    /// opener misses the writer entirely — while the FileFd endpoint
    /// reads byte-wise on demand (FdTable::read_file_bytes; only
    /// seekable disk files take the buffered + rewind path), so the
    /// stream is consumed exactly as GNU's unbuffered pipe reads are
    /// (read.def keeps pipes unbuffered for the same reason).
    fn compound_fd0_stream_file(&mut self, cmd: &CommandNode) -> Option<Rc<FileFd>> {
        if std::env::var_os("RB_721").is_some() {
            eprintln!(
                "[721] compound_stdin_open_failure redirect_in={:?} redirects={:?}",
                cmd.redirect_in
                    .as_deref()
                    .map(|r| format!("{} {:?}", r.target, r.kind)),
                cmd.redirects
                    .iter()
                    .map(|r| format!("{} {:?}", r.target, r.kind))
                    .collect::<Vec<_>>()
            );
        }
        let redirect = cmd.redirect_in.as_deref()?;
        if redirect.fd.unwrap_or(0) != 0 || redirect.fd_var.is_some() {
            return None;
        }
        if redirect.kind != crate::parser::RedirectKind::Input {
            return None;
        }
        if redirect.target.starts_with("<(") {
            return None;
        }
        let target = self.expand_redirect_target(redirect);
        if super::execution_misc::is_closed_redirect_target(&target)
            || super::execution_misc::dev_stdio_redirect_fd(&target).is_some()
            || is_null_device(&target)
        {
            return None;
        }
        if crate::special_paths::synthetic_proc_content(&target).is_some() {
            return None;
        }
        if crate::special_paths::is_synthetic_link_path(&target) {
            return None;
        }
        // niubash#177: same operand resolution as open_input_redirect so
        // this regular-file probe agrees with the read that follows.
        let path = self.session_path(super::path::windows_operand_file_path(
            super::path::shell_path_to_windows(&target, &self.shell_state.env_vars),
            &self.shell_state.env_vars,
        ));
        if !std::fs::metadata(&path)
            .map(|meta| Self::compound_fd0_stream_source(&meta))
            .unwrap_or(false)
        {
            return None;
        }
        FileFd::open_read(path).ok()
    }

    /// The descriptor-bindable source class for a compound's fd-0
    /// `< file` (see compound_fd0_stream_file): regular files plus —
    /// on unix — FIFOs and character/block devices, the filesystem
    /// objects that yield a live kernel stream from open(2). Windows
    /// has no filesystem FIFOs or device nodes of this kind, so the
    /// class is regular files only there.
    fn compound_fd0_stream_source(meta: &std::fs::Metadata) -> bool {
        if meta.is_file() {
            return true;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            let file_type = meta.file_type();
            file_type.is_fifo() || file_type.is_char_device() || file_type.is_block_device()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// GNU redir.c do_redirection_internal (redir.c:767-955) opens a
    /// compound command's fd-0 input redirection before the command runs;
    /// open failure aborts the compound with the diagnostic and status 1
    /// (rubash#137). Mirrors loop_redirect_input's target resolution:
    /// closed targets (`<&-`), process substitutions (`<(`), /dev/fd
    /// aliases, /proc synthetic files and append-creates never fail here.
    pub(in crate::executor) fn compound_stdin_open_failure(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        let redirect = cmd.redirect_in.as_deref()?;
        if redirect.fd.unwrap_or(0) != 0 {
            return None;
        }
        if redirect.target.starts_with("<(") {
            return None;
        }
        let target = self.expand_redirect_target(redirect);
        if super::execution_misc::is_closed_redirect_target(&target) {
            return None;
        }
        if let Some(source) = super::execution_misc::dev_stdio_redirect_fd(&target) {
            // rubash#721: `/dev/fd/N` names descriptor N. GNU's open of the
            // alias fails ENOENT when N is closed (redir.c
            // redirection_error over the dangling /proc/self/fd magic
            // symlink), and the compound is abandoned with that diagnostic.
            // Treating every alias as available staged an empty mirror, so
            // `while read ... done < /dev/fd/6` ran zero iterations and
            // reported 0.
            //
            // This must run BEFORE the `&N` probe below:
            // redirect_target_fd_and_move falls back to dev_stdio_redirect_fd
            // for a word without `&`, so it answers Some for `/dev/fd/N` too
            // and would short-circuit this check.
            if !super::redirection::dup_source_fd_open(
                &self.fd_table,
                source,
                &std::collections::HashMap::new(),
            ) {
                return Some(format!(
                    "{}{target}: No such file or directory",
                    self.error_prefix()
                ));
            }
            return None;
        }
        // `<&N` / `<&N-` dups are descriptor operations, not path opens —
        // their source validity is enforced by validate_builtin_dup_source
        // (builtins) and open_compound_numbered_input_redirects's dup_entry
        // (compounds); probing "&N" as a file here would fail every dup
        // form with a bogus ENOENT (rubash#575: `exec cat <&0` inside a
        // command substitution must run, not abort).
        if super::execution_misc::redirect_target_fd_and_move(&target).is_some() {
            return None;
        }
        if redirect.append {
            return None;
        }
        if crate::special_paths::synthetic_proc_content(&target).is_some() {
            return None;
        }
        if crate::special_paths::is_synthetic_link_path(&target) {
            return None;
        }
        // niubash#177: same operand resolution as open_input_redirect —
        // a virtual-root input operand may exist only under its `.exe`
        // spelling on disk.
        let path = self.session_path(super::path::windows_operand_file_path(
            super::path::shell_path_to_windows(&target, &self.shell_state.env_vars),
            &self.shell_state.env_vars,
        ));
        // `<> word` (r_input_output) opens O_RDWR|O_CREAT (make_cmd.c:682),
        // so a missing name is CREATED by the open and never fails here
        // (rubash#264); a genuine open error (bad directory, permission)
        // still aborts the compound.
        // rubash#1820: the plain-input probe must not open a FIFO — the
        // throwaway O_RDONLY open consumes the rendezvous pairing the
        // real acquisition (the FUNCTION_STDIN slurp, or a live fd-0
        // bind) needs, and blocks until a writer appears in its own
        // right. An existing FIFO validates through access(2) instead
        // (probe_input_redirect_gate), with the same errno surface.
        let probe_error = if redirect.kind == crate::parser::RedirectKind::ReadWrite {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&path)
                .err()
        } else if super::shell_options::redirect_path_is_fifo(&path) {
            super::shell_options::probe_input_redirect_gate(&path).err()
        } else {
            std::fs::File::open(&path).err()
        };
        match probe_error {
            None => None,
            Some(error) => Some(format!(
                "{}{target}: {}",
                self.error_prefix(),
                crate::posix_errors::message(&error)
            )),
        }
    }
}

impl Executor {
    /// Bind ONLY the numbered (>2) input and output redirects of `cmd` on
    /// the fd table for `body`'s duration — the narrow subset of
    /// with_command_input_redirects that a function-definition redirect
    /// list needs (GNU execute_cmd.c:854 applies tc->redirects when the
    /// stored body command runs). The full wrapper is wrong here: its
    /// fd-0/FUNCTION_STDIN text-channel bookkeeping belongs to compounds
    /// with stdin redirects, and saving/restoring it around a function
    /// body corrupted the driver's pending-stdin state.
    pub(in crate::executor) fn with_numbered_fd_bindings<T: Default>(
        &mut self,
        cmd: &CommandNode,
        body: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        if let Some((index, name)) = self.compound_output_dup_bad_fd(cmd) {
            return self.abandon_compound_for_dup_bad_fd(cmd, index, &name);
        }
        let saved_output = self.open_compound_output_redirects(cmd)?;
        // rubash#1867: the numbered heredoc/herestring entries stage
        // through the same wrap as every other compound — a definition
        // carrying `3<<<word` binds the text for the body's duration.
        let result = self.with_loop_fd_heredocs(cmd, body);
        self.restore_compound_output_redirects(saved_output);
        self.restore_compound_numbered_input_redirects(saved_numbered);
        result
    }

    /// rubash#1867: bind a function CALL's numbered input redirections
    /// and heredoc/herestring entries for the body's duration (GNU
    /// execute_cmd.c:5606 do_redirections over the call, undone at
    /// return). The call machinery already covers fd 0
    /// (function_call_stdin) and the outputs
    /// (with_compound_output_redirects); the numbered inputs were
    /// never opened at all, so `f 3<file` / `f 3<<<word` left the body
    /// reading a bad fd.
    pub(crate) fn with_call_numbered_input_redirects<T>(
        &mut self,
        cmd: &CommandNode,
        body: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        let result = self.with_loop_fd_heredocs(cmd, body);
        self.restore_compound_numbered_input_redirects(saved_numbered);
        result
    }

    /// GNU redir.c:1169-1171 (`else if (dup2 (redir_fd, redirector) < 0)
    /// return (errno)`) with redir.c:149-176 redirection_error: a dup whose
    /// source fd is not open fails EBADF, reports `<fd>: Bad file
    /// descriptor`, and ABANDONS the compound without running the body —
    /// the element's status is 1 (WSL 5.3.0 probe: `{ printf X >&2; } 2>&3
    /// 3>&1 | cat` prints the diagnostic once, the pipe stays empty,
    /// PIPESTATUS[0] = 1). The open walk itself
    /// (`open_compound_output_redirects_inner` → fd_table::dup_output)
    /// cannot express the abandonment, so the callers preflight the list
    /// (`compound_output_dup_bad_fd`) and route the failure here. The
    /// diagnostic goes through the fd 2 binding only the redirects BEFORE
    /// the failing dup installed (`{ :; } 2>/dev/null 2>&3` is silent),
    /// because do_redirections stops at the failing step.
    fn abandon_compound_for_dup_bad_fd<T: Default>(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        name: &str,
    ) -> Result<T, ExecuteError> {
        let mut stderr = Vec::new();
        writeln!(
            stderr,
            "{}{name}: Bad file descriptor",
            self.diagnostic_prefix()
        )?;
        let mut prefix_cmd = cmd.clone();
        prefix_cmd.redirects.truncate(index);
        let _ = self.write_redirect_diagnostic_routed(&prefix_cmd, &stderr)?;
        self.exit_code = 1;
        // The compound command is terminated without running: its result is
        // the default (the same shape as the stdin open failure path in
        // with_command_input_redirects_inner).
        Ok(T::default())
    }

    /// Ordered preflight of a compound's output-dup redirects: the first
    /// `M>&N` (or `M>&N-`) whose source fd N is not open — judged with the
    /// same left-to-right opened/closed tracking as the simple-command
    /// preflight (`command_output_redirect_fails` /
    /// `dup_source_fd_open`) — returns `(index, diagnostic name)`. Pure:
    /// opens nothing, dups nothing.
    fn compound_output_dup_bad_fd(&self, cmd: &CommandNode) -> Option<(usize, String)> {
        let mut opened: HashMap<u32, bool> = HashMap::new();
        for (index, redirect) in cmd.redirects.iter().enumerate() {
            if redirect.fd_var.is_some() {
                continue;
            }
            let fd = match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput
                | crate::parser::RedirectKind::DuplicateOutput
                | crate::parser::RedirectKind::CloseOutput => redirect.fd.unwrap_or(1),
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => 1,
                _ => continue,
            };
            let target = self.expand_redirect_target(redirect);
            match redirect.kind {
                crate::parser::RedirectKind::CloseOutput => {
                    opened.insert(fd, false);
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    if is_closed_redirect_target(&target) {
                        opened.insert(fd, false);
                        continue;
                    }
                    if let Some((source_fd, _move_source)) = redirect_target_fd_and_move(&target) {
                        // Self-dup/self-move is a no-op that succeeds even
                        // for a closed fd (redir.c `redir_fd !=
                        // redirector` guard; rubash#1236) and leaves the
                        // opened bookkeeping unchanged.
                        if source_fd == fd {
                            continue;
                        }
                        if !dup_source_fd_open(&self.fd_table, source_fd, &opened) {
                            let raw_word = redirect
                                .target_metadata
                                .raw
                                .trim_start_matches('&')
                                .trim_end_matches('-');
                            return Some((index, dup_bad_fd_name(raw_word, fd, source_fd)));
                        }
                        let open = dup_source_fd_open(&self.fd_table, source_fd, &opened);
                        opened.insert(fd, open);
                        continue;
                    }
                    // `>&word` / `1>&word` (r_err_and_out) opens the word —
                    // the open walk reports its failures.
                    opened.insert(fd, true);
                }
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    opened.insert(1, true);
                    opened.insert(2, true);
                }
                // Output | Append | ClobberOutput: a file open binds its
                // redirector; an fd-alias name (/dev/fd/N) dups the alias.
                _ => {
                    if is_closed_redirect_target(&target) {
                        opened.insert(fd, false);
                    } else if let Some(source) =
                        super::execution_misc::dev_stdio_redirect_fd(&target)
                    {
                        let open = dup_source_fd_open(&self.fd_table, source, &opened);
                        opened.insert(fd, open);
                    } else {
                        opened.insert(fd, true);
                    }
                }
            }
        }
        None
    }
}
