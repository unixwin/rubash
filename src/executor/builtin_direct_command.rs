use super::*;

impl Executor {
    pub(in crate::executor) fn execute_builtin_direct_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let args = &cmd.words[1..];
        if cmd.redirect_out.is_none()
            && cmd.append.is_none()
            && cmd.redirect_err.is_none()
            && cmd.redirect_err_append.is_none()
            && cmd.redirect_in.is_none()
            && cmd.heredoc.is_none()
            && cmd.here_string.is_none()
        {
            return self.execute_builtin_direct(args);
        }

        let Some(name) = args.first().map(String::as_str) else {
            self.exit_code = 0;
            return Ok(());
        };
        let mut builtin_cmd = cmd.clone();
        builtin_cmd.words = args.to_vec();

        if crate::builtins::enable::is_disabled(&self.env_vars, name) {
            self.write_builtin_not_found(cmd, name)?;
            self.exit_code = 1;
            return Ok(());
        }

        match name {
            ":" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code = crate::builtins::colon::colon();
                Ok(())
            }
            "true" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code = crate::builtins::colon::true_builtin();
                Ok(())
            }
            "false" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code = crate::builtins::colon::false_builtin();
                Ok(())
            }
            "echo" => {
                self.execute_echo(&builtin_cmd)?;
                self.exit_code = 0;
                Ok(())
            }
            "printf" => {
                self.exit_code = self.execute_printf(&builtin_cmd)?;
                Ok(())
            }
            "pwd" => {
                self.exit_code = self.execute_pwd(&builtin_cmd)?;
                Ok(())
            }
            "cd" => {
                self.exit_code = self.execute_cd(&builtin_cmd)?;
                Ok(())
            }
            "hash" => {
                self.exit_code = self.execute_hash(&builtin_cmd)?;
                Ok(())
            }
            "help" => {
                self.exit_code = self.execute_help(&builtin_cmd)?;
                Ok(())
            }
            "alias" => {
                self.exit_code = self.execute_alias(&builtin_cmd)?;
                Ok(())
            }
            "unalias" => {
                self.exit_code = self.execute_unalias(&builtin_cmd)?;
                Ok(())
            }
            "export" => {
                self.exit_code = self.execute_export(&builtin_cmd)?;
                Ok(())
            }
            "readonly" => {
                self.exit_code = self.execute_readonly(&builtin_cmd)?;
                Ok(())
            }
            "declare" | "typeset" => self.execute_declare_command(&builtin_cmd),
            "local" => {
                self.exit_code = self.execute_local(&builtin_cmd)?;
                Ok(())
            }
            "unset" => {
                self.exit_code = self.execute_unset(&builtin_cmd)?;
                Ok(())
            }
            "pushd" => {
                self.exit_code = self.execute_stack_builtin(
                    &builtin_cmd,
                    crate::builtins::pushd::StackBuiltin::Pushd,
                )?;
                Ok(())
            }
            "popd" => {
                self.exit_code = self.execute_stack_builtin(
                    &builtin_cmd,
                    crate::builtins::pushd::StackBuiltin::Popd,
                )?;
                Ok(())
            }
            "dirs" => {
                self.exit_code = self.execute_stack_builtin(
                    &builtin_cmd,
                    crate::builtins::pushd::StackBuiltin::Dirs,
                )?;
                Ok(())
            }
            "set" => self.execute_set_command(&builtin_cmd),
            "getopts" => {
                self.exit_code = self.execute_getopts_command(&builtin_cmd)?;
                Ok(())
            }
            "shopt" => {
                self.exit_code = self.execute_shopt(&builtin_cmd)?;
                Ok(())
            }
            "enable" => {
                self.exit_code = self.execute_enable(&builtin_cmd)?;
                Ok(())
            }
            "exec" => self.execute_exec_command(&builtin_cmd),
            "logout" => {
                self.exit_code = self.execute_logout(&builtin_cmd)?;
                Ok(())
            }
            "eval" => self.execute_eval(&builtin_cmd),
            "command" => self.execute_command_without_aliases(&builtin_cmd),
            "source" | "." => self.execute_source_command(&builtin_cmd),
            "return" => self.execute_return(&builtin_cmd),
            "break" => self.execute_loop_control(&builtin_cmd, LoopControlKind::Break),
            "continue" => self.execute_loop_control(&builtin_cmd, LoopControlKind::Continue),
            "kill" => {
                self.exit_code = self.execute_kill(&builtin_cmd)?;
                Ok(())
            }
            "let" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code = self.execute_let(&builtin_cmd.words[1..]);
                Ok(())
            }
            "umask" => {
                self.exit_code = self.execute_umask(&builtin_cmd)?;
                Ok(())
            }
            "ulimit" => {
                self.exit_code = self.execute_ulimit(&builtin_cmd)?;
                Ok(())
            }
            "read" => {
                self.exit_code = self.execute_read(&builtin_cmd);
                Ok(())
            }
            "mapfile" | "readarray" => {
                self.exit_code = self.execute_mapfile(&builtin_cmd);
                Ok(())
            }
            "times" => {
                self.exit_code = self.execute_times(&builtin_cmd)?;
                Ok(())
            }
            "caller" => {
                self.exit_code = self.execute_caller(&builtin_cmd)?;
                Ok(())
            }
            "jobs" => {
                self.exit_code = self.execute_jobs(&builtin_cmd)?;
                Ok(())
            }
            "disown" => {
                self.exit_code = self.execute_disown(&builtin_cmd)?;
                Ok(())
            }
            "wait" => {
                self.exit_code = self.execute_wait(&builtin_cmd)?;
                Ok(())
            }
            "fg" => {
                self.exit_code = self
                    .execute_fg_bg(&builtin_cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                Ok(())
            }
            "bg" => {
                self.exit_code = self
                    .execute_fg_bg(&builtin_cmd, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                Ok(())
            }
            "suspend" => {
                self.exit_code = self.execute_suspend(&builtin_cmd)?;
                Ok(())
            }
            "history" => {
                self.exit_code = self.execute_history(&builtin_cmd)?;
                Ok(())
            }
            "bind" => {
                self.exit_code = self.execute_bind(&builtin_cmd)?;
                Ok(())
            }
            "fc" => {
                self.exit_code = self.execute_fc(&builtin_cmd)?;
                Ok(())
            }
            "complete" => {
                self.exit_code = self.execute_completion_builtin(
                    &builtin_cmd,
                    crate::builtins::complete::CompletionBuiltin::Complete,
                )?;
                Ok(())
            }
            "compgen" => {
                self.exit_code = self.execute_completion_builtin(
                    &builtin_cmd,
                    crate::builtins::complete::CompletionBuiltin::Compgen,
                )?;
                Ok(())
            }
            "compopt" => {
                self.exit_code = self.execute_completion_builtin(
                    &builtin_cmd,
                    crate::builtins::complete::CompletionBuiltin::Compopt,
                )?;
                Ok(())
            }
            "time" => {
                self.execute_time_command_node(&builtin_cmd)?;
                Ok(())
            }
            "trap" => {
                self.exit_code = self.execute_trap(&builtin_cmd)?;
                Ok(())
            }
            "type" => {
                self.exit_code = self.execute_type_redirected(&builtin_cmd)?;
                Ok(())
            }
            "test" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code =
                    crate::builtins::test::execute(&builtin_cmd.words[1..], false, &self.env_vars)?;
                Ok(())
            }
            "[" => {
                self.apply_no_output_builtin_redirects(&builtin_cmd)?;
                self.exit_code =
                    crate::builtins::test::execute(&builtin_cmd.words[1..], true, &self.env_vars)?;
                Ok(())
            }
            "shift" => self.execute_shift_command(&builtin_cmd),
            _ => {
                self.write_builtin_not_found(cmd, name)?;
                self.exit_code = 1;
                Ok(())
            }
        }
    }
}
