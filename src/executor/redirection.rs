//! redirection module.
//!
//! GNU Bash source ownership:
// - redir.c
// - redir.h

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutputTarget {
    Stdout,
    Stderr,
    Null,
    CoprocStdin(u32),
    Path(String),
    /// GNU dup2 semantics: a write endpoint carried by fd_table refers to
    /// the same open file description — writes go through the live handle
    /// at the shared offset, not a fresh append-mode reopen. `N<>file`
    /// writes at offset 0 and `1>&6` shares fd 6's offset.
    SharedFile(Rc<FileFd>),
    Closed,
}

#[derive(Debug, Clone)]
struct OutputFdState {
    fds: HashMap<u32, OutputTarget>,
    saw_output_redirect: bool,
    redirect_failed: bool,
    /// Pipeline-routing mode: `OutputTarget::Stdout` stands for the pipe
    /// channel, not the process stdout. Diagnostics issued while resolving
    /// the element's redirects (a failed `>&N` dup after `2>&1`, say) are
    /// written to fd 2 — whose target may be the pipe — so writes to the
    /// Stdout target are captured here and merged into the routed pipe
    /// stream instead of escaping to real stdout.
    defer_stdout_writes: bool,
    deferred_stdout: std::cell::RefCell<Vec<u8>>,
}

impl Executor {
    /// A redirect injected from an enclosing compound
    /// (GROUP_REDIRECT_INJECTED_MARK) mirrors a redirection GNU applies once
    /// to the compound's real descriptors (redir.c:767-955
    /// do_redirection_internal). When the fd table already carries a File
    /// endpoint for that fd — the group scope's binding — the leaf must
    /// write through that shared open file description, not a fresh
    /// append-mode reopen whose offset starts at the file's end while the
    /// bound description's offset stays behind (or vice versa: a dup of the
    /// bound fd clobbers what append-channel leaves wrote). Returns true
    /// when the injected redirect is already realized by the live binding.
    pub(in crate::executor) fn injected_redirect_fd_is_bound(
        &self,
        redirect: &Redirect,
        fd: u32,
    ) -> bool {
        crate::executor::support_names::is_injected_group_redirect(redirect)
            && matches!(
                self.fd_table.write_endpoint(fd),
                Some(FdWriteEndpoint::File(_))
            )
    }

    pub(in crate::executor) fn reject_ambiguous_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        let mut redirects = cmd.redirects.iter();
        let mut candidates = Vec::new();
        candidates.extend(redirects.by_ref());
        for redirect in [
            cmd.redirect_in.as_ref(),
            cmd.redirect_out.as_ref(),
            cmd.append.as_ref(),
            cmd.redirect_err.as_ref(),
            cmd.redirect_err_append.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !candidates.iter().any(|candidate| *candidate == redirect) {
                candidates.push(redirect);
            }
        }

