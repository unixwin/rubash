use super::*;

impl Executor {
    pub fn last_exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn set_last_exit_code(&mut self, exit_code: i32) {
        self.set_exit_code(exit_code);
    }

    pub fn expand_prompt_string(&self, value: &str) -> String {
        self.expand_prompt_parameters(&self.decode_prompt_string(value))
    }

    pub fn expand_prompt_string_mut(&mut self, value: &str) -> String {
        let decoded = self.decode_prompt_string(value);
        self.expand_embedded_parameters_mut(&decoded)
    }

    pub fn mark_parse_error(&mut self) {
        self.parse_error_occurred = true;
    }

    pub fn take_parse_error(&mut self) -> bool {
        std::mem::take(&mut self.parse_error_occurred)
    }

    pub fn shell_state(&self) -> &crate::shell::ShellState {
        &self.shell_state
    }

    pub fn shell_state_mut(&mut self) -> &mut crate::shell::ShellState {
        &mut self.shell_state
    }

    pub(crate) fn set_exit_code(&mut self, exit_code: i32) {
        self.exit_code = exit_code;
        self.shell_state.set_exit_code(exit_code);
    }

    pub fn set_history_provider(&mut self, provider: crate::history::SharedHistoryProvider) {
        self.history_provider = Some(provider);
    }

    pub fn set_external_file_builtins_enabled(&mut self, enabled: bool) {
        self.external_file_builtins_enabled = enabled;
    }

    /// Configure the native directory that backs the shell-visible `/` root.
    ///
    /// This is a path spelling adapter, not a POSIX runtime: the configured
    /// directory itself contains the real `bin`, `usr/bin`, `etc`, and `tmp`
    /// directories used by the shell.
    pub fn set_shell_root(&mut self, root: impl AsRef<std::path::Path>) {
        let value = root.as_ref().to_string_lossy().into_owned();
        self.env_vars
            .insert("__RUBASH_SHELL_ROOT".to_string(), value.clone());
        self.env_vars.insert("WINUXSH_ROOT".to_string(), value);
        self.mark_exported("WINUXSH_ROOT");
    }

    /// Configure the native WinuxCmd dispatcher used for shell commands that
    /// are not backed by a file in the configured installation root.
    pub fn set_winuxcmd_path(&mut self, path: impl AsRef<std::path::Path>) {
        let path = path.as_ref();
        self.env_vars.insert(
            "WINUXCMD_PATH".to_string(),
            path.to_string_lossy().into_owned(),
        );
        self.mark_exported("WINUXCMD_PATH");
        let installation_root = crate::executor::path::winuxcmd_installation_root_from_path(path);
        self.env_vars.insert(
            "WINUXCMD_HOME".to_string(),
            installation_root.to_string_lossy().into_owned(),
        );
        self.mark_exported("WINUXCMD_HOME");
    }

    /// Configure an explicit external Bash-compatible shell for text-script
    /// fallbacks. Rubash does not probe sh or bash on Windows by default;
    /// host layers must opt in deliberately when they want that compatibility.
    pub fn set_compatible_shell_path(&mut self, path: impl AsRef<std::path::Path>) {
        self.env_vars.insert(
            crate::executor::path::COMPATIBLE_SHELL_PATH_ENV.to_string(),
            path.as_ref().to_string_lossy().into_owned(),
        );
    }

