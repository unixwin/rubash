use super::*;

/// rubash#1626: pin the shell's ambient fd-table descriptors at their
/// slot numbers in a forked child, between fork and exec — GNU's child
/// inherits every descriptor `exec N>file` bound, at the same number.
/// Two-phase so a pin never clobbers a not-yet-moved source: first move
/// every source above every target with F_DUPFD_CLOEXEC, then dup2 the
/// staged copies onto their slots (dup2 clears CLOEXEC on the target,
/// so the descriptor survives exec). A source already sitting at its
/// slot only needs its CLOEXEC flag cleared. Failures are ignored per
/// pin: the child then simply lacks that descriptor — the pre-fix
/// behavior — instead of failing the whole spawn. One target is never
/// clobbered: an anonymous pipe the spawn machinery itself parked at
/// that number in the child (its exec-error channel). It is
/// recognizable as an open FIFO at the target that is not one of the
/// pin sources; overwriting it would corrupt spawn error reporting.
#[cfg(unix)]
fn pin_ambient_fds_in_child(pins: &[(std::os::unix::io::RawFd, u32)]) -> std::io::Result<()> {
    use std::os::unix::io::RawFd;
    let sources: Vec<RawFd> = pins.iter().map(|&(source, _)| source).collect();
    let ceiling = pins
        .iter()
        .map(|&(source, slot)| source.max(slot as RawFd))
        .max()
        .unwrap_or(2);
    let mut staged: Vec<(RawFd, RawFd)> = Vec::with_capacity(pins.len());
    for &(source, slot) in pins {
        let target = slot as RawFd;
        if source == target {
            unsafe { libc::fcntl(source, libc::F_SETFD, 0) };
            continue;
        }
        let temp = unsafe { libc::fcntl(source, libc::F_DUPFD_CLOEXEC, ceiling + 1) };
        if temp >= 0 {
            staged.push((temp, target));
        }
    }
    for (temp, target) in staged {
        if !sources.contains(&target) && fd_is_anonymous_pipe(target) {
            unsafe { libc::close(temp) };
            continue;
        }
        if unsafe { libc::dup2(temp, target) } >= 0 {
            unsafe { libc::close(temp) };
        }
    }
    Ok(())
}

/// Whether `fd` is currently open in this process as an anonymous pipe.
#[cfg(unix)]
fn fd_is_anonymous_pipe(fd: std::os::unix::io::RawFd) -> bool {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return false;
    }
    // Widened through u32 like the sibling checks in src/fd/unix.rs:
    // bionic (Android) types the S_IF* constants as u16 while st_mode
    // is u32, so the raw comparison does not compile there.
    (u32::from(stat.st_mode) & u32::from(libc::S_IFMT)) == u32::from(libc::S_IFIFO)
}

impl Executor {
    /// The simple-command execution lookup. Under a `command -p`
    /// re-dispatch (standard_path_scope set) the lookup walks the
    /// standard path — by substituting PATH in a throwaway copy of the
    /// env map, never in the shell's own variables (rubash#1047) —
    /// and skips the hash table entirely (rubash#1648; GNU
    /// findcmd.c:364: CMDSRCH_STDPATH never consults hashed_filenames).
    pub(in crate::executor) fn find_scoped_user_command(
        &self,
        name: &str,
    ) -> Option<std::path::PathBuf> {
        match &self.standard_path_scope {
            Some(standard) => {
                let mut env_vars = self.shell_state.env_vars.clone();
                env_vars.insert("PATH".to_string(), standard.clone());
                find_user_command_without_hash(name, &env_vars)
            }
            None => find_user_command(name, &self.shell_state.env_vars),
        }
    }

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
        let Some(env_vars) = self.materialize_env_command_environment(cmd, &config)? else {
            return Ok(());
        };
        // Lookup view of the composed environment for the consumers
        // that take a name->value map (program resolution, argv0
        // detection, chdir translation); composition ORDER lives only
        // in env_vars itself.
        let env_map: HashMap<String, String> = env_vars.iter().cloned().collect();

        if config.command.is_empty() {
            let mut output = Vec::new();
            // Composition order is the output order (rubash#1806): GNU
            // env prints the environment array it was exec'd with,
            // which maybe_make_export_env built temporary-block first
            // and `_` last — no sorting anywhere in the chain.
            for (key, value) in &env_vars {
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

        let Some(program) = find_user_command(&config.command[0], &env_map) else {
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
            &env_map,
            env_map
                .get("BASH_ARGV0")
                .is_some_and(|value| !value.is_empty()),
        );
        // GNU env (coreutils) runs the command through execvp under the
        // name as typed, so the child's argv[0] is the bare word, not
        // the PATH-resolved path Command::new carries (rubash#1276:
        // a GNU grandchild's diagnostics were prefixed with the full
        // resolved path when the parent shell was niu). Shell-wrapped
        // children pick their own argv[0] (the script name).
        #[cfg(unix)]
        if !used_shell {
            use std::os::unix::process::CommandExt;
            process.arg0(&config.command[0]);
        }
        // G-02: children are born in the session cwd; the `env -C`
        // override below replaces it for its own child.
        process.current_dir(&self.shell_state.cwd);
        apply_env_command_environment(&mut process, &env_vars, !config.ignore_environment);
        if let Some(chdir) = &config.chdir {
            let directory = shell_path_to_windows(chdir, &env_map);
            if !directory.is_dir() {
                self.write_default_stderr(b"env: cannot change directory\n")?;
                self.exit_code = 125;
                return Ok(());
            }
            process.current_dir(directory);
        }
        self.apply_external_redirects(cmd, &mut process)?;
        // rubash#1832: the environment was composed to finality by
        // the env emulation above — the seam must not stamp SHLVL
        // over it (env_composed_final).
        self.spawn_external_process(cmd, &program, process, used_shell, dev_ops, true)
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
                config.assignments.push((name, value));
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
        cmd: &CommandNode,
        config: &EnvCommandConfig,
    ) -> Result<Option<Vec<(String, String)>>, ExecuteError> {
        // The composed environment is ORDERED (rubash#1806): GNU
        // variables.c maybe_make_export_env flattens the variable
        // contexts into the child env array in a fixed block order —
        // the temporary_env context (this command's prefix assignments)
        // FIRST, then the shell_variables contexts, then the exported
        // functions — with add_or_supercede semantics (a name already
        // present is replaced in place, never appended twice), so the
        // temporary binding shadows the global of the same name. Within
        // the global block GNU iterates hash chains, whose order is an
        // implementation artifact, not a contract; this composition
        // sorts that block by name so the emitted order is
        // deterministic. The bare-`env` print path emits this order
        // verbatim.
        let mut env_vars: Vec<(String, String)> = Vec::new();
        if !config.ignore_environment {
            // Block 1: this command's prefix assignments, in source
            // order, valued exactly as apply_external_environment
            // values them for the real spawn path (append assignments
            // read the value the dispatch seam already composed into
            // the table; plain assignments re-derive from the expanded
            // word — cmd.assignments are post-expansion text).
            let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
            for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
                let (base_name, append) = assignment_name_and_append(var_name);
                let Some(env_name) = self.tempenv_export_name(base_name) else {
                    continue;
                };
                if append {
                    if let Some(value) = self.shell_state.env_vars.get(base_name) {
                        ordered_env_insert(&mut env_vars, env_name, value.clone());
                    }
                    continue;
                }
                let raw = raws_aligned
                    .then(|| cmd.assignment_raws.get(index))
                    .flatten()
                    .map(String::as_str);
                let expanded_value =
                    self.expand_assignment_value_with_raw(var_name, var_value, raw);
                let expanded_value = expanded_value
                    .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                    .unwrap_or(&expanded_value);
                ordered_env_insert(&mut env_vars, env_name, expanded_value.to_string());
            }
            // Block 2: exported shell variables. A name the prefix block
            // already emitted is shadowed by it (GNU flatten rule) and
            // must not be appended a second time. GNU iterates its hash
            // chains here (an artifact order); rubash composes this
            // block sorted by name so the print order is deterministic.
            let mut global_names = marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS);
            global_names.sort();
            for name in global_names {
                if ordered_env_contains(&env_vars, &name) {
                    continue;
                }
                // GNU variables.c make_env_array_from_var_list: array and
                // associative variables never cross the child env boundary
                // (rubash#534) — `env`'s internal emulation mirrors the
                // real spawn path.
                if !is_exportable_env_name(&self.shell_state.env_vars, &name) {
                    continue;
                }
                if let Some(value) = self.shell_state.env_vars.get(&name) {
                    ordered_env_insert(&mut env_vars, name, value.clone());
                }
            }
            let mut stash_entries = local_export_env_values(&self.shell_state.env_vars);
            stash_entries.sort();
            for (name, value) in stash_entries {
                if ordered_env_contains(&env_vars, &name) {
                    continue;
                }
                // Same rule as apply_child_environment: the stash stands
                // in for the shadowed outer binding only while the local
                // is not itself a live export candidate (rubash#970).
                if !local_export_env_entry_applies(&self.shell_state.env_vars, &name) {
                    continue;
                }
                ordered_env_insert(&mut env_vars, name, value);
            }
            // GNU `env` is an external binary that inherits the shell's OS
            // environment, in which every exported function was
            // materialized as BASH_FUNC_<name>%% (variables.c:3989-4034
            // push_exported_function / make_env_array_from_var_list). The
            // internal emulation must materialize them the same way the
            // real spawn path does (apply_exported_functions_to_child),
            // or `env | grep BASH_FUNC` reports nothing while `printenv`
            // (a real external) sees the function (rubash#253).
            let mut function_names =
                marked_env_names(&self.shell_state.env_vars, EXPORTED_FUNCTIONS);
            function_names.sort();
            for name in function_names {
                if let Some(body) = self.shell_state.functions.get(&name) {
                    let def_redirects = self
                        .shell_state
                        .function_def_infos
                        .get(&name)
                        .map(|info| info.def_redirects.as_slice())
                        .unwrap_or(&[]);
                    ordered_env_insert(
                        &mut env_vars,
                        exported_function_env_name(&name),
                        exported_function_env_value(&body.commands, def_redirects),
                    );
                }
            }
            // Windows child processes need the session variables (SystemRoot
            // for crypto/socket setup, WINDIR/ComSpec for subprocess spawning)
            // even when the exporting shell state does not carry them. Unix
            // has no such hidden requirement — GNU env passes only what it is
            // given (builtins/../coreutils env semantics).
            #[cfg(windows)]
            for name in ["SystemRoot", "WINDIR", "ComSpec"] {
                if ordered_env_contains(&env_vars, name) {
                    continue;
                }
                if let Some(value) = self
                    .shell_state
                    .env_vars
                    .get(name)
                    .cloned()
                    .or_else(|| env::var(name).ok())
                {
                    ordered_env_insert(&mut env_vars, name.to_string(), value);
                }
            }
            // Final entry: `_`. GNU execute_cmd.c execute_disk_command
            // calls put_command_name_into_env (variables.c) with the
            // pathname search_for_command resolved for the command about
            // to run — for the `env` emulation that command is `env`
            // itself, so the composed environment ends with `_` naming
            // the resolved env program (coreutils env does not rewrite
            // `_` for its own child; it is inherited as-is). Resolution
            // runs against the environment composed so far, so a prefix
            // PATH assignment steers it exactly as the real spawn path
            // would steer the lookup of `env`. If `env` does not
            // resolve, no `_` is fabricated.
            let lookup_map: HashMap<String, String> = env_vars.iter().cloned().collect();
            if let Some(env_program) = find_user_command("env", &lookup_map) {
                ordered_env_insert(
                    &mut env_vars,
                    "_".to_string(),
                    env_program.to_string_lossy().into_owned(),
                );
            }
        }

