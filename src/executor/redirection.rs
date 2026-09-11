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
    Closed,
}

#[derive(Debug, Clone)]
struct OutputFdState {
    fds: HashMap<u32, OutputTarget>,
    saw_output_redirect: bool,
    redirect_failed: bool,
}

impl Executor {
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
            let target = self.expand_word(&redirect.target);
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
            if invalid_fd_target
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
                self.write_bad_output_fd_diagnostic(cmd, 1)?;
                self.exit_code = 1;
                return Ok(true);
            }
            if !stderr.is_empty()
                && state
                    .fd_target(2)
                    .is_some_and(|target| *target == OutputTarget::Closed)
            {
                self.write_bad_output_fd_diagnostic(cmd, 2)?;
                self.exit_code = 1;
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
            self.write_bad_output_fd_diagnostic(cmd, 1)?;
            self.exit_code = 1;
            return Ok(true);
        }
        if !stderr.is_empty()
            && state
                .fd_target(2)
                .is_some_and(|target| *target == OutputTarget::Closed)
        {
            self.write_bad_output_fd_diagnostic(cmd, 2)?;
            self.exit_code = 1;
            return Ok(true);
        }

        self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
        self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
        Ok(true)
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
                self.write_bad_output_fd_diagnostic(cmd, fd)?;
                self.exit_code = 1;
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
                    FdWriteEndpoint::File(path) => {
                        let path = shell_display_path(&path.to_string_lossy());
                        if is_null_device(&path) {
                            OutputTarget::Null
                        } else {
                            OutputTarget::Path(path)
                        }
                    }
                    FdWriteEndpoint::CoprocStdin(pid) => OutputTarget::CoprocStdin(*pid),
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
                    let target = self.expand_word(&redirect.target);
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
                    let target = self.expand_word(&redirect.target);
                    self.open_command_output_target(state, 1, &target, redirect)?;
                    let stdout_target = state.fd_target(1).cloned().unwrap_or(OutputTarget::Stdout);
                    state.fds.insert(2, stdout_target);
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    let target_fd = redirect_fd_or_default(redirect, 1);
                    let target = self.expand_word(&redirect.target);
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
                        self.open_command_output_target(state, target_fd, path, redirect)?;
                        if redirect.fd.is_none() {
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
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
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
        writeln!(
            &mut stderr,
            "{}{display_target}: Bad file descriptor",
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
            OutputTarget::Stdout => executor.write_default_stdout(output),
            OutputTarget::Stderr => executor.write_default_stderr(output),
            OutputTarget::Null | OutputTarget::Closed => Ok(()),
            OutputTarget::CoprocStdin(fd) => {
                if let Some(writer) = executor.coproc_stdin_writers.get_mut(&fd) {
                    writer.write_all(output)?;
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
                    .open(shell_path_to_windows(&path, &executor.env_vars))?;
                file.write_all(output)?;
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
