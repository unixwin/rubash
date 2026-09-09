use super::*;

impl Executor {
    pub(in crate::executor) fn execute_unalias(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::alias::unalias_with_io(
            &cmd.words[1..],
            &mut self.aliases,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_alias(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::alias::alias_with_io(
            &cmd.words[1..],
            &mut self.aliases,
            &mut stdout,
            &mut stderr,
        )?;
        self.record_alias_definition_lines(&cmd.words[1..], cmd.line);
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    fn record_alias_definition_lines(&mut self, args: &[String], line: Option<usize>) {
        let Some(line) = line else {
            return;
        };
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                index += 1;
                continue;
            }
            if let Some((name, _)) = arg.split_once('=') {
                if !name.is_empty() {
                    self.env_vars
                        .insert(format!("__RUBASH_ALIAS_LINE_{name}"), line.to_string());
                }
            }
            index += 1;
        }
    }

    pub(in crate::executor) fn execute_set(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::set::set_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &mut self.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        self.load_history_file_if_needed();
        Ok(status)
    }

    /// bashhist.c load_history: the first "set -o history" in a session
    /// reads HISTFILE into the list (only when no lines have been recorded
    /// yet and the file has not already been loaded).
    pub(in crate::executor) fn load_history_file_if_needed(&mut self) {
        let Some(session) = self.session_history.clone() else {
            return;
        };
        if !crate::builtins::set::shell_option_enabled(&self.env_vars, "history") {
            return;
        }
        let needs_load = {
            let shell = session.borrow_mut();
            !shell.histfile_loaded && shell.lines_this_session == 0
        };
        if !needs_load {
            return;
        }
        let Some(path) = self.get_env("HISTFILE") else {
            return;
        };
        let histsize = self
            .get_env("HISTSIZE")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let mut shell = session.borrow_mut();
        shell.histfile_loaded = true;
        let _ = shell.load_file(&path, histsize);
    }

    pub(in crate::executor) fn execute_set_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "restricted")
            && cmd.words[1..]
                .iter()
                .any(|word| word.starts_with('+') && word[1..].chars().any(|flag| flag == 'r'))
        {
            self.exit_code = self.execute_set(cmd)?;
            return Ok(());
        }
        if self.apply_simple_set_flags(&cmd.words[1..]) {
            self.exit_code = 0;
            return Ok(());
        }
        if self.apply_set_positional_operands(&cmd.words[1..]) {
            self.exit_code = 0;
            return Ok(());
        }
        if cmd.words.get(1).map(String::as_str) == Some("--") {
            // TODO(builtins/set.def/variables.c): `set --` replaces shell
            // positional parameters. Full set option parsing lives in
            // builtins::set; this branch covers upstream source tests that
            // inspect `$@`.
            self.set_positional_params(cmd.words[2..].to_vec());
            self.exit_code = 0;
            return Ok(());
        }
        self.exit_code = self.execute_set(cmd)?;
        Ok(())
    }
}
