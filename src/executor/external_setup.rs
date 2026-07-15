use super::*;

#[derive(Debug, Default)]
pub(in crate::executor) struct ProcessSubstitutionFiles {
    inputs: Vec<PathBuf>,
    outputs: Vec<OutputProcessSubstitution>,
}

#[derive(Debug)]
struct OutputProcessSubstitution {
    path: PathBuf,
    source: String,
}

fn exec_keeps_output_process_substitution(words: &[String], redirect: &Redirect) -> bool {
    if !redirect
        .target
        .strip_prefix(">(")
        .is_some_and(|target| target.ends_with(')'))
    {
        return false;
    }
    matches!(words, [command] if command == "exec")
        || matches!(words, [command, fd] if command == "exec" && dynamic_fd_word(fd).is_some())
}

pub(in crate::executor) fn shared_combined_output_process_substitution(
    first: Option<&Redirect>,
    second: Option<&Redirect>,
) -> Option<String> {
    let first = first?;
    let second = second?;
    if first.target != second.target
        || !matches!(first.operator.as_str(), "&>" | "&>>")
        || first.operator != second.operator
    {
        return None;
    }
    first
        .target
        .strip_prefix(">(")
        .and_then(|target| target.strip_suffix(')'))
        .map(str::to_string)
}

fn dynamic_fd_word(word: &str) -> Option<&str> {
    let name = word.strip_prefix('{')?.strip_suffix('}')?;
    (!name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric()))
    .then_some(name)
}