    pub fn clear_compatible_shell_path(&mut self) {
        self.env_vars
            .remove(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
    }

    /// Resolve a shell-visible path using the executor's current namespace.
    pub fn resolve_shell_path(&self, path: &str) -> std::path::PathBuf {
        Self::resolve_shell_path_from_env(path, &self.env_vars)
    }

    /// Resolve a shell-visible path using an executor environment snapshot.
    ///
    /// Host layers use this when they have an environment map but do not own
    /// the executor instance, such as process-plugin working-directory setup.
    pub fn resolve_shell_path_from_env(
        path: &str,
        env_vars: &std::collections::HashMap<String, String>,
    ) -> std::path::PathBuf {
        crate::executor::path::resolve_shell_path_from_env(path, env_vars)
    }

    /// Resolve one shell PATH entry into native directories for a Windows
    /// child. Logical command directories resolve to their real backing
    /// directory below the configured shell root.
    pub fn resolve_shell_path_process_entries_from_env(
        path: &str,
        env_vars: &std::collections::HashMap<String, String>,
    ) -> Vec<std::path::PathBuf> {
        crate::executor::path::shell_path_process_entries(path, env_vars)
    }

    pub fn set_host_external_command_handler<F>(&mut self, handler: F)
    where
        F: FnMut(&[String], &HashMap<String, String>) -> Option<HostExternalCommandOutput>
            + 'static,
    {
        self.host_external_command_handler = Some(HostExternalCommandHandler(Box::new(handler)));
    }

    #[cfg(windows)]
    pub fn set_elevation_handler<F>(&mut self, handler: F)
    where
        F: FnMut(ElevationRequest) -> Result<ElevationOutput, String> + 'static,
    {
        self.elevation_handler = Some(ElevationHandler(Box::new(handler)));
    }

    #[cfg(windows)]
    pub fn clear_elevation_handler(&mut self) {
        self.elevation_handler = None;
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
            let source_value = if self
                .env_vars
                .get("__RUBASH_TOP_LEVEL_NAME")
                .is_some_and(|name| name.rsplit(['/', '\\']).next() == Some("bashdb-generated"))
                && !value.starts_with('/')
            {
                std::fs::canonicalize(self.resolve_shell_path(&value))
                    .ok()
                    .map(|path| Self::debugger_source_path(&path))
                    .unwrap_or_else(|| value.clone())
            } else {
                value.clone()
            };
            // GNU evalfile.c:253 pushes the sourced file onto BASH_SOURCE
            // instead of resetting the stack: inside `source file`, frames for
            // the sourcing function and the main script stay visible
            // (dbg-support.sub traces "FUNCNAME[1]: source called from
            // ./dbg-support.tests"), so only a top-level script-name change
            // (startup, -c, bashdb launcher) rebinds BASH_SOURCE.
            if self.env_vars.contains_key("__RUBASH_IN_SOURCE") {
                store_indexed_array(
                    &mut self.env_vars,
                    "BASH_SOURCE",
                    self.bash_source_stack.clone(),
                );
                return;
            }
            self.bash_source_stack = vec![source_value.clone()];
            store_indexed_array(&mut self.env_vars, "BASH_SOURCE", vec![source_value]);
        }
    }

    pub fn export_env(&mut self, name: &str, value: &str) {
        self.set_env(name, value);
        self.mark_exported(name);
    }

    pub fn unset_env(&mut self, name: &str) {
        self.remove_env(name);
    }

    /// Enable or disable a dispatch builtin without executing shell source.
    /// This allows embedding hosts to apply policy defaults before startup files.
    pub fn set_builtin_disabled(&mut self, name: &str, disabled: bool) {
        crate::builtins::enable::set_disabled(&mut self.env_vars, name, disabled);
    }

    pub(crate) fn remove_env(&mut self, name: &str) {
        self.env_vars.remove(name);
        env::remove_var(name);
    }

    pub fn get_env(&self, name: &str) -> Option<&str> {
        self.env_vars.get(name).map(|s| s.as_str())
    }

    /// Attach the session history list used by scripts that enable history.
    pub fn set_session_history(
        &mut self,
        session: Option<std::rc::Rc<std::cell::RefCell<crate::history::SessionHistory>>>,
    ) {
        self.session_history = session;
    }

    pub(crate) fn push_bash_source(&mut self, source: String) {
        let source = if self
            .env_vars
            .get("__RUBASH_TOP_LEVEL_NAME")
            .is_some_and(|name| name.rsplit(['/', '\\']).next() == Some("bashdb-generated"))
        {
            if !source.starts_with('/') {
                std::fs::canonicalize(self.resolve_shell_path(&source))
                    .ok()
                    .map(|path| Self::debugger_source_path(&path))
                    .unwrap_or(source.clone())
            } else {
                source.clone()
            }
        } else {
            source
        };
        self.bash_source_stack.insert(0, source);
        store_indexed_array(
            &mut self.env_vars,
            "BASH_SOURCE",
            self.bash_source_stack.clone(),
        );
    }

