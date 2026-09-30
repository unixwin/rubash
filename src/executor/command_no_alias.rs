use super::*;

impl Executor {
    pub(in crate::executor) fn execute_command_without_aliases(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // TODO(builtins/command.def/execute_cmd.c): Bash `command` skips shell
        // functions and aliases while still resolving builtins and PATH. This
        // narrow path is enough for alias.tests cases like `command true`.
        let Some(word) = cmd.words.first() else {
            self.exit_code = 0;
            return Ok(());
        };

        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word) {
            return self.execute_external(cmd);
        }

        match word.as_str() {
            ":" => {
                // GNU command.def -> execute_builtin: the builtin's
                // redirections are applied before it runs (redir.c
                // do_redirections). `command : <&8` with a closed fd 8
                // fails "8: Bad file descriptor" like every other builtin
                // (modernish BUG_SCLOSEDFD probe).
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = crate::builtins::colon::colon();
                Ok(())
            }
            "true" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "true") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = crate::builtins::colon::true_builtin();
                Ok(())
            }
            "false" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "false") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = crate::builtins::colon::false_builtin();
                Ok(())
            }
            "echo" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "echo") {
                    return self.execute_external(cmd);
                }
                self.execute_echo(cmd)?;
                Ok(())
            }
            "cd" => {
                self.exit_code = self.execute_cd(cmd)?;
                Ok(())
            }
            "pwd" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "pwd") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_pwd(cmd)?;
                Ok(())
            }
            "exec" => self.execute_exec_command(cmd),
            // builtins/command.def execute_builtin -> execute_cmd.c: `command`
            // suppresses only functions and aliases; `exit` stays a builtin
            // (modernish calls `command exit "$status"` in _Msh_exit).
            "exit" => self.execute_exit_command_word(cmd)?,
            "logout" => {
                self.exit_code = self.execute_logout(cmd)?;
                Ok(())
            }
            "eval" => self.execute_eval(cmd),
            "set" => self.execute_set_command(cmd),
            "setopt" => {
                self.exit_code = self.execute_zsh_option_builtin(cmd, true)?;
                Ok(())
            }
            "getopts" => {
                self.exit_code = self.execute_getopts_command(cmd)?;
                Ok(())
            }
            "shopt" => {
                self.exit_code = self.execute_shopt(cmd)?;
                Ok(())
            }
            "unsetopt" => {
                self.exit_code = self.execute_zsh_option_builtin(cmd, false)?;
                Ok(())
            }
            "enable" => {
                self.exit_code = self.execute_enable(cmd)?;
                Ok(())
            }
            "." | "source" => self.execute_source_from_command_builtin(cmd),
            "return" => self.execute_return(cmd),
            "break" => self.execute_loop_control(cmd, LoopControlKind::Break),
            "continue" => self.execute_loop_control(cmd, LoopControlKind::Continue),
            "command" => self.execute_command_builtin_without_aliases(cmd),
            "builtin" => self.execute_builtin_direct_command(cmd),
            #[cfg(windows)]
            "sudo" => {
                self.exit_code = self.execute_sudo(cmd)?;
                Ok(())
            }
            "printf" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "printf") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_printf(cmd)?;
                Ok(())
            }
            "hash" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "hash") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_hash(cmd)?;
                Ok(())
            }
            "help" => {
                self.exit_code = self.execute_help(cmd)?;
                Ok(())
            }
            "alias" => {
                self.exit_code = self.execute_alias(cmd)?;
                Ok(())
            }
            "unalias" => {
                self.exit_code = self.execute_unalias(cmd)?;
                Ok(())
            }
            "export" => {
                self.exit_code = self.execute_export(cmd)?;
                Ok(())
            }
            "readonly" => {
                self.exit_code = self.execute_readonly(cmd)?;
                Ok(())
            }
            "declare" | "typeset" => self.execute_declare_command(cmd),
            "local" => {
                self.exit_code = self.execute_local(cmd)?;
                Ok(())
            }
            "unset" => {
                self.exit_code = self.execute_unset(cmd)?;
                Ok(())
            }
            "pushd" => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Pushd)?;
                Ok(())
            }
            "popd" => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Popd)?;
                Ok(())
            }
            "dirs" => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Dirs)?;
                Ok(())
            }
            "kill" => {
                self.exit_code = self.execute_kill(cmd)?;
                Ok(())
            }
            "let" => {
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_let(&cmd.words[1..]);
                Ok(())
            }
            "time" => {
                self.execute_time_command_node(cmd)?;
                Ok(())
            }
            "umask" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "umask") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_umask(cmd)?;
                Ok(())
            }
            "ulimit" => {
                self.exit_code = self.execute_ulimit(cmd)?;
                Ok(())
            }
            "read" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "read") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_read(cmd);
                Ok(())
            }
            "mapfile" | "readarray" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, &cmd.words[0]) {
                    return self.execute_external(cmd);
                }
                // Redirect preflight (GNU execute_builtin applies the
                // redirections before mapfile runs; rubash#324: `< missing`
                // reports and returns 1 with the array untouched).
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_mapfile(cmd);
                Ok(())
            }
            "shift" => self.execute_shift_command(cmd),
            other => self.execute_command_without_aliases_late_builtin(cmd, other),
        }
    }

    fn execute_command_builtin_without_aliases(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // rubash#289d: describe output always takes the buffered
        // single-truth path — the former raw `println!` fallback leaked
        // under comsub capture (see command_dispatch_primary.rs).
        if self.execute_command_describe_redirected(cmd)? {
            return Ok(());
        }

        // GNU execute_cmd.c: the builtin's own diagnostics and describe
        // output go to the shell's CURRENTLY BOUND fd 1/2 (redir.c), so an
        // `exec 2>/dev/null` contains them (rubash#218/#222-era leak: the
        // raw `builtins::command::execute` wrote straight to process
        // stdio, bypassing every enclosing redirect).
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action = crate::builtins::command::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        match action {
            crate::builtins::command::CommandAction::Complete(status) => {
                self.exit_code = status;
                Ok(())
            }
            crate::builtins::command::CommandAction::Execute {
                words,
                use_standard_path,
            } => {
                let mut command = cmd.clone();
                command.words = words;
                self.execute_command_without_aliases_with_path(&command, use_standard_path)
            }
        }
    }

    pub(in crate::executor) fn execute_command_without_aliases_with_path(
        &mut self,
        cmd: &CommandNode,
        use_standard_path: bool,
    ) -> Result<(), ExecuteError> {
        // GNU execute_cmd.c:4732-4736: the re-dispatched argument of the
        // `command' builtin runs with executing_command_builtin set (unwind-
        // protected, restored on the way out). The eval/source parse-error
        // containment reads it (evalstring.c:590): `command eval '( '' must
        // not take the posix ERREXIT exit that bare `eval '( '' takes.
        self.command_builtin_depth += 1;
        let result = self.execute_command_without_aliases_with_path_inner(cmd, use_standard_path);
        self.command_builtin_depth -= 1;
        // GNU execute_cmd.c:4657-4666: the special-builtin lookup — and with
        // it builtin_is_special and special_builtin_failed — never runs for
        // this re-dispatch (check_command_builtin set CMD_NO_FUNCTIONS at
        // :4723), so whatever the inner builtin set must not leak to the
        // OUTER command's posix fatal check (:1004-1017): `command return 16'
        // under `set -o posix' reports and continues (errors8.sub ok 5).
        self.special_builtin_failed.set(false);
        result
    }

    fn execute_command_without_aliases_with_path_inner(
        &mut self,
        cmd: &CommandNode,
        use_standard_path: bool,
    ) -> Result<(), ExecuteError> {
        if !use_standard_path {
            return self.execute_command_without_aliases(cmd);
        }

        let saved_path = self.shell_state.env_vars.get("PATH").cloned();
        let standard = standard_path(&self.shell_state.env_vars);
        self.shell_state
            .env_vars
            .insert("PATH".to_string(), standard);
        let result = self.execute_command_without_aliases(cmd);
        match saved_path {
            Some(path) => {
                self.shell_state.env_vars.insert("PATH".to_string(), path);
            }
            None => {
                self.shell_state.env_vars.remove("PATH");
            }
        }
        result
    }
}
