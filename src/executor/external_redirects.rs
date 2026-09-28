use super::*;

/// One resolved child-stdio endpoint for an external command's fd 1/2,
/// produced by the ordered walk in [`Executor::plan_external_stdio`].
/// Values follow POSIX dup2 semantics: cloning a slot duplicates the SAME
/// open file description (`File::try_clone` on Windows is a
/// `DuplicateHandle` onto the same kernel object, so the file pointer and a
/// console screen buffer stay shared) — redir.c:1169-1170
/// `/* This is correct.  2>&1 means dup2 (1, 2); */`.
enum ExternalStdioSlot {
    /// The live fd-1 object: the active capture pipe when a command
    /// substitution owns fd 1 (GNU subst.c:7143 command_substitute hands the
    /// child the pipe write end), otherwise the real process stdout. Every
    /// fd holding this slot shares ONE pipe/handle so merged writes
    /// interleave in true write order.
    LiveStdout,
    /// The live fd-2 object (real process stderr; stderr has no in-process
    /// capture on the external spawn path).
    LiveStderr,
    /// A dup2 snapshot of the REAL process stdout object taken when an
    /// earlier `exec 2>&1`-style dup bound this fd (rubash#170/#223: the
    /// record was taken with no capture active, so the object is the raw
    /// process stdout, never a later-rebound fd 1).
    RealStdoutSnapshot,
    /// A concrete open description: a redirect-opened file, a dup of an
    /// fd-table File endpoint, or a dup of a real stdio handle.
    Handle(File),
    /// A file redirect onto fd>=3 (`3>g`): the child's binding is owned by
    /// the `__RUBASH_FD_*` env-key channel in spawn_external_process, which
    /// opens the file itself — the plan must NOT open it too (a second
    /// noclobber open of the just-created file would fail and silently drop
    /// the child's fd). Resolving this slot as a dup source declines the
    /// whole plan (`3>g 2>&3` keeps the legacy route).
    EnvChannelFile,
    Null,
}

impl ExternalStdioSlot {
    fn try_clone(&self) -> Option<Self> {
        match self {
            Self::LiveStdout | Self::LiveStderr | Self::RealStdoutSnapshot | Self::Null => {
                Some(match self {
                    Self::LiveStdout => Self::LiveStdout,
                    Self::LiveStderr => Self::LiveStderr,
                    Self::RealStdoutSnapshot => Self::RealStdoutSnapshot,
                    _ => Self::Null,
                })
            }
            Self::Handle(file) => file.try_clone().ok().map(Self::Handle),
            Self::EnvChannelFile => None,
        }
    }
}

/// Outcome of a successful external stdio plan (see
/// [`Executor::plan_external_stdio`]). Debug skips the reader (a pipe
/// handle with no meaningful debug text).
pub(in crate::executor) enum ExternalStdioOutcome {
    /// The child owns real live handles for fd 1/2; nothing to drain.
    LiveHandles,
    /// The plan bound one or both fds to the active capture through ONE
    /// shared pipe: drain this reader to EOF and write the bytes into the
    /// capture (the merge order is already inside the stream).
    CapturePipe(os_pipe::PipeReader),
}

impl std::fmt::Debug for ExternalStdioOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LiveHandles => f.write_str("LiveHandles"),
            Self::CapturePipe(_) => f.write_str("CapturePipe(..)"),
        }
    }
}

/// The realized stdio plan handed to `std::process::Command`: which of the
/// child's fd 1/fd 2 stay inherited, and the reader side of the shared
/// capture pipe (when the plan bound any fd to the active capture) that
/// `spawn_external_process` drains into the capture after the child exits.
struct ExternalStdioPlan {
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    capture_reader: Option<os_pipe::PipeReader>,
}