        // rubash#1832: GNU's SHLVL adjustment is a mutation of the
        // shell variable BEFORE the export environment is built
        // (execute_cmd.c:5813-5817: on the nofork path
        // adjust_shell_level(-1) runs first, then
        // maybe_make_export_env), so when this env command is the
        // exec-replace tail, the environment env RECEIVES already
        // carries the adjusted level. Fold it into the composed base
        // here — after the base blocks above, before env's own
        // file/unset/assignment processing below, which must govern
        // the final value (a `-u SHLVL` removes the entry, an explicit
        // `SHLVL=` assignment supersedes it, a file entry wins over
        // the base). A prefix SHLVL word on the env command itself is
        // part of the base blocks and is superseded like on the
        // direct path (oracle: `SHLVL=99 env ...` prints the adjusted
        // level). Under `-i` there is no received environment to
        // adjust — the base blocks were skipped and nothing is
        // folded. The spawn seam must not stamp over this composition
        // afterwards (see spawn_external_process's env_composed_final).
        #[cfg(unix)]
        if !config.ignore_environment && self.exec_replace_tail_state_ready() {
            let adjusted = self.exec_replace_adjusted_shlvl();
            if ordered_env_contains(&env_vars, "SHLVL") {
                ordered_env_insert(&mut env_vars, "SHLVL".to_string(), adjusted);
            } else if let Some(pos) = env_vars.iter().position(|(name, _)| name == "_") {
                // Keep `_` the final entry (the composition invariant
                // above); the adjusted level joins the globals block.
                env_vars.insert(pos, ("SHLVL".to_string(), adjusted));
            } else {
                env_vars.push(("SHLVL".to_string(), adjusted));
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
                            ordered_env_insert(&mut env_vars, name, value);
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
            ordered_env_remove(&mut env_vars, name);
        }
        for (name, value) in &config.assignments {
            ordered_env_insert(&mut env_vars, name.clone(), value.clone());
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

        let Some(program) = self.find_scoped_user_command(&cmd.words[0]) else {
            // GNU execute_cmd.c (execute_simple_command /
            // command_not_found_handler): when the shell function
            // `command_not_found_handle` is defined, it is invoked with
            // the full command line as its arguments instead of the
            // `command not found` diagnostic, and its exit status becomes
            // the command's status. The executing_command_not_found_handler
            // guard keeps a missing command inside the handler itself on
            // the plain 127 path (rubash#1080).
            if !cmd.words[0].contains('/')
                && self
                    .shell_state
                    .functions
                    .contains_key("command_not_found_handle")
                && !self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_CNF_HANDLER_RUNNING")
            {
                self.shell_state
                    .env_vars
                    .insert("__RUBASH_CNF_HANDLER_RUNNING".to_string(), "1".to_string());
                // GNU passes the whole command line: $1 is the missing
                // command name, $2.. its arguments.
                let args: Vec<String> = cmd.words.to_vec();
                let mut handler_cmd = cmd.clone();
                handler_cmd.words = std::iter::once("command_not_found_handle".to_string())
                    .chain(cmd.words.iter().cloned())
                    .collect();
                let result = self.execute_function("command_not_found_handle", &args, &handler_cmd);
                self.shell_state
                    .env_vars
                    .remove("__RUBASH_CNF_HANDLER_RUNNING");
                result?;
                return Ok(());
            }
            let mut stderr = Vec::new();
            // GNU findcmd.c:385-386 (search_for_command): a name containing
            // a slash is never PATH-searched — it goes straight to execve,
            // so a missing file reports the errno text from
            // execute_cmd.c:6126-6159 shell_execve ("No such file or
            // directory", EX_NOTFOUND), a directory reports EISDIR
            // ("Is a directory", EX_NOEXEC), and an unexecutable file
            // reports "Permission denied" (EX_NOEXEC). Only a slash-free
            // name that PATH could not resolve is "command not found"
            // (rubash#173). All of these are internal_error via
            // error_prolog, so the prefix honors gnu_errfmt (rubash#1038).
            let error_prefix = self.error_prefix();
            let name = super::execution_misc::printable_filename(&cmd.words[0]);
            if let Some((message, status)) =
                slash_command_execve_error_message(&cmd.words[0], &self.shell_state.env_vars)
            {
                writeln!(&mut stderr, "{error_prefix}{name}: {message}",)?;
                self.finish_external_error(cmd, &stderr, status)?;
                return Ok(());
            }
            // GNU findcmd.c:279-293 + execute_cmd.c shell_execve: with
            // PATH unset the name is never searched, so the failure is
            // the execve ENOENT text, not "command not found" — bash
            // switches diagnostic on PATH being unset (rubash#1145). The
            // report is internal_error (error_prolog family), so the
            // prefix honors gnu_errfmt (rubash#1038).
            let not_found_message =
                if crate::executor::path::path_is_unset(&self.shell_state.env_vars) {
                    "No such file or directory"
                } else {
                    "command not found"
                };
            writeln!(
                &mut stderr,
                "{}{}: {}",
                self.error_prefix(),
                name,
                not_found_message
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
            if self.standard_path_scope.is_some() {
                // rubash#1648: a `command -p` resolution inserts
                // unconditionally (see record_standard_path_resolution).
                crate::builtins::hash::record_standard_path_resolution(
                    &mut self.shell_state.env_vars,
                    &cmd.words[0],
                    &display,
                );
            } else {
                crate::builtins::hash::record_command_resolution(
                    &mut self.shell_state.env_vars,
                    &cmd.words[0],
                    &display,
                );
            }
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

        // GNU execute_cmd.c:6139 shell_execve attempts execve directly
        // and classifies an ENOEXEC refusal by first bytes (refuse
        // binaries, run text as a script in THIS shell). std::process::
        // Command spawns through glibc execvp, whose own ENOEXEC
        // fallback silently substitutes /bin/sh — a foreign shell with
        // different syntax — so on unix the classification must happen
        // BEFORE the spawn: a regular file that is neither #!-headed
        // nor a native ELF image is exactly what execve refuses
        // (rubash#1037).
        #[cfg(unix)]
        if is_enoexec_candidate(&program) {
            if let Some((diagnostic, status)) = self.exec_format_refusal(cmd, &program) {
                let mut stderr = Vec::new();
                let _ = writeln!(&mut stderr, "{diagnostic}");
                self.finish_external_error(cmd, &stderr, status)?;
                return Ok(());
            }
            self.execute_enoexec_shell_script(cmd, &program)?;
            return Ok(());
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
            self.bash_argv0_env_supplied(&cmd.assignments),
        );
        // G-02: children are born in the session cwd, not the process cwd.
        process.current_dir(&self.shell_state.cwd);
        self.apply_external_environment(cmd, &mut process, &program);
        self.apply_external_redirects(cmd, &mut process)?;
        self.spawn_external_process(cmd, &program, process, used_shell, dev_ops, false)
    }

    fn handle_host_external_command(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // Top-level dispatch: the dispatch seam applied this command's
        // prefix assignments to the table before reaching here.
        let Some(output) = self.invoke_host_external_command(cmd, true) else {
            return Ok(false);
        };
        self.write_buffered_builtin_output(cmd, &output.stdout, &output.stderr)?;
        self.exit_code = output.status;
        Ok(true)
    }

    /// Build the env map for the embedded-host channel. `assignments_applied`
    /// records whether the command's prefix assignments were already applied
    /// to the shell variable table: true on the top-level dispatch path
    /// (the dispatch seam runs apply_temporary_assignments first), false on
    /// the pipeline-stage path (a stage's assignments live only on the
    /// cloned command node, never on the table — the same split the stage
    /// spawner faces). The handler contract itself
    /// (`FnMut(&[String], &HashMap<String, String>)`) is untouched; only
    /// the values composed into the map change (rubash#1766).
    pub(in crate::executor) fn invoke_host_external_command(
        &mut self,
        cmd: &CommandNode,
        assignments_applied: bool,
    ) -> Option<HostExternalCommandOutput> {
        let mut env_vars = self.shell_state.env_vars.clone();
        let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
        for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
            let (base_name, append) = assignment_name_and_append(var_name);
            // GNU variables.c:3564-3578 assign_in_env -> bind_variable: a
            // prefix assignment to a nameref binds the referenced variable,
            // so the child env carries the TARGET name and the nameref's
            // own env slot (its cell text) is unchanged (nameref14.sub:
            // `ref=$ref$str printenv ref` still prints "var").
            let Some(env_name) = self.tempenv_export_name(base_name) else {
                continue;
            };
            if append && assignments_applied {
                // Top-level path: the dispatch seam already composed the
                // append into the table, so the env clone above inherits
                // the composed value. Re-applying the raw RHS here would
                // overwrite it (a+=5 must see 145, not 5) — the skip the
                // direct path (apply_external_environment) performs, and
                // the rubash#1620 root-cause-2 shape on this channel.
                continue;
            }
            let raw = raws_aligned
                .then(|| cmd.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
            // GNU variables.c assign_in_env binds a compound `name=(...)`
            // tempenv word as the literal list text; the internal
            // COMPOUND_ASSIGNMENT_MARKER must not leak into the host env
            // map (mirror of apply_external_environment, niubash#121).
            let expanded_value = expanded_value
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded_value);
            let expanded_value = if append {
                // Pipeline-stage path: the stage's assignments were never
                // applied to the table, so the clone still holds the window
                // value; compose it the way the stage spawner does —
                // tempenv_scalar_view of the export target (scalar itself,
                // indexed element 0, assoc empty) plus the stripped RHS —
                // which reproduces the GNU oracle values (1(9), 10(30),
                // ([j]=w)) instead of the raw RHS.
                format!("{}{}", self.tempenv_scalar_view(&env_name), expanded_value)
            } else {
                expanded_value.to_string()
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
            // rubash#1488: the emulated /bin/echo is still the echo
            // builtin's escape path — same locale guard as execute_echo.
            let _locale_guard = crate::locale::ShellLocaleGuard::new(&self.shell_state.env_vars);
            crate::builtins::echo::write_echo(
                cmd.words[1..].iter().map(String::as_str),
                &mut stdout,
            )?;
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
            // rubash#1496: sh_chkwrite — a write failure the routing
            // folded into the flag (e.g. /bin/echo hi > /dev/full) fails
            // the command; the unconditional 0 here erased it.
            self.exit_code = if self.take_builtin_write_failed() {
                1
            } else {
                0
            };
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
        // GNU execute_cmd.c:6126 shell_execve refusal of a hashed pathname
        // (internal_error via error_prolog) honors gnu_errfmt.
        eprintln!("{}{}: No such file or directory", self.error_prefix(), path);
        self.exit_code = 127;
        Ok(true)
    }

    /// Whether a BASH_ARGV0 value crosses the child boundary for the
    /// command about to spawn (rubash#1338): a prefix assignment in
    /// `assignments`, or a bound value carrying the exported mark (the
    /// tempenv apply marks prefix names exported for the command, so the
    /// mark test covers both). GNU variables.c:1547-1555 (set_argv0)
    /// makes such a value the child's dollar_vars[0]; the spawn
    /// builder's fabricated $0 operand for `sh|bash -c` would outrank
    /// it in the child, so the builder stands down when this is true.
    /// A plain unexported binding stays in the parent (GNU
    /// make_env_array_from_var_list) and does not count.
    pub(in crate::executor) fn bash_argv0_env_supplied(
        &self,
        assignments: &[(String, String)],
    ) -> bool {
        assignments
            .iter()
            .any(|(name, _)| assignment_name_and_append(name).0 == "BASH_ARGV0")
            || (is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, "BASH_ARGV0")
                && self
                    .shell_state
                    .env_vars
                    .get("BASH_ARGV0")
                    .is_some_and(|value| !value.is_empty()))
    }

    pub(in crate::executor) fn apply_external_environment(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
        program: &std::path::Path,
    ) {
        self.apply_child_environment(process);
        let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
        for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
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
            let raw = raws_aligned
                .then(|| cmd.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
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
                // Prefix assignments overwrite apply_child_environment above;
                // retain its native TMPDIR boundary (niubash#162).
                if cfg!(windows) && env_name == "TMPDIR" {
                    process.env(&env_name, self.child_env_value(&env_name, expanded_value));
                } else {
                    process.env(env_name, expanded_value);
                }
            }
        }
        // GNU execute_cmd.c execute_disk_command finishes the child
        // environment with put_command_name_into_env (variables.c):
        // `_` is set to the pathname search_for_command resolved for
        // the command about to run, superseding any inherited or
        // prefix-assigned entry, for every external command
        // (rubash#1806). Setting it last in this composition is what
        // makes the supersede hold over both sources above.
        process.env("_", program.to_string_lossy().into_owned());
    }

    /// GNU variables.c:3564-3578 assign_in_env -> bind_variable (ASS_NAMEREF):
    /// a tempenv `name=value` whose name resolves through a nameref binds the
    /// final target, so the child environment exports the target name while
    /// the nameref keeps its cell text. A cell resolving to an array element
    /// (`ref -> a[1]`) has no env representation, so nothing is exported.
    /// Unresolvable namerefs keep the literal name (nameref11.sub `r=/ f`
    /// exports `r` as a plain variable).
    pub(in crate::executor) fn tempenv_export_name(&self, base_name: &str) -> Option<String> {
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
                    // rubash#1776: sample the reaped child's CPU for the
                    // Windows CHILDREN half while its handle is live.
                    super::read_helpers::note_reaped_child_cpu(child);
                    return Ok(status);
                }
                if let Err(error) = self.observe_signals_while_blocked() {
                    // Untrapped SIGINT (console CTRL_C via note_console_ctrl_c,
                    // or a mailbox delivery) aborts the wait. Reap the child so
                    // it does not linger when the console event did not kill it.
                    let _ = child.kill();
                    let _ = child.wait();
                    super::read_helpers::note_reaped_child_cpu(child);
                    return Err(error);
                }
            }
        }
        #[cfg(unix)]
        {
            // GNU jobs.c:3086-3102 wait_for installs wait_sigint_handler
            // for the wait: the handler only records (trap.c
            // trap_handler sets pending_traps[SIGINT]) instead of dying,
            // and after the wait the shell reports the dead child's
            // 128+SIGINT status and continues. Without this, SIG_DFL
            // kills an interactive shell inside waitpid on Ctrl-C
            // (rubash#1250; GNU survives with rc=130). Non-interactive
            // shells keep SIG_DFL and die like GNU scripts do.
            let sigint_guard = ForegroundWaitSigintGuard::hold_if_interactive_untrapped(self);
            self.note_foreground_wait();
            let status = child.wait().map_err(ExecuteError::IoError)?;
            // rubash#1639: the foreground job died by SIGINT in an
            // interactive reader session with SIGINT untrapped — GNU
            // jobs.c's waitchld tail (the job_control/IS_FOREGROUND/
            // WTERMSIG==SIGINT block) does ADDINTERRUPT there, and
            // the executing list throws to the top-level reader,
            // discarding the rest of the line instead of continuing
            // it with the 130 status. No shell-side receipt is
            // required under job control: the job's own SIGINT death
            // is the verdict. (Without job control the shell shares
            // the child's process group, the same ^C is what the
            // guard recorded, and the verdict is identical.) Mark the
            // event; the list boundary converts it (the callers still
            // record the 130 status from this ExitStatus first).
            if sigint_guard.is_armed()
                && crate::executor::wait_status::exit_status_signal(&status) == Some(libc::SIGINT)
            {
                self.fg_sigint_unwind = true;
            }
            Ok(status)
        }
    }

    /// rubash#1497: execution-time gates for tail exec replacement,
    /// mirroring GNU builtins/evalstring.c:106 should_optimize_fork as
    /// re-checked when the command runs (a trap armed by an earlier
    /// command of the same -c string still blocks the replacement),
    /// plus this port's capture bookkeeping: replacement is only sound
    /// when the prepared Command inherits the process's own stdio, no
    /// capture drain or deferred fd writeback is pending, and no
    /// virtual stdin text waits in this process's memory. The DEBUG
    /// trap does NOT block (it is not a signal trap; it still fires
    /// before the command, in the shell), and neither does a RETURN
    /// trap at top level (variable_context == 0 there).
    #[cfg(unix)]
    /// rubash#1832: the state-only half of the exec-replace tail
    /// conditions — everything `exec_replace_tail_ready` checks that
    /// depends on the dispatch stamp and shell state alone, not on
    /// the resolved program, the stdio plan, or the dev-fd
    /// materialization. The env emulation folds the SHLVL adjustment
    /// into its composition under this gate (the composition is what
    /// env receives, and GNU adjusts before building it); the spawn
    /// seam still re-checks the full predicate before replacing.
    fn exec_replace_tail_state_ready(&self) -> bool {
        if self.exec_replace_dispatch_stamp.get() == 0 {
            return false;
        }
        if self.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
            return false;
        }
        if self
            .shell_state
            .env_vars
            .contains_key(crate::executor::types::FUNCTION_STDIN)
            || self.virtual_fd_stdin_remaining(0).is_some()
        {
            return false;
        }
        let env = &self.shell_state.env_vars;
        // EXIT and ERR traps block by presence (GNU signal_is_trapped
        // counts `trap ''` too); get_trap_action is reset-aware.
        if crate::builtins::trap::get_trap_action(env, "EXIT").is_some()
            || crate::builtins::trap::get_trap_action(env, "ERR").is_some()
        {
            return false;
        }
        // Any real signal trap blocks, again by presence. Pseudo
        // signals and bookkeeping keys share the trap prefix without
        // being signal traps.
        let reset = env
            .get(crate::builtins::trap::TRAP_RESET)
            .map(String::as_str)
            .unwrap_or("");
        for key in env.keys() {
            let Some(signal) = key.strip_prefix(crate::builtins::trap::TRAP_PREFIX) else {
                continue;
            };
            match signal {
                "EXIT" | "ERR" | "ERROR" | "DEBUG" | "RETURN" | "RESET" | "ORIG_IGN" => {}
                _ if reset.split(':').any(|name| name == signal) => {}
                _ => return false,
            }
        }
        true
    }

    #[cfg(unix)]
    fn exec_replace_tail_ready(
        &self,
        cmd: &CommandNode,
        program: &PathBuf,
        dev_ops: &crate::executor::dev_fd_operands::DevOperandMaterialization,
    ) -> bool {
        if !self.exec_replace_tail_state_ready() {
            return false;
        }
        if !crate::executor::path::program_execs_directly(program) {
            return false;
        }
        // CapturePipe means the parent must drain the child's output
        // itself — impossible after a process replacement.
        // LiveHandles is the opposite shape: the prepared Command
        // already carries the real destination handles (files, pipes,
        // fd-table bindings), which an exec inherits unchanged, and
        // the parent's only post-spawn duty was the wait.
        if matches!(
            self.external_stdio_outcome,
            Some(super::external_redirects::ExternalStdioOutcome::CapturePipe(_))
        ) || self.external_needs_fd_copy_capture(cmd)
        {
            return false;
        }
        if dev_ops.has_deferred_flushes() {
            return false;
        }
        true
    }

    /// rubash#1497: GNU execute_cmd.c:5813-5819 — on the nofork path
    /// the shell decrements its level (adjust_shell_level(-1)) before
    /// building the export environment, so the replaced process sees
    /// the level the shell itself inherited. The adjustment reads the
    /// CURRENT SHLVL variable (prefix temp assignments are not yet
    /// committed when execute_disk_command runs), clamps at 0, and the
    /// variable is (re)created in the child environment even when the
    /// shell variable is absent or unexported.
    /// rubash#1497: the shell's current SHLVL as a number — an absent
    /// or unparseable value counts as 0 (GNU adjust_shell_level works
    /// on the variable's numeric value; see execute_cmd.c:5819).
    pub(in crate::executor) fn current_shlvl_value(&self) -> i64 {
        self.shell_state
            .env_vars
            .get("SHLVL")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0)
    }

    #[cfg(unix)]
    fn exec_replace_adjusted_shlvl(&self) -> String {
        // GNU reads the CURRENT variable, which never includes the
        // tail command's own prefix temp assignments; this port has
        // committed them into env_vars by spawn time, so the value
        // frozen at dispatch (exec_replace_shlvl_snapshot) is the one
        // to adjust. The live read remains as a defensive fallback.
        let current = self
            .exec_replace_shlvl_snapshot
            .get()
            .unwrap_or_else(|| self.current_shlvl_value());
        (current - 1).max(0).to_string()
    }

    /// rubash#1626/#1688: the ambient-fd pin table for one external
    /// child — (parent descriptor, slot) for every fd-table entry >= 3
    /// backed by a real descriptor (a File endpoint, or a coproc
    /// endpoint's raw descriptor), for `pin_ambient_fds_in_child` to
    /// dup2 onto the slot numbers between fork and exec. GNU forks every
    /// external child — simple command or pipeline element alike
    /// (execute_cmd.c:2620 execute_pipeline) — from the shell, so the
    /// child inherits the shell's descriptors at their numbers; the pin
    /// mirrors that inheritance for the table's CLOEXEC descriptors.
    /// Slots the command itself redirects are skipped — the command's
    /// binding is the more local one and travels its own pins / the env
    /// channel — except an injected group redirect, whose binding this
    /// entry already IS. Shared by spawn_external_process and the
    /// pipeline spawn loops, which build their stage processes by hand.
    #[cfg(unix)]
    pub(in crate::executor) fn ambient_fd_pins_for(
        &self,
        cmd: &crate::parser::CommandNode,
    ) -> Vec<(std::os::unix::io::RawFd, u32)> {
        let mut pins: Vec<(std::os::unix::io::RawFd, u32)> = Vec::new();
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed {
                continue;
            }
            if let Some(redirect) = cmd
                .redirects
                .iter()
                .find(|redirect| redirect.fd == Some(*fd))
            {
                // The command rebinds this slot itself (see the doc
                // comment); only the injected-group case falls through
                // and pins the entry that already is its binding.
                if !crate::executor::support_names::is_injected_group_redirect(redirect) {
                    continue;
                }
            }
            let mut take =
                |endpoint: Option<&Rc<crate::executor::fd_table::FileFd>>,
                 pins: &mut Vec<(std::os::unix::io::RawFd, u32)>| {
                    if let Some(file) = endpoint {
                        let pin = (file.handle, *fd);
                        if !pins.contains(&pin) {
                            pins.push(pin);
                        }
                    }
                };
            take(
                entry.read.as_ref().and_then(|read| match read {
                    crate::executor::fd_table::FdReadEndpoint::File(file) => Some(file),
                    crate::executor::fd_table::FdReadEndpoint::CoprocStdout { fd, .. } => Some(fd),
                    _ => None,
                }),
                &mut pins,
            );
            take(
                entry.write.as_ref().and_then(|write| match write {
                    crate::executor::fd_table::FdWriteEndpoint::File(file) => Some(file),
                    crate::executor::fd_table::FdWriteEndpoint::CoprocStdin { fd, .. } => Some(fd),
                    _ => None,
                }),
                &mut pins,
            );
        }
        pins
    }

    /// rubash#1688: register the ambient-fd pin on a stage process built
    /// by hand in the pipeline spawn loops (they bypass
    /// spawn_external_process, so without this a pipeline member sees
    /// EBADF on every ambient descriptor).
    #[cfg(unix)]
    pub(in crate::executor) fn attach_ambient_fd_pins(
        &self,
        cmd: &crate::parser::CommandNode,
        process: &mut std::process::Command,
    ) {
        let pins = self.ambient_fd_pins_for(cmd);
        if !pins.is_empty() {
            use std::os::unix::process::CommandExt;
            // SAFETY: same contract as the spawn_external_process
            // registration — the closure runs only in the forked child
            // before exec and only dup2/closes raw descriptors.
            unsafe {
                process.pre_exec(move || pin_ambient_fds_in_child(&pins));
            }
        }
    }

    #[cfg(not(unix))]
    pub(in crate::executor) fn attach_ambient_fd_pins(
        &self,
        _cmd: &crate::parser::CommandNode,
        _process: &mut std::process::Command,
    ) {
    }

    fn spawn_external_process(
        &mut self,
        cmd: &CommandNode,
        program: &PathBuf,
        mut process: Command,
        used_shell: bool,
        dev_ops: crate::executor::dev_fd_operands::DevOperandMaterialization,
        // rubash#1832: true when the caller composed the process
        // environment to finality itself (the `env` emulation — its
        // composition already folded the tail SHLVL adjustment in and
        // applied env's own -i/-u/assignments). The exec-replace seam
        // must not stamp SHLVL over such an environment after the
        // fact; the direct disk-command path composes from shell
        // state and passes false.
        env_composed_final: bool,
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
        // rubash#1686: on Unix each opened descriptor is also recorded as a
        // pin (merged into the pin registration below): the env channel
        // reaches only rubash children, so a foreign child otherwise gets
        // EBADF on the command's own fds even though the file was created.
        #[cfg(unix)]
        let mut command_fd_pins: Vec<(std::os::unix::io::RawFd, u32)> = Vec::new();
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
            if self.fd_table.entries.contains_key(&fd)
                && crate::executor::support_names::is_injected_group_redirect(redirect)
            {
                // The table entry already IS this redirect's binding (a
                // compound applied it to its real descriptors and injected
                // the mirror); re-opening would truncate the shared
                // description. The pin registration below delivers the
                // table entry instead (rubash#1686). A merely ambient
                // entry does NOT skip: the command's binding is the more
                // local one and must win the slot.
                continue;
            }
            let opened =
                if write {
                    if redirect.append {
                        open_redirect_append(&self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )))
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
            #[cfg(unix)]
            command_fd_pins.push((dup, fd));
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
        // rubash#1626: the env channel above only reaches rubash children
        // (init.rs restores the named handles at startup); a foreign child
        // — the /usr/bin/bash, cat, ls of the report — gets neither the
        // descriptor nor a usable number, and the table's handles are all
        // opened O_CLOEXEC, so nothing leaks through the raw fd space
        // either: every ambient fd bound by `exec N>file` / `exec N<file`
        // / `exec N<>file` fails EBADF there. GNU's fork simply inherits
        // the descriptors at their numbers. Mirror that on Unix with a
        // pre_exec pin (registered below); the pin table itself is
        // computed by ambient_fd_pins_for so hand-built pipeline stage
        // processes inherit the same descriptors (rubash#1688).
        #[cfg(unix)]
        let ambient_fd_pins: Vec<(std::os::unix::io::RawFd, u32)> = {
            let mut pins = self.ambient_fd_pins_for(cmd);
            // rubash#1686: the command's own redirect descriptors ride
            // the same registration, appended after the ambient pins so
            // a command binding wins any slot both sides name (the pin
            // helper dup2's in list order).
            pins.extend(command_fd_pins);
            pins
        };
        // rubash#229: restore GNU's child signal dispositions between fork
        // and exec (trap.c:1489 restore_original_signals) — without this a
        // policy/trap SIG_IGN in the shell survives exec and makes children
        // unkillable by that signal (sh -c 'kill -QUIT $$' exited 0).
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            process.pre_exec(crate::builtins::kill::child_pre_exec_reset);
        }
        // rubash#1626: pin the ambient descriptors at their slot numbers
        // (see the pin table built above). Registered before the /dev/fd
        // operand attach below so a command-scoped operand pin — the
        // most local binding — wins any slot collision.
        #[cfg(unix)]
        if !ambient_fd_pins.is_empty() {
            use std::os::unix::process::CommandExt;
            let pins = ambient_fd_pins;
            unsafe {
                process.pre_exec(move || pin_ambient_fds_in_child(&pins));
            }
        }
        // rubash#438: pin the buffered process-substitution pipe read ends at
        // their /dev/fd/N numbers in the child (fork-exec dup2), so the
        // literal argv word resolves through the OS /dev/fd layer like a
        // GNU fork.
        #[cfg(unix)]
        crate::executor::dev_fd_operands::attach_unix_dev_fd_operands(&mut process, &dev_ops);
        suppress_orphan_console(&mut process);
        // rubash#1497: GNU's nofork path (builtins/evalstring.c:126
        // should_suppress_fork, execute_cmd.c:5813
        // execute_disk_command): the last plain command of a -c string
        // REPLACES the shell process instead of forking. The Command
        // is fully prepared here (environment, stdio, fd channel,
        // pre_exec signal reset and /dev/fd pins), so exec it in
        // place: std's Command::exec runs the pre_exec closures
        // in-process, then execve; it returns only on failure, where
        // GNU's model has already committed — the shell diagnoses and
        // exits with the classified status rather than forking.
        #[cfg(unix)]
        if !used_shell && self.exec_replace_tail_ready(cmd, program, &dev_ops) {
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            // rubash#1832: the direct path composed this process
            // environment from the shell state, whose SHLVL is the
            // live level — stamp the adjusted one (GNU's
            // adjust_shell_level(-1) before maybe_make_export_env,
            // execute_cmd.c:5813-5817). An env-emulation process
            // (env_composed_final) already carries the composition's
            // final value: stamping here would resurrect a variable
            // env's -i/-u deliberately removed or supersede an
            // explicit assignment.
            if !env_composed_final {
                process.env("SHLVL", self.exec_replace_adjusted_shlvl());
            }
            use std::os::unix::process::CommandExt;
            let error = process.exec();
            let _ = self.report_external_spawn_error(cmd, program, error);
            std::process::exit(self.exit_code);
        }
        // rubash#1035: GNU shell_execve (execute_cmd.c:6126) execs the
        // program directly and only consults the file's first bytes when
        // the kernel refuses (ENOEXEC). Rust's spawn (posix_spawn) never
        // surfaces that refusal for scripts without `#!`: libc runs them
        // under /bin/sh instead — the WRONG shell (no `trap -p`, wrong
        // traps, wrong POSIX mode). Pre-empt the spawn for files the
        // kernel could not exec and feed the refusal classifier below a
        // synthetic ENOEXEC, so the ENOEXEC shell-script fallback (or the
        // bad-interpreter/binary refusal) applies exactly as if execve
        // had refused. The 4-byte read costs microseconds against the
        // millisecond spawn; `used_shell` spawns already run under a shell
        // and skip it. Unix-only: Windows routes scripts through the shell
        // fallback before reaching any spawn (should_run_with_shell), so a
        // magic sniff here would only misroute batch files and MZ images.
        // The synthetic refusal must be ENOEXEC specifically: a file
        // without any execute bit would fail execve with EACCES instead,
        // and GNU reports that as Permission denied / 126 with no
        // shell-script fallback (execute_cmd.c:6126-6159 shell_execve;
        // rubash#1459). Non-executable files therefore take the real
        // spawn so the genuine EACCES reaches the diagnostic path.
        let spawned = if !used_shell
            && cfg!(unix)
            && !crate::executor::path::program_execs_directly(program)
            && crate::executor::path::is_executable_file(program)
        {
            Err(std::io::Error::from_raw_os_error(8))
        } else {
            process.spawn()
        };
        // Drop the Command NOW: std retains every `Stdio::from(..)` it was
        // given (the #144 shared capture-pipe write clones among them), so a
        // still-live Command keeps the pipe's write end open in the parent
        // and the post-exit drain below would block on an EOF that cannot
        // arrive even after the child is gone. The child already holds its
        // own duplicated inheritable handles; GNU closes the parent's copies
        // right after the fork (redir.c do_redirection_internal runs in the
        // child).
        drop(process);
        match spawned {
            Ok(mut child) => {
                adopt_orphan_console(child.id());
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
                        // The feed itself must never fail the command or
                        // kill the shell when the child exits without
                        // reading (rubash#1822) — see feed_child_stdin.
                        crate::executor::shell_options::feed_child_stdin(
                            &mut stdin,
                            &crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                                &input,
                            ),
                        )?;
                    }
                }

                if let Some(outcome) = self.external_stdio_outcome.take() {
                    // niubash#144: the stdio plan already gave the child real
                    // handles; only the shared capture pipe needs collecting.
                    // Drain it to EOF first — the merged bytes stream in true
                    // write order through the ONE pipe both fds dup onto
                    // (GNU redir.c:1169-1170), unlike the retired two-pipe
                    // capture-replay.
                    let foreground_pid = child.id();
                    if let super::external_redirects::ExternalStdioOutcome::CapturePipe(
                        mut reader,
                    ) = outcome
                    {
                        use std::io::Read;
                        let mut merged = Vec::new();
                        reader.read_to_end(&mut merged)?;
                        let status = self.wait_external_child(&mut child)?;
                        if let Some(signal) =
                            crate::executor::wait_status::exit_status_signal(&status)
                        {
                            self.note_child_signal_death(
                                foreground_pid,
                                signal,
                                self.signal_notice_command_text(cmd),
                                true,
                            );
                        }
                        self.run_sigchld_trap_for_reaped_child()?;
                        // Route the merged stream into the active capture
                        // exactly like the legacy single-stream collection.
                        self.write_default_stdout(&merged)?;
                        self.exit_code = crate::executor::wait_status::process_exit_status(&status);
                    } else {
                        let status = self.wait_external_child(&mut child)?;
                        if let Some(signal) =
                            crate::executor::wait_status::exit_status_signal(&status)
                        {
                            self.note_child_signal_death(
                                foreground_pid,
                                signal,
                                self.signal_notice_command_text(cmd),
                                true,
                            );
                        }
                        self.exit_code = crate::executor::wait_status::process_exit_status(&status);
                        // A reaped foreground child delivers SIGCHLD in
                        // GNU bash; a set trap runs once at this boundary
                        // (trap8.sub).
                        self.run_sigchld_trap_for_reaped_child()?;
                    }
                } else if self.external_needs_fd_copy_capture(cmd) {
                    let foreground_pid = child.id();
                    self.note_foreground_wait();
                    match child.wait_with_output() {
                        Ok(output) => {
                            self.exit_code = 0;
                            // rubash#229: a foreground child killed by a
                            // signal gets the jobs.c notify_of_job_status
                            // stderr notice at the reader boundary.
                            if let Some(signal) =
                                crate::executor::wait_status::exit_status_signal(&output.status)
                            {
                                self.note_child_signal_death(
                                    foreground_pid,
                                    signal,
                                    self.signal_notice_command_text(cmd),
                                    true,
                                );
                            }
                            // A reaped foreground child delivers SIGCHLD in
                            // GNU bash; a set trap runs once at this
                            // boundary (trap8.sub).
                            self.run_sigchld_trap_for_reaped_child()?;
                            self.write_external_captured_streams(
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
                        Err(error) => self.report_external_spawn_error(cmd, program, error)?,
                    }
                } else {
                    let foreground_pid = child.id();
                    match self.wait_external_child(&mut child) {
                        Ok(status) => {
                            // rubash#229: queue the signal-death notice for
                            // the reader boundary.
                            if let Some(signal) =
                                crate::executor::wait_status::exit_status_signal(&status)
                            {
                                self.note_child_signal_death(
                                    foreground_pid,
                                    signal,
                                    self.signal_notice_command_text(cmd),
                                    true,
                                );
                            }
                            self.exit_code =
                                crate::executor::wait_status::process_exit_status(&status);
                            // A reaped foreground child delivers SIGCHLD in
                            // GNU bash; a set trap runs once at this boundary
                            // (trap8.sub).
                            self.run_sigchld_trap_for_reaped_child()?;
                        }
                        Err(ExecuteError::IoError(error)) => {
                            self.report_external_spawn_error(cmd, program, error)?
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
                // The failed spawn never consumed the stdio plan; drop it so
                // no later command mistakes it for its own.
                self.external_stdio_outcome = None;
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
                        // G-02: children are born in the session cwd.
                        shell_process.current_dir(&self.shell_state.cwd);
                        self.apply_external_environment(cmd, &mut shell_process, program);
                        self.apply_external_redirects(cmd, &mut shell_process)?;
                        return self.spawn_external_process(
                            cmd,
                            program,
                            shell_process,
                            true,
                            dev_ops,
                            env_composed_final,
                        );
                    }
                }
                self.report_external_spawn_error(cmd, program, error)?;
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
        program: &PathBuf,
        error: io::Error,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        // GNU report_prolog (errors.c:75-120): a script-context exec failure
        // prints `$0: line N: word: message` — the same prolog every other
        // diagnostic uses (rubash#231: the bare `rubash:` prefix dropped the
        // script/line attribution GNU prints). shell_execve reports via
        // internal_error, so the prefix honors gnu_errfmt (rubash#1038).
        // GNU shell_execve (execute_cmd.c:6126-6159) reports the execve
        // failure against the RESOLVED pathname (findcmd.c file_to_lose_on
        // fallback included), not the typed word — `noexec.sh` found via
        // PATH prints the full candidate path (rubash#231).
        //
        // The status is NOT uniformly EX_NOEXEC: shell_execve sets
        // last_command_exit_value = (errno == ENOENT) ? EX_NOTFOUND (127)
        // : EX_NOEXEC (126) (execute_cmd.c:6143). A command the PATH walk
        // resolved whose image the kernel then cannot load — an ELF whose
        // PT_INTERP loader is missing is the common case — fails execve
        // with ENOENT and must surface as a LOOKUP failure (127), not an
        // exec refusal (126): scripts branch on 126-vs-127 (rubash#1172).
        // The diagnostic follows the same switch: an existing executable
        // file with a #! line names its interpreter ("bad interpreter",
        // EX_NOEXEC); an existing executable non-#! file is "cannot
        // execute: required file not found" (EX_NOTFOUND); anything else
        // keeps the plain errno text via file_error.
        let (message, status) = spawn_error_diagnostic(program, &error);
        writeln!(
            &mut stderr,
            "{}{}: {}",
            self.error_prefix(),
            program.display(),
            message
        )?;
        self.finish_external_error(cmd, &stderr, status)
    }
}

#[cfg(unix)]
/// Holds untrapped SIGINT at the recording handler across one
/// foreground blocking wait when the shell is an interactive reader
/// session (kernel disposition is SIG_DFL). GNU jobs.c:3086-3102
/// wait_for swaps in wait_sigint_handler for the wait: the handler
/// only RECORDS the delivery (trap.c trap_handler ->
/// set_trap_state), so an interactive shell never dies mid-wait —
/// including when it shares the child's process group (no job
/// control) and its own copy of ^C arrives during waitpid
/// (rubash#1250). This used to install SIG_IGN:
/// the shell survived, but its own copy of the delivery vanished
/// unrecorded, leaving the line-discard verdict nothing to consult.
/// The hold is the same one the builtin-sleep guard uses
/// (kill::hold_fg_sigint): arrivals queue in the kernel backend while
/// FG_INT_HOLD keeps reconcile from flipping SIGINT back to Default
/// mid-wait; Drop releases the hold. Trapped SIGINT keeps its handler
/// (disposition != DFL) so trap recording is untouched, and
/// entry-ignored SIGINT stays ignored.
struct ForegroundWaitSigintGuard {
    armed: bool,
}

#[cfg(unix)]
impl ForegroundWaitSigintGuard {
    fn hold_if_interactive_untrapped(executor: &Executor) -> Self {
        if !foreground_sigint_needs_protection(executor) {
            return Self { armed: false };
        }
        crate::builtins::kill::hold_fg_sigint();
        Self { armed: true }
    }

    pub(in crate::executor) fn is_armed(&self) -> bool {
        self.armed
    }
}

/// Shared predicate for both foreground SIGINT guards (rubash#1250):
/// an interactive reader session and SIGINT is untrapped, with the
/// kernel disposition in one of the two untrapped steady states —
/// SIG_DFL, or the mailbox handler the rubash#1662 interactive pin
/// installs from the command boundary on (GNU sig.c:315-316: an
/// interactive shell catches SIGINT itself from startup). Before the
/// pin, SIG_DFL alone proved "untrapped"; the pin made that test
/// read Handler for every untrapped interactive SIGINT, the guards
/// never armed, and deliveries queued in the mailbox with no consumer
/// during in-process blocking waits — a `read` ate the next input
/// line and a subshell frame's virtual child slept through the ^C
/// (rubash#1824). A handler alone still does not arm the guard: the
/// trap table decides ownership — a trapped SIGINT (any table entry,
/// including an ignore) keeps the trap path, and an entry-ignored
/// SIGINT (SIG_IGN, no handler) stays untouched.
/// rubash#1639: the session test is interactive_reader_session, not
/// the full expansion-discard rule — the discard rule's frame
/// conditions (depth 0, no comsub body) govern where an expansion
/// error is FATAL, while a foreground wait needs protection at any
/// depth: in the flat model the subshell frame's wait is the
/// process's foreground wait, and GNU protects the corresponding
/// wait_for (the parent's, for the subshell job) identically.
#[cfg(unix)]
pub(in crate::executor) fn foreground_sigint_needs_protection(executor: &Executor) -> bool {
    if !executor.interactive_reader_session() {
        return false;
    }
    if sigint_disposition_is_default() {
        return true;
    }
    crate::builtins::kill::kernel_handler_installed(libc::SIGINT)
        && crate::builtins::trap::get_trap_action(&executor.shell_state.env_vars, "SIGINT")
            .is_none()
}

#[cfg(unix)]
fn sigint_disposition_is_default() -> bool {
    // sigaction(null) queries without installing (same probe as
    // kill.rs entry-ignored snapshot): only SIG_DFL means "no trap
    // installed, would die" — a handler (trapped) or SIG_IGN
    // (entry-ignored) must be left alone.
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigaction(libc::SIGINT, std::ptr::null(), &mut current) } != 0 {
        return false;
    }
    current.sa_sigaction == libc::SIG_DFL
}

