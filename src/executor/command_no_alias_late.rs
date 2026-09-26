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
                // niubash #108: always the buffered path — see
                // command_dispatch_late.rs. Command substitution is not an
                // explicit redirect, so the old gate leaked `$(type -t ls)`
                // output to the process stdout.
                self.exit_code = self.execute_type_redirected(cmd)?;
                return Ok(());
            }
            "test" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "test") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_test_words(&cmd.words[1..], false, cmd)?;
                Ok(())
            }
            "[" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "[") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_test_words(&cmd.words[1..], true, cmd)?;
                Ok(())
            }
            "dirname" => {
                if let Some(status) = self.try_execute_dirname_fast_path(cmd) {
                    self.exit_code = status;
                    Ok(())
                } else {
                    self.execute_external(cmd)
                }
            }
            "basename" => {
                if let Some(status) = self.try_execute_basename_fast_path(cmd) {
                    self.exit_code = status;
                    Ok(())
                } else {
                    self.execute_external(cmd)
                }
            }
            "uname" => {
                self.exit_code = self.execute_identity_tool(cmd, "uname");
                Ok(())
            }
            "arch" => {
                self.exit_code = self.execute_identity_tool(cmd, "arch");
                Ok(())
            }
            _ => self.execute_external(cmd),
        }
    }
}