        for redirect in candidates {
            if matches!(redirect.kind, crate::parser::RedirectKind::HereString) {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            // GNU redir.c:832-838: [N]>&WORD with a non-numeric WORD is not
            // an error when the redirector is stdout and there is no
            // varassign - it translates to r_err_and_out (>&file == >file
            // 2>&1). Only other redirectors ({var}>&word, 2>&word, <&word)
            // report AMBIGUOUS_REDIRECT (redir.c:839-843).
            let dup_output_err_and_out =
                matches!(redirect.kind, crate::parser::RedirectKind::DuplicateOutput)
                    && redirect.fd.unwrap_or(1) == 1
                    && redirect.fd_var.is_none();
            let invalid_fd_target = target.starts_with('&')
                && !is_closed_redirect_target(&target)
                && redirect_target_fd_and_move(&target).is_none()
                && !dup_output_err_and_out;
            // GNU redir.c:325-333 redirection_expand: a redirect word that
            // expands to ZERO words (empty result, e.g. `>&$(true)`) returns
            // NULL -> report_ambiguous_redirect, same as a multi-word split.
            // `>&WORD` targets carry a leading `&` in this representation,
            // so the emptiness test applies to the word body.
            let empty_target = target.strip_prefix('&').unwrap_or(&target).is_empty()
                && !matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
                );
            if invalid_fd_target
                || empty_target
                || redirect_target_is_ambiguous(&redirect.target_metadata.raw, &target)
            {
                // GNU redir.c report_ambiguous_redirect (redir.c:846-870).
                // File redirections report the word as written ($z keeps its
                // raw spelling even when expansion is non-empty; redir.tests
                // line 58). Dup redirections report the expanded fd word
                // (redir.tests line 52: fd=-1 reports "-1") and fall back to
                // the raw word only when expansion is empty (redir4.sub
                // lines 45-46: unset fd reports "$fd").
                let is_dup_kind = matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::DuplicateInput
                        | crate::parser::RedirectKind::DuplicateOutput
                );
                let raw_word = redirect
                    .target_metadata
                    .raw
                    .strip_prefix('&')
                    .unwrap_or(&redirect.target_metadata.raw);
                let expanded_word = target.strip_prefix('&').unwrap_or(&target);
                let diagnostic_target = if is_dup_kind && !expanded_word.is_empty() {
                    expanded_word
                } else {
                    raw_word
                };
                let mut stderr = Vec::new();
                writeln!(
                    &mut stderr,
                    "{}{diagnostic_target}: ambiguous redirect",
                    self.diagnostic_prefix()
                )?;
                self.write_default_stderr(&stderr)?;
                self.exit_code = 1;
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub(in crate::executor) fn command_output_redirect_fails(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        let mut state = self.command_output_fd_state();
        if !self.apply_ordered_output_redirects(cmd, &mut state)? {
            return Ok(false);
        }
        Ok(state.redirect_failed)
    }

    pub(in crate::executor) fn write_ordered_command_output(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<bool, ExecuteError> {
        self.last_builtin_write_failed.set(false);
        let mut state = self.command_output_fd_state();
        if !self.apply_ordered_output_redirects(cmd, &mut state)? {
            // A persistent `exec >&N-` closes the shell fd without adding a
            // redirect to the current command. Buffered builtins still need
            // Bash's write diagnostic instead of silently dropping output.
            if !stdout.is_empty()
                && state
                    .fd_target(1)
                    .is_some_and(|target| *target == OutputTarget::Closed)
            {
                if builtin_output_write_is_unchecked(cmd) {
                    return Ok(true);
                }
                self.write_bad_output_fd_diagnostic(cmd, 1)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                return Ok(true);
            }
            if !stderr.is_empty()
                && state
                    .fd_target(2)
                    .is_some_and(|target| *target == OutputTarget::Closed)
            {
                self.write_bad_output_fd_diagnostic(cmd, 2)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                return Ok(true);
            }
            if !stdout.is_empty()
                && matches!(state.fd_target(1), Some(OutputTarget::CoprocStdin(_)))
            {
                self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
                return Ok(true);
            }
            if !stderr.is_empty()
                && matches!(state.fd_target(2), Some(OutputTarget::CoprocStdin(_)))
            {
                self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
                return Ok(true);
            }
            return Ok(false);
        }
        if state.redirect_failed {
            return Ok(true);
        }

        if !stdout.is_empty()
            && state
                .fd_target(1)
                .is_some_and(|target| *target == OutputTarget::Closed)
        {
            if builtin_output_write_is_unchecked(cmd) {
                return Ok(true);
            }
            self.write_bad_output_fd_diagnostic(cmd, 1)?;
            self.exit_code = 1;
            self.last_builtin_write_failed.set(true);
            return Ok(true);
        }
        if !stderr.is_empty()
            && state
                .fd_target(2)
                .is_some_and(|target| *target == OutputTarget::Closed)
        {
            self.write_bad_output_fd_diagnostic(cmd, 2)?;
            self.exit_code = 1;
            self.last_builtin_write_failed.set(true);
            return Ok(true);
        }

        self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
        self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
        Ok(true)
    }

    /// Builtin wrappers that return their status (`Ok(status)` after a
    /// buffered write) must merge the write-failure flag the way GNU's
    /// `sh_chkwrite` folds a flush error into the builtin's return value
    /// (builtins/common.c:320-334). Returns and clears the flag.
    pub(in crate::executor) fn take_builtin_write_failed(&mut self) -> bool {
        self.last_builtin_write_failed.replace(false)
    }

    /// GNU execute_cmd.c execute_simple_command runs `expand_words`
    /// (execute_cmd.c:4617) BEFORE the command's own `do_redirections`
    /// (execute_builtin_or_function at execute_cmd.c:5606, or the forked-child
    /// path at execute_cmd.c:5522). A word-expansion diagnostic
    /// (`${x?word}`, bad substitution) therefore writes to fd 2 as bound by
    /// the *enclosing* context only — a `{ }`/`( )` compound redirect or the
    /// pipeline dup — never to the failing command's own `2>file`/`2>&1`.
    /// The fd state is seeded from the ambient fd table, which already
    /// carries those enclosing bindings.
    pub(in crate::executor) fn write_redirected_command_stderr(
        &mut self,
        cmd: &CommandNode,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        let _ = cmd;
        let state = self.command_output_fd_state();
        state.write_to_fd(self, 2, output)
    }

    fn write_state_output_or_diagnostic(
        &mut self,
        cmd: &CommandNode,
        state: &OutputFdState,
        fd: u32,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        match state.write_to_fd(self, fd, output) {
            Ok(()) => Ok(()),
            Err(error) if is_closed_output_error(&error) => {
                if fd == 1 && builtin_output_write_is_unchecked(cmd) {
                    return Ok(());
                }
                self.write_bad_output_fd_diagnostic(cmd, fd)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(in crate::executor) fn command_needs_ordered_output_capture(
        &self,
        cmd: &CommandNode,
    ) -> bool {
        cmd.redirects.iter().any(|redirect| {
            matches!(
                redirect.kind,
                crate::parser::RedirectKind::DuplicateOutput
                    | crate::parser::RedirectKind::CloseOutput
            )
        })
    }

    fn command_output_fd_state(&self) -> OutputFdState {
        let mut state = OutputFdState {
            fds: HashMap::new(),
            saw_output_redirect: false,
            redirect_failed: false,
            defer_stdout_writes: false,
            deferred_stdout: std::cell::RefCell::new(Vec::new()),
        };
        state.fds.insert(1, OutputTarget::Stdout);
        state.fds.insert(2, OutputTarget::Stderr);

        // FdTable is the semantic source of truth for every output endpoint.
        for (fd, entry) in &self.fd_table.entries {
            let target = if entry.closed || entry.write.is_none() {
                OutputTarget::Closed
            } else {
                match entry.write.as_ref().expect("checked above") {
                    FdWriteEndpoint::Stdout => OutputTarget::Stdout,
                    FdWriteEndpoint::Stderr => OutputTarget::Stderr,
                    FdWriteEndpoint::File(file_fd) => {
                        let path = shell_display_path(&file_fd.path.to_string_lossy());
                        if is_null_device(&path) {
                            OutputTarget::Null
                        } else {
                            OutputTarget::SharedFile(file_fd.clone())
                        }
                    }
                    FdWriteEndpoint::CoprocStdin { pid, .. } => OutputTarget::CoprocStdin(*pid),
                    FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                        OutputTarget::Path(path.to_string_lossy().into_owned())
                    }
                }
            };
            state.fds.insert(*fd, target);
        }

        state
    }

    fn apply_ordered_output_redirects(
        &mut self,
        cmd: &CommandNode,
        state: &mut OutputFdState,
    ) -> Result<bool, ExecuteError> {
        for redirect in &cmd.redirects {
            match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput => {
                    let fd = redirect_fd_or_default(redirect, 1);
                    let target = self.expand_redirect_target(redirect);
                    // See injected_redirect_fd_is_bound: an injected group
                    // redirect is already realized when the seeded fd holds
                    // the group's shared File binding (the seeded state maps
                    // fd-table File endpoints to SharedFile one-to-one).
                    if self.injected_redirect_fd_is_bound(redirect, fd)
                        && matches!(state.fds.get(&fd), Some(OutputTarget::SharedFile(_)))
                    {
                        state.saw_output_redirect = true;
                        continue;
                    }
                    if let Some(source_fd) = redirect_target_fd(&target) {
                        let Some(source_target) = state.fd_target(source_fd).cloned() else {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        };
                        if source_target == OutputTarget::Closed {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        }
                        state.fds.insert(fd, source_target);
                        state.saw_output_redirect = true;
                        continue;
                    }
                    self.open_command_output_target(state, fd, &target, redirect)?;
                }
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    let target = self.expand_redirect_target(redirect);
                    self.open_command_output_target(state, 1, &target, redirect)?;
                    let stdout_target = state.fd_target(1).cloned().unwrap_or(OutputTarget::Stdout);
                    state.fds.insert(2, stdout_target);
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    let target_fd = redirect_fd_or_default(redirect, 1);
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        state.fds.insert(target_fd, OutputTarget::Closed);
                        state.saw_output_redirect = true;
                        continue;
                    }
                    if let Some(source_fd) = redirect_target_fd(&target) {
                        let Some(source_target) = state.fd_target(source_fd).cloned() else {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        };
                        if source_target == OutputTarget::Closed {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        }
                        state.fds.insert(target_fd, source_target);
                        state.saw_output_redirect = true;
                        continue;
                    }

                    if target_fd == 1 {
                        let path = target.strip_prefix('&').unwrap_or(&target);
                        // Empty `>&word`/`1>&word` expansion is an ambiguous
                        // redirect (redirection_expand returned NULL); the
                        // precheck normally catches it, keep the invariant
                        // here for paths that bypass it.
                        if path.is_empty() {
                            self.write_ambiguous_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        }
                        self.open_command_output_target(state, target_fd, path, redirect)?;
                        // GNU redir.c:832-838: `>&word` and `1>&word` alike
                        // (redirector == 1, no varassign) become
                        // r_err_and_out — stderr follows stdout to word.
                        if redirect.fd_var.is_none() {
                            let stdout_target =
                                state.fd_target(1).cloned().unwrap_or(OutputTarget::Stdout);
                            state.fds.insert(2, stdout_target);
                        }
                    } else {
                        // report_ambiguous_redirect (redir.c:846-870): dup
                        // redirections report the expanded fd word when
                        // non-empty and the raw word when expansion is
                        // empty (see the precheck note above).
                        if target.strip_prefix('&').unwrap_or(&target).is_empty() {
                            self.write_ambiguous_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                        } else {
                            self.write_ambiguous_redirect_diagnostic(state, &target)?;
                        }
                        self.exit_code = 1;
                        state.redirect_failed = true;
                        return Ok(true);
                    }
                }
                crate::parser::RedirectKind::CloseOutput => {
                    state
                        .fds
                        .insert(redirect_fd_or_default(redirect, 1), OutputTarget::Closed);
                    state.saw_output_redirect = true;
                }
                _ => {}
            }
        }

        Ok(state.saw_output_redirect)
    }

    fn open_command_output_target(
        &self,
        state: &mut OutputFdState,
        fd: u32,
        target: &str,
        redirect: &Redirect,
    ) -> Result<(), ExecuteError> {
        if is_closed_redirect_target(target) {
            state.fds.insert(fd, OutputTarget::Closed);
        } else if is_null_device(target) {
            state.fds.insert(fd, OutputTarget::Null);
        } else {
            let target = self.redirect_output_path_target(target);
            if redirect.append {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
            } else {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            state.fds.insert(fd, OutputTarget::Path(target));
        }
        state.saw_output_redirect = true;
        Ok(())
    }

    fn write_bad_fd_redirect_diagnostic(
        &mut self,
        state: &OutputFdState,
        source_target: &str,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        let display_target = source_target.strip_prefix('&').unwrap_or(source_target);
        // GNU reports a failed dup2 of `>&N` as EBADF ("Bad file
        // descriptor"), but an unopened `/dev/fd/N` (or /dev/std*) path
        // fails at open() with ENOENT ("No such file or directory") —
        // redir.c redirection_error uses the syscall errno (niubash#118).
        let reason =
            if crate::executor::execution_misc::dev_stdio_redirect_fd(display_target).is_some() {
                "No such file or directory"
            } else {
                "Bad file descriptor"
            };
        writeln!(
            &mut stderr,
            "{}{display_target}: {reason}",
            self.diagnostic_prefix()
        )?;
        state.write_to_fd(self, 2, &stderr)
    }

    fn write_ambiguous_redirect_diagnostic(
        &mut self,
        state: &OutputFdState,
        target: &str,
    ) -> Result<(), ExecuteError> {
        let target = target.strip_prefix('&').unwrap_or(target);
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "{}{target}: ambiguous redirect",
            self.diagnostic_prefix()
        )?;
        state.write_to_fd(self, 2, &stderr)
    }

    /// GNU execute_cmd.c execute_pipeline + redir.c do_redirection_internal:
    /// inside each pipeline element's subshell the pipe is bound to fd 1
    /// first and the element's own redirections run after it — `cmd >f |
    /// next` sends the element's bytes to f and hands next an empty pipe,
    /// `cmd 1>&2 | next` sends them to stderr, `cmd 2>&1 | next` merges
    /// stderr into the pipe. Sequential stages capture their raw fd-1/fd-2
    /// streams; this resolves the element's output redirects (fd 1 seeded
    /// as the pipe) and routes the captures to the same targets. Compound
    /// stages applied their redirections internally and skip this.
    /// Returns true when the stage carried output redirects to resolve.
    pub(in crate::executor) fn route_pipeline_stage_streams(
        &mut self,
        cmd: &CommandNode,
        stdout: &mut String,
        stderr: &mut String,
        status: &mut i32,
    ) -> Result<bool, ExecuteError> {
        if cmd.redirect_out.is_none()
            && cmd.append.is_none()
            && cmd.redirect_err.is_none()
            && cmd.redirect_err_append.is_none()
            && !cmd
                .redirects
                .iter()
                .any(|redirect| redirect.is_output_side())
        {
            return Ok(false);
        }
        let mut state = self.command_output_fd_state();
        // The element's fd 1 starts bound to the pipe/output channel
        // regardless of the ambient fd 1 binding; fd 2 stays ambient.
        state.fds.insert(1, OutputTarget::Stdout);
        state.defer_stdout_writes = true;
        if !self.apply_ordered_output_redirects(cmd, &mut state)? {
            return Ok(false);
        }
        let deferred_stdout = std::mem::take(&mut *state.deferred_stdout.borrow_mut());
        if state.redirect_failed {
            stdout.clear();
            stderr.clear();
            stdout.push_str(&String::from_utf8_lossy(&deferred_stdout));
            *status = 1;
            return Ok(true);
        }
        // `cmd 2>&1 >f` resolves fd 2 to the pipe while `cmd >f 2>&1`
        // resolves it to the file, so fd 1's stream routes first and an
        // fd2-to-Stdout merge lands in the post-redirect pipe content.
        let stdout_target = state.fds.get(&1).cloned().unwrap_or(OutputTarget::Stdout);
        let stderr_target = state.fds.get(&2).cloned().unwrap_or(OutputTarget::Stderr);
        match &stdout_target {
            OutputTarget::Stdout => {}
            OutputTarget::Stderr => {
                let taken = std::mem::take(stdout);
                stderr.push_str(&taken);
            }
            target => {
                let target = target.clone();
                let taken = std::mem::take(stdout);
                self.write_stage_stream_to_target(cmd, 1, &target, &taken, stderr, status)?;
            }
        }
        match &stderr_target {
            OutputTarget::Stderr => {}
            OutputTarget::Stdout => {
                let taken = std::mem::take(stderr);
                stdout.push_str(&taken);
            }
            target => {
                let target = target.clone();
                let taken = std::mem::take(stderr);
                self.write_stage_stream_to_target(cmd, 2, &target, &taken, stdout, status)?;
            }
        }
        Ok(true)
    }

    /// Writes one captured stage stream to a non-stdio resolved target.
    /// Stdout/Stderr are handled by the caller (pipe merge vs ambient
    /// channel); a Closed target reports the write error the way GNU's
    /// flushed-write check does (builtins/common.c:320 sh_chkwrite), into
    /// `diag` — the opposite stream, which the caller still routes through
    /// the resolved state afterwards.
    fn write_stage_stream_to_target(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        target: &OutputTarget,
        output: &str,
        diag: &mut String,
        status: &mut i32,
    ) -> Result<(), ExecuteError> {
        let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(output);
        match target {
            OutputTarget::Stdout | OutputTarget::Stderr => {}
            OutputTarget::Null => {}
            OutputTarget::Closed => {
                if !output.is_empty() && !(fd == 1 && builtin_output_write_is_unchecked(cmd)) {
                    let command = cmd.words.first().map(String::as_str).unwrap_or("command");
                    diag.push_str(&format!(
                        "{}{command}: write error: Bad file descriptor\n",
                        self.diagnostic_prefix()
                    ));
                    self.exit_code = 1;
                    self.last_builtin_write_failed.set(true);
                    *status = 1;
                }
            }
            OutputTarget::CoprocStdin(coproc_fd) => {
                if let Some(pipe) = self.coproc_write_file(*coproc_fd) {
                    crate::fd::write_all(pipe.handle, &bytes)?;
                }
            }
            OutputTarget::Path(path) => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(path, &self.shell_state.env_vars))?;
                file.write_all(&bytes)?;
            }
            OutputTarget::SharedFile(file) => {
                crate::fd::write_all(file.handle, &bytes)?;
            }
        }
        Ok(())
    }

    fn write_bad_output_fd_diagnostic(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Result<(), ExecuteError> {
        let command = cmd.words.first().map(String::as_str).unwrap_or("command");
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "{}{command}: write error: Bad file descriptor",
            self.diagnostic_prefix()
        )?;
        if fd == 2 {
            self.write_default_stdout(&stderr)
        } else {
            self.write_default_stderr(&stderr)
        }
    }
}