#[cfg(unix)]
impl Drop for ForegroundWaitSigintGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        crate::builtins::kill::release_fg_sigint();
    }
}

/// SIGINT guard for builtins that block in-process (rubash#1250: the
/// `sleep` fast path, which is what the issue's `sleep 30` repro actually
/// exercises — `sleep` is a rubash builtin). SIG_IGN would be WRONG here:
/// GNU's sleep is an external child that DIES from the pgroup SIGINT and
/// the shell reports 130, while an ignored SIGINT would let the builtin
/// sleep run to completion with rc=0. So this guard routes untrapped SIGINT
/// to the mailbox handler and HOLDS it there (kill::hold_fg_sigint) for
/// the wait: the delivery becomes an observable pending entry, the slice
/// loop's observe_signals_while_blocked converts it to Err(ExitCode(130))
/// via the same untrapped path external deaths take, and Drop restores
/// Default. Trapped SIGINT keeps its handler (predicate false, mailbox
/// path untouched); non-interactive shells keep SIG_DFL and die like GNU
/// scripts. (A pthread_sigmask BLOCK was tried first and reverted: the
/// process is multithreaded, so a per-thread mask cannot stop delivery
/// to the DFL main thread — verified by probe, guard armed yet shell
/// still died.)
#[cfg(unix)]
pub(in crate::executor) struct ForegroundBuiltinSigintGuard {
    armed: bool,
}

