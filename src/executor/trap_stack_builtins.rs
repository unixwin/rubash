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
        // Exact-once consumer boundary for U+E000 raw-byte markers carried
        // in word data (ANSI-C \xHH / octal bytes >= 0x80, raw-byte
        // substitution payloads). Builtins write to the real byte sinks
        // here, so the markers must decode to their original bytes now;
        // marker-free and non-UTF-8 slices pass through untouched.
        let stdout = decode_raw_byte_marker_bytes(stdout);
        let stderr = decode_raw_byte_marker_bytes(stderr);
        if self.write_ordered_command_output(cmd, &stdout, &stderr)? {
            return Ok(());
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if self.write_output_fd_redirect(&target, &stdout)? {
            } else if redirect_target_fd(&target) == Some(2) {
                std::io::stderr().lock().write_all(&stdout)?;
            } else if redirect_target_fd(&target) == Some(1) {
                std::io::stdout().lock().write_all(&stdout)?;
            } else {
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(&stdout)?;
            }
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if self.write_output_fd_redirect(&target, &stdout)? {
            } else if redirect_target_fd(&target) == Some(2) {
                std::io::stderr().lock().write_all(&stdout)?;
            } else if redirect_target_fd(&target) == Some(1) {
                std::io::stdout().lock().write_all(&stdout)?;
            } else {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(&stdout)?;
            }
        } else {
            self.write_default_stdout(&stdout)?;
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if self.write_output_fd_redirect(&target, &stderr)? {
            } else if redirect_target_fd(&target) == Some(1) {
                if let Some(capture) = &mut self.stdout_capture {
                    capture.write_all(&stderr)?;
                } else {
                    std::io::stdout().lock().write_all(&stderr)?;
                }
            } else if !is_null_device(&target) {
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(&stderr)?;
            }
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
            } else if self.write_output_fd_redirect(&target, &stderr)? {
            } else if redirect_target_fd(&target) == Some(1) {
                if let Some(capture) = &mut self.stdout_capture {
                    capture.write_all(&stderr)?;
                } else {
                    std::io::stdout().lock().write_all(&stderr)?;
                }
            } else {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(&stderr)?;
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
        self.note_return_trap_scope(&cmd.words[1..]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::trap::execute_with_io(
            &cmd.words[1..],
            &mut self.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
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
            &mut self.env_vars,
            &diagnostic_prefix,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        self.sync_cd_variables();
        Ok(status)
    }
}
