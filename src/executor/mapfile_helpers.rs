use super::*;

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
            return self.stdin_string_for_command(cmd);
        };

        if let Some(input) = self.mapfile_redirected_fd_input(cmd, fd) {
            return Some(input);
        }
        self.mapfile_virtual_fd_input(fd)
            .or_else(|| self.mapfile_heredoc_fd_input(cmd, fd))
    }

    pub(in crate::executor) fn mapfile_fd_is_available(&self, cmd: &CommandNode, fd: u32) -> bool {
        if self.env_vars.contains_key(&fd_stdin_key(fd)) {
            return true;
        }
        if cmd
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.fd == Some(fd) && redirect.body.is_some())
        {
            return true;
        }
        cmd.redirect_in.as_ref().is_some_and(|redirect| {
            redirect.fd == Some(fd)
                && !is_closed_redirect_target(&self.expand_word(&redirect.target))
        })
    }

    fn mapfile_redirected_fd_input(&mut self, cmd: &CommandNode, fd: u32) -> Option<String> {
        let redirect = cmd.redirect_in.as_ref()?;
        if redirect.fd != Some(fd) {
            return None;
        }

        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            return self.process_substitution_output(source);
        }

        let target = self.expand_word(&redirect.target);
        if is_closed_redirect_target(&target) {
            return None;
        }
        let path = shell_path_to_windows(&target, &self.env_vars);
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
        let input_key = fd_stdin_key(fd);
        let offset_key = fd_stdin_offset_key(fd);
        let input = self.env_vars.get(&input_key)?.clone();
        if input == FD_PROCESS_STDIN_TARGET {
            return self.read_inherited_process_stdin_to_string();
        }
        let offset = self
            .env_vars
            .get(&offset_key)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if offset >= input.len() {
            return None;
        }
        self.env_vars.insert(offset_key, input.len().to_string());
        Some(input[offset..].to_string())
    }

    fn mapfile_heredoc_fd_input(&self, cmd: &CommandNode, fd: u32) -> Option<String> {
        let body = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?
            .body
            .as_deref()?;
        if let Some(word) = body.strip_prefix('\x1d') {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            return Some(input);
        }
        Some(strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string())
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
