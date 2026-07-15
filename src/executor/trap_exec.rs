use super::*;

impl Executor {
    pub(in crate::executor) fn execute_eval(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        let args = cmd.words[1..]
            .iter()
            .map(|word| unescape_remaining_shell_escapes(word))
            .collect::<Vec<_>>();
        match crate::builtins::eval::execute_with_io(args.iter().map(String::as_str), &mut stderr)?
        {
            crate::builtins::eval::EvalAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = status;
                Ok(())
            }
            crate::builtins::eval::EvalAction::Execute(source) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                let source = eval_source_for_reparse(&source);
                let tokens = crate::lexer::tokenize(&source);
                let mut ast = crate::parser::parse(&tokens);
                self.apply_command_output_redirects(cmd, &mut ast)?;
                self.execute_ast(&ast)
            }
        }
    }

    pub fn run_exit_trap(&mut self) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(self.exit_code)
    }

    pub fn run_exit_trap_with_status(&mut self, exit_status: i32) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(exit_status)
    }

    pub(in crate::executor) fn run_exit_trap_for_status(
        &mut self,
        exit_status: i32,
    ) -> Result<i32, ExecuteError> {
        let Some(action) = crate::builtins::trap::take_exit_trap(&mut self.env_vars) else {
            return Ok(exit_status);
        };
        if action.is_empty() {
            return Ok(exit_status);
        }

        self.exit_code = exit_status;
        let tokens = crate::lexer::tokenize(&action);
        let ast = crate::parser::parse(&tokens);
        match self.execute_ast(&ast) {
            Ok(()) => {
                self.exit_code = exit_status;
                Ok(exit_status)
            }
            Err(ExecuteError::ExitCode(code)) => {
                self.exit_code = code;
                Ok(code)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn apply_command_output_redirects(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: ">>".to_string(),
                operator_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    ">>".to_string(),
                    ">>".to_string(),
                )),
                kind: crate::parser::RedirectKind::Append,
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: redirect.operator.clone(),
                operator_metadata: redirect.operator_metadata.clone(),
                kind: redirect.kind.clone(),
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target)
                && redirect_target_fd(&target).is_none()
                && !is_null_device(&target)
            {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: "2>>".to_string(),
                operator_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    "2>>".to_string(),
                    "2>>".to_string(),
                )),
                kind: crate::parser::RedirectKind::Append,
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: redirect.operator.clone(),
                operator_metadata: redirect.operator_metadata.clone(),
                kind: redirect.kind.clone(),
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_exec(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(status) = self.execute_dynamic_fd_exec_redirect(cmd)? {
            return Ok(status);
        }

        if cmd.words.len() == 1 {
            if let Some(status) = self.execute_stdio_only_exec_redirect(cmd)? {
                return Ok(status);
            }
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        self.apply_no_output_builtin_redirects(cmd)?;
        Ok(crate::builtins::exec::execute(
            &cmd.words[1..],
            &self.env_vars,
        )?)
    }

    fn execute_stdio_only_exec_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<i32>, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let fd = redirect.fd.unwrap_or(1);
            if is_closed_redirect_target(&target) {
                self.close_persistent_output_fd(fd)?;
                self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                return Ok(Some(0));
            }
            if let Some(source_fd) = redirect_target_fd(&target) {
                self.copy_persistent_output_fd(fd, source_fd);
                return Ok(Some(0));
            }
            if self.open_persistent_output_process_substitution(fd, &target)? {
                return Ok(Some(0));
            }
            self.create_redirect_output(&target, redirect.clobber)?;
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.env_vars.insert(fd_output_key(fd), target);
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let fd = redirect.fd.unwrap_or(1);
            if is_closed_redirect_target(&target) {
                self.close_persistent_output_fd(fd)?;
                self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                return Ok(Some(0));
            }
            if self.open_persistent_output_process_substitution(fd, &target)? {
                return Ok(Some(0));
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.env_vars.insert(fd_output_key(fd), target);
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            let fd = redirect.fd.unwrap_or(2);
            if is_closed_redirect_target(&target) {
                self.close_persistent_output_fd(fd)?;
                self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                return Ok(Some(0));
            }
            if let Some(source_fd) = redirect_target_fd(&target) {
                self.copy_persistent_output_fd(fd, source_fd);
                return Ok(Some(0));
            }
            if self.open_persistent_output_process_substitution(fd, &target)? {
                return Ok(Some(0));
            }
            if !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.env_vars.insert(fd_output_key(fd), target);
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let fd = redirect.fd.unwrap_or(2);
            if is_closed_redirect_target(&target) {
                self.close_persistent_output_fd(fd)?;
                self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                return Ok(Some(0));
            }
            if self.open_persistent_output_process_substitution(fd, &target)? {
                return Ok(Some(0));
            }
            if !is_null_device(&target) {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
            }
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.env_vars.insert(fd_output_key(fd), target);
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.redirect_in {
            let fd = redirect.fd.unwrap_or(0);
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                self.env_vars.remove(&fd_stdin_key(fd));
                self.env_vars.remove(&fd_stdin_offset_key(fd));
                self.env_vars.remove(&fd_dynamic_input_key(fd));
                self.env_vars.insert(fd_closed_key(fd), "1".to_string());
                return Ok(Some(0));
            }
            if let Some(source_fd) = redirect_target_fd(&target) {
                self.copy_persistent_input_fd(fd, source_fd);
                return Ok(Some(0));
            }

            if let Some(source) = target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                if let Some(input) = self.process_substitution_output(source) {
                    self.env_vars.remove(&fd_closed_key(fd));
                    self.env_vars.insert(fd_stdin_key(fd), input);
                    self.env_vars
                        .insert(fd_stdin_offset_key(fd), "0".to_string());
                    if fd != 0 {
                        self.env_vars
                            .insert(fd_dynamic_input_key(fd), "1".to_string());
                    }
                    return Ok(Some(0));
                }
            }

            let path = shell_path_to_windows(&target, &self.env_vars);
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path)?;
            }
            let input = fs::read_to_string(path)?;
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars.insert(fd_stdin_key(fd), input);
            self.env_vars
                .insert(fd_stdin_offset_key(fd), "0".to_string());
            if fd != 0 {
                self.env_vars
                    .insert(fd_dynamic_input_key(fd), "1".to_string());
            }
            return Ok(Some(0));
        }

        if let Some((fd, input)) = self.exec_heredoc_fd_input(cmd) {
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars.insert(fd_stdin_key(fd), input);
            self.env_vars
                .insert(fd_stdin_offset_key(fd), "0".to_string());
            if fd != 0 {
                self.env_vars
                    .insert(fd_dynamic_input_key(fd), "1".to_string());
            }
            return Ok(Some(0));
        }

        Ok(None)
    }

    fn exec_heredoc_fd_input(&self, cmd: &CommandNode) -> Option<(u32, String)> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd.is_some() && redirect.body.is_some())?;
        let fd = redirect.fd?;
        let body = redirect.body.as_deref()?;
        let input = if let Some(word) = body.strip_prefix('\x1d') {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            input
        } else {
            strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
        };
        Some((fd, input))
    }

    fn copy_persistent_output_fd(&mut self, target_fd: u32, source_fd: u32) {
        if self.env_vars.contains_key(&fd_closed_key(source_fd)) {
            let _ = self.close_persistent_output_fd(target_fd);
            self.env_vars
                .insert(fd_closed_key(target_fd), "1".to_string());
        } else if let Some(target) = self.env_vars.get(&fd_output_key(source_fd)).cloned() {
            self.env_vars.remove(&fd_closed_key(target_fd));
            self.env_vars.insert(fd_output_key(target_fd), target);
            if let Some(source) = self
                .env_vars
                .get(&fd_output_process_substitution_key(source_fd))
                .cloned()
            {
                self.env_vars
                    .insert(fd_output_process_substitution_key(target_fd), source);
            } else {
                self.env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
        } else if let Some(target) = stdio_output_target(source_fd) {
            self.env_vars.remove(&fd_closed_key(target_fd));
            self.env_vars
                .insert(fd_output_key(target_fd), target.to_string());
            self.env_vars
                .remove(&fd_output_process_substitution_key(target_fd));
        } else {
            let _ = self.close_persistent_output_fd(target_fd);
            self.env_vars.remove(&fd_closed_key(target_fd));
        }
    }

    fn open_persistent_output_process_substitution(
        &mut self,
        fd: u32,
        target: &str,
    ) -> Result<bool, ExecuteError> {
        let Some(source) = target
            .strip_prefix(">(")
            .and_then(|target| target.strip_suffix(')'))
        else {
            return Ok(false);
        };

        let path = self.empty_process_substitution_temp()?;
        self.env_vars.remove(&fd_closed_key(fd));
        self.env_vars.insert(
            fd_output_key(fd),
            shell_display_path(&path.to_string_lossy()),
        );
        self.env_vars
            .insert(fd_output_process_substitution_key(fd), source.to_string());
        Ok(true)
    }

    fn close_persistent_output_fd(&mut self, fd: u32) -> Result<(), ExecuteError> {
        let target = self.env_vars.remove(&fd_output_key(fd));
        let source = self
            .env_vars
            .remove(&fd_output_process_substitution_key(fd));
        if let (Some(target), Some(source)) = (target, source) {
            let path = shell_path_to_windows(&target, &self.env_vars);
            let input = fs::read_to_string(&path).unwrap_or_default();
            self.execute_persistent_output_process_substitution(&source, input)?;
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_persistent_output_process_substitution(
        &mut self,
        source: &str,
        input: String,
    ) -> Result<(), ExecuteError> {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        let old_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        self.env_vars.insert(FUNCTION_STDIN.to_string(), input);
        self.env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        let result = self.execute_ast(&ast);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(&mut self.env_vars, FUNCTION_STDIN_OFFSET, old_offset);
        result
    }

    fn execute_dynamic_fd_exec_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<i32>, ExecuteError> {
        let Some(name) = cmd.words.get(1).and_then(|word| dynamic_fd_var_name(word)) else {
            return Ok(None);
        };
        if cmd.words.len() != 2 {
            return Ok(None);
        }

        if cmd.here_string.is_some() || cmd.heredoc.is_some() {
            let Some(input) = self.stdin_string_for_command(cmd) else {
                return Ok(None);
            };
            let fd = self.allocate_dynamic_fd();
            self.env_vars.insert(name.to_string(), fd.to_string());
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars.insert(fd_stdin_key(fd), input);
            self.env_vars
                .insert(fd_stdin_offset_key(fd), "0".to_string());
            self.env_vars
                .insert(fd_dynamic_input_key(fd), "1".to_string());
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.redirect_in {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                self.close_dynamic_fd(name);
                return Ok(Some(0));
            }

            if let Some(source_fd) = redirect_target_fd(&target) {
                let fd = self.allocate_dynamic_fd();
                self.env_vars.insert(name.to_string(), fd.to_string());
                self.copy_persistent_input_fd(fd, source_fd);
                self.copy_persistent_output_fd(fd, source_fd);
                return Ok(Some(0));
            }

            if let Some(source) = target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                if let Some(input) = self.process_substitution_output(source) {
                    let fd = self.allocate_dynamic_fd();
                    self.env_vars.insert(name.to_string(), fd.to_string());
                    self.env_vars.remove(&fd_closed_key(fd));
                    self.env_vars.insert(fd_stdin_key(fd), input);
                    self.env_vars
                        .insert(fd_stdin_offset_key(fd), "0".to_string());
                    self.env_vars
                        .insert(fd_dynamic_input_key(fd), "1".to_string());
                    return Ok(Some(0));
                }
            }

            let path = shell_path_to_windows(&target, &self.env_vars);
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path)?;
            }
            let input = fs::read_to_string(path)?;
            let fd = self.allocate_dynamic_fd();
            self.env_vars.insert(name.to_string(), fd.to_string());
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars.insert(fd_stdin_key(fd), input);
            self.env_vars
                .insert(fd_stdin_offset_key(fd), "0".to_string());
            self.env_vars
                .insert(fd_dynamic_input_key(fd), "1".to_string());
            if redirect.operator == "<>" {
                self.env_vars.insert(fd_output_key(fd), target);
            }
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                self.close_dynamic_fd(name);
                return Ok(Some(0));
            }

            let fd = self.allocate_dynamic_fd();
            self.env_vars.insert(name.to_string(), fd.to_string());
            if let Some(source_fd) = redirect_target_fd(&target) {
                self.copy_persistent_output_fd(fd, source_fd);
                return Ok(Some(0));
            }
            if self.open_persistent_output_process_substitution(fd, &target)? {
                return Ok(Some(0));
            }
            self.create_redirect_output(&target, redirect.clobber)?;
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.env_vars.insert(fd_output_key(fd), target);
            return Ok(Some(0));
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                self.close_dynamic_fd(name);
                return Ok(Some(0));
            }
            let fd = self.allocate_dynamic_fd();
            if self.open_persistent_output_process_substitution(fd, &target)? {
                self.env_vars.insert(name.to_string(), fd.to_string());
                return Ok(Some(0));
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            self.env_vars.insert(name.to_string(), fd.to_string());
            self.env_vars.remove(&fd_closed_key(fd));
            self.env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.env_vars.insert(fd_output_key(fd), target);
            return Ok(Some(0));
        }

        Ok(None)
    }

    fn copy_persistent_input_fd(&mut self, target_fd: u32, source_fd: u32) {
        if self.env_vars.contains_key(&fd_closed_key(source_fd)) {
            self.env_vars.remove(&fd_stdin_key(target_fd));
            self.env_vars.remove(&fd_stdin_offset_key(target_fd));
            self.env_vars.remove(&fd_dynamic_input_key(target_fd));
            self.env_vars
                .insert(fd_closed_key(target_fd), "1".to_string());
            return;
        }

        let source_key = fd_stdin_key(source_fd);
        let Some(input) = self.env_vars.get(&source_key).cloned() else {
            if source_fd == 0
                && self
                    .env_vars
                    .get(INHERIT_PROCESS_STDIN)
                    .is_some_and(|value| value == "1")
            {
                self.env_vars
                    .insert(fd_stdin_key(target_fd), FD_PROCESS_STDIN_TARGET.to_string());
                self.env_vars.remove(&fd_stdin_offset_key(target_fd));
                self.env_vars.remove(&fd_dynamic_input_key(target_fd));
                self.env_vars.remove(&fd_closed_key(target_fd));
                return;
            }
            self.env_vars.remove(&fd_stdin_key(target_fd));
            self.env_vars.remove(&fd_stdin_offset_key(target_fd));
            self.env_vars.remove(&fd_dynamic_input_key(target_fd));
            self.env_vars.remove(&fd_closed_key(target_fd));
            return;
        };
        if input == FD_PROCESS_STDIN_TARGET {
            self.env_vars.insert(fd_stdin_key(target_fd), input);
            self.env_vars.remove(&fd_stdin_offset_key(target_fd));
            self.env_vars.remove(&fd_dynamic_input_key(target_fd));
            self.env_vars.remove(&fd_closed_key(target_fd));
            return;
        }
        let offset = self
            .env_vars
            .get(&fd_stdin_offset_key(source_fd))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let remaining = input.get(offset..).unwrap_or_default().to_string();
        self.env_vars.insert(fd_stdin_key(target_fd), remaining);
        self.env_vars
            .insert(fd_stdin_offset_key(target_fd), "0".to_string());
        self.env_vars
            .insert(fd_dynamic_input_key(target_fd), "1".to_string());
        self.env_vars.remove(&fd_closed_key(target_fd));
    }

    fn allocate_dynamic_fd(&self) -> u32 {
        (10..1024)
            .find(|fd| {
                !self.env_vars.contains_key(&fd_output_key(*fd))
                    && !self.env_vars.contains_key(&fd_stdin_key(*fd))
            })
            .unwrap_or(10)
    }

    fn close_dynamic_fd(&mut self, name: &str) {
        if let Some(fd) = self
            .env_vars
            .get(name)
            .and_then(|value| value.parse::<u32>().ok())
        {
            let _ = self.close_persistent_output_fd(fd);
            self.env_vars.remove(&fd_stdin_key(fd));
            self.env_vars.remove(&fd_stdin_offset_key(fd));
            self.env_vars.remove(&fd_dynamic_input_key(fd));
            self.env_vars.remove(&fd_closed_key(fd));
        }
        self.env_vars.remove(name);
    }

    pub(in crate::executor) fn execute_exec_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let status = self.execute_exec(cmd)?;
        let dynamic_fd_redirect = is_dynamic_fd_exec_redirect(cmd);
        self.exit_code = status;
        if !dynamic_fd_redirect && crate::builtins::exec::replaces_shell(&cmd.words[1..]) {
            return Err(ExecuteError::ExitCode(status));
        }
        Ok(())
    }
}

fn is_dynamic_fd_exec_redirect(cmd: &CommandNode) -> bool {
    cmd.words.len() == 2
        && cmd
            .words
            .get(1)
            .and_then(|word| dynamic_fd_var_name(word))
            .is_some()
        && (cmd.redirect_in.is_some()
            || cmd.redirect_out.is_some()
            || cmd.append.is_some()
            || cmd.here_string.is_some()
            || cmd.heredoc.is_some())
}

fn dynamic_fd_var_name(word: &str) -> Option<&str> {
    let name = word.strip_prefix('{')?.strip_suffix('}')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    chars
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        .then_some(name)
}
