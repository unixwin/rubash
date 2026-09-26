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

    /// Emit the stored `__RUBASH_PARSE_ERROR__`/`__RUBASH_PARSE_SOURCE__`
    /// diagnostic for a command node — the same text the parse-error arm in
    /// execute_command prints. Shared so a command-substitution body that
    /// parsed to an error node can report GNU's `syntax error near ...`
    /// lines without executing the broken ast.
    pub(in crate::executor) fn report_command_parse_error(&self, cmd: &crate::parser::CommandNode) {
        let message = cmd
            .get_assignment("__RUBASH_PARSE_ERROR__")
            .map(String::as_str)
            .unwrap_or("unexpected token");
        if message.starts_with("syntax error:") || message.starts_with("arithmetic syntax error:") {
            eprintln!("{}{}", self.parser_diagnostic_prefix(), message);
            if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                eprintln!(
                    "{}syntax error: `{}'",
                    self.parser_diagnostic_prefix(),
                    super::command_execute::parse_error_source_display(source)
                );
            }
        } else {
            let message = super::command_execute::bash_style_unexpected_token_message(message);
            eprintln!(
                "{}syntax error near {message}",
                self.parser_diagnostic_prefix(),
            );
            if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                eprintln!(
                    "{}`{}'",
                    self.parser_diagnostic_prefix(),
                    super::command_execute::parse_error_source_display(source)
                );
            }
        }
    }

    pub fn take_parse_error(&mut self) -> bool {
        std::mem::take(&mut self.parse_error_occurred)
    }

    /// GNU execute_cmd.c:652-656: `!` gives the command CMD_IGNORE_RETURN
    /// under errexit — the grouped drivers' post-group `status != 0 &&
    /// errexit` check reads this to honor the exemption.
    pub fn last_command_inverted(&self) -> bool {
        self.last_command_inverted.get()
    }

    /// GNU jump_to_top_level(EXITPROG/ERREXIT/FORCE_EOF): the group's
    /// execute_ast ended by unwinding rather than by an ordinary command
    /// status — the grouped drivers must stop reading. Take-style: each
    /// check clears the flag.
    pub fn take_exit_jump_pending(&mut self) -> bool {
        self.exit_jump_pending.replace(false)
    }

    pub fn shell_state(&self) -> &crate::shell::ShellState {
        &self.shell_state
    }

    pub fn shell_state_mut(&mut self) -> &mut crate::shell::ShellState {
        &mut self.shell_state
    }

    pub(crate) fn set_exit_code(&mut self, exit_code: i32) {
        self.exit_code = exit_code;
    }

    pub fn set_history_provider(&mut self, provider: crate::history::SharedHistoryProvider) {
        self.history_provider = Some(provider);
    }

    /// Host completion hook. Given the in-progress command `line` and the
    /// `cursor` position, return completion candidates for the word under the
    /// cursor, honoring the compspec registered for the command (if any) and
    /// falling back to command completion (first word) or file completion.
    ///
    /// Candidate generation reuses the same GNU programmable-completion engine
    /// as the `compgen` builtin (see
    /// `crate::builtins::complete::complete_line_candidates`). The host (niubash)
    /// wires this into its reedline completer so the completion *engine* lives in
    /// rubash while the *UI* stays in the host — the same split as
    /// `HistoryProvider` keeps storage on the host.
    ///
    /// Dynamic compspec actions `-C` (external command) and `-F` (shell function
    /// filling `COMPREPLY`) are resolved by the executor and merged here in a
    /// follow-up; the static-action engine already covers file/path/command/
    /// wordlist completion.
    pub fn complete_line(&self, line: &str, cursor: usize) -> Vec<String> {
        let function_names: Vec<String> = self.shell_state.functions.keys().cloned().collect();
        let job_names: Vec<String> = self
            .shell_state
            .job_table
            .jobs
            .values()
            .filter(|job| job.background)
            .map(|job| job.command.clone())
            .collect();
        crate::builtins::complete::complete_line_candidates(
            line,
            cursor,
            &self.shell_state.completion_specs,
            &self.shell_state.env_vars,
            &self.shell_state.aliases,
            &function_names,
            &job_names,
        )
    }

    /// The canonical list of shell builtin command names (mirrors
    /// `BUILTIN_NAMES`). Host completion uses this instead of hardcoding the
    /// list, so the two never drift apart when builtins are added or removed.
    pub fn builtin_command_names() -> &'static [&'static str] {
        crate::executor::builtin_names::builtin_names()
    }

    /// Whether a compspec is registered for `command` (via the `complete`
    /// builtin). Hosts use this as a cheap gate before calling
    /// [`Executor::complete_line`], so they only pay for GNU-engine candidate
    /// generation when a compspec actually applies.
    pub fn has_compspec(&self, command: &str) -> bool {
        self.shell_state.completion_specs.get(command).is_some()
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
        self.shell_state
            .env_vars
            .insert("__RUBASH_SHELL_ROOT".to_string(), value.clone());
        // deprecated: niu bridge, remove after niu stops reading
        self.shell_state
            .env_vars
            .insert("WINUXSH_ROOT".to_string(), value);
        self.mark_exported("WINUXSH_ROOT");
    }

    /// Configure the native WinuxCmd dispatcher used for shell commands that
    /// are not backed by a file in the configured installation root.
    pub fn set_winuxcmd_path(&mut self, path: impl AsRef<std::path::Path>) {
        let path = path.as_ref();
        self.shell_state.env_vars.insert(
            "WINUXCMD_PATH".to_string(),
            path.to_string_lossy().into_owned(),
        );
        self.mark_exported("WINUXCMD_PATH");
        let installation_root = crate::executor::path::winuxcmd_installation_root_from_path(path);
        self.shell_state.env_vars.insert(
            "WINUXCMD_HOME".to_string(),
            installation_root.to_string_lossy().into_owned(),
        );
        self.mark_exported("WINUXCMD_HOME");
    }

    /// Configure an explicit external Bash-compatible shell for text-script
    /// fallbacks. Rubash does not probe sh or bash on Windows by default;
    /// host layers must opt in deliberately when they want that compatibility.
    pub fn set_compatible_shell_path(&mut self, path: impl AsRef<std::path::Path>) {
        self.shell_state.env_vars.insert(
            crate::executor::path::COMPATIBLE_SHELL_PATH_ENV.to_string(),
            path.as_ref().to_string_lossy().into_owned(),
        );
    }

    pub fn clear_compatible_shell_path(&mut self) {
        self.shell_state
            .env_vars
            .remove(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
    }

    /// Resolve a shell-visible path using the executor's current namespace.
    pub fn resolve_shell_path(&self, path: &str) -> std::path::PathBuf {
        Self::resolve_shell_path_from_env(path, &self.shell_state.env_vars)
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
        self.shell_state
            .env_vars
            .insert(name.to_string(), value.clone());
        if is_valid_process_env(name, &value) {
            set_process_env(name, &value);
        }
        // Keep the shell-variable view in sync with the environment mirror.
        // `shell_state.variables` is seeded from `env_vars` at `Executor::new`
        // (init.rs `from_environment`), so every exported env var also exists
        // as a scalar shell variable. `set_env` only updated `env_vars`, which
        // made `$VAR` (shell_variable_value reads shell_state first) return a
        // stale value while tilde/`home_value` (reads env_vars) returned the
        // new one — the `$HOME` vs `~` inconsistency (niubash issue #90) and the
        // same class of bug that PWD/OPTIND/export already paper over with
        // manual syncs. Update the scalar in place so attributes (readonly,
        // integer, exported, case-mod) are preserved; arrays/assoc and absent
        // variables are left untouched, matching the existing sync sites.
        if let Some(variable) = self.shell_state.variables.get_mut(name) {
            if let crate::shell::ShellValue::Scalar(current) = &mut variable.value {
                *current = value.clone();
            }
        }
        if name == "__RUBASH_SCRIPT_NAME" {
            let source_value = if self
                .shell_state
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
            if self.shell_state.env_vars.contains_key("__RUBASH_IN_SOURCE") {
                store_indexed_array(
                    &mut self.shell_state.env_vars,
                    "BASH_SOURCE",
                    self.shell_state.bash_source_stack.clone(),
                );
                return;
            }
            self.shell_state.bash_source_stack = vec![source_value.clone()];
            store_indexed_array(
                &mut self.shell_state.env_vars,
                "BASH_SOURCE",
                vec![source_value],
            );
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
        crate::builtins::enable::set_disabled(&mut self.shell_state.env_vars, name, disabled);
    }

    pub(crate) fn remove_env(&mut self, name: &str) {
        self.shell_state.env_vars.remove(name);
        env::remove_var(name);
    }

    pub fn get_env(&self, name: &str) -> Option<&str> {
        self.shell_state.env_vars.get(name).map(|s| s.as_str())
    }

    /// Attach the session history list used by scripts that enable history.
    pub fn set_session_history(
        &mut self,
        session: Option<std::rc::Rc<std::cell::RefCell<crate::history::SessionHistory>>>,
    ) {
        self.shell_state.session_history = session;
    }

    /// Returns a clone of the session history handle, if any.
    pub fn get_session_history(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<crate::history::SessionHistory>>> {
        self.shell_state.session_history.clone()
    }

    /// Run `body` with user trap delivery (DEBUG/RETURN/ERR) suspended.
    ///
    /// GNU has no notion of host housekeeping commands: when an embedding
    /// host runs internal commands through `execute_ast` — startup or
    /// shutdown hook probes, rc cleanup such as `unset HISTFILE`, plugin
    /// existence checks — those commands must not fire the script's
    /// traps. Without this boundary, commands a host issues after the
    /// script reached EOF emit extra DEBUG/RETURN trap lines that GNU
    /// Bash never produces (engine sinking list S2: the one-line
    /// dbg-support/dbg-support2 product-layer diffs).
    pub fn with_traps_suspended<R>(&mut self, body: impl FnOnce(&mut Self) -> R) -> R {
        self.host_internal_depth
            .set(self.host_internal_depth.get() + 1);
        let result = body(self);
        self.host_internal_depth
            .set(self.host_internal_depth.get().saturating_sub(1));
        result
    }

    pub(crate) fn push_bash_source(&mut self, source: String) {
        let source = if self
            .shell_state
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
        self.shell_state.bash_source_stack.insert(0, source);
        store_indexed_array(
            &mut self.shell_state.env_vars,
            "BASH_SOURCE",
            self.shell_state.bash_source_stack.clone(),
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
        if !self.shell_state.bash_source_stack.is_empty() {
            self.shell_state.bash_source_stack.remove(0);
        }
        store_indexed_array(
            &mut self.shell_state.env_vars,
            "BASH_SOURCE",
            self.shell_state.bash_source_stack.clone(),
        );
    }

    /// Pushes the synthetic "source" call frame for a `source`/`.` of a
    /// file: GNU executes a sourced file with FUNCNAME[0]="source" and
    /// BASH_LINENO[0]=the source command's line (dbg-support.tests: the
    /// DEBUG trap inside dbg-support.sub reports FUNCNAME[1]="source", and
    /// dbg-support.sub's stack trace shows a "source" frame between
    /// sourced_fn and the sourcing function).
    pub(crate) fn push_source_call_frame(&mut self, call_line: String) {
        self.shell_state
            .function_name_stack
            .insert(0, "source".to_string());
        self.shell_state.bash_lineno_stack.insert(0, call_line);
        store_indexed_array(
            &mut self.shell_state.env_vars,
            "BASH_LINENO",
            self.shell_state.bash_lineno_stack.clone(),
        );
    }

    pub(crate) fn pop_source_call_frame(&mut self) {
        if self
            .shell_state
            .function_name_stack
            .first()
            .map(String::as_str)
            == Some("source")
        {
            self.shell_state.function_name_stack.remove(0);
        }
        if !self.shell_state.bash_lineno_stack.is_empty() {
            self.shell_state.bash_lineno_stack.remove(0);
        }
        store_indexed_array(
            &mut self.shell_state.env_vars,
            "BASH_LINENO",
            self.shell_state.bash_lineno_stack.clone(),
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
            .map(|(name, _)| (name.clone(), self.shell_state.env_vars.get(name).cloned()))
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
        crate::builtins::set::set_shell_option(&mut self.shell_state.env_vars, name, enabled);
    }

    /// GNU shell.c:1587-1601 (open_shell_script): a script filename that is
    /// not found as given and carries no path separator is searched in
    /// $PATH (find_path_file, findcmd.c:258) before the shell gives up.
    pub fn find_script_on_path(&self, name: &str) -> Option<std::path::PathBuf> {
        crate::executor::path::find_user_command(name, &self.shell_state.env_vars)
    }

    pub fn is_shell_option(&self, name: &str) -> bool {
        crate::builtins::set::is_shell_option(name)
    }

    pub fn set_shopt_option(&mut self, name: &str, enabled: bool) -> bool {
        if !crate::builtins::shopt::is_supported_option(name) {
            return false;
        }
        crate::builtins::shopt::set_option(&mut self.shell_state.env_vars, name, enabled);
        true
    }

    pub(in crate::executor) fn restore_shell_env(&mut self, saved_env: HashMap<String, String>) {
        let old_names: Vec<String> = self.shell_state.env_vars.keys().cloned().collect();
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

        self.shell_state.env_vars = saved_env;
    }

    /// GNU execute_cmd.c:1576 execute_in_subshell: a `( )` subshell's parent
    /// state is the pre-entry ShellState clone — restore it wholesale so
    /// every semantic field (aliases, functions, scopes, positional params,
    /// env-carried traps, job bookkeeping) returns to the parent value.
    /// Process env and cwd are shared in-place resources, so they resync
    /// separately: restore_shell_env must run before the swap because it
    /// diffs the child's env_vars against the saved map.
    pub(in crate::executor) fn restore_flat_subshell(
        &mut self,
        saved_state: crate::shell::ShellState,
        saved_cwd: Option<PathBuf>,
    ) {
        self.restore_shell_env(saved_state.env_vars.clone());
        self.shell_state = saved_state;
        if let Some(dir) = saved_cwd {
            let _ = env::set_current_dir(dir);
        }
    }

    pub fn aliases_snapshot(&self) -> HashMap<String, String> {
        self.shell_state
            .aliases
            .iter()
            .map(|(name, alias)| (name.clone(), alias.value.clone()))
            .collect()
    }

    pub fn functions_snapshot(&self) -> Vec<String> {
        let mut names = self
            .shell_state
            .functions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn env_vars_snapshot(&self) -> HashMap<String, String> {
        self.shell_state.env_vars.clone()
    }

    pub fn env_vars(&self) -> &HashMap<String, String> {
        &self.shell_state.env_vars
    }

    pub(crate) fn positional_params(&self) -> Vec<String> {
        self.shell_state.positional_params.clone()
    }

    pub fn set_positional_params(&mut self, positional_params: Vec<String>) {
        self.shell_state.positional_params = positional_params;
    }

    pub fn inherit_process_stdin(&mut self) {
        self.shell_state
            .env_vars
            .insert(INHERIT_PROCESS_STDIN.to_string(), "1".to_string());
    }

    /// Fetch the next command-source line when fd 0 has been retargeted.
    ///
    /// GNU input.c (bash_input) reads commands through fd 0, so a permanent
    /// `exec 0<file` redirection (redir.c do_redirections under
    /// REDIR_PERSIST via builtins/exec.def) moves the script reader onto the
    /// new input — redir1.sub:4-6 relies on the next commands coming from
    /// the redirected file. Returns `None` while fd 0 still designates the
    /// process's real stdin (caller reads it byte-wise), `Some(n)` for a
    /// buffered fd-0 line, and `Some(0)` at EOF or on a closed/unreadable
    /// fd 0.
    pub fn script_fd0_line(&mut self, output: &mut String) -> Option<usize> {
        match self.fd_table.read_endpoint(0) {
            Some(FdReadEndpoint::InheritedProcessStdin) => None,
            Some(FdReadEndpoint::Text(_))
            | Some(FdReadEndpoint::ProcessSubstitution(_))
            | Some(FdReadEndpoint::File(_))
            | Some(FdReadEndpoint::CoprocStdout { .. }) => {
                let line = self
                    .fd_table
                    .take_buffered_input_line(0)
                    .unwrap_or_default();
                if line.is_empty() {
                    return Some(0);
                }
                output.push_str(&bytes_to_shell_text(&line));
                Some(line.len())
            }
            // Closed or otherwise unreadable fd 0 means end of input for the
            // command reader (`exec 0<&-` retires the stream).
            _ => Some(0),
        }
    }

    pub(in crate::executor) fn set_current_line(&mut self, cmd: &CommandNode) {
        // GNU execute_cmd.c: only some command kinds stamp `line_number`
        // from their own `->line` (cm_simple :936, cm_subshell :696,
        // cm_for :3001, cm_select :3513, cm_case :3653, cm_arith :3904,
        // cm_cond :4138, cm_arith_for :3236). while/until/if/group and the
        // connection/pipeline/&/! wrappers run under the ambient line —
        // the enclosing body's frozen line or, at top level, this
        // command's own parse-end line (see CommandNode::end_line).
        let line = if command_sets_own_line(cmd) {
            cmd.line
        } else {
            self.ambient_line.get().or(cmd.end_line).or(cmd.line)
        };
        if let Some(line) = line {
            let line = line.to_string();
            self.shell_state
                .env_vars
                .insert("__RUBASH_CURRENT_LINE".to_string(), line.clone());
            if command_needs_process_line_env(cmd) {
                set_process_env("__RUBASH_CURRENT_LINE", line);
            }
        }
    }

    /// Runs `f` with GNU's ambient `line_number` pinned to `line` for its
    /// duration — the enclosing line-setting command's `->line` in force
    /// while its body executes (execute_cmd.c SET_LINE_NUMBER sites).
    /// Non-line-setting commands inside the body report diagnostics
    /// against this line rather than their own keyword line.
    pub(in crate::executor) fn with_ambient_line<T>(
        &mut self,
        line: Option<usize>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let saved = self.ambient_line.replace(line);
        let result = f(self);
        self.ambient_line.set(saved);
        result
    }

    pub(in crate::executor) fn set_current_command(&mut self, cmd: &CommandNode) {
        // GNU the_printed_command_except_trap is only refreshed when no trap
        // action is running — every print site guards with
        // `signal_in_progress (DEBUG_TRAP) == 0 && running_trap == 0`
        // (execute_cmd.c:4499-4501 and the compound heads), so commands
        // executed inside a DEBUG/ERR/RETURN/signal trap action never replace
        // the trapped command's text that BASH_COMMAND exposes
        // (variables.c:1558 get_bash_command).
        if self.debug_trap_running
            || self.error_trap_running
            || self.return_trap_running
            || self.signal_trap_running
        {
            return;
        }
        // GNU the_printed_command is produced by print_command
        // (print_cmd.c), which covers every command form — a `[[ ]]`
        // node must record `[[ -n $unset ]]`, not a word-list dump
        // (`'[[' '-n' '$unset' ']]'`); pipelines record the full
        // `a | b` text. bash_command_source_text is that renderer;
        // bash_command_text only knows simple commands.
        let command = bash_command_source_text(cmd);
        self.shell_state
            .env_vars
            .insert("__RUBASH_LAST_COMMAND".to_string(), command.clone());
        // GNU the_printed_command_except_trap is refreshed unconditionally
        // for every executed command (execute_cmd.c compound heads), and
        // variables.c:1558 get_bash_command reads it directly — there is no
        // "only when the command text names BASH_COMMAND" gate. Indirect
        // references like ${!name} resolve through the same dynamic var, so
        // the command text is always recorded.
        self.shell_state
            .env_vars
            .insert("__RUBASH_CURRENT_COMMAND".to_string(), command);
    }

    pub(in crate::executor) fn set_pipestatus<I>(&mut self, statuses: I)
    where
        I: IntoIterator<Item = i32>,
    {
        self.shell_state.pipestatus.clear();
        self.shell_state.pipestatus.extend(statuses);
        if self.shell_state.pipestatus.is_empty() {
            self.shell_state.pipestatus.push(0);
        }
    }

    pub(in crate::executor) fn pipestatus_values(&self) -> Vec<String> {
        self.shell_state
            .pipestatus
            .iter()
            .map(i32::to_string)
            .collect()
    }

    pub fn diagnostic_prefix(&self) -> String {
        // GNU error.c:75-86 (report_prolog): runtime errors (command not
        // found, file not found, etc.) use only get_name_for_error() —
        // BASH_SOURCE[0] or dollar_vars[0] — as the prolog name, with no
        // input-stream segment. The "-c:" segment is exclusive to
        // parser_error (error.c:300-316) which appends yy_input_name().
        // Interactive mode (shell reading input from a terminal) omits the
        // line segment entirely (error.c:88-120 get_name_for_error returns
        // only base_pathname(shell_name), no line number).
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            // Interactive mode: report only the shell name, no line segment.
            // GNU error.c:88-120 (get_name_for_error) for interactive shells
            // returns base_pathname(shell_name) with no line number.
            if let Some(shell_name) = self.shell_state.env_vars.get("__RUBASH_SHELL_NAME") {
                return format!("{shell_name}: ");
            }
            return "bash: ".to_string();
        }

        // Script/-c mode: line segment present
        let line = self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE");
        let script = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME");
        match (script, line) {
            (Some(script), Some(line)) => {
                // Script mode: "script: line N:"
                format!("{script}: line {line}: ")
            }
            (None, Some(line)) => {
                // -c mode: "bash: line N:"
                format!("bash: line {line}: ")
            }
            _ => {
                // GNU error.c:88-120 (get_name_for_error): without a script/$0
                // context the prolog falls back to base_pathname(shell_name), i.e.
                // the canonical shell name. Rubash reports as "bash"; the upstream
                // suites normalize the baseline's invoked path to the same name.
                "bash: ".to_string()
            }
        }
    }

    /// Parser/syntax-error diagnostic prefix. GNU parser_error (error.c:300-316)
    /// appends yy_input_name() — the input stream name — when it differs from
    /// get_name_for_error(). For `bash -c` the stream name is "-c", producing
    /// "bash: -c: line N:". For script files the stream name equals the script
    /// name, so no extra segment appears.
    pub(crate) fn parser_diagnostic_prefix(&self) -> String {
        let is_c = self.shell_state.env_vars.contains_key("__RUBASH_IS_C");
        if let (Some(script), Some(line)) = (
            self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME"),
            self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE"),
        ) {
            if self
                .shell_state
                .env_vars
                .contains_key("__RUBASH_EVAL_CONTEXT")
            {
                return format!("{script}: eval: line {line}: ");
            }
            if is_c {
                return format!("{script}: -c: line {line}: ");
            }
            return format!("{script}: line {line}: ");
        }
        if is_c {
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                return format!("{script}: -c: line 1: ");
            }
            return "bash: -c: line 1: ".to_string();
        }

        "bash: ".to_string()
    }

    /// GNU parser_error (error.c:300-316) with yy_input_name() ==
    /// "command substitution" (parse_and_execute's `whom`, subst.c:7413):
    /// `script: command substitution: line N: `. The comsub parse inherits
    /// the outer line_number (evalstring.c push_stream(0)), so `line` is the
    /// script line where the substitution's input ran out.
    pub(in crate::executor) fn comsub_eof_diagnostic(&self, line: usize) -> String {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            // parser_error's interactive branch prints only the shell name.
            let name = self
                .shell_state
                .env_vars
                .get("__RUBASH_SHELL_NAME")
                .cloned()
                .unwrap_or_else(|| "bash".to_string());
            return format!("{name}: ");
        }
        let name = self
            .shell_state
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .cloned()
            .unwrap_or_else(|| "bash".to_string());
        format!("{name}: command substitution: line {line}: ")
    }

    pub fn diagnostic_prefix_for_line(&self, line: usize) -> String {
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            return format!("{script}: line {line}: ");
        }

        "bash: ".to_string()
    }

    /// Parser/syntax-error diagnostic prefix for a specific line. Like
    /// parser_diagnostic_prefix but for a caller-supplied line number.
    pub fn parser_diagnostic_prefix_for_line(&self, line: usize) -> String {
        let is_c = self.shell_state.env_vars.contains_key("__RUBASH_IS_C");
        // error.c yy_input_name: inside `eval` the input stream name is
        // "eval" and overrides the -c tag (`bash: eval: line N:`).
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_EVAL_CONTEXT")
        {
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                return format!("{script}: eval: line {line}: ");
            }
            return format!("bash: eval: line {line}: ");
        }
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            if is_c {
                return format!("{script}: -c: line {line}: ");
            }
            return format!("{script}: line {line}: ");
        }
        if is_c {
            return format!("bash: -c: line {line}: ");
        }

        "bash: ".to_string()
    }

    pub(in crate::executor) fn report_unterminated_heredoc(&self, cmd: &CommandNode) {
        // GNU parse.y:3130/make_cmd.c:627: `lineno` is the parser's
        // line_number when gather_here_documents ran make_here_document —
        // the physical line where the command's logical line ended (advanced
        // by any earlier heredoc bodies of the same command), not the `<<`
        // line. internal_warning's prefix uses line_number after the body
        // scan, i.e. gather_line + body lines consumed.
        let gather_line = cmd.heredoc_gather_line.or(cmd.line).unwrap_or(1);
        let body_lines = cmd
            .heredoc
            .as_deref()
            .map(unterminated_heredoc_body_line_count)
            .unwrap_or(0);
        let warning_line = gather_line + body_lines;
        let delimiter = cmd.heredoc_delimiter.as_deref().unwrap_or("");
        eprintln!(
            "{}warning: here-document at line {gather_line} delimited by end-of-file (wanted `{delimiter}')",
            self.diagnostic_prefix_for_line(warning_line)
        );
    }

    /// Report a here-document whose delimiter was found but not on a line by
    /// itself (e.g. `EOF)` inside a command substitution).  GNU make_cmd.c:606-627
    /// sets `full_line = 0` via the PST_EOFTOKEN backwards-compat path and
    /// issues the same warning as a truly unterminated heredoc.  The prefix
    /// line is the delimiter line (start_line + body_lines + 1), and "at line
    /// N" is the heredoc start line.
    pub(in crate::executor) fn report_warned_heredoc(&self, cmd: &CommandNode) {
        let gather_line = cmd.heredoc_gather_line.or(cmd.line).unwrap_or(1);
        let body_lines = cmd
            .heredoc
            .as_deref()
            .map(unterminated_heredoc_body_line_count)
            .unwrap_or(0);
        let delimiter_line = gather_line + body_lines + 1;
        let delimiter = cmd.heredoc_delimiter.as_deref().unwrap_or("");
        eprintln!(
            "{}warning: here-document at line {gather_line} delimited by end-of-file (wanted `{delimiter}')",
            self.diagnostic_prefix_for_line(delimiter_line)
        );
    }

    /// Report an unclosed `$(` that ran to end of input. GNU make_cmd.c
    /// gather_here_documents scans heredoc headers inside the comsub text
    /// first (each unterminated body warns at the last input line, with
    /// "at line N" = the header's script line); parse.y:6883-6896 then issues
    /// `unexpected EOF while looking for matching ')'` at the parser's
    /// line_number — the line after the last physical input line.
    /// `raw` is the token text starting at `start_line` that contains the
    /// unclosed `$(` and everything it swallowed.
    pub(in crate::executor) fn report_unclosed_comsub_eof(&self, raw: &str, start_line: usize) {
        // Locate the last `$(` whose tail never closes — its text is the
        // comsub body GNU's nested parse consumed.
        let bytes = raw.as_bytes();
        let mut innermost: Option<(usize, &str)> = None;
        for (idx, window) in bytes.windows(2).enumerate() {
            if window == b"$("
                && (idx == 0 || bytes[idx - 1] != b'\\')
                && crate::lexer::has_unclosed_command_substitution(&raw[idx..])
            {
                innermost = Some((idx, &raw[idx + 2..]));
            }
        }
        let last_line = start_line + raw.matches('\n').count();
        if let Some((idx, content)) = innermost {
            let content_line = start_line + raw[..idx].matches('\n').count();
            // Replay the gather: track pending headers and consumed
            // delimiter lines; whatever is still open at EOF warns.
            let mut pending: Vec<(String, usize)> = Vec::new();
            let mut line_no = content_line;
            for line in content.split('\n') {
                if let Some((delim, _)) = pending.first().cloned() {
                    if line == delim {
                        pending.remove(0);
                    }
                } else {
                    let (headers, _) = crate::lexer::scan_line_for_comsub_heredoc_headers(line);
                    for header in headers {
                        pending.push((header.delimiter, line_no));
                    }
                }
                line_no += 1;
            }
            for (delim, at_line) in pending {
                eprintln!(
                    "{}warning: here-document at line {at_line} delimited by end-of-file (wanted `{delim}')",
                    self.diagnostic_prefix_for_line(last_line)
                );
            }
        }
        eprintln!(
            "{}unexpected EOF while looking for matching `)'",
            self.parser_diagnostic_prefix_for_line(last_line + 1)
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
        let warning_line = cmd.heredoc_gather_line.or(cmd.line).unwrap_or(1) + body_lines;
        let syntax_line = warning_line + 1;
        eprintln!(
            "{}syntax error: unexpected end of file from `(' command on line {start_line}",
            self.parser_diagnostic_prefix_for_line(syntax_line)
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
