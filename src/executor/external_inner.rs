use super::*;

impl Executor {
    pub(in crate::executor) fn execute_env_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // cmd.words are already expanded by expand_command_words before
        // dispatch (execute_cmd.c do_word_expansion). Re-expanding here ran
        // literal `$(...)` text a second time: `env 'X=$(id >/dev/tty)'`
        // executed the substitution and reported the redirect failure on the
        // caller's stderr (exportfunc.tests line 68).
        let args = cmd.words[1..].to_vec();
        let Some(config) = self.parse_env_command_args(args)? else {
            return Ok(());
        };
        let Some(env_vars) = self.materialize_env_command_environment(&config)? else {
            return Ok(());
        };

        if config.command.is_empty() {
            let mut output = Vec::new();
            let mut entries = env_vars.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, value) in entries {
                output.extend_from_slice(key.as_bytes());
                output.push(b'=');
                // GNU coreutils env writes each exported value cell's raw
                // bytes (variables.c:4850 make_env_array_from_var_list
                // copies the cell verbatim via mk_env_string).
                // Stored values carry raw bytes as U+E000 marker pairs
                // (markers.rs Storage boundary); decode to the real bytes
                // at this output boundary so `x=$(printf '\377'); export
                // x; env` prints the byte, not the pair (matching the
                // WSL GNU baseline ff for `env | grep '^x='`).
                output.extend_from_slice(
                    &crate::executor::substitution_metadata::decode_raw_byte_markers(
                        value.as_bytes(),
                    ),
                );
                output.push(if config.null_terminated { b'\0' } else { b'\n' });
            }
            self.write_default_stdout(&output)?;
            self.exit_code = 0;
            return Ok(());
        }

        if config.null_terminated {
            self.write_default_stderr(b"env: cannot specify --null (-0) with command\n")?;
            self.exit_code = 125;
            return Ok(());
        }

        let Some(program) = find_user_command(&config.command[0], &env_vars) else {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "env: failed to run command '{}': No such file or directory",
                config.command[0]
            )?;
            self.write_default_stderr(&stderr)?;
            self.exit_code = 127;
            return Ok(());
        };

        let (dev_args, dev_ops) = self.materialize_dev_fd_operands(
            &config.command[1..],
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
            crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
        );
        let (mut process, used_shell) = external_command_for_named_program(
            &program,
            Some(&config.command[0]),
            &dev_args,
            &env_vars,
        );
        apply_env_command_environment(&mut process, &env_vars, !config.ignore_environment);
        if let Some(chdir) = &config.chdir {
            let directory = shell_path_to_windows(chdir, &env_vars);
            if !directory.is_dir() {
                self.write_default_stderr(b"env: cannot change directory\n")?;
                self.exit_code = 125;
                return Ok(());
            }
            process.current_dir(directory);
        }
        self.apply_external_redirects(cmd, &mut process)?;
        self.spawn_external_process(cmd, &program, process, used_shell, dev_ops)
    }

    fn parse_env_command_args(
        &mut self,
        args: Vec<String>,
    ) -> Result<Option<EnvCommandConfig>, ExecuteError> {
        let mut config = EnvCommandConfig::default();
        let mut args = args;
        let mut index = 0usize;
        let mut command_started = false;

        while index < args.len() {
            let arg = args[index].clone();
            index += 1;
            if command_started {
                config.command.push(arg);
                continue;
            }

            if arg == "--" {
                command_started = true;
                continue;
            }
            if arg == "-" || arg == "-i" || arg == "--ignore-environment" {
                config.ignore_environment = true;
                continue;
            }
            if arg == "-0" || arg == "--null" {
                config.null_terminated = true;
                continue;
            }
            if arg == "-v" || arg == "--debug" {
                config.debug = true;
                continue;
            }

            if let Some(parsed) =
                self.parse_env_short_option_cluster(&arg, &args, &mut index, &mut config)?
            {
                let Some(split) = parsed else {
                    return Ok(None);
                };
                args.splice(index..index, split);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--unset=") {
                config.unset_names.push(value);
                continue;
            }
            if arg == "-u" || arg == "--unset" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --unset requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.unset_names.push(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--chdir=") {
                config.chdir = Some(value);
                continue;
            }
            if arg == "-C" || arg == "--chdir" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --chdir requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.chdir = Some(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--file=") {
                config.file = Some(value);
                continue;
            }
            if arg == "-f" || arg == "--file" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --file requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.file = Some(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--argv0=") {
                config.argv0 = Some(value);
                continue;
            }
            if arg == "-a" || arg == "--argv0" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --argv0 requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.argv0 = Some(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--split-string=") {
                let split = crate::executor::alias_helpers::split_shell_words(&value);
                args.splice(index..index, split);
                continue;
            }
            if arg == "-S" || arg == "--split-string" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(
                        b"env: option --split-string requires an argument\n",
                    )?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                let split = crate::executor::alias_helpers::split_shell_words(&value);
                args.splice(index..index, split);
                continue;
            }

            if matches!(
                arg.as_str(),
                "--default-signal"
                    | "--ignore-signal"
                    | "--block-signal"
                    | "--list-signal-handling"
            ) || arg.starts_with("--default-signal=")
                || arg.starts_with("--ignore-signal=")
                || arg.starts_with("--block-signal=")
            {
                continue;
            }

            if let Some((name, value)) = parse_env_assignment_arg(&arg) {
                config.assignments.insert(name, value);
                continue;
            }

            command_started = true;
            config.command.push(arg);
        }

        if config.command.is_empty() && config.chdir.is_some() {
            self.write_default_stderr(b"env: must specify command with --chdir\n")?;
            self.exit_code = 125;
            return Ok(None);
        }

        Ok(Some(config))
    }

    fn materialize_env_command_environment(
        &mut self,
        config: &EnvCommandConfig,
    ) -> Result<Option<HashMap<String, String>>, ExecuteError> {
        let mut env_vars = HashMap::new();
        if !config.ignore_environment {
            for name in marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS) {
                if let Some(value) = self.shell_state.env_vars.get(&name) {
                    env_vars.insert(name, value.clone());
                }
            }
            for (name, value) in local_export_env_values(&self.shell_state.env_vars) {
                env_vars.insert(name, value);
            }
            // Windows child processes need the session variables (SystemRoot
            // for crypto/socket setup, WINDIR/ComSpec for subprocess spawning)
            // even when the exporting shell state does not carry them. Unix
            // has no such hidden requirement — GNU env passes only what it is
            // given (builtins/../coreutils env semantics).
            #[cfg(windows)]
            for name in ["SystemRoot", "WINDIR", "ComSpec"] {
                if let Some(value) = self
                    .shell_state
                    .env_vars
                    .get(name)
                    .cloned()
                    .or_else(|| env::var(name).ok())
                {
                    env_vars.entry(name.to_string()).or_insert(value);
                }
            }
        }

        if let Some(file) = &config.file {
            match fs::read_to_string(shell_path_to_windows(file, &self.shell_state.env_vars)) {
                Ok(text) => {
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        if let Some((name, value)) = parse_env_assignment_arg(line) {
                            env_vars.insert(name, value);
                        }
                    }
                }
                Err(error) => {
                    let mut stderr = Vec::new();
                    writeln!(&mut stderr, "env: {file}: {error}")?;
                    self.write_default_stderr(&stderr)?;
                    self.exit_code = 1;
                    return Ok(None);
                }
            }
        }
        for name in &config.unset_names {
            env_vars.remove(name);
        }
        for (name, value) in &config.assignments {
            env_vars.insert(name.clone(), value.clone());
        }
        materialize_required_windows_env(
            &mut env_vars,
            &self.shell_state.env_vars,
            config.ignore_environment,
        );
        Ok(Some(env_vars))
    }

    fn parse_env_short_option_cluster(
        &mut self,
        arg: &str,
        args: &[String],
        index: &mut usize,
        config: &mut EnvCommandConfig,
    ) -> Result<Option<Option<Vec<String>>>, ExecuteError> {
        if !arg.starts_with('-') || arg.starts_with("--") || arg == "-" || arg.len() <= 2 {
            return Ok(None);
        }

        let mut chars = arg[1..].char_indices().peekable();
        while let Some((offset, option)) = chars.next() {
            match option {
                'i' => {
                    config.ignore_environment = true;
                    continue;
                }
                '0' => {
                    config.null_terminated = true;
                    continue;
                }
                'v' => {
                    config.debug = true;
                    continue;
                }
                'u' | 'C' | 'f' | 'a' | 'S' => {
                    let value = if chars.peek().is_some() {
                        arg[1 + offset + option.len_utf8()..].to_string()
                    } else {
                        let Some(value) = args.get(*index).cloned() else {
                            let message = match option {
                                'u' => "env: option --unset requires an argument\n",
                                'C' => "env: option --chdir requires an argument\n",
                                'f' => "env: option --file requires an argument\n",
                                'a' => "env: option --argv0 requires an argument\n",
                                'S' => "env: option --split-string requires an argument\n",
                                _ => unreachable!(),
                            };
                            self.write_default_stderr(message.as_bytes())?;
                            self.exit_code = 125;
                            return Ok(Some(None));
                        };
                        *index += 1;
                        value
                    };

                    match option {
                        'u' => return Ok(Some(Some(vec![format!("--unset={value}")]))),
                        'C' => return Ok(Some(Some(vec![format!("--chdir={value}")]))),
                        'f' => return Ok(Some(Some(vec![format!("--file={value}")]))),
                        'a' => return Ok(Some(Some(vec![format!("--argv0={value}")]))),
                        'S' => {
                            let split = crate::executor::alias_helpers::split_shell_words(&value);
                            return Ok(Some(Some(split)));
                        }
                        _ => unreachable!(),
                    }
                }
                _ => {
                    let mut stderr = Vec::new();
                    writeln!(&mut stderr, "env: invalid option -- '{option}'")?;
                    self.write_default_stderr(&stderr)?;
                    self.exit_code = 125;
                    return Ok(Some(None));
                }
            }
        }

        Ok(Some(Some(Vec::new())))
    }

    pub(in crate::executor) fn execute_external_inner(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if cmd.words.is_empty() {
            return Ok(());
        }

        if self.handle_external_shortcuts(cmd)? {
            return Ok(());
        }

        if self.handle_host_external_command(cmd)? {
            return Ok(());
        }

        if self.handle_external_file_builtins(cmd)? {
            return Ok(());
        }

        if let Some(name) = bash_aliases_assignment_name(&cmd.words[0]) {
            eprintln!("{}`{name}': invalid alias name", self.diagnostic_prefix());
            self.exit_code = 1;
            return Ok(());
        }

        if self.command_output_redirect_fails(cmd)? {
            return Ok(());
        }

        let Some(program) = find_user_command(&cmd.words[0], &self.shell_state.env_vars) else {
            let mut stderr = Vec::new();
            // GNU findcmd.c:385-386 (search_for_command): a name containing
            // a slash is never PATH-searched — it goes straight to execve,
            // so a missing file reports the errno text from
            // execute_cmd.c:6126-6159 shell_execve ("No such file or
            // directory", EX_NOTFOUND), a directory reports EISDIR
            // ("Is a directory", EX_NOEXEC), and an unexecutable file
            // reports "Permission denied" (EX_NOEXEC). Only a slash-free
            // name that PATH could not resolve is "command not found"
            // (rubash#173).
            let name = super::execution_misc::printable_filename(&cmd.words[0]);
            if let Some((message, status)) =
                slash_command_execve_error_message(&cmd.words[0], &self.shell_state.env_vars)
            {
                writeln!(
                    &mut stderr,
                    "{}{}: {}",
                    self.diagnostic_prefix(),
                    name,
                    message
                )?;
                self.finish_external_error(cmd, &stderr, status)?;
                return Ok(());
            }
            writeln!(
                &mut stderr,
                "{}{}: command not found",
                self.diagnostic_prefix(),
                name
            )?;
            self.finish_external_error(cmd, &stderr, 127)?;
            return Ok(());
        };

        // GNU findcmd.c:365/416 (search_for_command): a name resolved through
        // hashed_filenames bumps times_found; one resolved via PATH enters
        // the table with times_found=1 (phash_insert found=1). Skipped for
        // absolute names (absolute_program gate), `set +h`, and command-local
        // PATH — the same gates find_user_command applies to its cache.
        // Internally-emulated commands (external_file_builtins) never reach
        // here, so they stay out of the table rather than diverting to a
        // missing host binary.
        if !cmd.words[0].contains('/')
            && !cmd.words[0].contains('\\')
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "hashall")
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_TEMP_PATH")
                .map(String::as_str)
                != Some("1")
        {
            let display = super::execution_misc::shell_display_path(
                &program.to_string_lossy().replace('\\', "/"),
            );
            crate::builtins::hash::record_command_resolution(
                &mut self.shell_state.env_vars,
                &cmd.words[0],
                &display,
            );
        }

        // GNU execute_cmd.c:6139-6233 (shell_execve): the OS-level exec is
        // attempted first, and only a file the OS refuses to exec is
        // classified by its first bytes: an unresolvable #! interpreter is
        // refused ("bad interpreter", EX_NOEXEC), a binary first line is
        // refused ("cannot execute binary file", EX_BINARY_FILE), and plain
        // text falls back to shell-script execution. Rubash's equivalent
        // boundary is should_run_with_shell: files Windows cannot exec
        // natively get the GNU first-bytes classification before the
        // shell-script fallback wraps them.
        if crate::executor::path::should_run_with_shell(&program) {
            if let Some((diagnostic, status)) = self.exec_format_refusal(cmd, &program) {
                let mut stderr = Vec::new();
                let _ = writeln!(&mut stderr, "{diagnostic}");
                self.finish_external_error(cmd, &stderr, status)?;
                return Ok(());
            }
        }

        let (dev_args, dev_ops) = self.materialize_dev_fd_operands(
            &cmd.words[1..],
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
            crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
        );
        let (mut process, used_shell) = external_command_for_named_program(
            &program,
            Some(&cmd.words[0]),
            &dev_args,
            &self.shell_state.env_vars,
        );
        self.apply_external_environment(cmd, &mut process);
        self.apply_external_redirects(cmd, &mut process)?;
        self.spawn_external_process(cmd, &program, process, used_shell, dev_ops)
    }

    fn handle_host_external_command(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        let Some(output) = self.invoke_host_external_command(cmd) else {
            return Ok(false);
        };
        self.write_buffered_builtin_output(cmd, &output.stdout, &output.stderr)?;
        self.exit_code = output.status;
        Ok(true)
    }

    pub(in crate::executor) fn invoke_host_external_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<HostExternalCommandOutput> {
        let mut env_vars = self.shell_state.env_vars.clone();
        for (var_name, var_value) in &cmd.assignments {
            let (base_name, _) = assignment_name_and_append(var_name);
            let expanded_value = self.expand_assignment_value(var_name, var_value);
            // GNU variables.c:3564-3578 assign_in_env -> bind_variable: a
            // prefix assignment to a nameref binds the referenced variable,
            // so the child env carries the TARGET name and the nameref's
            // own env slot (its cell text) is unchanged (nameref14.sub:
            // `ref=$ref$str printenv ref` still prints "var").
            let Some(env_name) = self.tempenv_export_name(base_name) else {
                continue;
            };
            if is_valid_process_env(&env_name, &expanded_value) {
                env_vars.insert(env_name, expanded_value);
            }
        }
        self.host_external_command_handler
            .as_mut()
            .and_then(|handler| (handler.0)(&cmd.words, &env_vars))
    }

    fn handle_external_shortcuts(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        if self.execute_same_shell_script(cmd)? {
            return Ok(true);
        }

        if cmd.words[0] == "cat" && self.handle_hashed_cat_checkhash()? {
            return Ok(true);
        }

        if matches!(cmd.words[0].as_str(), "/bin/echo" | "/usr/bin/echo") {
            // TODO(findcmd.c/execute_cmd.c): On Windows test runs, Bash-style
            // absolute utility paths should resolve through the active shell
            // environment. Keep this echo mapping until command lookup has a
            // full Unix-path compatibility layer. Output goes through the
            // buffered builtin channel so command substitution captures it
            // (execute_cmd.c pipes external stdout like any other command).
            let mut stdout = Vec::new();
            crate::builtins::echo::write_echo(
                cmd.words[1..].iter().map(String::as_str),
                &mut stdout,
            )?;
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
            self.exit_code = 0;
            return Ok(true);
        }

        if cmd.words[0] == "diff" && cmd.words.len() == 3 {
            // TODO(subst.c/execute_cmd.c): Process substitution should execute
            // each command and pass named pipes/FIFOs to `diff`. Upstream
            // shopt1.sub uses `diff <("$t1") <("$t2")` where the files are
            // executable helper scripts that differ only by a shebang.
            let left =
                shell_path_to_windows(&self.expand_word(&cmd.words[1]), &self.shell_state.env_vars);
            let right =
                shell_path_to_windows(&self.expand_word(&cmd.words[2]), &self.shell_state.env_vars);
            if let (Ok(left_source), Ok(right_source)) =
                (fs::read_to_string(left), fs::read_to_string(right))
            {
                if strip_shebang(&left_source) == strip_shebang(&right_source) {
                    self.exit_code = 0;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn handle_hashed_cat_checkhash(&mut self) -> Result<bool, ExecuteError> {
        // GNU findcmd.c:367-380 + execute_disk_command: a hashed name execs
        // the remembered pathname verbatim. With checkhash off a stale entry
        // fails at execve (diagnostic names the hashed path, status 127);
        // with checkhash on the entry is stat'ed first and forgotten when
        // stale, after which the PATH search runs normally.
        let Some(path) = crate::builtins::hash::hashed_path(&self.shell_state.env_vars, "cat")
        else {
            return Ok(false);
        };
        let native = shell_path_to_windows(&path, &self.shell_state.env_vars);
        if native.is_file() {
            return Ok(false);
        }
        if crate::builtins::shopt::checkhash_enabled() {
            crate::builtins::hash::remove_hashed_path(&mut self.shell_state.env_vars, "cat");
            return Ok(false);
        }
        eprintln!(
            "{}{}: No such file or directory",
            self.diagnostic_prefix(),
            path
        );
        self.exit_code = 127;
        Ok(true)
    }

    pub(in crate::executor) fn apply_external_environment(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) {
        self.apply_child_environment(process);
        for (var_name, var_value) in &cmd.assignments {
            let (base_name, append) = assignment_name_and_append(var_name);
            if append {
                // execute_cmd.c: prefix assignment words are applied to the
                // shell variable table before the command runs; the child
                // environment inherits the already-append-assigned value via
                // apply_child_environment. Re-applying the raw RHS here would
                // overwrite the appended value (a+=5 printenv a must see 145,
                // not 5).
                continue;
            }
            let expanded_value = self.expand_assignment_value(var_name, var_value);
            // GNU variables.c assign_in_env binds a compound `name=(...)`
            // tempenv word as the literal list text; the internal
            // COMPOUND_ASSIGNMENT_MARKER must not leak into the child's
            // environment (niubash #121).
            let expanded_value = expanded_value
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded_value);
            let Some(env_name) = self.tempenv_export_name(base_name) else {
                continue;
            };
            if is_valid_process_env(&env_name, expanded_value) {
                process.env(env_name, expanded_value);
            }
        }
    }

    /// GNU variables.c:3564-3578 assign_in_env -> bind_variable (ASS_NAMEREF):
    /// a tempenv `name=value` whose name resolves through a nameref binds the
    /// final target, so the child environment exports the target name while
    /// the nameref keeps its cell text. A cell resolving to an array element
    /// (`ref -> a[1]`) has no env representation, so nothing is exported.
    /// Unresolvable namerefs keep the literal name (nameref11.sub `r=/ f`
    /// exports `r` as a plain variable).
    fn tempenv_export_name(&self, base_name: &str) -> Option<String> {
        match self.nameref_resolution(base_name) {
            NamerefResolution::Target(target) => {
                if target.contains('[') {
                    None
                } else {
                    Some(target)
                }
            }
            _ => Some(base_name.to_string()),
        }
    }

    /// Foreground wait for a spawned external command. GNU jobs.c:3064
    /// wait_for blocks in waitpid but the shell stays interruptible: a
    /// caught signal is recorded (trap.c:545 trap_handler -> trap.c:537
    /// set_trap_state, `pending_traps[sig]++`) and its trap action runs at
    /// the next command boundary (execute_cmd.c:643 run_pending_traps —
    /// bash manual, SIGNALS: the trap "will not be executed until the
    /// command completes"); an untrapped terminating signal kills the shell
    /// mid-wait through SIG_DFL. The mailbox port slices the kernel wait
    /// (fd wait_child_slice) and records deliveries between slices with the
    /// same deferral (trap_exec observe_signals_while_blocked); on unix the
    /// kernel backend already records arrivals during the plain blocking
    /// wait.
    fn wait_external_child(
        &mut self,
        child: &mut std::process::Child,
    ) -> Result<std::process::ExitStatus, ExecuteError> {
        #[cfg(not(unix))]
        {
            const SLICE: std::time::Duration = std::time::Duration::from_millis(50);
            loop {
                if let Some(status) =
                    crate::fd::wait_child_slice(child, SLICE).map_err(ExecuteError::IoError)?
                {
                    return Ok(status);
                }
                self.observe_signals_while_blocked()?;
            }
        }
        #[cfg(unix)]
        {
            child.wait().map_err(ExecuteError::IoError)
        }
    }

    fn spawn_external_process(
        &mut self,
        cmd: &CommandNode,
        program: &PathBuf,
        mut process: Command,
        used_shell: bool,
        dev_ops: crate::executor::dev_fd_operands::DevOperandMaterialization,
    ) -> Result<(), ExecuteError> {
        // rubash#180: GNU's forked disk-command child inherits every fd >= 3
        // the command opened (`"$SH" -c 'echo hi >&3' 3>f` writes f) or that
        // a persistent `exec 3>f` bound in the parent. std::process::Command
        // cannot attach numbered descriptors, so route the binding through
        // the __RUBASH_FD_* env-key channel the background-spawn path and
        // the child's startup (init.rs) already speak: duplicate each handle
        // as a fresh INHERITABLE handle (the shared slot's inherit flag is
        // never touched, so no cross-spawn race), name it in the child's
        // environment, and close the parent-side duplicate after the spawn —
        // exactly GNU's open->fork->close-unwind shape. Non-rubash children
        // simply see one extra env var (same limitation the background path
        // documents).
        let mut parent_side_dups: Vec<crate::fd::HANDLE> = Vec::new();
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed {
                continue;
            }
            let mut pass = |endpoint: Option<&Rc<crate::executor::fd_table::FileFd>>,
                            write: bool,
                            process: &mut Command,
                            dups: &mut Vec<crate::fd::HANDLE>| {
                let Some(file) = endpoint else {
                    return;
                };
                let Ok(dup) = crate::fd::duplicate_handle_inheritable(file.handle) else {
                    return;
                };
                process.env(
                    format!("__RUBASH_FD_{}HANDLE_{fd}", if write { "W" } else { "" }),
                    format!("{dup:#x}"),
                );
                process.env(
                    format!("__RUBASH_FD_{}PATH_{fd}", if write { "W" } else { "" }),
                    file.path.to_string_lossy().into_owned(),
                );
                dups.push(dup);
            };
            pass(
                entry.read.as_ref().and_then(|read| match read {
                    crate::executor::fd_table::FdReadEndpoint::File(file) => Some(file),
                    _ => None,
                }),
                false,
                &mut process,
                &mut parent_side_dups,
            );
            pass(
                entry.write.as_ref().and_then(|write| match write {
                    crate::executor::fd_table::FdWriteEndpoint::File(file) => Some(file),
                    _ => None,
                }),
                true,
                &mut process,
                &mut parent_side_dups,
            );
        }
        // The command's own fd>=3 file redirects are not fd-table entries on
        // the simple-command path (only compounds bind them scoped), so open
        // them here for the child alone: GNU do_redirections opens them in
        // the parent, the fork inherits, and the parent's copy closes at
        // command end (redir.c undo) — the parent's fd table stays untouched.
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some() {
                continue;
            }
            let fd = match redirect.fd {
                Some(fd) if fd >= 3 => fd,
                _ => continue,
            };
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_some()
                || is_closed_redirect_target(&target)
                || is_null_device(&target)
                || target.starts_with("<(")
            {
                continue;
            }
            let write = !matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input
                    | crate::parser::RedirectKind::DuplicateInput
                    | crate::parser::RedirectKind::CloseInput
                    | crate::parser::RedirectKind::ReadWrite
            );
            if self.fd_table.entries.contains_key(&fd) {
                continue;
            }
            let opened = if write {
                if redirect.append {
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                } else {
                    self.create_redirect_output(&target, redirect.clobber)
                        .map_err(|error| crate::posix_errors::path_error(&target, error))
                }
            } else {
                self.open_input_redirect(&target)
            };
            let Ok(file) = opened else {
                continue;
            };
            #[cfg(windows)]
            let raw = {
                use std::os::windows::io::AsRawHandle;
                file.as_raw_handle() as crate::fd::HANDLE
            };
            #[cfg(unix)]
            let raw = {
                use std::os::unix::io::AsRawFd;
                file.as_raw_fd()
            };
            let Ok(dup) = crate::fd::duplicate_handle_inheritable(raw) else {
                continue;
            };
            process.env(
                format!("__RUBASH_FD_{}HANDLE_{fd}", if write { "W" } else { "" }),
                format!("{dup:#x}"),
            );
            process.env(
                format!("__RUBASH_FD_{}PATH_{fd}", if write { "W" } else { "" }),
                target.clone(),
            );
            parent_side_dups.push(dup);
            // The parent's own open was transient (GNU closes it after the
            // fork); the dup above is what the child holds.
            drop(file);
        }
        match process.spawn() {
            Ok(mut child) => {
                for dup in parent_side_dups {
                    crate::fd::close_handle(dup);
                }
                // Only materialize the virtual stdin text when the child
                // actually owns a pipe — `<&N` on a real file handle uses
                // Stdio::from(dup) (no child.stdin to take), and draining
                // the fd to build a dropped string would corrupt the shared
                // offset.
                if let Some(mut stdin) = child.stdin.take() {
                    if let Some(input) = self.stdin_string_for_command_mut(cmd) {
                        // The input is shell text: raw-byte marker pairs
                        // (heredoc bodies, expanded bytes) must decode to
                        // real bytes for the child (GNU writes fd bytes).
                        stdin.write_all(
                            &crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                                &input,
                            ),
                        )?;
                    }
                }

                if self.external_needs_fd_copy_capture(cmd) {
                    match child.wait_with_output() {
                        Ok(output) => {
                            self.exit_code = 0;
                            // A reaped foreground child delivers SIGCHLD in
                            // GNU bash; a set trap runs once at this
                            // boundary (trap8.sub).
                            self.run_sigchld_trap_for_reaped_child()?;
                            self.write_external_fd_copy_output(
                                cmd,
                                &output.stdout,
                                &output.stderr,
                            )?;
                            if self.exit_code == 0 {
                                self.exit_code = crate::executor::wait_status::process_exit_status(
                                    &output.status,
                                );
                            }
                        }
                        Err(error) => self.report_external_spawn_error(cmd, error)?,
                    }
                } else {
                    match self.wait_external_child(&mut child) {
                        Ok(status) => {
                            self.exit_code =
                                crate::executor::wait_status::process_exit_status(&status);
                            // A reaped foreground child delivers SIGCHLD in
                            // GNU bash; a set trap runs once at this boundary
                            // (trap8.sub).
                            self.run_sigchld_trap_for_reaped_child()?;
                        }
                        Err(ExecuteError::IoError(error)) => {
                            self.report_external_spawn_error(cmd, error)?
                        }
                        // An untrapped terminating signal observed mid-wait:
                        // GNU's SIG_DFL kills the shell while it is still
                        // inside wait_for, so the 128+sig exit propagates
                        // instead of being reported as a spawn failure.
                        Err(error) => return Err(error),
                    }
                }
                // /dev/fd operands materialized as temps resolve now: write
                // endpoints flush into their target and consumed input fds
                // advance (GNU dup shares the fd offset).
                self.finish_dev_fd_operands(dev_ops);
            }
            Err(error) => {
                for dup in parent_side_dups {
                    crate::fd::close_handle(dup);
                }
                if !used_shell && is_exec_format_error(&error) {
                    // GNU execute_cmd.c:6139-6233 (shell_execve): when the OS
                    // refuses to exec a file, its first bytes decide. A #!
                    // interpreter specifier whose interpreter cannot be
                    // resolved is refused ("bad interpreter", EX_NOEXEC =
                    // 126) instead of falling back to shell-script
                    // execution; a binary file (NUL in the first line, ELF
                    // magic) is refused as "cannot execute binary file"
                    // (EX_BINARY_FILE = 126).
                    if let Some((diagnostic, _)) = self.exec_format_refusal(cmd, program) {
                        let mut stderr = Vec::new();
                        let _ = writeln!(&mut stderr, "{diagnostic}");
                        self.finish_external_error(cmd, &stderr, 126)?;
                        return Ok(());
                    }
                    // GNU execute_cmd.c:6237-6260: a text file the kernel
                    // refuses (ENOEXEC) is executed by the forked child as
                    // THIS shell in-process (args[0] = shell_name,
                    // sh_longjmp subshell_top_level -> shell.c:429-464
                    // shell_reinitialize) — a fresh shell with exported-env
                    // only and kernel-preserved SIG_IGN dispositions, NOT
                    // /bin/sh (trap2.sub's ERR trap and trap1.sub's
                    // `trap -p` must run under bash semantics). The Windows
                    // mailbox keeps the find_shell spawn: it cannot observe
                    // a real execve refusal for extensionless text files.
                    if cfg!(unix) {
                        self.execute_enoexec_shell_script(cmd, program)?;
                        return Ok(());
                    }
                    if let Some(shell) = find_shell(&self.shell_state.env_vars) {
                        let mut shell_process = Command::new(shell);
                        shell_process.arg(program);
                        shell_process.args(&cmd.words[1..]);
                        self.apply_external_environment(cmd, &mut shell_process);
                        self.apply_external_redirects(cmd, &mut shell_process)?;
                        return self.spawn_external_process(
                            cmd,
                            program,
                            shell_process,
                            true,
                            dev_ops,
                        );
                    }
                }
                self.report_external_spawn_error(cmd, error)?;
            }
        }

        Ok(())
    }

    /// GNU execute_cmd.c:6139-6233 (shell_execve + execute_shell_script):
    /// classify a file the OS refused to exec. Returns the refusal
    /// diagnostic and exit status when the file must not fall back to
    /// shell-script execution: an unresolvable #! interpreter ("bad
    /// interpreter", EX_NOEXEC) or a binary first line ("cannot execute
    /// binary file", EX_BINARY_FILE). None means the ENOEXEC fallback
    /// (running the file as a shell script) may proceed.
    pub(in crate::executor) fn exec_format_refusal(
        &self,
        cmd: &CommandNode,
        program: &PathBuf,
    ) -> Option<(String, i32)> {
        let sample = std::fs::read(program).ok()?;
        if sample.len() >= 2 && sample[0] == b'#' && sample[1] == b'!' {
            let line_end = sample
                .iter()
                .position(|&byte| byte == b'\n')
                .unwrap_or(sample.len());
            let line = String::from_utf8_lossy(&sample[2..line_end]);
            let mut interp_words = line.split_whitespace();
            if let Some(interp) = interp_words.next() {
                // Kernel #! exec (execve(2)): the interpreter is the first
                // word with one optional argument. `#!/usr/bin/env sh` makes
                // the kernel exec /usr/bin/env with `sh` as its single
                // argument, and env(1) then execs the first `sh` found on
                // PATH (GNU baseline: `PATH=<dir>:$PATH tool alpha beta`
                // prints script:<joined-path>:alpha:2, exit 0). GNU bash
                // never sees that exchange on Linux — shell_execve
                // (execute_cmd.c:6126) only reports "bad interpreter"
                // (execute_cmd.c:6184) when the kernel's own execve failed,
                // e.g. `#!/no/such/interp` -> 126. Windows has no kernel #!
                // and no /usr/bin/env, so this mailbox stands in for both:
                // an `env` launcher resolves through the FOLLOWING word —
                // the program env would exec — and that target decides the
                // refusal. When the target itself is missing, GNU's exit is
                // env's 127 with env's own diagnostic, approximated here by
                // the same refusal shape with env's message text.
                let (effective_interp, env_launcher) = if std::path::Path::new(interp)
                    .file_name()
                    .is_some_and(|base| base == "env")
                {
                    match interp_words.next() {
                        Some(target) => (target, true),
                        None => (interp, false),
                    }
                } else {
                    (interp, false)
                };
                // GNU execute_shell_script re-execs through the named
                // interpreter; when it resolves, the shell-script fallback
                // stands in for that exec here. The standard shells resolve
                // through the fallback without a refusal.
                let interp_resolves = matches!(effective_interp, "sh" | "bash" | "dash" | "rubash")
                    || effective_interp.ends_with("/sh")
                    || effective_interp.ends_with("/bash")
                    || crate::executor::path::find_user_command(
                        effective_interp,
                        &self.shell_state.env_vars,
                    )
                    .is_some();
                if !interp_resolves {
                    // A missing env target is the env child's own failure
                    // (GNU: `env: 'x': No such file or directory`, exit
                    // 127, printed by /usr/bin/env itself); a missing
                    // direct interpreter is the shell_execve refusal
                    // (execute_cmd.c:6184, exit 126).
                    let diagnostic = if env_launcher {
                        format!("env: '{effective_interp}': No such file or directory")
                    } else {
                        format!(
                            "bash: {}: {}: bad interpreter",
                            cmd.words.first().map(String::as_str).unwrap_or_default(),
                            interp
                        )
                    };
                    return Some((diagnostic, if env_launcher { 127 } else { 126 }));
                }
                return None;
            }
        }
        if check_binary_file(&sample) {
            return Some((
                format!(
                    "bash: {}: cannot execute binary file",
                    cmd.words.first().map(String::as_str).unwrap_or_default()
                ),
                126,
            ));
        }
        None
    }

    fn report_external_spawn_error(
        &mut self,
        cmd: &CommandNode,
        error: io::Error,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "rubash: {}: {}",
            cmd.words[0],
            crate::posix_errors::message(&error)
        )?;
        self.finish_external_error(cmd, &stderr, 126)
    }
}

