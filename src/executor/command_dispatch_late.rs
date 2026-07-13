use super::*;

impl Executor {
    pub(in crate::executor) fn execute_late_builtin_command(
        &mut self,
        cmd: &CommandNode,
        word: &str,
    ) -> Result<(), ExecuteError> {
        match word {
            "let" => {
                self.exit_code = self.execute_let(&cmd.words[1..]);
                Ok(())
            }
            "umask" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "umask") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_umask(cmd)?;
                    Ok(())
                }
            }
            "ulimit" => {
                self.exit_code = self.execute_ulimit(cmd)?;
                Ok(())
            }
            "unset" => {
                self.exit_code = self.execute_unset(cmd)?;
                Ok(())
            }
            "read" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "read") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_read(cmd);
                    Ok(())
                }
            }
            "mapfile" | "readarray" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, word) {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_mapfile(cmd);
                    Ok(())
                }
            }
            "recho" => {
                self.execute_recho(&cmd.words[1..]);
                self.exit_code = 0;
                Ok(())
            }
            "shift" => self.execute_shift_command(cmd),
            "times" => {
                self.exit_code = self.execute_times(cmd)?;
                Ok(())
            }
            "caller" => {
                self.exit_code = self.execute_caller(cmd)?;
                Ok(())
            }
            "jobs" => {
                self.exit_code = self.execute_jobs(cmd)?;
                Ok(())
            }
            "disown" => {
                self.exit_code = self.execute_disown(cmd)?;
                Ok(())
            }
            "wait" => {
                self.exit_code = self.execute_wait(cmd)?;
                Ok(())
            }
            "fg" => {
                self.exit_code =
                    self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                Ok(())
            }
            "bg" => {
                self.exit_code =
                    self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                Ok(())
            }
            "suspend" => {
                self.exit_code = self.execute_suspend(cmd)?;
                Ok(())
            }
            "history" => {
                self.exit_code = self.execute_history(cmd)?;
                Ok(())
            }
            "bind" => {
                self.exit_code = self.execute_bind(cmd)?;
                Ok(())
            }
            "fc" => {
                self.exit_code = self.execute_fc(cmd)?;
                Ok(())
            }
            "complete" => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Complete,
                )?;
                Ok(())
            }
            "compgen" => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Compgen,
                )?;
                Ok(())
            }
            "compopt" => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Compopt,
                )?;
                Ok(())
            }
            "time" => {
                self.execute_time_command_node(cmd)?;
                Ok(())
            }
            "trap" => {
                self.exit_code = self.execute_trap(cmd)?;
                Ok(())
            }
            "type" => {
                if command_has_output_redirects(cmd) {
                    self.exit_code = self.execute_type_redirected(cmd)?;
                    Ok(())
                } else if self.execute_type_with_disabled_builtin_state(&cmd.words[1..])? {
                    Ok(())
                } else {
                    self.exit_code = self.execute_type(&cmd.words[1..]);
                    Ok(())
                }
            }
            "test" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "test") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code =
                        crate::builtins::test::execute(&cmd.words[1..], false, &self.env_vars)?;
                    Ok(())
                }
            }
            "[" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "[") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code =
                        crate::builtins::test::execute(&cmd.words[1..], true, &self.env_vars)?;
                    Ok(())
                }
            }
            "[[" => {
                self.exit_code = self.execute_conditional(&cmd.words[1..]);
                Ok(())
            }
            "((" => {
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_arithmetic_command(cmd);
                Ok(())
            }
            "dirname" => {
                self.exit_code = self.execute_dirname(cmd);
                Ok(())
            }
            "basename" => {
                self.exit_code = self.execute_basename(cmd);
                Ok(())
            }
            _ if self.functions.contains_key(word) => {
                self.execute_function(word, &cmd.words[1..], cmd)
            }
            _ => self.execute_external(cmd),
        }
    }
}
