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

impl Executor {
    pub(in crate::executor) fn execute_external(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let (cmd, process_substitutions) = self.command_with_process_substitution_files(cmd)?;
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
        }
        if let Some(redirect) = &mut rewritten.redirect_out {
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
        Ok((rewritten, files))
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

    fn empty_process_substitution_temp(&self) -> Result<PathBuf, ExecuteError> {
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
}