/// GNU builtins route their exit status through `sh_chkwrite`
/// (builtins/common.c:320), which reports `write error` and fails the
/// builtin when stdout is closed — but only on code paths that call it.
/// `help` with no topic operands returns `EXECUTION_SUCCESS` directly
/// (builtins/help.def:114-118: `if (list == 0) { ...; return
/// (EXECUTION_SUCCESS); }`), so `help >&-` is silent with status 0 while
/// `help topic >&-` reports the write error. Mirror that per-path split.
fn builtin_output_write_is_unchecked(cmd: &CommandNode) -> bool {
    if cmd.words.first().map(String::as_str) != Some("help") {
        return false;
    }
    // After internal_getopt consumes `-dms` options, `list == 0' means no
    // topic operands remain (options themselves still count as args here,
    // so reject any word that is not a recognized option cluster).
    // internal_getopt stops at `--' or the first non-option word; only
    // `-dms' clusters optionally closed by a trailing `--' leave `list == 0'.
    let mut args = cmd.words[1..].iter();
    loop {
        match args.next() {
            None => return true,
            Some(word) if word == "--" => return args.next().is_none(),
            Some(word)
                if word.starts_with('-')
                    && word.len() > 1
                    && word[1..].chars().all(|ch| matches!(ch, 'd' | 'm' | 's')) =>
            {
                continue
            }
            _ => return false,
        }
    }
}

