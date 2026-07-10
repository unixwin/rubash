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

        if crate::builtins::enable::is_disabled(&self.env_vars, word) {
            return self.execute_external(cmd);
        }

        match word.as_str() {
            ":" => {
                self.exit_code = crate::builtins::colon::colon();
                Ok(())
            }
            "true" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "true") {
                    return self.execute_external(cmd);
                }
                self.exit_code = crate::builtins::colon::true_builtin();
                Ok(())
            }
            "false" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "false") {
                    return self.execute_external(cmd);
                }
                self.exit_code = crate::builtins::colon::false_builtin();
                Ok(())
            }
            "echo" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "echo") {
                    return self.execute_external(cmd);
                }
                self.execute_echo(cmd)?;
                self.exit_code = 0;
                Ok(())
            }
            "cd" => {
                self.exit_code = self.execute_cd(cmd)?;
                Ok(())
            }
            "pwd" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "pwd") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_pwd(cmd)?;
                Ok(())
            }
            "exec" => self.execute_exec_command(cmd),
            "logout" => {
                self.exit_code = self.execute_logout(cmd)?;
                Ok(())
            }
            "eval" => self.execute_eval(cmd),
            "set" => self.execute_set_command(cmd),
            "getopts" => {
                self.exit_code = self.execute_getopts_command(cmd)?;
                Ok(())
            }
            "shopt" => {
                self.exit_code = self.execute_shopt(cmd)?;
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
            "recho" => {
                self.execute_recho(&cmd.words[1..]);
                self.exit_code = 0;
                Ok(())
            }
            "command" => self.execute_command_builtin_without_aliases(cmd),
            "builtin" => self.execute_builtin_direct_command(cmd),
            "printf" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "printf") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_printf(cmd)?;
                Ok(())
            }
            "hash" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "hash") {
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
            "umask" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "umask") {
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
                if crate::builtins::enable::is_disabled(&self.env_vars, "read") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_read(cmd);
                Ok(())
            }
            "mapfile" | "readarray" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, &cmd.words[0]) {
                    return self.execute_external(cmd);
                }
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
        let described = if command_has_output_redirects(cmd) {
            self.execute_command_describe_redirected(cmd)?
        } else {
            false
        };
        if described || self.execute_command_describe(&cmd.words[1..]) {
            return Ok(());
        }

        match crate::builtins::command::execute(&cmd.words[1..])? {
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
        if !use_standard_path {
            return self.execute_command_without_aliases(cmd);
        }

        let saved_path = self.env_vars.get("PATH").cloned();
        self.env_vars
            .insert("PATH".to_string(), standard_path(&self.env_vars));
        let result = self.execute_command_without_aliases(cmd);
        match saved_path {
            Some(path) => {
                self.env_vars.insert("PATH".to_string(), path);
            }
            None => {
                self.env_vars.remove("PATH");
            }
        }
        result
    }
}