impl Executor {
    pub(in crate::executor) fn apply_external_stdin_redirect(
        &self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        if cmd.heredoc.is_some() || cmd.here_string.is_some() {
            process.stdin(Stdio::piped());
        } else if let Some(ref redirect) = cmd.redirect_in {
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                if redirect.fd.unwrap_or(0) == 0 {
                    process.stdin(Stdio::null());
                }
                return Ok(());
            }
            if redirect.fd.unwrap_or(0) == 0 && self.input_fd_redirects_to_process_stdin(&target) {
                return Ok(());
            }
            let path = shell_path_to_windows(&target, &self.env_vars);
            let file = if redirect.append {
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(path)?
            } else {
                File::open(path)?
            };
            if redirect.fd.unwrap_or(0) == 0 {
                process.stdin(Stdio::from(file));
            }
        } else if self.env_vars.contains_key(&fd_closed_key(0)) {
            process.stdin(Stdio::null());
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_external(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let (mut cmd, mut process_substitutions) =
            self.command_with_process_substitution_files(cmd)?;
        self.apply_default_external_stdin_file(&mut cmd, &mut process_substitutions)?;
        let result = self.execute_external_inner(&cmd);
        self.finish_process_substitutions(process_substitutions)?;
        result
    }

    pub(in crate::executor) fn command_with_process_substitution_files(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(CommandNode, ProcessSubstitutionFiles), ExecuteError> {
        let mut rewritten = cmd.clone();
        let mut files = ProcessSubstitutionFiles::default();
        for word in &mut rewritten.words {
            if let Some(source) = word
                .strip_prefix("<(")
                .and_then(|word| word.strip_suffix(')'))
            {
                let Some(output) = self.process_substitution_output(source) else {
                    continue;
                };
                let path = self.write_process_substitution_temp(&output)?;
                *word = shell_display_path(&path.to_string_lossy());
                files.inputs.push(path);
            } else if let Some(source) = word
                .strip_prefix(">(")
                .and_then(|word| word.strip_suffix(')'))
            {
                let source = source.to_string();
                let path = self.empty_process_substitution_temp()?;
                *word = shell_display_path(&path.to_string_lossy());
                files
                    .outputs
                    .push(OutputProcessSubstitution { path, source });
            }
        }
        if let Some(redirect) = &mut rewritten.redirect_in {
            if let Some(source) = redirect
                .target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                if let Some(output) = self.process_substitution_output(source) {
                    let path = self.write_process_substitution_temp(&output)?;
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    files.inputs.push(path);
                }
            }
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_word(&redirect.target);
                if let Some(fd) = redirect_target_fd(&target) {
                    if self.env_vars.contains_key(&fd_dynamic_input_key(fd)) {
                        if let Some(input) = self.virtual_fd_stdin_remaining(fd) {
                            let path = self.write_process_substitution_temp(&input)?;
                            self.env_vars
                                .insert(fd_stdin_offset_key(fd), self.virtual_fd_stdin_len(fd));
                            redirect.target = shell_display_path(&path.to_string_lossy());
                            files.inputs.push(path);
                        }
                    } else if let Some(input) = self.external_fd_heredoc_input(cmd, fd) {
                        let path = self.write_process_substitution_temp(&input)?;
                        redirect.target = shell_display_path(&path.to_string_lossy());
                        files.inputs.push(path);
                    }
                }
            }
        }
        let exec_words = rewritten.words.clone();
        if let Some(source) = shared_combined_output_process_substitution(
            rewritten.redirect_out.as_ref(),
            rewritten.redirect_err_append.as_ref(),
        ) {
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut rewritten.redirect_out {
                redirect.target = display_path.clone();
            }
            if let Some(redirect) = &mut rewritten.redirect_err_append {
                redirect.target = display_path;
            }
            files
                .outputs
                .push(OutputProcessSubstitution { path, source });
        }
        if let Some(source) = shared_combined_output_process_substitution(
            rewritten.append.as_ref(),
            rewritten.redirect_err_append.as_ref(),
        ) {
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut rewritten.append {
                redirect.target = display_path.clone();
            }
            if let Some(redirect) = &mut rewritten.redirect_err_append {
                redirect.target = display_path;
            }
            files
                .outputs
                .push(OutputProcessSubstitution { path, source });
        }
        if let Some(redirect) = &mut rewritten.redirect_out {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    files
                        .outputs
                        .push(OutputProcessSubstitution { path, source });
                }
            }
        }
        if let Some(redirect) = &mut rewritten.append {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    files
                        .outputs
                        .push(OutputProcessSubstitution { path, source });
                }
            }
        }
        if let Some(redirect) = &mut rewritten.redirect_err {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    files
                        .outputs
                        .push(OutputProcessSubstitution { path, source });
                }
            }
        }
        if let Some(redirect) = &mut rewritten.redirect_err_append {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    files
                        .outputs
                        .push(OutputProcessSubstitution { path, source });
                }
            }
        }
        Ok((rewritten, files))
    }

    fn apply_default_external_stdin_file(
        &mut self,
        rewritten: &mut CommandNode,
        files: &mut ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        if rewritten.redirect_in.is_some()
            || rewritten.heredoc.is_some()
            || rewritten.here_string.is_some()
            || !self.env_vars.contains_key(&fd_stdin_key(0))
        {
            return Ok(());
        }

        let input = self.virtual_fd_stdin_remaining(0).unwrap_or_default();
        let path = self.write_process_substitution_temp(&input)?;
        self.env_vars
            .insert(fd_stdin_offset_key(0), self.virtual_fd_stdin_len(0));
        let target = shell_display_path(&path.to_string_lossy());
        rewritten.redirect_in = Some(Redirect {
            fd: Some(0),
            fd_var: None,
            operator: "<".to_string(),
            operator_metadata: Box::new(crate::parser::WordMetadata::new(
                0,
                "<".to_string(),
                "<".to_string(),
            )),
            kind: crate::parser::RedirectKind::Input,
            target_metadata: Box::new(crate::parser::WordMetadata::new(
                0,
                target.clone(),
                target.clone(),
            )),
            target,
            append: false,
            clobber: false,
        });
        files.inputs.push(path);
        Ok(())
    }

    pub(in crate::executor) fn finish_process_substitutions(
        &mut self,
        files: ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        let mut error = None;
        for output in &files.outputs {
            if error.is_none() {
                if let Err(output_error) = self.execute_output_process_substitution(output) {
                    error = Some(output_error);
                }
            }
        }
        self.cleanup_process_substitution_files(files);
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }

    pub(in crate::executor) fn cleanup_process_substitution_files(
        &self,
        files: ProcessSubstitutionFiles,
    ) {
        for path in files.inputs {
            let _ = fs::remove_file(path);
        }
        for output in files.outputs {
            let _ = fs::remove_file(output.path);
        }
    }

    fn execute_output_process_substitution(
        &mut self,
        output: &OutputProcessSubstitution,
    ) -> Result<(), ExecuteError> {
        let input = fs::read_to_string(&output.path).unwrap_or_default();
        let tokens = crate::lexer::tokenize(&output.source);
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

    pub(in crate::executor) fn empty_process_substitution_temp(
        &self,
    ) -> Result<PathBuf, ExecuteError> {
        let path = self.process_substitution_temp_path()?;
        File::create(&path)?;
        Ok(path)
    }

    fn process_substitution_temp_path(&self) -> Result<PathBuf, ExecuteError> {
        let dir_value = self
            .env_vars
            .get("TMPDIR")
            .cloned()
            .unwrap_or_else(safe_temp_dir_string);
        let mut dir = shell_path_to_windows(&dir_value, &self.env_vars);
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        dir.push(format!(
            "rubash-process-subst-{}-{nanos}.tmp",
            std::process::id()
        ));
        Ok(dir)
    }

    pub(in crate::executor) fn write_process_substitution_temp(
        &self,
        output: &str,
    ) -> Result<PathBuf, ExecuteError> {
        let path = self.process_substitution_temp_path()?;
        fs::write(&path, output)?;
        Ok(path)
    }

    fn virtual_fd_stdin_remaining(&self, fd: u32) -> Option<String> {
        let input = self.env_vars.get(&fd_stdin_key(fd))?;
        let offset = self
            .env_vars
            .get(&fd_stdin_offset_key(fd))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        Some(input.get(offset..).unwrap_or_default().to_string())
    }

    fn virtual_fd_stdin_len(&self, fd: u32) -> String {
        self.env_vars
            .get(&fd_stdin_key(fd))
            .map(|input| input.len().to_string())
            .unwrap_or_else(|| "0".to_string())
    }

    fn external_fd_heredoc_input(&self, cmd: &CommandNode, fd: u32) -> Option<String> {
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
}