#[cfg(unix)]
impl ForegroundBuiltinSigintGuard {
    pub(in crate::executor) fn hold_if_interactive_untrapped(executor: &Executor) -> Self {
        if !foreground_sigint_needs_protection(executor) {
            return Self { armed: false };
        }
        crate::builtins::kill::hold_fg_sigint();
        Self { armed: true }
    }

    pub(in crate::executor) fn is_armed(&self) -> bool {
        self.armed
    }

    /// Consume a pending untrapped SIGINT for this wait (true = caller aborts
    /// with 128+SIGINT). Gated on armed: a TRAPPED SIGINT must flow through
    /// observe_signals_while_blocked so its action runs — stealing it here
    /// would silently drop the trap. kernel_handler_installed rejects stale
    /// registry flags (same stale rule observe uses); anything else goes
    /// back for the trapped-abort check and observe.
    pub(in crate::executor) fn take_sigint_abort(&self) -> bool {
        if !self.armed {
            return false;
        }
        let pending =
            crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
        if pending.is_empty() {
            return false;
        }
        let mut rest = Vec::new();
        let mut interrupted = false;
        for signal in pending {
            // rubash#1640: only a KERNEL-caused SIGINT (the terminal
            // driver's group delivery, ^C) kills the virtual child —
            // that is the delivery GNU's real foreground child dies
            // from. A kill(2) addressed to the shell alone leaves the
            // real child running, so it is not an abort: put it back
            // for observe_signals_while_blocked, which consumes it
            // without ending the wait (the recorded interrupt is
            // forgotten when the child completes normally, exactly
            // like the trapped path's origin check in
            // sleep_interruptibly).
            if signal == libc::SIGINT
                && crate::builtins::kill::kernel_handler_installed(signal)
                && crate::builtins::kill::last_delivery_kernel_caused(signal)
            {
                interrupted = true;
            } else {
                rest.push(signal);
            }
        }
        crate::builtins::kill::requeue_pending_signals(rest);
        interrupted
    }
}

