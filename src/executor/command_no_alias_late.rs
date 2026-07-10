use super::*;

impl Executor {
    pub(in crate::executor) fn execute_command_without_aliases_late_builtin(
        &mut self,
        cmd: &CommandNode,
        word: &str,
    ) -> Result<(), ExecuteError> {
        match word {
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
            "trap" => {
                self.exit_code = self.execute_trap(cmd)?;
                Ok(())
            }
            "type" => {
                if command_has_output_redirects(cmd) {
                    self.exit_code = self.execute_type_redirected(cmd)?;
                    return Ok(());
                }
                if self.execute_type_with_disabled_builtin_state(&cmd.words[1..])? {
                    return Ok(());
                }
                self.exit_code = self.execute_type(&cmd.words[1..]);
                Ok(())
            }
            "test" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "test") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code =
                    crate::builtins::test::execute(&cmd.words[1..], false, &self.env_vars)?;
                Ok(())
            }
            "[" => {
                if crate::builtins::enable::is_disabled(&self.env_vars, "[") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code =
                    crate::builtins::test::execute(&cmd.words[1..], true, &self.env_vars)?;
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
            _ => self.execute_external(cmd),
        }
    }
}
