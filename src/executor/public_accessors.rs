use super::*;

impl Executor {
    pub fn last_exit_code(&self) -> i32 {
        self.exit_code
    }

    pub(crate) fn set_exit_code(&mut self, exit_code: i32) {
        self.exit_code = exit_code;
    }

    pub fn set_env(&mut self, name: &str, value: &str) {
        let value = if name == "TMPDIR" && value.contains('\0') {
            safe_temp_dir_string()
        } else {
            value.to_string()
        };
        self.env_vars.insert(name.to_string(), value.clone());
        if is_valid_process_env(name, &value) {
            set_process_env(name, &value);
        }
        if name == "__RUBASH_SCRIPT_NAME" {
            store_indexed_array(&mut self.env_vars, "BASH_SOURCE", vec![value]);
        }
    }

    pub(crate) fn remove_env(&mut self, name: &str) {
        self.env_vars.remove(name);
        env::remove_var(name);
    }

    pub fn get_env(&self, name: &str) -> Option<&str> {
        self.env_vars.get(name).map(|s| s.as_str())
    }

    pub fn set_shell_option(&mut self, name: &str, enabled: bool) {
        crate::builtins::set::set_shell_option(&mut self.env_vars, name, enabled);
    }

    pub fn set_shopt_option(&mut self, name: &str, enabled: bool) -> bool {
        if !crate::builtins::shopt::is_supported_option(name) {
            return false;
        }
        crate::builtins::shopt::set_option(&mut self.env_vars, name, enabled);
        true
    }

    pub(in crate::executor) fn restore_shell_env(&mut self, saved_env: HashMap<String, String>) {
        let old_names: Vec<String> = self.env_vars.keys().cloned().collect();
        for name in old_names {
            if !saved_env.contains_key(&name) {
                env::remove_var(&name);
            }
        }

        for (name, value) in &saved_env {
            if is_valid_process_env(name, value) {
                set_process_env(name, value);
            } else {
                env::remove_var(name);
            }
        }

        self.env_vars = saved_env;
    }

    pub(crate) fn env_vars(&self) -> &HashMap<String, String> {
        &self.env_vars
    }

    pub(crate) fn positional_params(&self) -> Vec<String> {
        self.positional_params.clone()
    }

    pub fn set_positional_params(&mut self, positional_params: Vec<String>) {
        self.positional_params = positional_params;
    }

    pub fn inherit_process_stdin(&mut self) {
        self.env_vars
            .insert(INHERIT_PROCESS_STDIN.to_string(), "1".to_string());
    }

    pub(in crate::executor) fn set_current_line(&mut self, cmd: &CommandNode) {
        if let Some(line) = cmd.line {
            let line = line.to_string();
            self.env_vars
                .insert("__RUBASH_CURRENT_LINE".to_string(), line.clone());
            set_process_env("__RUBASH_CURRENT_LINE", line);
        }
    }

    pub(in crate::executor) fn set_current_command(&mut self, cmd: &CommandNode) {
        let command = bash_command_text(cmd);
        self.env_vars
            .insert("__RUBASH_CURRENT_COMMAND".to_string(), command.clone());
        set_process_env("__RUBASH_CURRENT_COMMAND", command);
    }

    pub(in crate::executor) fn set_pipestatus<I>(&mut self, statuses: I)
    where
        I: IntoIterator<Item = i32>,
    {
        let values = statuses
            .into_iter()
            .map(|status| status.to_string())
            .collect();
        store_indexed_array(&mut self.env_vars, "PIPESTATUS", values);
    }

    pub(crate) fn diagnostic_prefix(&self) -> String {
        if let (Some(script), Some(line)) = (
            self.env_vars.get("__RUBASH_SCRIPT_NAME"),
            self.env_vars.get("__RUBASH_CURRENT_LINE"),
        ) {
            return format!("{script}: line {line}: ");
        }

        "rubash: ".to_string()
    }

    pub(in crate::executor) fn diagnostic_prefix_for_line(&self, line: usize) -> String {
        if let Some(script) = self.env_vars.get("__RUBASH_SCRIPT_NAME") {
            return format!("{script}: line {line}: ");
        }

        "rubash: ".to_string()
    }

    pub(in crate::executor) fn report_unterminated_heredoc(&self, cmd: &CommandNode) {
        let start_line = cmd.line.unwrap_or(1);
        let body_lines = cmd
            .heredoc
            .as_deref()
            .map(unterminated_heredoc_body_line_count)
            .unwrap_or(0);
        let warning_line = start_line + body_lines;
        let delimiter = cmd.heredoc_delimiter.as_deref().unwrap_or("");
        eprintln!(
            "{}warning: here-document at line {start_line} delimited by end-of-file (wanted `{delimiter}')",
            self.diagnostic_prefix_for_line(warning_line)
        );
    }

    pub(in crate::executor) fn report_unterminated_subshell_heredoc(&self, cmd: &CommandNode) {
        self.report_unterminated_heredoc(cmd);
        let start_line = cmd.line.unwrap_or(1);
        let body_lines = cmd
            .heredoc
            .as_deref()
            .map(unterminated_heredoc_body_line_count)
            .unwrap_or(0);
        let warning_line = start_line + body_lines;
        let syntax_line = warning_line + 1;
        eprintln!(
            "{}syntax error: unexpected end of file from `(' command on line {start_line}",
            self.diagnostic_prefix_for_line(syntax_line)
        );
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        let current_names: Vec<String> = env::vars().map(|(name, _)| name).collect();
        for name in current_names {
            if !self.process_env_snapshot.contains_key(&name) {
                env::remove_var(name);
            }
        }

        for (name, value) in &self.process_env_snapshot {
            if is_valid_process_env(name, value) {
                set_process_env(name, value);
            } else {
                env::remove_var(name);
            }
        }
    }
}
