use super::types::INHERIT_PROCESS_STDIN;
use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    pub(in crate::executor) fn parse_mapfile_usize(
        &self,
        command_name: &str,
        value: &str,
        diagnostic: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<usize, i32> {
        value.parse::<usize>().map_err(|_| {
            let _ = writeln!(
                stderr,
                "{}{command_name}: {value}: {diagnostic}",
                self.diagnostic_prefix()
            );
            1
        })
    }

    pub(in crate::executor) fn parse_mapfile_callback_quantum(
        &self,
        command_name: &str,
        value: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<usize, i32> {
        let quantum =
            self.parse_mapfile_usize(command_name, value, "invalid callback quantum", stderr)?;
        if quantum == 0 {
            let _ = writeln!(
                stderr,
                "{}{command_name}: {value}: invalid callback quantum",
                self.diagnostic_prefix()
            );
            return Err(1);
        }
        Ok(quantum)
    }

    pub(in crate::executor) fn parse_mapfile_fd(
        &self,
        command_name: &str,
        value: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<u32, i32> {
        value
            .parse::<i32>()
            .ok()
            .and_then(|fd| u32::try_from(fd).ok())
            .ok_or_else(|| {
                let _ = writeln!(
                    stderr,
                    "{}{command_name}: {value}: invalid file descriptor specification",
                    self.diagnostic_prefix()
                );
                1
            })
    }

    pub(in crate::executor) fn mapfile_bad_file_descriptor(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        fd: u32,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: {fd}: invalid file descriptor: Bad file descriptor",
            self.diagnostic_prefix()
        );
        self.finish_mapfile_error(cmd, stderr, 1)
    }

    pub(in crate::executor) fn mapfile_invalid_identifier(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        name: &str,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: `{name}': not a valid identifier",
            self.diagnostic_prefix()
        );
        self.finish_mapfile_error(cmd, stderr, 1)
    }

    /// GNU builtins/mapfile.def:330: `builtin_error (_("empty array variable
    /// name"))` for an empty array name argument.
    pub(in crate::executor) fn mapfile_empty_array_name(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: empty array variable name",
            self.diagnostic_prefix()
        );
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn mapfile_missing_option_argument(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        option: &str,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: -{option}: option requires an argument",
            self.diagnostic_prefix()
        );
        self.print_mapfile_usage(command_name, stderr);
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn mapfile_invalid_option(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        option: char,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: -{option}: invalid option",
            self.diagnostic_prefix()
        );
        self.print_mapfile_usage(command_name, stderr);
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn print_mapfile_usage(
        &self,
        command_name: &str,
        stderr: &mut Vec<u8>,
    ) {
        let _ = writeln!(
            stderr,
            "{command_name}: usage: {command_name} [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]"
        );
    }

    pub(in crate::executor) fn finish_mapfile_error(
        &mut self,
        cmd: &CommandNode,
        stderr: &[u8],
        status: i32,
    ) -> i32 {
        if self
            .write_buffered_builtin_output(cmd, &[], stderr)
            .is_err()
        {
            return 1;
        }
        status
    }

    pub(in crate::executor) fn mapfile_input_for_command(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
    ) -> Option<String> {
        let Some(fd) = read_fd else {
            // mapfile preserves carriage returns from CRLF input files. The generic
            // stdin helper is text-oriented, so use the raw-byte reader for a plain
            // fd-0 file redirect before falling back to heredoc/process-substitution
            // and inherited stdin handling.
            if let Some(redirect) = &cmd.redirect_in {
                if redirect.fd.unwrap_or(0) == 0 {
                    let target = self.expand_redirect_target(redirect);
                    if !target.starts_with("<(") && !is_closed_redirect_target(&target) {
                        let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
                        if let Ok(input) =
                            crate::executor::substitution_metadata::read_shell_input_file(path)
                        {
                            return Some(input);
                        }
                    }
                }
            }
            if let Some(input) = self.stdin_string_for_command_mut(cmd) {
                return Some(input);
            }
            // Fallback: read from inherited process stdin when INHERIT_PROCESS_STDIN is set
            // (e.g., printf '...' | rubash -c 'mapfile arr')
            if self
                .shell_state
                .env_vars
                .get(INHERIT_PROCESS_STDIN)
                .map(String::as_str)
                == Some("1")
            {
                return self.read_inherited_process_stdin_to_string();
            }
            return None;
        };

        if let Some(input) = self.mapfile_redirected_fd_input(cmd, fd) {
            return Some(input);
        }
        if let Some(input) = self.mapfile_virtual_fd_input(fd) {
            return Some(input);
        }
        self.mapfile_heredoc_fd_input(cmd, fd)
    }

    pub(in crate::executor) fn mapfile_fd_is_available(&self, cmd: &CommandNode, fd: u32) -> bool {
        if self.fd_table.is_open_for_read(fd) {
            return true;
        }
        if cmd
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.fd == Some(fd) && redirect.body.is_some())
        {
            return true;
        }
        cmd.redirects.iter().rev().any(|redirect| {
            redirect.fd == Some(fd)
                && !is_closed_redirect_target(&self.expand_redirect_target(redirect))
        })
    }

    fn mapfile_redirected_fd_input(&mut self, cmd: &CommandNode, fd: u32) -> Option<String> {
        // Redirections are applied in source order; the last matching input
        // redirect is the descriptor's effective source.
        let redirect = cmd
            .redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?;

        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            return self.process_substitution_output(source);
        }

        let target = self.expand_redirect_target(redirect);
        if is_closed_redirect_target(&target) {
            return None;
        }
        let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
        if redirect.append {
            let _ = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path);
        }
        fs::read_to_string(path).ok()
    }

    fn mapfile_virtual_fd_input(&mut self, fd: u32) -> Option<String> {
        if self.fd_table.is_open_for_read(fd) {
            if let Some(input) = self.fd_table.read_all_text(fd) {
                if let Some((_, offset)) = self.fd_table.input_snapshot(fd) {
                    self.shell_state
                        .env_vars
                        .insert(fd_stdin_offset_key(fd), offset.to_string());
                }
                return Some(input);
            }
        }
        if matches!(
            self.fd_table.read_endpoint(fd),
            Some(FdReadEndpoint::InheritedProcessStdin)
        ) {
            return self.read_inherited_process_stdin_to_string();
        }
        None
    }

    fn mapfile_heredoc_fd_input(&mut self, cmd: &CommandNode, fd: u32) -> Option<String> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?;
        if redirect.body_carrier.is_some() {
            let mut input = if redirect.here_string {
                self.expand_here_string_mut_from_carrier(&redirect.body_carrier)
            } else {
                self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier)
            };
            if redirect.here_string {
                input.push('\n');
            }
            return Some(input);
        }
        let body = redirect.body.as_deref()?;
        if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            return Some(input);
        }
        Some(self.expand_heredoc_body_mut(body))
    }

    pub(in crate::executor) fn execute_mapfile_callback(
        &mut self,
        callback: &str,
        index: usize,
        value: &str,
    ) -> Result<(), ExecuteError> {
        let mut words = callback
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if words.is_empty() {
            return Ok(());
        }
        words.push(index.to_string());
        words.push(value.to_string());

        let mut callback_cmd = CommandNode::new();
        callback_cmd.words = words;
        self.execute_command(&callback_cmd)
    }
}
