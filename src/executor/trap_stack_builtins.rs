use super::*;

/// Decode U+E000 raw-byte marker pairs in builtin output back to real
/// bytes (substitution_metadata.rs owner contract: this is an exact-once
/// consumer boundary). Slices that are not valid UTF-8 (already raw bytes)
/// or that contain no marker sentinel pass through unchanged, so the helper
/// is safe for every builtin caller.
fn decode_raw_byte_marker_bytes(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    if !text
        .chars()
        .any(|ch| ch as u32 == crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
    {
        return bytes.to_vec();
    }
    crate::executor::substitution_metadata::shell_text_to_raw_bytes(text)
}

impl Executor {
    pub(in crate::executor) fn write_buffered_builtin_output(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        self.write_buffered_builtin_output_ordered(cmd, stdout, stderr, false)
    }

    /// printf variant (rubash#1134): GNU emits printf's diagnostics
    /// unbuffered while its output flushes at builtin exit, so the
    /// stderr buffer is routed before the stdout buffer.
    pub(in crate::executor) fn write_buffered_builtin_output_stderr_first(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        self.write_buffered_builtin_output_ordered(cmd, stdout, stderr, true)
    }

    fn write_buffered_builtin_output_ordered(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
        stderr_first: bool,
    ) -> Result<(), ExecuteError> {
        // Silent-builtin short-circuit (perf): `shift` and friends succeed
        // with empty stdout/stderr buffers on the hot path. GNU runs such a
        // builtin's writes through its redirected descriptors, but a zero-
        // byte write is a no-op on every sink (capture buffers, pipes,
        // files, closed fds — the fd-closed diagnostic in
        // route_builtin_buffered_output only fires on non-empty output).
        // With no per-command redirects there is no open/truncate side
        // effect to preserve either (GNU do_redirections opens before the
        // builtin runs; here there is nothing to open), so skip the full
        // pipeline (marker decode alloc + fd-table Hash seeding + ordered
        // redirect walk + 0-byte writes) outright. Any redirect present,
        // or any non-empty buffer, takes the slow path. Anything
        // semantically doubtful likewise stays on the slow path by
        // construction (this gate only fires on the provably empty case).
        if stdout.is_empty()
            && stderr.is_empty()
            && cmd.redirects.is_empty()
            && cmd.redirect_in.is_none()
            && cmd.redirect_out.is_none()
            && cmd.append.is_none()
            && cmd.redirect_err.is_none()
            && cmd.redirect_err_append.is_none()
            && cmd.heredoc.is_none()
            && cmd.heredoc_body.is_none()
            && cmd.here_string.is_none()
            && cmd.here_string_carrier.is_none()
            && cmd.heredoc_redirects.is_empty()
        {
            self.last_builtin_write_failed.set(false);
            return Ok(());
        }
        // rubash#1968: GNU runs do_redirections BEFORE the builtin
        // body (execute_cmd.c:5606 execute_builtin_or_function), so a
        // failed input open suppresses the builtin entirely — no
        // output, status 1, the redirection_error diagnostic. This
        // buffered route is where echo/printf-class builtins flush,
        // and it never validated input redirects: the ordered walk
        // below only routes fds 1/2, so `echo hi < /dev/fd/63` and
        // `echo hi <> /dev/fd/63` printed with status 0. The plain
        // filesystem face is already settled before execution (the
        // reject gate's Input probe and the setup walk), so this
        // pass owns exactly the registry classes those passes skip:
        // net targets (probed through the classified ladder — the
        // probe's netopen is the command's one open), fd aliases
        // (judged against the fd table, no drain), and synthetic
        // /proc targets (served by their ladder). A failure routes
        // its diagnostic through the command's fd-2 binding, drops
        // both buffers, and folds status 1 into the builtin's return
        // through the write-failure flag.
        if let Some(error) = self.buffered_builtin_input_redirect_error(cmd) {
            let mut line = Vec::new();
            let _ = writeln!(
                &mut line,
                "{}{}",
                self.diagnostic_prefix(),
                crate::posix_errors::message(&error)
            );
            let _ = self.write_redirect_diagnostic_routed(cmd, &line);
            self.exit_code = 1;
            self.last_builtin_write_failed.set(true);
            return Ok(());
        }
        // Exact-once consumer boundary for U+E000 raw-byte markers carried
        // in word data (ANSI-C \xHH / octal bytes >= 0x80, raw-byte
        // substitution payloads). Builtins write to the real byte sinks
        // here, so the markers must decode to their original bytes now;
        // marker-free and non-UTF-8 slices pass through untouched.
        let stdout = decode_raw_byte_marker_bytes(stdout);
        let stderr = decode_raw_byte_marker_bytes(stderr);
        if self.route_builtin_buffered_output_ordered(cmd, &stdout, &stderr, stderr_first)? {
            return Ok(());
        }
        if stderr_first {
            self.write_buffered_stderr_block(cmd, &stderr)?;
            return self.write_buffered_stdout_block(cmd, &stdout);
        }
        self.write_buffered_stdout_block(cmd, &stdout)?;
        self.write_buffered_stderr_block(cmd, &stderr)
    }

    /// The first failing input-side redirect of a buffered builtin,
    /// restricted to the special-path registry classes the earlier
    /// passes skip (rubash#1968; see the call site). The returned
    /// error already carries the `<target>: <strerror>` path_error
    /// shape. Plain filesystem, null-device and fd-alias targets
    /// return None — the reject gate and the setup walk settled
    /// those before the builtin ran (the gate settles dangling
    /// aliases since the #1963 gate fix), and re-probing them here
    /// could only repeat their verdict. Net and synthetic targets
    /// are the residue: the gate's Input/ReadWrite probes exempt
    /// every registry class, and nothing else on the buffered route
    /// ever attempts them — a refused /dev/tcp connect or an
    /// unservable synthetic name would otherwise let the builtin
    /// execute silently, where GNU suppresses it with status 1.
    fn buffered_builtin_input_redirect_error(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<std::io::Error> {
        use crate::parser::RedirectKind;
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some()
                || crate::executor::support_names::is_injected_group_redirect(redirect)
            {
                continue;
            }
            if !matches!(redirect.kind, RedirectKind::Input | RedirectKind::ReadWrite) {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) || target.starts_with("<(") {
                continue;
            }
            let write = redirect.kind == RedirectKind::ReadWrite;
            let result = match crate::special_paths::classify_special_path(&target) {
                crate::special_paths::SpecialPathClass::NetRedirect
                | crate::special_paths::SpecialPathClass::SyntheticProc
                | crate::special_paths::SpecialPathClass::SyntheticLink => {
                    if write {
                        self.probe_input_redirect_readwrite(&target)
                            .map(|_| ())
                            .map_err(|e| crate::posix_errors::path_error(&target, e))
                    } else {
                        self.probe_input_redirect(&target).map(|_| ())
                    }
                }
                _ => continue,
            };
            if let Err(error) = result {
                return Some(error);
            }
        }
        None
    }

    fn write_buffered_stdout_block(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
            } else if self.write_output_fd_redirect(&target, &stdout)? {
            } else if redirect_target_fd(&target) == Some(2) {
                super::write_stderr_bytes(&stdout)?;
            } else if redirect_target_fd(&target) == Some(1) {
                super::write_stdout_bytes(&stdout)?;
            } else {
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(&stdout)?;
            }
        } else if let Some(redirect) = &cmd.append {
            // Injected group redirect already bound on fd 1: write through
            // the shared open file description (see
            // injected_redirect_fd_is_bound).
            if self.injected_redirect_fd_is_bound(redirect, 1) {
                self.write_default_stdout(&stdout)?;
            } else {
                let target = self.expand_redirect_target(redirect);
                if is_closed_redirect_target(&target) {
                } else if self.write_output_fd_redirect(&target, &stdout)? {
                } else if redirect_target_fd(&target) == Some(2) {
                    super::write_stderr_bytes(&stdout)?;
                } else if redirect_target_fd(&target) == Some(1) {
                    super::write_stdout_bytes(&stdout)?;
                } else {
                    let mut file =
                        open_redirect_append(&self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )))?;
                    file.write_all(&stdout)?;
                }
            }
        } else {
            self.write_default_stdout(&stdout)?;
        }

        Ok(())
    }

    fn write_buffered_stderr_block(
        &mut self,
        cmd: &CommandNode,
        stderr: &[u8],
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
            } else if self.write_output_fd_redirect(&target, &stderr)? {
            } else if redirect_target_fd(&target) == Some(1) {
                if let Some(capture) = &mut self.stdout_capture {
                    capture.write_all(&stderr)?;
                } else {
                    super::write_stdout_bytes(&stderr)?;
                }
            } else if !is_null_device(&target) {
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(&stderr)?;
            }
        } else if let Some(redirect) = &cmd.redirect_err_append {
            // Injected group redirect already bound on fd 2: write through
            // the shared open file description.
            if self.injected_redirect_fd_is_bound(redirect, 2) {
                self.write_default_stderr(&stderr)?;
            } else {
                let target = self.expand_redirect_target(redirect);
                if is_closed_redirect_target(&target) {
                } else if self.write_output_fd_redirect(&target, &stderr)? {
                } else if redirect_target_fd(&target) == Some(1) {
                    if let Some(capture) = &mut self.stdout_capture {
                        capture.write_all(&stderr)?;
                    } else {
                        super::write_stdout_bytes(&stderr)?;
                    }
                } else {
                    let mut file =
                        open_redirect_append(&self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )))?;
                    file.write_all(&stderr)?;
                }
            }
        } else {
            self.write_default_stderr(&stderr)?;
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_trap(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::trap::execute_with_io(
            &cmd.words[1..],
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        // GNU execute_cmd.c:4886-4888: a special builtin whose status
        // exceeds EX_SHERRBASE has it converted (EX_USAGE 258 ->
        // EX_BADUSAGE 2, EX_UTILERROR 263 -> EXECUTION_FAILURE 1) and
        // marks special_builtin_failed; the :1004-1017 check then exits
        // a non-interactive POSIX shell in an untested context. trap's
        // layer returns the RAW code only from the POSIX reset-by-name
        // face (builtins/trap.rs EX_USAGE_INTERNAL, `trap USR1`); its
        // other usage faces (invalid option, -p with -P, -P without a
        // signal, a single non-signal argument) return the converted
        // EX_USAGE (2) — and within trap's status vocabulary 2 is only
        // ever a usage face (bad signal specs are EXECUTION_FAILURE),
        // so both conventions mark the failure here.
        //
        // rubash#2117: the exit itself is adjudicated by the ONE gate
        // in execute_materialized_command. This wrapper previously
        // converted inline (an immediate ExitCode when posix +
        // non-interactive), which pre-empted the gate's tested-context
        // (execute_cmd.c:1004's ignore_return/invert), `command'-builtin
        // suppression (execute_cmd.c:4657-4666) and ERR-trap
        // adjudication: `trap USR1 || x`, `command trap USR1` and a
        // benign ERR trap all exited where GNU continues. Every
        // dispatch arm for `trap` (late slot, direct `builtin' route,
        // `command' re-dispatch) completes inside an enclosing simple
        // command whose gate — or, for `command', the re-dispatch's
        // flag clear — owns the verdict, so marking is sufficient.
        if status > 256 {
            self.special_builtin_failed.set(true);
            let converted = match status {
                258..=259 => 2, // EX_USAGE, EX_BADSYNTAX -> EX_BADUSAGE
                _ => 1,         // EX_UTILERROR etc. -> EXECUTION_FAILURE
            };
            return Ok(converted);
        }
        if status == crate::builtins::trap::EX_USAGE {
            self.special_builtin_failed.set(true);
        }
        Ok(status)
    }

    pub(in crate::executor) fn execute_help(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status =
            crate::builtins::help::execute_with_io(&cmd.words[1..], &mut stdout, &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_stack_builtin(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::pushd::StackBuiltin,
    ) -> Result<i32, ExecuteError> {
        let diagnostic_prefix = self.diagnostic_prefix();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::pushd::execute_with_io(
            builtin,
            cmd.words[1..].iter().map(String::as_str),
            &mut self.shell_state.env_vars,
            &mut self.shell_state.cwd,
            &diagnostic_prefix,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        self.sync_cd_variables();
        Ok(status)
    }
}