impl Executor {
    pub(in crate::executor) fn apply_external_redirects(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        self.apply_external_stdin_redirect(cmd, process)?;
        // niubash#144: for EXTERNAL children the merged-output family gets
        // real shared handles (one open file description, or one capture
        // pipe) — the single true path; there is no capture-replay route
        // anymore. The planner declines only shapes it cannot express as
        // live handles; those flow into the per-fd legacy branches below,
        // which mark any stream they pipe so the parent drains it
        // (external_stdio_piped_fallback).
        self.external_stdio_piped_fallback = false;
        match self.plan_external_stdio(cmd)? {
            Some(plan) => {
                let ExternalStdioPlan {
                    stdout,
                    stderr,
                    capture_reader,
                } = plan;
                if let Some(stdio) = stdout {
                    process.stdout(stdio);
                }
                if let Some(stdio) = stderr {
                    process.stderr(stdio);
                }
                self.external_stdio_outcome = Some(match capture_reader {
                    Some(reader) => ExternalStdioOutcome::CapturePipe(reader),
                    None => ExternalStdioOutcome::LiveHandles,
                });
                return Ok(());
            }
            None => self.external_stdio_outcome = None,
        }
        // GNU redir.c r_close_this (do_redirection_internal): a closed fd
        // 1/2 stays closed in the child, its write() fails EBADF, and the
        // shell reports `write error: Bad file descriptor` with status 1
        // (builtins/common.c:320-334 sh_chkwrite folds the failed write
        // into the status). The child itself writes into these pipes; the
        // parent routes the bytes through the ordered state
        // (write_external_captured_streams), which produces the diagnostic
        // and the status. A null-device Stdio instead would silently
        // swallow the writes (fd_redirects
        // c_external_command_reports_write_error_for_closed_stdout).
        if self.external_output_fds_closed(cmd) {
            process.stdout(Stdio::piped());
            process.stderr(Stdio::piped());
            self.external_stdio_piped_fallback = true;
            return Ok(());
        }
        if self.apply_external_combined_output_redirect(cmd, process)? {
            return Ok(());
        }
        self.apply_external_stdout_redirect(cmd, process)?;
        self.apply_external_stderr_redirect(cmd, process)?;
        Ok(())
    }

    /// GNU redir.c r_close_this: `>&-` / `2>&-` (RedirectKind::CloseOutput,
    /// which also covers `>&N-`-with-N-closed via the parser's kind
    /// classification) leaves fd 1/2 CLOSED in the child, and an ambient
    /// fd-table close (`exec >&-`) has the same effect. True when either
    /// std fd of this command resolves closed.
    fn external_output_fds_closed(&self, cmd: &CommandNode) -> bool {
        (1..=2).any(|fd| self.fd_table.has_entry(fd) && !self.fd_table.is_open_for_write(fd))
            || cmd.redirects.iter().any(|redirect| {
                redirect.fd.map_or(true, |fd| fd <= 2)
                    && matches!(redirect.kind, crate::parser::RedirectKind::CloseOutput)
            })
    }

