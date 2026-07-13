use super::*;

impl Executor {
    pub(in crate::executor) fn finish_read_error(
        &mut self,
        cmd: &CommandNode,
        stderr: &[u8],
        status: i32,
    ) -> i32 {
        self.write_buffered_builtin_output(cmd, &[], stderr)
            .map(|_| status)
            .unwrap_or(1)
    }

    pub(in crate::executor) fn read_input_for_command(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        if let Some(fd) = read_fd {
            if let Some(line) =
                self.read_redirected_fd(cmd, fd, delimiter, char_limit, exact_char_limit)
            {
                return Some(line);
            }
            return self
                .read_virtual_fd_stdin(fd, delimiter, char_limit, exact_char_limit)
                .or_else(|| {
                    self.read_heredoc_fd_input(cmd, fd, delimiter, char_limit, exact_char_limit)
                });
        }

        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) != 0 {
                return None;
            }
            if is_closed_redirect_target(&self.expand_word(&redirect.target)) {
                return None;
            }
            if let Some(source) = redirect
                .target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                if let Some(output) = self.process_substitution_output(source) {
                    return Some(trim_read_input(
                        output,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                    ));
                }
            }

            if let Some(fd) = redirect.target.strip_prefix('&') {
                if let Ok(fd) = fd.parse::<u32>() {
                    if let Some(line) =
                        self.read_virtual_fd_stdin(fd, delimiter, char_limit, exact_char_limit)
                    {
                        return Some(line);
                    }
                    return self.read_heredoc_fd_input(
                        cmd,
                        fd,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                    );
                }
            }
        }

        if let Some(line) = self.stdin_string_for_command(cmd) {
            return Some(trim_read_input(
                line,
                delimiter,
                char_limit,
                exact_char_limit,
            ));
        }

        if let Some(line) = self.read_virtual_fd_stdin(0, delimiter, char_limit, exact_char_limit) {
            return Some(line);
        }

        if self.env_vars.contains_key(&fd_closed_key(0)) {
            return None;
        }

        // If FUNCTION_STDIN is set (from heredoc or redirect), only read from it.
        // Do NOT fall through to process stdin - that would block on the terminal.
        if self.env_vars.contains_key(FUNCTION_STDIN) {
            return self.read_function_stdin(delimiter, char_limit, exact_char_limit);
        }

        self.read_function_stdin(delimiter, char_limit, exact_char_limit)
            .or_else(|| self.read_inherited_process_stdin(delimiter, char_limit, exact_char_limit))
    }

    pub(crate) fn process_substitution_output(&mut self, source: &str) -> Option<String> {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        if ast.commands.is_empty() {
            return None;
        }

        let saved_env = self.env_vars.clone();
        let saved_exit_code = self.exit_code;
        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        let result = self.execute_ast(&ast);
        let output = self.stdout_capture.take().unwrap_or_default();
        self.stdout_capture = saved_capture;
        self.env_vars = saved_env;
        self.exit_code = saved_exit_code;

        match result {
            Ok(()) | Err(ExecuteError::ExitCode(_)) | Err(ExecuteError::Return(_)) => {
                Some(String::from_utf8_lossy(&output).to_string())
            }
            Err(_) => None,
        }
    }

    pub(in crate::executor) fn read_virtual_fd_stdin(
        &mut self,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        let input_key = fd_stdin_key(fd);
        let offset_key = fd_stdin_offset_key(fd);
        let input = self.env_vars.get(&input_key)?.clone();
        if input == FD_PROCESS_STDIN_TARGET {
            return self.read_inherited_process_stdin(delimiter, char_limit, exact_char_limit);
        }
        let offset = self
            .env_vars
            .get(&offset_key)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if offset >= input.len() {
            return None;
        }
        if char_limit == Some(0) {
            return Some(String::new());
        }

        let slice = &input[offset..];
        let mut output = String::new();
        let mut consumed = 0usize;
        let mut took_any = false;
        for (index, ch) in slice.char_indices() {
            if !exact_char_limit && ch == delimiter {
                consumed = index + ch.len_utf8();
                took_any = true;
                break;
            }

            output.push(ch);
            consumed = index + ch.len_utf8();
            took_any = true;
            if char_limit.is_some_and(|limit| output.chars().count() >= limit) {
                break;
            }
        }
        if !took_any {
            return None;
        }

        self.env_vars
            .insert(offset_key, (offset + consumed).to_string());
        Some(trim_read_input(
            output,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }

    pub(in crate::executor) fn read_function_stdin(
        &mut self,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        let input = self.env_vars.get(FUNCTION_STDIN)?.clone();
        let offset = self
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if offset >= input.len() {
            return None;
        }
        if char_limit == Some(0) {
            return Some(String::new());
        }

        let slice = &input[offset..];
        let mut output = String::new();
        let mut consumed = 0usize;
        let mut took_any = false;
        for (index, ch) in slice.char_indices() {
            if !exact_char_limit && ch == delimiter {
                consumed = index + ch.len_utf8();
                took_any = true;
                break;
            }

            output.push(ch);
            consumed = index + ch.len_utf8();
            took_any = true;
            if char_limit.is_some_and(|limit| output.chars().count() >= limit) {
                break;
            }
        }
        if !took_any {
            return None;
        }

        self.env_vars.insert(
            FUNCTION_STDIN_OFFSET.to_string(),
            (offset + consumed).to_string(),
        );
        Some(trim_read_input(
            output,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }

    pub(in crate::executor) fn read_inherited_process_stdin(
        &self,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        if self.env_vars.get(INHERIT_PROCESS_STDIN).map(String::as_str) != Some("1") {
            return None;
        }
        if char_limit == Some(0) {
            return Some(String::new());
        }

        let mut stdin = io::stdin().lock();
        let mut bytes = [0_u8; 1];
        let mut output = String::new();
        loop {
            let count = stdin.read(&mut bytes).ok()?;
            if count == 0 {
                break;
            }

            let ch = bytes[0] as char;
            if !exact_char_limit && ch == delimiter {
                break;
            }

            output.push(ch);
            if char_limit.is_some_and(|limit| output.chars().count() >= limit) {
                break;
            }
        }

        if output.is_empty() {
            return None;
        }

        Some(trim_read_input(
            output,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }

    pub(in crate::executor) fn read_inherited_process_stdin_to_string(&self) -> Option<String> {
        if self.env_vars.get(INHERIT_PROCESS_STDIN).map(String::as_str) != Some("1") {
            return None;
        }

        let mut stdin = io::stdin().lock();
        let mut output = String::new();
        stdin.read_to_string(&mut output).ok()?;
        if output.is_empty() {
            return None;
        }
        Some(output)
    }
}