#[cfg(unix)]
impl Drop for ForegroundBuiltinSigintGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        crate::builtins::kill::release_fg_sigint();
    }
}

/// GNU shell_execve (execute_cmd.c:6126-6159) classification of a failed
/// execve/spawn against the resolved program path. Returns the diagnostic
/// body (the caller prepends the report prolog and the resolved pathname)
/// and the exit status. Non-ENOENT failures are EX_NOEXEC with the plain
/// errno text, exactly the historical behavior of this call site.
fn spawn_error_diagnostic(program: &PathBuf, error: &io::Error) -> (String, i32) {
    const EX_NOTFOUND: i32 = 127;
    const EX_NOEXEC: i32 = 126;
    // ENOENT on unix and ERROR_FILE_NOT_FOUND on Windows share the value 2;
    // no libc dependency is needed for this classification.
    const OS_ERROR_FILE_NOT_FOUND: i32 = 2;
    if error.raw_os_error() != Some(OS_ERROR_FILE_NOT_FOUND) {
        return (crate::posix_errors::message(error).to_string(), EX_NOEXEC);
    }
    // errno == ENOENT: the status is EX_NOTFOUND unless the file itself
    // proves the #! bad-interpreter refusal (EX_NOEXEC).
    let metadata = std::fs::metadata(program).ok();
    let executable = metadata
        .as_ref()
        .is_some_and(|meta| meta.is_file() && file_has_exec_bit(meta));
    if executable {
        if let Ok(sample) = std::fs::read(program) {
            if sample.len() >= 2 && sample[0] == b'#' && sample[1] == b'!' {
                let line_end = sample
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .unwrap_or(sample.len());
                let line = String::from_utf8_lossy(&sample[2..line_end]);
                // GNU getinterp (execute_cmd.c) terminates the interpreter
                // name at a blank or tab only — a CR stays part of the
                // name (and is displayed as ^M below), so do not split on
                // the full whitespace class here.
                let interp = line.split([' ', '\t']).next().unwrap_or_default();
                if !interp.is_empty() {
                    // GNU getinterp + sys_error (execute_cmd.c:6170-6184):
                    // a trailing CR in the interpreter name is displayed
                    // as ^M (`#!/bin/sh\r` -> "/bin/sh^M").
                    let interp = match interp.strip_suffix('\r') {
                        Some(stem) => format!("{stem}^M"),
                        None => interp.to_string(),
                    };
                    return (
                        format!("{interp}: bad interpreter: No such file or directory"),
                        EX_NOEXEC,
                    );
                }
            }
        }
        // An executable file the kernel cannot load at all (missing
        // PT_INTERP loader): execute_cmd.c:6188-6192 internal_error.
        return (
            "cannot execute: required file not found".to_string(),
            EX_NOTFOUND,
        );
    }
    (crate::posix_errors::message(error).to_string(), EX_NOTFOUND)
}

