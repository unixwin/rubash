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

pub(in crate::executor) fn command_needs_process_substitution_materialization(
    cmd: &CommandNode,
) -> bool {
    // Shell-owned `read` must consume virtual descriptors directly. Its
    // ordinary `<&N` redirects are not process substitutions; materializing
    // them into a temporary file would advance or replace the descriptor on
    // every loop iteration.
    if cmd.words.first().map(String::as_str) == Some("read")
        && cmd.process_substitutions.is_empty()
        && !cmd
            .redirects
            .iter()
            .any(|redirect| redirect.target.starts_with("<(") || redirect.target.starts_with(">("))
    {
        return false;
    }
    // Persistent fd-prefixed output substitutions are opened by the exec
    // builtin and must retain their backing file across later commands.
    if cmd.words.first().map(String::as_str) == Some("exec")
        && cmd.redirects.iter().any(|redirect| {
            matches!(
                redirect.kind,
                crate::parser::RedirectKind::Output
                    | crate::parser::RedirectKind::Append
                    | crate::parser::RedirectKind::ClobberOutput
            ) && redirect.target.starts_with(">(")
                && redirect.target.ends_with(')')
        })
    {
        return false;
    }

    if cmd.redirects.is_empty()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.heredoc.is_none()
        && cmd.heredoc_redirects.is_empty()
        && cmd.here_string.is_none()
        && cmd.process_substitutions.is_empty()
        && !cmd
            .word_metadata
            .iter()
            .any(word_metadata_needs_process_substitution_materialization)
    {
        return cmd.words.iter().enumerate().any(|(index, word)| {
            if cmd.word_metadata.get(index).is_some() {
                return false;
            }
            word.strip_prefix("<(")
                .or_else(|| word.strip_prefix(">("))
                .is_some_and(|word| word.ends_with(')'))
        });
    }

    true
}