    fn debugger_source_path(path: &std::path::Path) -> String {
        let display = path.to_string_lossy().replace('\\', "/");
        let display = display.strip_prefix("//?/").unwrap_or(&display);
        if let Some((drive, rest)) = display.split_once(":") {
            if drive.len() == 1 && !rest.is_empty() {
                return format!("/{}{}", drive.to_lowercase(), rest);
            }
        }
        display.to_string()
    }

    pub(crate) fn pop_bash_source(&mut self) {
        if !self.bash_source_stack.is_empty() {
            self.bash_source_stack.remove(0);
        }
        store_indexed_array(
            &mut self.env_vars,
            "BASH_SOURCE",
            self.bash_source_stack.clone(),
        );
    }

    /// Pushes the synthetic "source" call frame for a `source`/`.` of a
    /// file: GNU executes a sourced file with FUNCNAME[0]="source" and
    /// BASH_LINENO[0]=the source command's line (dbg-support.tests: the
    /// DEBUG trap inside dbg-support.sub reports FUNCNAME[1]="source", and
    /// dbg-support.sub's stack trace shows a "source" frame between
    /// sourced_fn and the sourcing function).
    pub(crate) fn push_source_call_frame(&mut self, call_line: String) {
        self.function_name_stack.insert(0, "source".to_string());
        self.bash_lineno_stack.insert(0, call_line);
        store_indexed_array(
            &mut self.env_vars,
            "BASH_LINENO",
            self.bash_lineno_stack.clone(),
        );
    }

    pub(crate) fn pop_source_call_frame(&mut self) {
        if self.function_name_stack.first().map(String::as_str) == Some("source") {
            self.function_name_stack.remove(0);
        }
        if !self.bash_lineno_stack.is_empty() {
            self.bash_lineno_stack.remove(0);
        }
        store_indexed_array(
            &mut self.env_vars,
            "BASH_LINENO",
            self.bash_lineno_stack.clone(),
        );
    }

    /// Returns whether a shell function is currently defined in this executor.
    pub fn has_function(&self, name: &str) -> bool {
        self.function_name_for_command_word(name).is_some()
    }

