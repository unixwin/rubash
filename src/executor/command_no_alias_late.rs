use super::command_registry::{cmd_late_dispatch_slot, CmdLateSlot};
use super::*;

impl Executor {
    pub(in crate::executor) fn execute_command_without_aliases_late_builtin(
        &mut self,
        cmd: &CommandNode,
        word: &str,
    ) -> Result<(), ExecuteError> {
        // rubash#2036 S3: the name set is the registry's CmdLate
        // projection (command_registry::CMD_LATE_DISPATCH); the slot
        // match carries the historical per-name bodies verbatim.
        match cmd_late_dispatch_slot(word) {
            Some(CmdLateSlot::Times) => {
                self.exit_code = self.execute_times(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Caller) => {
                self.exit_code = self.execute_caller(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Jobs) => {
                self.exit_code = self.execute_jobs(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Disown) => {
                self.exit_code = self.execute_disown(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Wait) => {
                self.exit_code = self.execute_wait(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Fg) => {
                self.exit_code =
                    self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                Ok(())
            }
            Some(CmdLateSlot::Bg) => {
                self.exit_code =
                    self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                Ok(())
            }
            Some(CmdLateSlot::Suspend) => {
                self.exit_code = self.execute_suspend(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::History) => {
                self.exit_code = self.execute_history(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Bind) => {
                self.exit_code = self.execute_bind(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Fc) => {
                self.exit_code = self.execute_fc(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Complete) => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Complete,
                )?;
                Ok(())
            }
            Some(CmdLateSlot::Compgen) => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Compgen,
                )?;
                Ok(())
            }
            Some(CmdLateSlot::Compopt) => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Compopt,
                )?;
                Ok(())
            }
            Some(CmdLateSlot::Trap) => {
                self.exit_code = self.execute_trap(cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Type) => {
                // niubash #108: always the buffered path — see
                // command_dispatch_late.rs. Command substitution is not an
                // explicit redirect, so the old gate leaked `$(type -t ls)`
                // output to the process stdout.
                self.exit_code = self.execute_type_redirected(cmd)?;
                return Ok(());
            }
            Some(CmdLateSlot::Test) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "test") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_test_words(&cmd.words[1..], false, cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::TestBracket) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "[") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_test_words(&cmd.words[1..], true, cmd)?;
                Ok(())
            }
            Some(CmdLateSlot::Dirname) => {
                if !self.emulated_external_stands_down("dirname") {
                    if let Some(status) = self.try_execute_dirname_fast_path(cmd) {
                        self.exit_code = status;
                        return Ok(());
                    }
                }
                self.execute_external(cmd)
            }
            Some(CmdLateSlot::Basename) => {
                if !self.emulated_external_stands_down("basename") {
                    if let Some(status) = self.try_execute_basename_fast_path(cmd) {
                        self.exit_code = status;
                        return Ok(());
                    }
                }
                self.execute_external(cmd)
            }
            Some(CmdLateSlot::Uname) => {
                // Registry-driven stand-down, as in the simple
                // dispatch (ScriptYield/FailClosed, #2104).
                if self.registered_stand_down("uname") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_identity_tool(cmd, "uname");
                Ok(())
            }
            Some(CmdLateSlot::Arch) => {
                if self.registered_stand_down("arch") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_identity_tool(cmd, "arch");
                Ok(())
            }
            Some(CmdLateSlot::Stty) => {
                if self.emulated_external_stands_down("stty") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_console_mode_tool(cmd);
                Ok(())
            }
            _ => self.execute_external(cmd),
        }
    }
}