fn word_metadata_needs_process_substitution_materialization(metadata: &WordMetadata) -> bool {
    !metadata.process_substitutions.is_empty()
        || raw_word_contains_process_substitution(&metadata.raw)
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
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        if cmd.heredoc.is_some() || cmd.here_string.is_some() {
            process.stdin(Stdio::piped());
        } else if let Some(ref redirect) = cmd.redirect_in {
            let target = self.expand_word(&redirect.target);
            if redirect.fd.unwrap_or(0) == 0 {
                if let Some(fd) = redirect_target_fd(&target) {
                    if let Some(FdReadEndpoint::CoprocStdout(pid)) = self.fd_table.read_endpoint(fd)
                    {
                        let reader = self
                            .coproc_stdout_readers
                            .get(&pid)
                            .ok_or_else(|| {
                                io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    "coprocess output is closed",
                                )
                            })?
                            .try_clone()?;
                        process.stdin(Stdio::from(reader));
                        return Ok(());
                    }
                }
            }
            if is_closed_redirect_target(&target) {
                if redirect.fd.unwrap_or(0) == 0 {
                    process.stdin(Stdio::null());
                }
                return Ok(());
            }
            if redirect.fd.unwrap_or(0) == 0 && self.input_fd_redirects_to_process_stdin(&target) {
                return Ok(());
            }
            let file = if redirect.append {
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?
            } else {
                self.open_input_redirect(&target)?
            };
            if redirect.fd.unwrap_or(0) == 0 {
                process.stdin(Stdio::from(file));
            }
        } else if self.env_vars.contains_key(FUNCTION_STDIN)
            || self.virtual_fd_stdin_remaining(0).is_some()
        {
            process.stdin(Stdio::piped());
        } else if self.fd_table.is_closed(0)
            || (self.fd_table.has_entry(0) && !self.fd_table.is_open_for_read(0))
        {
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
        let mut redirect_target_rewrites = Vec::new();
        // GNU subst.c: an unquoted `name=<( )` word is an assignment whose
        // value is the process substitution (make_dev_fd_filename,
        // subst.c:6362+). The first parse splits that word into `name=` and
        // a procsub token when it appears as an eval argument, which turned
        // the materialized temp path into a command word and lost the
        // assignment (procsub.tests:25 `eval f=<(echo test4) "; cat \$f"`).
        // Re-merge the pair so the word materializes into the assignment
        // value the way a direct `name=<( )` statement parses.
        let mut merge_index = 0;
        while merge_index + 1 < rewritten.words.len() {
            let assignment_word = &rewritten.words[merge_index];
            let is_bare_assignment = assignment_word.ends_with('=')
                && !assignment_word.contains(' ')
                && is_shell_name(&assignment_word[..assignment_word.len() - 1]);
            let procsub_word = rewritten.words[merge_index + 1].clone();
            let is_whole_procsub = (procsub_word.starts_with("<(")
                || procsub_word.starts_with(">("))
                && procsub_word.ends_with(')');
            if is_bare_assignment && is_whole_procsub {
                let next_metadata = rewritten.word_metadata.get(merge_index + 1).cloned();
                rewritten.words[merge_index].push_str(&procsub_word);
                // Drop the absorbed procsub word and hand its metadata to
                // the merged word so the loop below materializes the
                // substitution inside the assignment value.
                rewritten.words.remove(merge_index + 1);
                if merge_index + 1 < rewritten.word_metadata.len() {
                    rewritten.word_metadata.remove(merge_index + 1);
                }
                if let Some(metadata) = next_metadata {
                    if merge_index < rewritten.word_metadata.len() {
                        rewritten.word_metadata[merge_index] = metadata;
                    } else {
                        rewritten.word_metadata.push(metadata);
                    }
                }
                continue;
            }
            merge_index += 1;
        }
        for word_index in 0..rewritten.words.len() {
            let metadata = rewritten.word_metadata.get(word_index).cloned();
            let substitutions = metadata
                .as_ref()
                .map(|metadata| metadata.process_substitutions.clone())
                .unwrap_or_default();
            let substitutions = if substitutions.is_empty()
                && metadata
                    .as_ref()
                    .is_some_and(|metadata| raw_word_contains_process_substitution(&metadata.raw))
            {
                WordMetadata::new(
                    word_index,
                    rewritten.words[word_index].clone(),
                    rewritten.words[word_index].clone(),
                )
                .process_substitutions
            } else {
                substitutions
            };
            if substitutions.is_empty() {
                if metadata.is_none() {
                    self.materialize_standalone_process_substitution_word(
                        &mut rewritten.words[word_index],
                        &mut files,
                    )?;
                }
            } else {
                self.materialize_process_substitution_word(
                    &mut rewritten.words[word_index],
                    substitutions,
                    &mut files,
                )?;
            }
        }
        if let Some(redirect) = &mut rewritten.redirect_in {
            if let Some(source) = redirect
                .target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                if let Some(output) = self.process_substitution_output_bytes(source) {
                    let path = self.write_process_substitution_temp_bytes(&output)?;
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                    files.inputs.push(path);
                }
            }
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_word(&redirect.target);
                if let Some(fd) = redirect_target_fd(&target) {
                    if self.fd_table.is_open_for_read(fd) {
                        if let Some(input) = self.virtual_fd_stdin_remaining_bytes(fd) {
                            let path = self.write_process_substitution_temp_bytes(&input)?;
                            let input_len = self.virtual_fd_stdin_len(fd);
                            self.fd_table.consume_all_text(fd);
                            self.env_vars.insert(fd_stdin_offset_key(fd), input_len);
                            redirect.target = shell_display_path(&path.to_string_lossy());
                            files.inputs.push(path);
                        } else if let Some(MaterializedRead::File(path)) = self
                            .fd_table
                            .materialize_for_child()
                            .get(&fd)
                            .and_then(|materialized| materialized.read.clone())
                        {
                            redirect.target = shell_display_path(&path.to_string_lossy());
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
                let old_target = redirect.target.clone();
                redirect.target = display_path.clone();
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
            }
            if let Some(redirect) = &mut rewritten.redirect_err_append {
                let old_target = redirect.target.clone();
                redirect.target = display_path;
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
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
                let old_target = redirect.target.clone();
                redirect.target = display_path.clone();
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
            }
            if let Some(redirect) = &mut rewritten.redirect_err_append {
                let old_target = redirect.target.clone();
                redirect.target = display_path;
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
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
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
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
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
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
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
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
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                    files
                        .outputs
                        .push(OutputProcessSubstitution { path, source });
                }
            }
        }
        for redirect in &mut rewritten.redirects {
            if let Some((_, new_target)) = redirect_target_rewrites
                .iter()
                .find(|(old_target, _)| redirect.target == *old_target)
            {
                redirect.target = new_target.clone();
            }
        }
        sync_ordered_redirect_targets(&mut rewritten);
        Ok((rewritten, files))
    }

    fn materialize_standalone_process_substitution_word(
        &mut self,
        word: &mut String,
        files: &mut ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        if let Some(source) = word
            .strip_prefix("<(")
            .and_then(|word| word.strip_suffix(')'))
        {
            let Some(output) = self.process_substitution_output_bytes(source) else {
                return Ok(());
            };
            let path = self.write_process_substitution_temp_bytes(&output)?;
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
        Ok(())
    }

    fn materialize_process_substitution_word(
        &mut self,
        word: &mut String,
        substitutions: Vec<crate::parser::ProcessSubstitution>,
        files: &mut ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        for substitution in substitutions {
            let path = if substitution.output {
                let path = self.empty_process_substitution_temp()?;
                files.outputs.push(OutputProcessSubstitution {
                    path: path.clone(),
                    source: substitution.source,
                });
                path
            } else {
                let Some(output) = self.process_substitution_output_bytes(&substitution.source)
                else {
                    continue;
                };
                let path = self.write_process_substitution_temp_bytes(&output)?;
                files.inputs.push(path.clone());
                path
            };
            let display_path = shell_display_path(&path.to_string_lossy());
            *word = word.replacen(&substitution.target, &display_path, 1);
        }
        Ok(())
    }

    pub(in crate::executor) fn materialize_assignment_process_substitutions(
        &mut self,
        value: &str,
    ) -> Result<String, ExecuteError> {
        if !value.contains("<(") && !value.contains(">(") {
            return Ok(value.to_string());
        }

        let mut word = value.to_string();
        let substitutions =
            crate::parser::WordMetadata::new(0, value.to_string(), value.to_string())
                .process_substitutions;

        for substitution in substitutions {
            let path = if substitution.output {
                let path = self.empty_process_substitution_temp()?;
                self.assignment_output_process_substitutions.insert(
                    shell_display_path(&path.to_string_lossy()),
                    substitution.source,
                );
                path
            } else {
                let Some(output) = self.process_substitution_output_bytes(&substitution.source)
                else {
                    continue;
                };
                self.write_process_substitution_temp_bytes(&output)?
            };
            let display_path = shell_display_path(&path.to_string_lossy());
            word = word.replacen(&substitution.target, &display_path, 1);
        }
        Ok(word)
    }

    fn apply_default_external_stdin_file(
        &mut self,
        rewritten: &mut CommandNode,
        files: &mut ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        if rewritten.redirect_in.is_some()
            || rewritten.heredoc.is_some()
            || rewritten.here_string.is_some()
            || self.virtual_fd_stdin_remaining_bytes(0).is_none()
        {
            return Ok(());
        }

        let input = self.virtual_fd_stdin_remaining_bytes(0).unwrap_or_default();
        let path = self.write_process_substitution_temp_bytes(&input)?;
        let input_len = self.virtual_fd_stdin_len(0);
        self.fd_table.consume_all_text(0);
        self.env_vars.insert(fd_stdin_offset_key(0), input_len);
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

    pub(in crate::executor) fn finish_assignment_output_process_substitutions_for_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if self.assignment_output_process_substitutions.is_empty() {
            return Ok(());
        }

        let targets = [
            cmd.redirect_out.as_ref(),
            cmd.append.as_ref(),
            cmd.redirect_err.as_ref(),
            cmd.redirect_err_append.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|redirect| {
            let target = self.expand_word(&redirect.target);
            self.assignment_output_process_substitutions
                .contains_key(&target)
                .then_some(target)
        })
        .collect::<Vec<_>>();

        for target in targets {
            self.finish_assignment_output_process_substitution_target(&target)?;
        }

        let remaining_targets = self
            .assignment_output_process_substitutions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for target in remaining_targets {
            self.finish_assignment_output_process_substitution_target(&target)?;
        }

        Ok(())
    }

    fn finish_assignment_output_process_substitution_target(
        &mut self,
        target: &str,
    ) -> Result<(), ExecuteError> {
        let Some(source) = self.assignment_output_process_substitutions.remove(target) else {
            return Ok(());
        };
        let path = shell_path_to_windows(target, &self.env_vars);
        let input = fs::read_to_string(&path).unwrap_or_default();
        self.execute_persistent_output_process_substitution(&source, input)?;
        let _ = fs::remove_file(path);
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
        let input = fs::read(&output.path).unwrap_or_default();
        let tokens = crate::lexer::tokenize(&output.source);
        let ast = crate::parser::parse(&tokens);
        let old_stdin = self.env_vars.get(FUNCTION_STDIN).cloned();
        let old_offset = self.env_vars.get(FUNCTION_STDIN_OFFSET).cloned();
        let old_fd0 = self.fd_table.entries.get(&0).cloned();
        let fd0_key = fd_stdin_key(0);
        let fd0_offset_key = fd_stdin_offset_key(0);
        let fd0_dynamic_key = fd_dynamic_input_key(0);
        let fd0_closed_key = fd_closed_key(0);
        let old_fd0_stdin = self.env_vars.get(&fd0_key).cloned();
        let old_fd0_offset = self.env_vars.get(&fd0_offset_key).cloned();
        let old_fd0_dynamic = self.env_vars.get(&fd0_dynamic_key).cloned();
        let old_fd0_closed = self.env_vars.get(&fd0_closed_key).cloned();
        self.set_fd_input_bytes(0, input.clone(), false);
        if std::str::from_utf8(&input).is_ok() {
            self.env_vars.insert(
                FUNCTION_STDIN.to_string(),
                crate::executor::substitution_metadata::bytes_to_shell_text(&input),
            );
        } else {
            self.env_vars.remove(FUNCTION_STDIN);
        }
        self.env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        let result = self.execute_ast(&ast);
        // GNU process_substitute forks for `>(...)` as well (subst.c:6362),
        // so $! points at that child and `wait $!` reports its status. The
        // substitution body ran inline on this executor; register its status
        // the same way the input form does.
        let status = match &result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::ExitCode(code))
            | Err(ExecuteError::ExpansionFailure(code))
            | Err(ExecuteError::Return(code)) => *code,
            _ => 1,
        };
        self.register_process_substitution_status(status);
        match old_fd0 {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }
        restore_optional_env_var(&mut self.env_vars, &fd0_key, old_fd0_stdin);
        restore_optional_env_var(&mut self.env_vars, &fd0_offset_key, old_fd0_offset);
        restore_optional_env_var(&mut self.env_vars, &fd0_dynamic_key, old_fd0_dynamic);
        restore_optional_env_var(&mut self.env_vars, &fd0_closed_key, old_fd0_closed);
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
        self.write_process_substitution_temp_bytes(output.as_bytes())
    }

    /// GNU subst.c:6362 process_substitute forks an asynchronous child for
    /// every process substitution; jobs.c adds that fork to the job list so
    /// `$!` expands to its pid (execute_cmd.c) and `wait $!` reports the
    /// child's exit status (procsub.tests: `cat <(exit 123); wait $!` ->
    /// 123). Rubash evaluates `<(...)` substitutions inline while
    /// materializing the containing command, so the already-finished status
    /// is recorded under a synthetic pid in the dedicated high range that
    /// cannot collide with a real OS child pid.
    pub(in crate::executor) fn register_process_substitution_status(&mut self, status: i32) {
        const PROCSUB_SEQ_KEY: &str = "RUBASH_INTERNAL_PROCSUB_SEQ";
        let seq = self
            .env_vars
            .get(PROCSUB_SEQ_KEY)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        self.env_vars
            .insert(PROCSUB_SEQ_KEY.to_string(), seq.wrapping_add(1).to_string());
        let pid = self.shell_pid.wrapping_add(0x4000_0000).wrapping_add(seq);
        self.job_table.completed_statuses.insert(pid, status);
        self.last_background_pid = Some(pid);
    }

    pub(in crate::executor) fn write_process_substitution_temp_bytes(
        &self,
        output: &[u8],
    ) -> Result<PathBuf, ExecuteError> {
        let path = self.process_substitution_temp_path()?;
        fs::write(&path, output)?;
        Ok(path)
    }

    pub(in crate::executor) fn virtual_fd_stdin_remaining_bytes(&self, fd: u32) -> Option<Vec<u8>> {
        match self.fd_table.materialize_for_child().remove(&fd)?.read? {
            MaterializedRead::Bytes(input) => Some(input),
            MaterializedRead::InheritedProcessStdin => None,
            _ => None,
        }
    }

    pub(in crate::executor) fn virtual_fd_stdin_remaining(&self, fd: u32) -> Option<String> {
        self.virtual_fd_stdin_remaining_bytes(fd)
            .map(|bytes| crate::executor::substitution_metadata::bytes_to_shell_text(&bytes))
    }

    fn virtual_fd_stdin_len(&self, fd: u32) -> String {
        if let Some((input, _)) = self.fd_table.input_snapshot_bytes(fd) {
            return input.len().to_string();
        }
        "0".to_string()
    }

    fn external_fd_heredoc_input(&mut self, cmd: &CommandNode, fd: u32) -> Option<String> {
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
        Some(self.expand_heredoc_body_mut(body))
    }
}

fn sync_ordered_redirect_targets(command: &mut CommandNode) {
    for redirect in &mut command.redirects {
        if !redirect.target.starts_with(">(") {
            continue;
        }
        let source = if redirect.fd.unwrap_or(1) == 2 {
            command
                .redirect_err_append
                .as_ref()
                .or(command.redirect_err.as_ref())
        } else {
            command.redirect_out.as_ref().or(command.append.as_ref())
        };
        if let Some(source) = source {
            redirect.target = source.target.clone();
            redirect.target_metadata = source.target_metadata.clone();
        }
    }
}

fn raw_word_contains_process_substitution(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double && matches!(ch, '<' | '>') && chars.get(index + 1) == Some(&'(') {
            return true;
        }
        index += 1;
    }
    false
}
