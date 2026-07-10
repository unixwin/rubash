use super::*;

impl Executor {
    pub(in crate::executor) fn handle_external_file_builtins(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        match cmd.words[0].as_str() {
            "mkdir" => self.external_mkdir(cmd),
            "touch" => self.external_touch(cmd),
            "chmod" => {
                self.exit_code = 0;
                Ok(true)
            }
            "cp" => self.external_cp(cmd),
            "rm" => self.external_rm(cmd),
            "rmdir" => self.external_rmdir(cmd),
            "cat" => self.external_cat(cmd),
            "mkfifo" => self.external_mkfifo(cmd),
            _ => Ok(false),
        }
    }

    fn external_mkdir(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        for path in &cmd.words[1..] {
            fs::create_dir_all(shell_path_to_windows(
                &self.expand_word(path),
                &self.env_vars,
            ))?;
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_touch(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        for path in &cmd.words[1..] {
            let expanded = self.expand_word(path);
            let target = shell_path_to_windows(&expanded, &self.env_vars);
            if let Err(error) = File::create(target) {
                if !(cfg!(windows) && contains_windows_forbidden_posix_filename_char(&expanded)) {
                    return Err(error.into());
                }
            }
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_cp(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let mut args = Vec::new();
        for word in &cmd.words[1..] {
            if !word.starts_with('-') {
                args.push(self.expand_word(word));
            }
        }

        if args.len() < 2 {
            eprintln!("{}cp: missing file operand", self.diagnostic_prefix());
            self.exit_code = 1;
            return Ok(true);
        }

        let destination =
            shell_path_to_windows(args.last().expect("cp destination"), &self.env_vars);
        if args.len() > 2 && !destination.is_dir() {
            eprintln!(
                "{}cp: target '{}' is not a directory",
                self.diagnostic_prefix(),
                args.last().expect("cp destination")
            );
            self.exit_code = 1;
            return Ok(true);
        }

        for source in &args[..args.len() - 1] {
            let source_path = shell_path_to_windows(source, &self.env_vars);
            let target_path = if destination.is_dir() {
                let Some(name) = source_path.file_name() else {
                    eprintln!(
                        "{}cp: cannot stat '{}': No such file or directory",
                        self.diagnostic_prefix(),
                        source
                    );
                    self.exit_code = 1;
                    return Ok(true);
                };
                destination.join(name)
            } else {
                destination.clone()
            };

            if let Err(error) = fs::copy(&source_path, &target_path) {
                eprintln!("{}cp: {error}", self.diagnostic_prefix());
                self.exit_code = 1;
                return Ok(true);
            }
        }

        self.exit_code = 0;
        Ok(true)
    }

    fn external_rm(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        for path in cmd.words.iter().skip(1).filter(|arg| !arg.starts_with('-')) {
            let target = shell_path_to_windows(&self.expand_word(path), &self.env_vars);
            if target.is_dir() {
                let _ = fs::remove_dir_all(target);
            } else {
                let _ = fs::remove_file(target);
            }
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_rmdir(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        for path in &cmd.words[1..] {
            let _ = fs::remove_dir(shell_path_to_windows(
                &self.expand_word(path),
                &self.env_vars,
            ));
        }
        self.exit_code = 0;
        Ok(true)
    }

    fn external_cat(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        if cmd.heredoc.is_some() {
            let input = self.stdin_string_for_command(cmd).unwrap_or_default();
            if let Some(redirect) = &cmd.append {
                let target = self.expand_word(&redirect.target);
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.env_vars))?;
                file.write_all(input.as_bytes())?;
                self.exit_code = 0;
                return Ok(true);
            }

            if let Some(redirect) = &cmd.redirect_out {
                let target = self.expand_word(&redirect.target);
                let mut file = self.create_redirect_output(&target, redirect.clobber)?;
                file.write_all(input.as_bytes())?;
                self.exit_code = 0;
                return Ok(true);
            }
        }

        if let Some(input) = self.stdin_string_for_command(cmd) {
            self.write_cat_output(cmd, input.as_bytes())?;
            self.exit_code = 0;
            return Ok(true);
        }

        if cmd.words.len() <= 1 {
            if let Some(input) = self.read_function_stdin('\0', None, false) {
                self.write_cat_output(cmd, input.as_bytes())?;
                self.exit_code = 0;
                return Ok(true);
            }
            return Ok(false);
        }

        let mut output = Vec::new();
        for word in cmd
            .words
            .iter()
            .skip(1)
            .filter(|word| !word.starts_with('-'))
        {
            let target = self.expand_word(word);
            match fs::read(shell_path_to_windows(&target, &self.env_vars)) {
                Ok(bytes) => output.extend(bytes),
                Err(_) => {
                    eprintln!(
                        "{}cat: {}: No such file or directory",
                        self.diagnostic_prefix(),
                        target
                    );
                    self.exit_code = 1;
                    return Ok(true);
                }
            }
        }
        self.write_cat_output(cmd, &output)?;
        self.exit_code = 0;
        Ok(true)
    }

    fn external_mkfifo(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        for path in &cmd.words[1..] {
            let target = shell_path_to_windows(&self.expand_word(path), &self.env_vars);
            let _ = File::create(target)?;
        }
        self.exit_code = 0;
        Ok(true)
    }
}