/// GNU executable_file (execute_cmd.c / findcmd.c file_status FS_EXECABLE):
/// any execute permission bit set. Non-unix hosts have no mode bits; an
/// existing regular file counts as executable there.
fn file_has_exec_bit(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
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
    // Kept in argument order: coreutils env applies its NAME=value
    // operands in order (later operands supersede in place), and the
    // composed environment's print order exposes it (rubash#1806).
    assignments: Vec<(String, String)>,
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

/// Insert into an ordered environment composition with GNU
/// add_or_supercede semantics (variables.c): an existing name is
/// replaced in place, a new name is appended (rubash#1806).
fn ordered_env_insert(entries: &mut Vec<(String, String)>, name: String, value: String) {
    if let Some(slot) = entries.iter_mut().find(|(existing, _)| *existing == name) {
        slot.1 = value;
    } else {
        entries.push((name, value));
    }
}

fn ordered_env_contains(entries: &[(String, String)], name: &str) -> bool {
    entries.iter().any(|(existing, _)| existing == name)
}

fn ordered_env_get<'a>(entries: &'a [(String, String)], name: &str) -> Option<&'a String> {
    entries
        .iter()
        .find(|(existing, _)| existing == name)
        .map(|(_, value)| value)
}

fn ordered_env_remove(entries: &mut Vec<(String, String)>, name: &str) {
    entries.retain(|(existing, _)| existing != name);
}

