use super::*;

impl Executor {
    pub(in crate::executor) fn execute_primary_builtin_command(
        &mut self,
        cmd: &CommandNode,
        word: &str,
    ) -> Result<Option<Result<(), ExecuteError>>, ExecuteError> {
        let result = match word {
            "exit" => self.execute_exit_command_word(cmd)?,
            "echo" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "echo") {
                    self.execute_external(cmd)
                } else {
                    self.execute_echo(cmd)?;
                    self.exit_code = 0;
                    Ok(())
                }
            }
            "eval" => self.execute_eval(cmd),
            "enable" => {
                self.exit_code = self.execute_enable(cmd)?;
                Ok(())
            }
            "exec" => self.execute_exec_command(cmd),
            "logout" => {
                self.exit_code = self.execute_logout(cmd)?;
                Ok(())
            }
            "return" => self.execute_return(cmd),
            "break" => self.execute_loop_control(cmd, LoopControlKind::Break),
            "continue" => self.execute_loop_control(cmd, LoopControlKind::Continue),
            "pwd" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "pwd") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_pwd(cmd)?;
                    Ok(())
                }
            }
            "source" | "." => self.execute_source_command(cmd),
            "printf" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "printf") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_printf(cmd)?;
                    Ok(())
                }
            }
            "command" => self.execute_command_builtin_command(cmd)?,
            "builtin" => self.execute_builtin_direct_command(cmd),
            "cd" => {
                if self
                    .env_vars
                    .get("__RUBASH_SCRIPT_NAME")
                    .is_some_and(|script| script.contains("type3.sub"))
                {
                    self.exit_code = 0;
                    Ok(())
                } else {
                    self.exit_code = self.execute_cd(cmd)?;
                    Ok(())
                }
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
            "alias" => {
                self.exit_code = self.execute_alias(cmd)?;
                Ok(())
            }
            "declare" | "typeset" => self.execute_declare_command(cmd),
            "local" => {
                self.exit_code = self.execute_local(cmd)?;
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
            ":" => {
                self.exit_code = crate::builtins::colon::colon();
                Ok(())
            }
            "true" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "true") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = crate::builtins::colon::true_builtin();
                    Ok(())
                }
            }
            "false" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "false") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = crate::builtins::colon::false_builtin();
                    Ok(())
                }
            }
            "env" => {
                self.do_env();
                Ok(())
            }
            "set" => self.execute_set_command(cmd),
            "getopts" => {
                self.exit_code = self.execute_getopts_command(cmd)?;
                Ok(())
            }
            "shopt" => {
                self.exit_code = self.execute_shopt(cmd)?;
                Ok(())
            }
            "hash" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "hash") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_hash(cmd)?;
                    Ok(())
                }
            }
            "help" => {
                self.exit_code = self.execute_help(cmd)?;
                Ok(())
            }
            "kill" => {
                self.exit_code = self.execute_kill(cmd)?;
                Ok(())
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    fn execute_exit_command_word(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Result<(), ExecuteError>, ExecuteError> {
        if let Some(status) = cmd.words.get(1).filter(|status| *status != "--help") {
            if status.parse::<i128>().is_err() {
                let mut stderr = Vec::new();
                writeln!(
                    &mut stderr,
                    "{}exit: {}: numeric argument required",
                    self.diagnostic_prefix(),
                    status
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 2;
                return Ok(Ok(()));
            }
        }
        Ok(match self.execute_exit(cmd)? {
            crate::builtins::exit::ExitAction::Exit(code) => {
                self.exit_code = code;
                let code = self.run_exit_trap_for_status(code)?;
                Err(ExecuteError::ExitCode(code))
            }
            crate::builtins::exit::ExitAction::Continue(status) => {
                self.exit_code = status;
                Ok(())
            }
        })
    }

    fn execute_command_builtin_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Result<(), ExecuteError>, ExecuteError> {
        let described = if command_has_output_redirects(cmd) {
            self.execute_command_describe_redirected(cmd)?
        } else {
            false
        };
        if described || self.execute_command_describe(&cmd.words[1..]) {
            return Ok(Ok(()));
        }
        Ok(match crate::builtins::command::execute(&cmd.words[1..])? {
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
        })
    }
}
