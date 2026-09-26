use super::*;
use crate::executor::ast_exec::is_closed_output_io_error;
use crate::executor::markers::DATA_DOLLAR_STR;

impl Executor {
    pub(in crate::executor) fn write_cat_output(
        &mut self,
        cmd: &CommandNode,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if self.has_output_fd_target(&target) {
                self.write_output_fd_redirect(&target, output)?;
                return Ok(());
            }
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            file.write_all(output)?;
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_redirect_target(redirect);
            if self.has_output_fd_target(&target) {
                self.write_output_fd_redirect(&target, output)?;
                return Ok(());
            }
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
            file.write_all(output)?;
        } else {
            self.write_default_stdout(output)?;
        }
        Ok(())
    }

    pub(in crate::executor) fn finish_external_error(
        &mut self,
        cmd: &CommandNode,
        stderr: &[u8],
        status: i32,
    ) -> Result<(), ExecuteError> {
        self.write_buffered_builtin_output(cmd, &[], stderr)?;
        self.exit_code = status;
        Ok(())
    }

    pub(in crate::executor) fn execute_same_shell_script(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        // TODO(execute_cmd.c/shell.c/input.c): Bash forks a new shell process
        // here while preserving the underlying input stream for redirected
        // stdin. On Windows test runs, launching the wrapper loses the next
        // stdin line before `read` can consume it, so execute the same Rubash
        // script in-process for tests/input-line.sh.
        let Some(command_name) = cmd.words.first() else {
            return Ok(false);
        };
        // GNU execute_cmd.c:6139 shell_execve decides same-shell by path
        // equivalence, not by the command text naming THIS_SH — after
        // expansion `${THIS_SH}` resolves to the same path, while a literal
        // `THIS_SHxx` word must not match.
        let expanded_command_name = self.expand_word(command_name);
        let expanded_is_this_shell =
            self.shell_state
                .env_vars
                .get("THIS_SH")
                .is_some_and(|this_sh| {
                    shell_path_to_windows(this_sh, &self.shell_state.env_vars)
                        == shell_path_to_windows(&expanded_command_name, &self.shell_state.env_vars)
                });
        if !expanded_is_this_shell {
            if let Some(script_path) =
                direct_windows_shell_script_path(&expanded_command_name, &self.shell_state.env_vars)
            {
                // GNU execute_cmd.c:6139-6233: a file the OS cannot exec
                // directly is classified by its first bytes before the
                // shell-script fallback runs; an unresolvable #! interpreter
                // ("bad interpreter") or a binary first line ("cannot
                // execute binary file") is refused with exit status 126.
                if let Some((diagnostic, status)) = self.exec_format_refusal(cmd, &script_path) {
                    let mut stderr = Vec::new();
                    let _ = writeln!(&mut stderr, "{diagnostic}");
                    self.finish_external_error(cmd, &stderr, status)?;
                    return Ok(true);
                }
                self.execute_direct_shell_script(cmd, &expanded_command_name, &script_path, false)?;
                return Ok(true);
            }
        }
        // A nested same-shell script still needs to run in-process when its
        // parent is consuming virtual stdin (for example a script supplied
        // through `< input-line.sh`).  Spawning the wrapper in that case
        // loses the unread portion of the parent's input stream.  Keep the
        // recursion guard for ordinary nested scripts, where no virtual
        // input needs to be transferred.
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_SCRIPT_NAME")
            && !self.shell_state.env_vars.contains_key(FUNCTION_STDIN)
            && self.fd_table.input_snapshot(0).is_none()
            && !expanded_is_this_shell
        {
            return Ok(false);
        }
        let command_name = expanded_command_name;
        let normalized_command = shell_display_path(&command_name).replace('\\', "/");
        let normalized_current_exe = env::current_exe()
            .ok()
            .map(|path| shell_display_path(&path.to_string_lossy()).replace('\\', "/"));
        if !expanded_is_this_shell
            && normalized_current_exe.as_deref() != Some(normalized_command.as_str())
            && !normalized_command.ends_with("/rubash-wrapper")
            && normalized_command != "rubash-wrapper"
        {
            return Ok(false);
        }

        let Some(script) = cmd.words.get(1) else {
            return Ok(false);
        };
        let script = self.expand_word(script);
        let script_path = shell_path_to_windows(&script, &self.shell_state.env_vars);
        if !script_path.is_file() {
            return Ok(false);
        }
        self.execute_direct_shell_script(cmd, &script, &script_path, false)?;
        Ok(true)
    }

    /// GNU execute_cmd.c:6237-6260 (shell_execve ENOEXEC tail): a text file
    /// the kernel refuses to exec is run by the forked child as THIS shell,
    /// in-process — args[0] = shell_name, then `sh_longjmp
    /// (subshell_top_level, 1)` lands in shell.c:429-464 where main()
    /// restarts with `shell_reinitialize()`. The script therefore runs as a
    /// FRESH shell (only the exported environment carries over; unexported
    /// variables and functions are gone) while kernel-preserved SIG_IGN
    /// dispositions survive and become SIG_HARD_IGNORE
    /// (trap.c set_signal). It is not /bin/sh. `used_shell` callers and the
    /// Windows mailbox keep the find_shell spawn path (runtime cfg!(unix)
    /// gate at the call site).
    pub(in crate::executor) fn execute_enoexec_shell_script(
        &mut self,
        cmd: &CommandNode,
        program: &std::path::Path,
    ) -> Result<(), ExecuteError> {
        // $0 keeps the invoked spelling (`./trap2.sub`), like
        // shell.c:720 dollar_vars[0] = shell_script_filename.
        let Some(command_name) = cmd.words.first() else {
            return Ok(());
        };
        let script = self.expand_word(command_name);
        self.execute_direct_shell_script(cmd, &script, program, true)
    }

    fn execute_direct_shell_script(
        &mut self,
        cmd: &CommandNode,
        script: &str,
        script_path: &std::path::Path,
        exec_model_entry: bool,
    ) -> Result<(), ExecuteError> {
        let source = fs::read_to_string(script_path)?;
        // bashhist.c pre_process_line: a fresh `bash script` child expands
        // history per input line, so an in-process THIS_SH child whose
        // script enables history must run the grouped history driver rather
        // than one whole-file parse. Gate on the exec-model child only: a
        // fork-model script child inherits the parent's history list, which
        // the driver's fresh session would discard.
        let uses_history_driver = crate::script_driver::script_uses_history(&source)
            || crate::script_driver::script_uses_aliases(&source);
        let mut ast = if uses_history_driver {
            crate::parser::Ast {
                commands: Vec::new(),
            }
        } else {
            let tokens = crate::lexer::tokenize(&source);
            // GNU parse.y push_heredoc -> report_syntax_error + exit_shell
            // (EX_BADUSAGE): more than HEREDOC_MAX (16) here-documents is fatal.
            // This in-process child path mirrors the same check in
            // script_driver::run_source_with_line_offset.
            if let Some(line) = crate::lexer::heredoc_overflow_line() {
                let mut stderr = Vec::new();
                let _ = writeln!(
                    &mut stderr,
                    "{script}: line {line}: maximum here-document count exceeded"
                );
                self.finish_external_error(cmd, &stderr, 2)?;
                return Ok(());
            }
            crate::parser::parse(&tokens)
        };
        self.apply_command_output_redirects(cmd, &mut ast)?;
        // A `${THIS_SH} script 3>&1`-style invocation is a process boundary:
        // the child's fd table is the parent's copy, so the command's
        // numbered output redirections bind real slots for the child's whole
        // run (redir.c do_redirection_internal) — restored wholesale with
        // saved_fd_table below.
        let _bound_output_fds = self.open_compound_output_redirects(cmd)?;

        let saved_env = self.shell_state.env_vars.clone();
        let this_shell_invocation = cmd.words.first().is_some_and(|command| {
            self.shell_state
                .env_vars
                .get("THIS_SH")
                .is_some_and(|this_sh| {
                    shell_path_to_windows(this_sh, &self.shell_state.env_vars)
                        == shell_path_to_windows(
                            &self.expand_word(command),
                            &self.shell_state.env_vars,
                        )
                })
        });
        // The exec-model child semantics (fresh shell: exported env only,
        // empty job table, EXIT trap runs at child exit) apply both to
        // `${THIS_SH} script` invocations and to the ENOEXEC fallback
        // (execute_enoexec_shell_script). The word shape still differs:
        // THIS_SH words are [shell, script, params...] while an ENOEXEC
        // command is [script, params...], so param_start below stays keyed
        // on this_shell_invocation.
        let fresh_shell = exec_model_entry || this_shell_invocation;
        // Save parent state BEFORE the fresh_shell block clears it.
        let saved_shell_state = fresh_shell.then(|| self.shell_state.clone_for_child_save());
        // GNU execute_cmd.c:6139-6233 / jobs.c: a script child is a separate
        // process, so its job table is process-local — a fresh exec child
        // starts with an empty table, and a fork-model child only inherits a
        // private copy whose additions die with it. In-process emulation
        // shares the parent's table, so snapshot it and clear the child's
        // view (exec model), then restore the parent's entries afterwards
        // regardless of mode. Without this, jobs started inside
        // `${THIS_SH} script` children leak into the parent table and later
        // `wait %N`/`jobs` block on or report them (jobs.tests hang).
        let saved_coproc_names = self.shell_state.coproc_names.clone();
        let saved_last_background_pid = self.shell_state.last_background_pid;
        let saved_functions = self.shell_state.functions.clone();
        let saved_function_redirects = self.shell_state.function_definition_redirects.clone();
        let saved_function_def_infos = self.shell_state.function_def_infos.clone();
        let saved_aliases = self.shell_state.aliases.clone();
        // A child script is a process boundary: its tempenv stack (POSIX-mode
        // persistent `var=2 :` bindings included) is process-local in GNU and
        // must not leak back into the parent's variable context. Save the
        // parent's tempenv bookkeeping; the exec-mode child starts fresh
        // (initialize_shell_variables), the fork-mode child inherits a copy.
        let saved_tempenv_names = self.tempenv_names.clone();
        let saved_tempenv_marks = self.tempenv_marks.clone();
        let saved_tempenv_promoted = self.tempenv_promoted_names.clone();
        let saved_tempenv_previous = self.tempenv_previous.clone();
        if fresh_shell {
            self.tempenv_names.clear();
            self.tempenv_marks.clear();
            self.tempenv_promoted_names.clear();
            self.tempenv_previous.clear();
        }
        if fresh_shell {
            let mut child_env = self.child_shell_environment();
            // GNU variables.c:511-526 (initialize_shell_variables): a fresh
            // shell rebuilds its managed variables (BASH_CMDS/BASH_ALIASES
            // assoc marks, FUNCNAME/DIRSTACK array marks, BASH_VERSINFO
            // readonly, UID/EUID/PPID, SHELLOPTS/BASHOPTS replay, SHLVL+1,
            // IFS default). Without this the in-process child fell back to
            // indexed-array handling for ${!BASH_CMDS[@]} (assoc audit C1).
            Self::initialize_fresh_shell_env_vars(&mut child_env);
            // The in-process child shares the parent's OS process, so the
            // child's PPID is the parent shell's pid (a real child would see
            // getppid() == the parent shell's getpid()).
            child_env.insert("PPID".to_string(), self.shell_pid.to_string());
            self.shell_state.env_vars = child_env;
            self.shell_state.variables =
                crate::shell::VariableStore::from_environment(&self.shell_state.env_vars);
            // GNU variables.c:511-526 (initialize_shell_variables): a fresh
            // shell invocation inherits only exported variables and exported
            // functions (via BASH_FUNC_<name>%% env vars). Non-exported
            // functions, aliases, and shell-local state are NOT inherited.
            // Clear the parent's functions/aliases and import only the
            // exported ones from the child environment.
            let (imported_funcs, imported_def_infos) =
                import_exported_functions_from_env(&self.shell_state.env_vars);
            self.shell_state.functions = imported_funcs;
            self.shell_state.function_definition_redirects = HashMap::new();
            self.shell_state.function_def_infos = imported_def_infos;
            self.shell_state.aliases = HashMap::new();
            // Exec-model child (fresh process): empty job table — the
            // registry itself is emptied by the mem::take below; here only
            // the non-registry bookkeeping is reset.
            self.shell_state.coproc_names.clear();
            self.shell_state.last_background_pid = None;
            // A fresh shell invocation entering a script derives
            // SIG_HARD_IGNORE from the inherited dispositions (trap.c
            // ignore_signal: "A signal ignored on entry to the shell cannot
            // be trapped or reset, but no error is reported"). Runtime
            // ignores of plain subshells stay mutable; only this
            // shell-entry boundary freezes them.
            // A spawned rubash child seeds its startup trap table in
            // Executor::new (init.rs seed_startup_traps): orig-ignored
            // entries are rebuilt and the WSL-inherited SIGRTMIN ignore is
            // added, so `trap` in the child lists it. The in-process child
            // takes the same fresh-shell boundary and must seed identically,
            // or varenv22's last `trap` loses the SIGRTMIN line.
            // The parent's runtime SIG_IGN dispositions (empty trap
            // actions) cross this boundary in GNU — execve preserves
            // ignored dispositions, and the ENOEXEC fork never restores
            // them — so transport them the same way the real-spawn path
            // does (readonly_functions.rs apply_external_environment) and
            // let seed_startup_traps rebuild the `trap -- '' SIG` entries
            // (trap1.sub: `trap -p USR2` lists the parent's `trap '' USR2`
            // and refuses to re-trap it). saved_env is the parent's
            // pre-swap environment: the parent's '' trap keys are already
            // gone from env_vars at this point.
            let inherited_ignores = crate::builtins::trap::transport_inherited_ignores(&saved_env);
            if !inherited_ignores.is_empty() {
                self.shell_state.env_vars.insert(
                    crate::builtins::trap::TRAP_ORIG_IGNORES.to_string(),
                    inherited_ignores,
                );
            }
            crate::builtins::trap::seed_startup_traps(&mut self.shell_state.env_vars);
            crate::builtins::trap::mark_startup_ignores(&mut self.shell_state.env_vars);
        }
        let saved_pipestatus = self.shell_state.pipestatus.clone();
        let saved_positional_params = self.shell_state.positional_params.clone();
        let saved_bash_source_stack = self.shell_state.bash_source_stack.clone();
        let saved_bash_lineno_stack = self.shell_state.bash_lineno_stack.clone();
        let saved_bash_argc_stack = self.shell_state.bash_argc_stack.clone();
        let saved_bash_argv_stack = self.shell_state.bash_argv_stack.clone();
        let saved_cwd = env::current_dir().ok();
        // GNU execute_cmd.c:6139-6233: a ${THIS_SH} script invocation is a
        // fresh shell process, not a subshell. subshell_depth must NOT be
        // incremented, or run_sigchld_trap_for_reaped_child suppresses
        // SIGCHLD traps (trap8.sub: four CHLD firings for reaped children).
        let saved_depth = self.shell_state.subshell_depth.get();
        // The child is a fresh shell process (shell.c open_shell_script):
        // it must not inherit the parent's loop/function/compound-condition
        // depths, or a word-expansion failure inside the child unwinds past
        // its own top level (ast_exec ExpansionFailure requires
        // loop_depth==0 to be command-list-local) and kills the child's
        // remaining commands instead of just skipping the line.
        let saved_loop_depth = self.shell_state.loop_depth;
        let saved_function_depth = self.shell_state.function_depth;
        let saved_inside_compound_condition = self.inside_compound_condition.get();
        self.shell_state.loop_depth = 0;
        self.shell_state.function_depth = 0;
        self.inside_compound_condition.set(false);
        // The child is a separate process in GNU: an evalerror/DISCARD
        // abort pending at child exit dies with it and must not leak back
        // into the parent's reader (nameref18.sub `ref[foo]=bar` under
        // ${THIS_SH} discarded the parent's remaining same-line commands).
        let saved_evalerror_pending = self.evalerror_pending.get();
        let saved_evalerror_line = self.evalerror_line.get();
        let saved_reader_command_line = self.reader_command_line.get();
        let saved_parameter_assignment_failure = self.parameter_assignment_failure.get();
        // evalerror_exec_depth counts execute_ast_inner nesting: the child
        // is a fresh reader (its own process in GNU), so a pending abort
        // inside it discards only same-line commands rather than unwinding
        // the whole child AST as a nested list.
        let saved_evalerror_exec_depth = self.evalerror_exec_depth.get();
        self.evalerror_exec_depth.set(0);
        self.evalerror_pending.set(false);
        self.evalerror_line.set(None);
        self.parameter_assignment_failure.set(false);
        // A parse error inside the child dies with it (GNU shell.c: the
        // child exits 2 and the parent's reader is untouched). The flag
        // lives on Executor outside shell_state, so save/reset/restore it
        // here — a stale parent value would otherwise abort the child's
        // first grouped command, and a child error would leak back into
        // the parent's next parse check (heredoc3.sub -> heredoc10.sub).
        let saved_parse_error = self.parse_error_occurred;
        self.parse_error_occurred = false;
        // jump_to_top_level unwinding is also per-process: a pending flag
        // inherited by the child would abort its first grouped command, and
        // a jump inside the child (comsub6.sub's parse error exits 2) must
        // not break the parent's next group (comsub7.sub printed nothing).
        let saved_exit_jump_pending = self.exit_jump_pending.replace(false);
        let saved_comsub_parse_error = self.last_command_substitution_parse_error.get();
        self.last_command_substitution_parse_error.set(false);
        // GNU's this_command_name belongs to the executing command only;
        // the in-process ${THIS_SH} child is a fresh shell whose own
        // commands set their own command name, so the parent's word (the
        // expanded shell path) must not leak into child diagnostics
        // (nameref8.sub warnings reported `rubash.exe:` prologs).
        let saved_assignment_command_name = self.assignment_command_name.take();

        // The fd table is a process boundary too: a child's persistent
        // `exec 2>/dev/null` (redir.c do_redirection_internal without undo)
        // dies with the real child process, but the in-process emulation
        // would leak it into the parent's table and silence every later
        // diagnostic (jobs1.sub's trailing `exec 2>/dev/null` ate all
        // parent stderr). Restore the parent's table; the child's entries
        // drop — and close — like a real exit. The snapshot must precede
        // the file_stdin install below: GNU applies `< file` in the CHILD
        // (do_redirections after fork), so the parent's fd 0 slot is the
        // pre-redirect state — restoring it also unwinds our emulation.
        let saved_fd_table = self.fd_table.clone();
        // GNU execute_cmd.c redirects before the child runs: `< file` puts
        // the open file on the child's fd 0, so `exec 3<&0` inside the child
        // dups the file (shared offset). Install a real File endpoint on
        // fd 0 for plain file redirects; text sources (heredoc/here-string)
        // keep the FUNCTION_STDIN channel.
        let file_stdin = cmd.redirect_in.as_ref().and_then(|redirect| {
            if redirect.fd.unwrap_or(0) != 0 {
                return None;
            }
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target)
                || redirect_target_fd(&target).is_some()
                || is_null_device(&target)
                || target.starts_with('<')
            {
                return None;
            }
            let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
            FileFd::open_read(path).ok()
        });
        if let Some(file) = file_stdin {
            self.set_fd_input_file(0, file, false);
            self.shell_state.env_vars.remove(FUNCTION_STDIN);
            self.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
            self.shell_state.env_vars.remove(INHERIT_PROCESS_STDIN);
        } else if let (Some(input), _, _) = self.function_call_stdin(cmd)? {
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN.to_string(), input);
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
            self.shell_state.env_vars.remove(INHERIT_PROCESS_STDIN);
        } else {
            self.shell_state
                .env_vars
                .insert(INHERIT_PROCESS_STDIN.to_string(), "1".to_string());
        }
        // Process-side job state, taken only after the last fallible call
        // above — an early `?` return must not strand the parent's job
        // handles. The exec-model child gets an empty JobTable; the
        // fork-model child sees a copy of the parent's (GNU subshell
        // semantics). background_children holds real Child handles that
        // cannot be duplicated — parking the map for the duration means the
        // child cannot try_wait/reap the parent's processes (a GNU child's
        // wait only covers its own children), and the child's own spawned
        // processes are orphaned on restore like real grandchildren.
        let saved_job_table = if fresh_shell {
            std::mem::take(&mut self.shell_state.job_table)
        } else {
            self.shell_state.job_table.clone()
        };
        let saved_background_children = std::mem::take(&mut self.background_children);
        // A ${THIS_SH} child is a process boundary for signal delivery too:
        // the pending-signal mailbox is keyed by process id, which the
        // in-process child shares with the parent, so signals queued for
        // the parent would otherwise be consumed by the child's
        // run_pending_signal_traps. Park the parent's queue for the run;
        // signals delivered while the child lives belong to the emulated
        // child process and are discarded at restore below (GNU: the real
        // child's pending queue dies with its pid — a signal like
        // jobs9.sub's `kill -USR1 $$` must never leak into the parent and
        // kill it at a later command boundary).
        let saved_pending_signals = if fresh_shell {
            crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default()
        } else {
            Vec::new()
        };
        // GNU's queued child-death notifications are process state: the
        // child's queue dies with it (jobs.c waitchld runs in the child's
        // own image) and the parent's counter must be exactly what it was.
        // sigchld_notifications_pending is an Executor cell shared by the
        // in-process emulation, so a reap that lands while the child runs a
        // signal-trap action (trap9.sub: the `{ sleep 1; kill -USR1 $$; } &`
        // job dies during the USR1 trap) would otherwise leave a leftover
        // count that re-fires the parent's SIGCHLD trap once during the
        // monitor `wait` (trap.tests: a fourth "caught a child death").
        let saved_sigchld_notifications = self.sigchld_notifications_pending.replace(0);
        self.set_env("__RUBASH_SCRIPT_NAME", script);
        // When this_shell_invocation is true, cmd.words[0] is the shell
        // command (e.g. ${THIS_SH}) and cmd.words[1] is the script path;
        // positional params start at cmd.words[2]. Otherwise cmd.words[0]
        // is the script path and params start at cmd.words[1].
        let param_start = if this_shell_invocation { 2 } else { 1 }; // word shape, not child model
        self.set_positional_params(cmd.words[param_start..].to_vec());
        if fresh_shell {
            // GNU variables.c:initialize_shell_variables sets OPTIND=1 for
            // every new shell invocation. OPTIND is not exported, so
            // child_shell_environment doesn't carry it over; set it here
            // so getopts in the child starts fresh.
            self.shell_state
                .env_vars
                .insert("OPTIND".to_string(), "1".to_string());
        }
        if !this_shell_invocation {
            self.shell_state.subshell_depth.set(saved_depth + 1);
        }

        // A ${THIS_SH} child is a process boundary, so the process
        // environment must show the CHILD's env for the whole run:
        // builtins write shell state through std::env
        // (builtins/setattr/apply.rs:66,166) and lookups fall back to
        // env::var (embedded_parameters.rs:421, parameter_errors.rs:900+),
        // which otherwise lets non-exported parent values (e.g. a readonly
        // var) leak into the "fresh" child. Any writes the child makes die
        // with the scope, exactly like a real child's env block dying on
        // exit.
        let saved_process_env: Option<HashMap<String, String>> = if fresh_shell {
            let saved: HashMap<String, String> = env::vars().collect();
            for (name, _) in env::vars() {
                env::remove_var(&name);
            }
            for (name, value) in &self.shell_state.env_vars {
                if crate::executor::local_helpers::is_valid_process_env(name, value) {
                    env::set_var(name, value);
                }
            }
            Some(saved)
        } else {
            None
        };

        let result = if uses_history_driver && fresh_shell {
            // bashhist.c pre_process_line / shell.c reader loop: expand and
            // record history group-by-group so `!!`/`!str` see the entries
            // the child's earlier lines recorded. The child gets a fresh
            // session history (exec model); the parent's session is
            // restored afterwards.
            let saved_session = self.get_session_history();
            let code = crate::script_driver::run_script_with_history(self, &source, Some(cmd));
            self.exit_code = code;
            self.set_session_history(saved_session);
            Ok(())
        } else {
            self.execute_ast(&ast)
        };
        let mut status = self.exit_code;
        // GNU shell.c exit_shell -> run_exit_trap: both ${THIS_SH} and
        // ENOEXEC children are fresh shells (shell.c:429-464
        // shell_reinitialize; execute_cmd.c:6237-6260), so an EXIT trap the
        // child script installed fires before the status returns to the
        // parent (trap9.sub probe: ./p4.sub with its own EXIT trap runs it
        // at script end). The parent's EXIT trap does not: the fresh-shell
        // env carries no trap table.
        if fresh_shell {
            if let Ok(trap_status) = self.run_exit_trap_for_status(status) {
                status = trap_status;
            }
        }

        if let Some(saved) = saved_process_env {
            for (name, _) in env::vars() {
                env::remove_var(&name);
            }
            for (name, value) in saved {
                env::set_var(name, value);
            }
        }

        self.restore_shell_env(saved_env);
        if let Some(saved_shell_state) = saved_shell_state {
            self.shell_state = saved_shell_state;
        }
        self.shell_state.coproc_names = saved_coproc_names;
        self.shell_state.last_background_pid = saved_last_background_pid;
        self.shell_state.job_table = saved_job_table;
        self.background_children = saved_background_children;
        self.fd_table = saved_fd_table;
        if fresh_shell {
            // Discard signals that arrived while the emulated child was
            // alive — GNU's real child exits with its queue — then hand the
            // parent's parked queue back so nothing addressed to this
            // process is lost.
            let _ = crate::builtins::kill::take_pending_signals_now(std::process::id());
            crate::builtins::kill::requeue_pending_signals(saved_pending_signals);
        }
        // Restore the parent's child-death notification count (the child's
        // leftover was already zeroed at entry; see
        // saved_sigchld_notifications above).
        self.sigchld_notifications_pending
            .set(saved_sigchld_notifications);
        self.parse_error_occurred = saved_parse_error;
        self.exit_jump_pending.set(saved_exit_jump_pending);
        self.last_command_substitution_parse_error
            .set(saved_comsub_parse_error);
        self.shell_state.pipestatus = saved_pipestatus;
        self.set_positional_params(saved_positional_params);
        self.shell_state.functions = saved_functions;
        self.shell_state.function_definition_redirects = saved_function_redirects;
        self.shell_state.function_def_infos = saved_function_def_infos;
        self.shell_state.aliases = saved_aliases;
        self.tempenv_names = saved_tempenv_names;
        self.tempenv_marks = saved_tempenv_marks;
        self.tempenv_promoted_names = saved_tempenv_promoted;
        self.tempenv_previous = saved_tempenv_previous;
        self.shell_state.bash_source_stack = saved_bash_source_stack;
        self.shell_state.bash_lineno_stack = saved_bash_lineno_stack;
        self.shell_state.bash_argc_stack = saved_bash_argc_stack;
        self.shell_state.bash_argv_stack = saved_bash_argv_stack;
        self.shell_state.subshell_depth.set(saved_depth);
        self.shell_state.loop_depth = saved_loop_depth;
        self.shell_state.function_depth = saved_function_depth;
        self.inside_compound_condition
            .set(saved_inside_compound_condition);
        self.evalerror_pending.set(saved_evalerror_pending);
        self.evalerror_line.set(saved_evalerror_line);
        self.reader_command_line.set(saved_reader_command_line);
        self.parameter_assignment_failure
            .set(saved_parameter_assignment_failure);
        self.evalerror_exec_depth.set(saved_evalerror_exec_depth);
        self.assignment_command_name = saved_assignment_command_name;
        if let Some(cwd) = saved_cwd {
            let _ = env::set_current_dir(cwd);
        }
        self.exit_code = status;

        // A child script is a process boundary: every fatal error becomes
        // the child's exit status and can never propagate into the parent's
        // command list (GNU shell.c/error.c: jump_to_top_level and
        // exit_shell stay inside the child process). Previously only
        // ExitCode was converted, so a child's ExpansionFailure escaping
        // here would abort an enclosing `for`/`while` in the parent.
        match result {
            Err(error) => {
                self.exit_code = self.child_process_exit_status(error)?;
                Ok(())
            }
            Ok(()) => Ok(()),
        }
    }

    /// Converts a fatal `ExecuteError` escaping an in-process child script
    /// into the child's exit status, mirroring how GNU's `exit_shell` /
    /// `jump_to_top_level` terminate only the child process.
    fn child_process_exit_status(&self, error: ExecuteError) -> Result<i32, ExecuteError> {
        match error {
            // `exit N`, errexit, and the lastpipe variant all terminate the
            // child with a chosen status.
            ExecuteError::ExitCode(code)
            | ExecuteError::LastpipeExit(code)
            // Expansion failures and fatal function errors carry the status
            // the child died with (error.c jump_to_top_level -> exit_shell).
            | ExecuteError::ExpansionFailure(code)
            | ExecuteError::FatalFunctionError(code)
            // A `return` that reaches the script top level is just the
            // child's status; it must not return from a parent function.
            | ExecuteError::Return(code) => Ok(code),
            // Loop control can never escape a child process.
            ExecuteError::Break(_) | ExecuteError::Continue(_) => Ok(1),
            ExecuteError::CommandNotFound(name) => {
                eprintln!("{name}: command not found");
                Ok(127)
            }
            ExecuteError::FunctionNotFound(name) => {
                eprintln!("{name}: command not found");
                Ok(127)
            }
            ExecuteError::UnknownBuiltin(name) => {
                eprintln!("{name}: command not found");
                Ok(1)
            }
            // A dead shared stdout (SIGPIPE analogue) is fatal to the whole
            // process, not just the child — keep propagating it.
            ExecuteError::IoError(error) if is_closed_output_io_error(&error) => {
                Err(ExecuteError::IoError(error))
            }
            ExecuteError::IoError(error) => {
                eprintln!("{error}");
                Ok(1)
            }
        }
    }

    pub(in crate::executor) fn child_shell_environment(&self) -> HashMap<String, String> {
        let exported = marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS);
        let mut child = exported
            .iter()
            .filter_map(|name| {
                self.shell_state
                    .env_vars
                    .get(name)
                    .map(|value| (name.clone(), value.clone()))
            })
            .collect::<HashMap<_, _>>();
        // GNU variables.c:511-526 (initialize_shell_variables): a fresh
        // shell invocation inherits only exported variables from the parent
        // environment. IFS is not exported by default, so a child shell
        // starts with the default " \t\n". Copying the parent's IFS into the
        // child environment would leak non-default IFS values (e.g.
        // intl.tests sets IFS=$(printf '%b' '\303\251') before running
        // ${THIS_SH} ./intl1.sub, and the child must not inherit it).
        // OLDPWD and SHELL are shell-local values maintained for every
        // shell instance even when not exported to native children.
        if let Ok(current_dir) = env::current_dir() {
            child.insert(
                "PWD".to_string(),
                shell_display_path(&current_dir.to_string_lossy().replace('\\', "/")),
            );
        }
        for name in ["OLDPWD", "SHELL"] {
            if let Some(value) = self.shell_state.env_vars.get(name) {
                child
                    .entry(name.to_string())
                    .or_insert_with(|| value.clone());
            }
        }
        child.insert(EXPORTED_VARS.to_string(), exported.join(DATA_DOLLAR_STR));
        child
    }
}

fn direct_windows_shell_script_path(
    command_name: &str,
    env_vars: &std::collections::HashMap<String, String>,
) -> Option<std::path::PathBuf> {
    if !cfg!(windows) {
        return None;
    }

    if !command_name.contains('/') && !command_name.contains('\\') {
        return None;
    }

    let path = shell_path_to_windows(command_name, env_vars);
    if !path.is_file() {
        return None;
    }

    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sh"))
    {
        return Some(path);
    }

    // Bash's execute_cmd.c retries a readable text file with the current
    // shell after an ENOEXEC result.  Windows cannot produce that errno for
    // extensionless files because the command resolver may select `sh.exe`
    // first, so identify the same script shape before spawning an external
    // interpreter.  Keep binary files on the native process path.
    if path.extension().is_some() {
        return None;
    }
    let source = std::fs::read(&path).ok()?;
    (!source.contains(&0)).then_some(path)
}