fn apply_env_command_environment(
    process: &mut Command,
    env_vars: &[(String, String)],
    inherit_required_windows: bool,
) {
    process.env_clear();
    let env_map: HashMap<String, String> = env_vars.iter().cloned().collect();
    for (name, value) in env_vars {
        if is_valid_process_env(name, value) {
            let value = if cfg!(windows) && name.eq_ignore_ascii_case("PATH") {
                shell_path_to_process(value, &env_map)
            } else if cfg!(windows) && name == "TMPDIR" {
                // Same child-boundary contract as child_env_value
                // (unixwin/niubash#162): exported TMPDIR crosses in
                // Windows-native forward-slash form, never the slash-drive
                // `/c/...` display form that foreign children (Bun-compiled
                // executables) resolve drive-relative and fail on.
                shell_path_to_windows(value, &env_map)
                    .to_string_lossy()
                    .replace('\\', "/")
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
        apply_required_windows_child_environment(process, &env_map);
    }
}

#[cfg(windows)]
fn materialize_required_windows_env(
    env_vars: &mut Vec<(String, String)>,
    shell_env_vars: &HashMap<String, String>,
    ignore_environment: bool,
) {
    if ignore_environment {
        return;
    }

    for name in ["SystemRoot", "WINDIR", "ComSpec"] {
        if ordered_env_contains(env_vars, name) {
            continue;
        }
        if let Some(value) = shell_env_vars
            .get(name)
            .cloned()
            .or_else(|| env::var(name).ok())
        {
            ordered_env_insert(env_vars, name.to_string(), value);
        }
    }

    let home = ordered_env_get(env_vars, "USERPROFILE")
        .cloned()
        .or_else(|| shell_env_vars.get("USERPROFILE").cloned())
        .or_else(|| env::var("USERPROFILE").ok())
        .or_else(|| {
            ordered_env_get(env_vars, "HOME")
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
    if !ordered_env_contains(env_vars, "USERPROFILE") {
        ordered_env_insert(env_vars, "USERPROFILE".to_string(), native_home.clone());
    }
    if !ordered_env_contains(env_vars, "HOME") {
        ordered_env_insert(env_vars, "HOME".to_string(), native_home.clone());
    }
    if let Some((drive, path)) = windows_drive_and_home_path(&native_home) {
        if !ordered_env_contains(env_vars, "HOMEDRIVE") {
            ordered_env_insert(env_vars, "HOMEDRIVE".to_string(), drive);
        }
        if !ordered_env_contains(env_vars, "HOMEPATH") {
            ordered_env_insert(env_vars, "HOMEPATH".to_string(), path);
        }
    }
    let base = native_home.trim_end_matches('\\');
    if !ordered_env_contains(env_vars, "APPDATA") {
        ordered_env_insert(
            env_vars,
            "APPDATA".to_string(),
            format!("{base}\\AppData\\Roaming"),
        );
    }
    if !ordered_env_contains(env_vars, "LOCALAPPDATA") {
        ordered_env_insert(
            env_vars,
            "LOCALAPPDATA".to_string(),
            format!("{base}\\AppData\\Local"),
        );
    }
}

#[cfg(not(windows))]
fn materialize_required_windows_env(
    _env_vars: &mut Vec<(String, String)>,
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

/// Whether execve(2) would refuse `program` with ENOEXEC: a regular
/// file whose first bytes are neither a `#!` interpreter line nor the
/// ELF magic of a native image (the only two formats the kernel takes
/// directly on modern Linux). Directories and unreadable files are
/// left to the normal spawn-error classification.
#[cfg(unix)]
pub(in crate::executor) fn is_enoexec_candidate(program: &std::path::Path) -> bool {
    use std::io::Read;
    if !std::fs::metadata(program).is_ok_and(|meta| meta.is_file()) {
        return false;
    }
    // The ENOEXEC shell-script fallback (execute_cmd.c:6139-6233
    // shell_execve) is only reached when execve actually fails with
    // ENOEXEC. A file without any execute bit fails earlier with
    // EACCES, and GNU reports it as Permission denied / exit 126
    // (findcmd.c:591/695 keep such files out of the runnable set the
    // same way). Without this gate a non-executable text file was run
    // as a script with status 0 (rubash#1459).
    if !crate::executor::path::is_executable_file(program) {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(program) else {
        return false;
    };
    let mut head = [0u8; 4];
    let read = file.read(&mut head).unwrap_or(0);
    let head = &head[..read];
    !head.starts_with(b"#!") && !head.starts_with(b"\x7fELF")
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
    // Mirror path.rs has_path_separator (general.c:843 absolute_program):
    // the backslash is a separator only on Windows/MSYS hosts; on unix
    // `a\b` is an ordinary name that goes through the PATH search and
    // earns "command not found", not the execve classification
    // (rubash#1482).
    let has_separator = if cfg!(windows) {
        name.contains('/') || name.contains('\\')
    } else {
        name.contains('/')
    };
    if !has_separator {
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