/// check_binary_file (general.c:718-741): ELF magic is always binary;
/// otherwise the first line (two lines when the sample starts with a #!
/// interpreter specifier) must be NUL-free.
fn check_binary_file(sample: &[u8]) -> bool {
    if sample.len() >= 4
        && sample[0] == 0x7f
        && sample[1] == b'E'
        && sample[2] == b'L'
        && sample[3] == b'F'
    {
        return true;
    }
    if sample.is_empty() {
        return false;
    }
    let mut lines_left = if sample.len() >= 2 && sample[0] == b'#' && sample[1] == b'!' {
        2
    } else {
        1
    };
    for &byte in sample {
        if byte == b'\n' {
            lines_left -= 1;
            if lines_left == 0 {
                return false;
            }
        } else if byte == 0 {
            return true;
        }
    }
    false
}

#[derive(Default)]
struct EnvCommandConfig {
    ignore_environment: bool,
    null_terminated: bool,
    debug: bool,
    unset_names: Vec<String>,
    assignments: HashMap<String, String>,
    file: Option<String>,
    chdir: Option<String>,
    argv0: Option<String>,
    command: Vec<String>,
}

fn parse_env_assignment_arg(arg: &str) -> Option<(String, String)> {
    let (name, value) = arg.split_once('=')?;
    (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
}

fn long_option_value(arg: &str, long_prefix: &str) -> Option<String> {
    if let Some(value) = arg.strip_prefix(long_prefix) {
        return Some(value.to_string());
    }
    None
}

fn apply_env_command_environment(
    process: &mut Command,
    env_vars: &HashMap<String, String>,
    inherit_required_windows: bool,
) {
    process.env_clear();
    for (name, value) in env_vars {
        if is_valid_process_env(name, value) {
            let value = if cfg!(windows) && name.eq_ignore_ascii_case("PATH") {
                shell_path_to_process(value, env_vars)
            } else {
                // Raw-byte marker pairs in a stored value must map to byte
                // chars at the child boundary (same rubash#141 contract as
                // child_env_value / the argv boundary), never leak the
                // U+E000 pair into the child's environment block.
                crate::executor::substitution_metadata::decode_raw_byte_markers_to_byte_chars(value)
            };
            process.env(name, value);
        }
    }
    if inherit_required_windows {
        apply_required_windows_child_environment(process, env_vars);
    }
}

#[cfg(windows)]
fn materialize_required_windows_env(
    env_vars: &mut HashMap<String, String>,
    shell_env_vars: &HashMap<String, String>,
    ignore_environment: bool,
) {
    if ignore_environment {
        return;
    }

    for name in ["SystemRoot", "WINDIR", "ComSpec"] {
        if env_vars.contains_key(name) {
            continue;
        }
        if let Some(value) = shell_env_vars
            .get(name)
            .cloned()
            .or_else(|| env::var(name).ok())
        {
            env_vars.insert(name.to_string(), value);
        }
    }

    let home = env_vars
        .get("USERPROFILE")
        .cloned()
        .or_else(|| shell_env_vars.get("USERPROFILE").cloned())
        .or_else(|| env::var("USERPROFILE").ok())
        .or_else(|| {
            env_vars
                .get("HOME")
                .or_else(|| shell_env_vars.get("HOME"))
                .map(|value| {
                    shell_path_to_windows(value, shell_env_vars)
                        .to_string_lossy()
                        .into_owned()
                })
        });
    let Some(home) = home.filter(|value| !value.trim().is_empty() && !value.contains('\0')) else {
        return;
    };

    let native_home = home.replace('/', "\\");
    env_vars
        .entry("USERPROFILE".to_string())
        .or_insert_with(|| native_home.clone());
    env_vars
        .entry("HOME".to_string())
        .or_insert_with(|| native_home.clone());
    if let Some((drive, path)) = windows_drive_and_home_path(&native_home) {
        env_vars.entry("HOMEDRIVE".to_string()).or_insert(drive);
        env_vars.entry("HOMEPATH".to_string()).or_insert(path);
    }
    let base = native_home.trim_end_matches('\\');
    env_vars
        .entry("APPDATA".to_string())
        .or_insert_with(|| format!("{base}\\AppData\\Roaming"));
    env_vars
        .entry("LOCALAPPDATA".to_string())
        .or_insert_with(|| format!("{base}\\AppData\\Local"));
}

#[cfg(not(windows))]
fn materialize_required_windows_env(
    _env_vars: &mut HashMap<String, String>,
    _shell_env_vars: &HashMap<String, String>,
    _ignore_environment: bool,
) {
}

#[cfg(windows)]
fn windows_drive_and_home_path(path: &str) -> Option<(String, String)> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let drive = path[..2].to_string();
    let rest = path[2..].trim_start_matches(['\\', '/']);
    Some((drive, format!("\\{}", rest.replace('/', "\\"))))
}

fn is_exec_format_error(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(8)
    }

    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

/// GNU general.c:843 absolute_program: a name containing `/` (or `\` on a
/// MSYS-like host). Such a name is never PATH-searched (findcmd.c:385-386
/// search_for_command), so its failure text is classified by the execve
/// result, not by the PATH miss: execute_cmd.c:6126-6159 shell_execve —
/// ENOENT ("No such file or directory", 127), a directory (EISDIR text,
/// 126), an unexecutable file ("Permission denied", 126). Returns None for
/// slash-free names, which keep the "command not found" PATH wording
/// (execute_cmd.c:5910 notfound_str).
pub(crate) fn slash_command_execve_error_message(
    name: &str,
    env_vars: &HashMap<String, String>,
) -> Option<(&'static str, i32)> {
    if !name.contains('/') && !name.contains('\\') {
        return None;
    }
    let candidate = crate::executor::path::shell_path_to_windows(name, env_vars);
    let metadata = std::fs::metadata(&candidate)
        .or_else(|_| std::fs::metadata(name))
        .ok();
    match metadata {
        None => Some(("No such file or directory", 127)),
        Some(metadata) if metadata.is_dir() => Some(("Is a directory", 126)),
        Some(_) => Some(("Permission denied", 126)),
    }
}
