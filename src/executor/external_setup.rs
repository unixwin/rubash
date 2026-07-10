use super::*;

impl Executor {
    pub(in crate::executor) fn execute_external(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let (cmd, temp_files) = self.command_with_process_substitution_files(cmd)?;
        let result = self.execute_external_inner(&cmd);
        self.cleanup_process_substitution_files(temp_files);
        result
    }

    pub(in crate::executor) fn command_with_process_substitution_files(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(CommandNode, Vec<PathBuf>), ExecuteError> {
        let mut rewritten = cmd.clone();
        let mut temp_files = Vec::new();
        for word in &mut rewritten.words {
            let Some(source) = word
                .strip_prefix("<(")
                .and_then(|word| word.strip_suffix(')'))
            else {
                continue;
            };
            let Some(output) = self.process_substitution_output(source) else {
                continue;
            };
            let path = self.write_process_substitution_temp(&output)?;
            *word = shell_display_path(&path.to_string_lossy());
            temp_files.push(path);
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
                    temp_files.push(path);
                }
            }
        }
        Ok((rewritten, temp_files))
    }

    pub(in crate::executor) fn cleanup_process_substitution_files(&self, temp_files: Vec<PathBuf>) {
        for path in temp_files {
            let _ = fs::remove_file(path);
        }
    }

    pub(in crate::executor) fn write_process_substitution_temp(
        &self,
        output: &str,
    ) -> Result<PathBuf, ExecuteError> {
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
        fs::write(&dir, output)?;
        Ok(dir)
    }
}