    /// GNU redir.c do_redirection_internal for an external child's fd 1/2:
    /// walk the command's redirection list left to right keeping real open
    /// descriptions, so `> f 2>&1` ends with both fds on ONE file handle
    /// (redir.c:1030-1037 r_err_and_out does `dup2 (1, 2)` after fd 1's
    /// open), `2>&1 > f` keeps fd 2 on the original stdout (the dup saw the
    /// pre-open fd 1), and `>&word` with redirector 1 becomes the shared
    /// r_err_and_out open (redir.c:832-838 converting
    /// r_duplicating_output_word with redirector==1). Returns None when a
    /// shape is outside the planner's coverage (fd_var forms, coproc /
    /// process-substitution endpoints, dup snapshots recorded against an old
    /// capture generation — rubash#223, fd-1 Stderr endpoints whose stderr
    /// staleness is untracked — so the legacy routes keep owning them.
    fn plan_external_stdio(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<ExternalStdioPlan>, ExecuteError> {
        use crate::parser::RedirectKind;

        let capture_active = self.stdout_capture.is_some()
            || crate::executor::shell_options::stdout_capture_active();

        // Ambient fd 1/2 seeding from the fd table (the shell's current
        // bindings, exactly what a forked GNU child inherits).
        let mut slots: HashMap<u32, ExternalStdioSlot> = HashMap::new();
        let mut tracked_high_fds: HashMap<u32, ExternalStdioSlot> = HashMap::new();
        // fds whose ambient binding a compound already REPLACED (a group's
        // open or dup — anything but the pristine default fd 1=stdout /
        // fd 2=stderr). An injected group redirect on such a fd is already
        // realized by that binding (see injected_redirect_fd_is_bound and
        // apply_stderr_append_redirect, which injects `2>&1`-style dups
        // into leaves as marked entries); re-walking them would dup the
        // FINAL fd 1 instead of the stdout the dup saw.
        let mut compound_bound_fds: Vec<u32> = Vec::new();
        for fd in [1u32, 2u32] {
            let default_slot = if fd == 1 {
                ExternalStdioSlot::LiveStdout
            } else {
                ExternalStdioSlot::LiveStderr
            };
            let is_default_slot = |slot: &ExternalStdioSlot| match (fd, slot) {
                (1, ExternalStdioSlot::LiveStdout) | (2, ExternalStdioSlot::LiveStderr) => true,
                _ => false,
            };
            let Some(entry) = self.fd_table.entries.get(&fd) else {
                slots.insert(fd, default_slot);
                continue;
            };
            if entry.closed || entry.write.is_none() {
                // Closed fd (legacy maps it to /dev/null for the child too).
                slots.insert(fd, ExternalStdioSlot::Null);
                compound_bound_fds.push(fd);
                continue;
            }
            let slot = match entry.write.as_ref().expect("checked above") {
                FdWriteEndpoint::Stdout => {
                    if fd == 1 {
                        // The live fd-1 alias.
                        ExternalStdioSlot::LiveStdout
                    } else {
                        // dup2 snapshot of the stdout OBJECT (rubash#170/#223):
                        // the record names WHICH stdout object the fd holds.
                        // When the recorded capture generation is STILL the
                        // active thread-local capture, that object is the live
                        // capture pipe fd 1 uses — the one-open-description
                        // merge GNU's dup2 produces (redir.c:1169-1170
                        // `2>&1 means dup2 (1, 2)`), so both fds share ONE
                        // pipe and the merged bytes interleave in true write
                        // order. This is exactly the shape a compound stage
                        // leaves behind (`{ sh ...; } 2>&1 | cat`: the group
                        // dup records the stage capture's generation, and the
                        // leaf's external child still runs inside that same
                        // capture scope). Any other record — the real process
                        // stdout (no capture active at dup time), or a STALE
                        // generation (an outer capture's buffer, which a live
                        // child handle cannot express) — keeps the legacy
                        // ambient route.
                        match self.fd_table.stdout_alias_generation.get(&fd) {
                            Some(Some(generation)) => {
                                let still_active =
                                    crate::executor::shell_options::stdout_capture_active()
                                        && *generation == crate::executor::shell_options::stdout_capture_generation();
                                if still_active {
                                    ExternalStdioSlot::LiveStdout
                                } else {
                                    return Ok(None);
                                }
                            }
                            Some(None) | None => ExternalStdioSlot::RealStdoutSnapshot,
                        }
                    }
                }
                FdWriteEndpoint::Stderr => {
                    if fd == 2 {
                        ExternalStdioSlot::LiveStderr
                    } else {
                        // No stderr generation bookkeeping exists (#279 keeps
                        // the write-side split); staleness of an fd-1 Stderr
                        // endpoint is untrackable here — decline.
                        return Ok(None);
                    }
                }
                FdWriteEndpoint::File(file_fd) => {
                    let dup = crate::fd::duplicate_handle(file_fd.handle)?;
                    ExternalStdioSlot::Handle(crate::fd::handle_to_file(dup))
                }
                // Coproc / process-substitution endpoints have dedicated
                // legacy spawn plumbing.
                FdWriteEndpoint::CoprocStdin { .. }
                | FdWriteEndpoint::ProcessSubstitution { .. } => return Ok(None),
            };
            if !is_default_slot(&slot) {
                compound_bound_fds.push(fd);
            }
            slots.insert(fd, slot);
        }

        let resolve_dup_source = |_this: &Executor,
                                  slots: &HashMap<u32, ExternalStdioSlot>,
                                  high: &HashMap<u32, ExternalStdioSlot>,
                                  source: u32|
         -> Option<ExternalStdioSlot> {
            if let Some(slot) = slots.get(&source) {
                return slot.try_clone();
            }
            // A file OPENED onto fd>=3 (`3>g`) is re-opened independently by
            // the __RUBASH_FD_* env-key channel; dup'ing it here would create
            // a second description whose offset diverges from the child's
            // own fd. GNU shares one description — the EnvChannelFile marker
            // declines instead.
            high.get(&source)?.try_clone()
        };

        for redirect in &cmd.redirects {
            match redirect.kind {
                RedirectKind::CombinedOutput | RedirectKind::CombinedAppend => {}
                RedirectKind::Output
                | RedirectKind::Append
                | RedirectKind::ClobberOutput
                | RedirectKind::DuplicateOutput
                | RedirectKind::CloseOutput => {}
                _ => continue,
            }
            if redirect.fd_var.is_some()
                || redirect.target.starts_with("<(")
                || redirect.target.starts_with(">(")
            {
                return Ok(None);
            }
            let target = self.expand_redirect_target(redirect);
            let fd = redirect.fd.unwrap_or(1);

            // An injected group redirect mirrors a redirection the compound
            // already applied to its real descriptors (both the fd-1 file
            // opens of apply_stdout_append_redirect and the `2>&1`-style
            // dups of apply_stderr_append_redirect); when the ambient
            // seeding above carries a REPLACED binding for that fd — the
            // group scope's open or dup — the leaf must inherit it, not
            // re-apply the redirect (a fresh truncating open would clobber
            // the shared description's bytes, and re-dup'ing `2>&1` would
            // copy the FINAL fd 1 instead of the stdout the dup saw). See
            // injected_redirect_fd_is_bound.
            if crate::executor::support_names::is_injected_group_redirect(redirect)
                && compound_bound_fds.contains(&fd)
            {
                continue;
            }

            match redirect.kind {
                RedirectKind::CombinedOutput | RedirectKind::CombinedAppend => {
                    // `&>file` / `&>>file` (make_cmd.c r_err_and_out family):
                    // one open, fd 2 dups it (redir.c:1030-1037).
                    let Some(file) = self.open_external_combined_target(&target, redirect)? else {
                        return Ok(None);
                    };
                    let stderr = file.try_clone().ok().map(ExternalStdioSlot::Handle);
                    let Some(stderr) = stderr else {
                        return Ok(None);
                    };
                    slots.insert(1, ExternalStdioSlot::Handle(file));
                    slots.insert(2, stderr);
                }
                RedirectKind::CloseOutput => {
                    // Closing fd 1/2 must keep the legacy ordered-capture
                    // route: the child's write to the closed fd FAILS
                    // (EBADF) and the replay surfaces "write error: Bad
                    // descriptor" with status 1 in the parent — a null-device
                    // Stdio would silently swallow the writes instead
                    // (fd_redirects c_external_command_reports_write_error_
                    // for_closed_stdout).
                    if fd <= 2 {
                        return Ok(None);
                    }
                }
                RedirectKind::DuplicateOutput => {
                    if is_closed_redirect_target(&target) {
                        // Same write-failure semantics as CloseOutput.
                        if fd <= 2 {
                            return Ok(None);
                        }
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        let Some(slot) =
                            resolve_dup_source(self, &slots, &tracked_high_fds, source_fd)
                        else {
                            return Ok(None);
                        };
                        if fd <= 2 {
                            slots.insert(fd, slot);
                        } else {
                            tracked_high_fds.insert(fd, slot);
                        }
                        if move_source {
                            if source_fd <= 2 {
                                slots.insert(source_fd, ExternalStdioSlot::Null);
                            } else {
                                tracked_high_fds.remove(&source_fd);
                            }
                        }
                        continue;
                    }
                    let word = target.strip_prefix('&').unwrap_or(&target);
                    if fd == 1 {
                        // r_err_and_out (redir.c:832-838): `>&word` /
                        // `1>&word` with no varassign opens the word ONCE and
                        // dups fd 2 onto it (redir.c:1030-1037).
                        if word.is_empty() {
                            // Ambiguous/empty expansion was rejected by the
                            // precheck; do not open "" here.
                            return Ok(None);
                        }
                        let Some(file) = self.open_external_combined_target(word, redirect)? else {
                            return Ok(None);
                        };
                        let stderr = file.try_clone().ok().map(ExternalStdioSlot::Handle);
                        let Some(stderr) = stderr else {
                            return Ok(None);
                        };
                        slots.insert(1, ExternalStdioSlot::Handle(file));
                        slots.insert(2, stderr);
                    } else {
                        // Other dup-word forms are AMBIGUOUS_REDIRECT in GNU;
                        // the precheck owns the diagnostic — decline.
                        return Ok(None);
                    }
                }
                RedirectKind::Output | RedirectKind::Append | RedirectKind::ClobberOutput => {
                    if is_closed_redirect_target(&target) {
                        // Same write-failure semantics as CloseOutput.
                        if fd <= 2 {
                            return Ok(None);
                        }
                        continue;
                    }
                    if let Some(source_fd) = redirect_target_fd(&target) {
                        let Some(slot) =
                            resolve_dup_source(self, &slots, &tracked_high_fds, source_fd)
                        else {
                            return Ok(None);
                        };
                        if fd <= 2 {
                            slots.insert(fd, slot);
                        } else {
                            tracked_high_fds.insert(fd, slot);
                        }
                        continue;
                    }
                    if let Some(source) = dev_stdio_redirect_fd(&target) {
                        // `/dev/fd/N`, `/dev/stdout`, `/dev/stderr` resolve as
                        // dups of those objects.
                        let Some(slot) =
                            resolve_dup_source(self, &slots, &tracked_high_fds, source)
                        else {
                            return Ok(None);
                        };
                        if fd <= 2 {
                            slots.insert(fd, slot);
                        } else {
                            tracked_high_fds.insert(fd, slot);
                        }
                        continue;
                    }
                    if fd >= 3 {
                        // The child's fd>=3 file binding is opened by the
                        // __RUBASH_FD_* env-key channel in
                        // spawn_external_process; the plan must not open the
                        // file too (a second noclobber open of the
                        // just-created file would fail and silently drop the
                        // child's fd). The marker declines any later dup of
                        // this fd to the legacy route.
                        tracked_high_fds.insert(fd, ExternalStdioSlot::EnvChannelFile);
                        continue;
                    }
                    let Some(file) = self.open_external_combined_target(&target, redirect)? else {
                        return Ok(None);
                    };
                    let slot = ExternalStdioSlot::Handle(file);
                    slots.insert(fd, slot);
                }
                _ => unreachable!("filtered above"),
            }
        }

        // Realize the final fd 1/2 slots. `LiveStdout` is the one object
        // every merged fd must share: the active capture's pipe (one pipe,
        // both ends of the merge, drained into the capture at exit — GNU's
        // child holds the substitution pipe on fd 1 and dup2's fd 2 onto
        // it), or the real process stdout (fd 1 stays inherited; any OTHER
        // fd pointing there gets a DuplicateHandle so `rubash > log` still
        // merges into log, not the console).
        let mut capture_reader = None;
        let mut capture_writer: Option<os_pipe::PipeWriter> = None;
        let mut realize = |slot: &ExternalStdioSlot,
                           is_fd1: bool|
         -> Result<Option<Stdio>, ExecuteError> {
            Ok(match slot {
                ExternalStdioSlot::Null | ExternalStdioSlot::EnvChannelFile => {
                    // EnvChannelFile only reaches tracked_high_fds (fd
                    // >= 3), never the realized fd 1/2 slots.
                    Some(Stdio::null())
                }
                ExternalStdioSlot::LiveStderr => {
                    if is_fd1 {
                        Some(Stdio::from(dup_process_handle(2)?))
                    } else {
                        None
                    }
                }
                ExternalStdioSlot::RealStdoutSnapshot => Some(Stdio::from(dup_process_handle(1)?)),
                ExternalStdioSlot::LiveStdout => {
                    if capture_active {
                        if capture_writer.is_none() {
                            let (reader, writer) =
                                os_pipe::pipe().map_err(ExecuteError::IoError)?;
                            capture_reader = Some(reader);
                            capture_writer = Some(writer);
                        }
                        let clone = capture_writer.as_ref().unwrap().try_clone().ok();
                        let Some(clone) = clone else {
                            return Ok(None);
                        };
                        Some(Stdio::from(clone))
                    } else if is_fd1 {
                        None
                    } else {
                        Some(Stdio::from(dup_process_handle(1)?))
                    }
                }
                ExternalStdioSlot::Handle(file) => match file.try_clone() {
                    Ok(clone) => Some(Stdio::from(clone)),
                    Err(_) => return Ok(None),
                },
            })
        };

        let stdout_slot = slots
            .get(&1)
            .map(|slot| slot.try_clone())
            .unwrap_or(Some(ExternalStdioSlot::LiveStdout))
            .unwrap_or(ExternalStdioSlot::LiveStdout);
        let stderr_slot = slots
            .get(&2)
            .map(|slot| slot.try_clone())
            .unwrap_or(Some(ExternalStdioSlot::LiveStderr))
            .unwrap_or(ExternalStdioSlot::LiveStderr);
        let stdout = realize(&stdout_slot, true)?;
        let stderr = realize(&stderr_slot, false)?;
        Ok(Some(ExternalStdioPlan {
            stdout,
            stderr,
            capture_reader,
        }))
    }

    /// Opens one output target for the external stdio plan: `/dev/null` maps
    /// to the null slot, append forms open O_APPEND, plain/clobber forms go
    /// through the shell's noclobber-aware `create_redirect_output`. Returns
    /// `Ok(None)` for shapes the planner declines (fd-target words).
    fn open_external_combined_target(
        &self,
        target: &str,
        redirect: &Redirect,
    ) -> Result<Option<File>, ExecuteError> {
        if is_null_device(target) {
            let null = File::create(if cfg!(windows) { "NUL" } else { "/dev/null" })?;
            return Ok(Some(null));
        }
        if redirect_target_fd(target).is_some() || dev_stdio_redirect_fd(target).is_some() {
            return Ok(None);
        }
        let file = if redirect.append {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(target, &self.shell_state.env_vars))?
        } else {
            self.create_redirect_output(target, redirect.clobber)?
        };
        Ok(Some(file))
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
                    self.external_stdio_piped_fallback = true;
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
                self.external_stdio_piped_fallback = true;
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
                self.external_stdio_piped_fallback = true;
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
            self.external_stdio_piped_fallback = true;
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
                self.external_stdio_piped_fallback = true;
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
                    self.external_stdio_piped_fallback = true;
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
                self.external_stdio_piped_fallback = true;
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

    /// Does this external command's spawn need the parent to drain piped
    /// streams and route the bytes after exit? Only for shapes the stdio
    /// planner declined AND that carry an active capture (a child writing
    // the real stdout would leak past the capture, unixwin/niubash#93), a
    /// stream one of the per-fd fallback branches piped
    /// (external_stdio_piped_fallback), or a closed fd 1/2 whose write
    /// failure must be reported. A command the stdio plan owns never gets
    /// here (external_stdio_outcome is Some).
    pub(in crate::executor) fn external_needs_fd_copy_capture(&self, cmd: &CommandNode) -> bool {
        if self.external_stdio_outcome.is_some() {
            return false;
        }
        self.stdout_capture.is_some()
            || crate::executor::shell_options::stdout_capture_active()
            || self.external_stdio_piped_fallback
            || self.external_output_fds_closed(cmd)
    }

    /// Routes the drained stream pair of a piped-fallback external child.
    /// Every piped fallback shape resolves its fds to DISTINCT destinations
    /// (the stdio plan owns all same-object merges — one file handle or one
    /// shared capture pipe, niubash#144), so per-stream routing through the
    /// fd endpoints is exact: write_fd_endpoint follows the fd table's
    /// bound files, dup2 snapshot records (rubash#223 — including an outer
    /// capture generation), coproc pipes, and the active captures. The
    /// closed-output shapes report the sh_chkwrite `write error: Bad file
    /// descriptor` through the ordered state instead (the child's writes
    /// cannot reach a closed fd, so the routing outcome IS the observable
    /// behavior, status 1).
    pub(in crate::executor) fn write_external_captured_streams(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        if self.external_output_fds_closed(cmd) {
            let _ = self.route_builtin_buffered_output(cmd, stdout, stderr)?;
            return Ok(());
        }
        if !stdout.is_empty() {
            self.write_default_stdout(stdout)?;
        }
        if !stderr.is_empty() {
            self.write_default_stderr(stderr)?;
        }
        Ok(())
    }
}

/// A DuplicateHandle of the shell's own stdio object for fd 1/2 — the
/// handle-level equivalent of GNU's inherited descriptor: same console
/// screen buffer / file object, shared file pointer, written live by the
/// child (redir.c:1169-1170 dup2 semantics). Falls back to the null device
/// when the process has no such handle (GUI-subsystem edge).
fn dup_process_handle(fd: u32) -> std::io::Result<File> {
    let handle = crate::fd::process_std_handle(fd);
    if handle == 0 || handle == -1 {
        return File::create(if cfg!(windows) { "NUL" } else { "/dev/null" });
    }
    let dup = crate::fd::duplicate_handle(handle)?;
    Ok(crate::fd::handle_to_file(dup))
}