    /// Invokes a defined shell function directly, bypassing builtin and PATH lookup.
    ///
    /// Returns the function body's final shell status. If the function is not
    /// defined, returns `ExecuteError::FunctionNotFound`.
    pub fn call_function<I, S>(&mut self, name: &str, args: I) -> Result<i32, ExecuteError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.call_function_with_env(name, args, std::iter::empty::<(&str, &str)>())
    }

    /// Invokes a shell function with temporary environment variables.
    ///
    /// Each provided temporary variable is restored to its previous value after
    /// the call, while unrelated function side effects remain in the executor.
    pub fn call_function_with_env<I, S, E, K, V>(
        &mut self,
        name: &str,
        args: I,
        temporary_env: E,
    ) -> Result<i32, ExecuteError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string())
            .collect::<Vec<_>>();
        let temporary_env = temporary_env
            .into_iter()
            .map(|(name, value)| (name.as_ref().to_string(), value.as_ref().to_string()))
            .collect::<Vec<_>>();
        self.call_function_owned(name, &args, &temporary_env)
    }

    fn call_function_owned(
        &mut self,
        name: &str,
        args: &[String],
        temporary_env: &[(String, String)],
    ) -> Result<i32, ExecuteError> {
        if EXECUTION_LOCK_DEPTH.with(|depth| depth.get() > 0) {
            return self.call_function_inner(name, args, temporary_env);
        }

        let _guard = EXECUTION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original_dir = env::current_dir().ok();
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(1));
        let result = self.call_function_inner(name, args, temporary_env);
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(0));
        if let Some(original_dir) = original_dir {
            let _ = env::set_current_dir(original_dir);
        }
        result
    }

    fn call_function_inner(
        &mut self,
        name: &str,
        args: &[String],
        temporary_env: &[(String, String)],
    ) -> Result<i32, ExecuteError> {
        let Some(function_name) = self.function_name_for_command_word(name) else {
            return Err(ExecuteError::FunctionNotFound(name.to_string()));
        };
        let saved_env = temporary_env
            .iter()
            .map(|(name, _)| (name.clone(), self.env_vars.get(name).cloned()))
            .collect::<Vec<_>>();
        for (name, value) in temporary_env {
            self.set_env(name, value);
        }

        let call_cmd = CommandNode::new();
        let result = self.execute_function(&function_name, args, &call_cmd);
        let status = self.exit_code;

        for (name, value) in saved_env.into_iter().rev() {
            match value {
                Some(value) => self.set_env(&name, &value),
                None => self.remove_env(&name),
            }
        }

        result.map(|_| status)
    }

    pub fn set_shell_option(&mut self, name: &str, enabled: bool) {
        crate::builtins::set::set_shell_option(&mut self.env_vars, name, enabled);
    }

    pub fn is_shell_option(&self, name: &str) -> bool {
        crate::builtins::set::is_shell_option(name)
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

    pub fn aliases_snapshot(&self) -> HashMap<String, String> {
        self.aliases
            .iter()
            .map(|(name, alias)| (name.clone(), alias.value.clone()))
            .collect()
    }

    pub fn functions_snapshot(&self) -> Vec<String> {
        let mut names = self.functions.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn env_vars_snapshot(&self) -> HashMap<String, String> {
        self.env_vars.clone()
    }

    pub(crate) fn env_vars(&self) -> &HashMap<String, String> {
        &self.env_vars
    }

    pub(crate) fn positional_params(&self) -> Vec<String> {
        self.positional_params.clone()
    }

    pub fn set_positional_params(&mut self, positional_params: Vec<String>) {
        self.shell_state.positional.set(positional_params.clone());
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
            if command_needs_process_line_env(cmd) {
                set_process_env("__RUBASH_CURRENT_LINE", line);
            }
        }
    }

    pub(in crate::executor) fn set_current_command(&mut self, cmd: &CommandNode) {
        let command = bash_command_text(cmd);
        self.env_vars
            .insert("__RUBASH_LAST_COMMAND".to_string(), command.clone());
        if !command_references_bash_command(cmd) {
            self.env_vars.remove("__RUBASH_CURRENT_COMMAND");
            return;
        }
        self.env_vars
            .insert("__RUBASH_CURRENT_COMMAND".to_string(), command);
    }

    pub(in crate::executor) fn set_pipestatus<I>(&mut self, statuses: I)
    where
        I: IntoIterator<Item = i32>,
    {
        self.pipestatus.clear();
        self.pipestatus.extend(statuses);
        if self.pipestatus.is_empty() {
            self.pipestatus.push(0);
        }
        self.shell_state.set_pipestatus(self.pipestatus.clone());
    }

    pub(in crate::executor) fn pipestatus_values(&self) -> Vec<String> {
        self.pipestatus.iter().map(i32::to_string).collect()
    }

    pub(crate) fn diagnostic_prefix(&self) -> String {
        if let (Some(script), Some(line)) = (
            self.env_vars.get("__RUBASH_SCRIPT_NAME"),
            self.env_vars.get("__RUBASH_CURRENT_LINE"),
        ) {
            // Errors inside eval carry the "eval:" segment and report the
            // caller-relative line, like GNU evalstring diagnostics.
            if self.env_vars.contains_key("__RUBASH_EVAL_CONTEXT") {
                return format!("{script}: eval: line {line}: ");
            }
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
        if self.owns_signal_mailbox {
            crate::builtins::kill::unregister_signal_mailbox(std::process::id());
        }

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