fn redirect_fd_or_default(redirect: &Redirect, default_fd: u32) -> u32 {
    redirect.fd.unwrap_or_else(|| {
        redirect
            .operator
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(default_fd)
    })
}

impl OutputFdState {
    fn fd_target(&self, fd: u32) -> Option<&OutputTarget> {
        self.fds.get(&fd).or_else(|| match fd {
            1 => self.fds.get(&1),
            2 => self.fds.get(&2),
            _ => None,
        })
    }

    fn write_to_fd(
        &self,
        executor: &mut Executor,
        fd: u32,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        if output.is_empty() {
            return Ok(());
        }

        match self.fd_target(fd).cloned().unwrap_or(match fd {
            2 => OutputTarget::Stderr,
            _ => OutputTarget::Stdout,
        }) {
            OutputTarget::Stdout => {
                if self.defer_stdout_writes {
                    self.deferred_stdout.borrow_mut().extend_from_slice(output);
                    Ok(())
                } else {
                    executor.write_default_stdout(output)
                }
            }
            OutputTarget::Stderr => executor.write_default_stderr(output),
            OutputTarget::Null | OutputTarget::Closed => Ok(()),
            OutputTarget::CoprocStdin(fd) => {
                if let Some(pipe) = executor.coproc_write_file(fd) {
                    crate::fd::write_all(pipe.handle, output)?;
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "coprocess input is closed",
                    )
                    .into());
                }
                Ok(())
            }
            OutputTarget::Path(path) => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&path, &executor.shell_state.env_vars))?;
                file.write_all(output)?;
                Ok(())
            }
            OutputTarget::SharedFile(file) => {
                crate::fd::write_all(file.handle, output)?;
                Ok(())
            }
        }
    }
}

fn is_closed_output_error(error: &ExecuteError) -> bool {
    matches!(
        error,
        ExecuteError::IoError(error)
            if error.kind() == std::io::ErrorKind::BrokenPipe
                || error.raw_os_error() == Some(232)
    )
}
