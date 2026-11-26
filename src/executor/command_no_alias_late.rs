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
            Some(CmdLateSlot::Times) => self.execute_status_builtin(cmd, Executor::execute_times),
            Some(CmdLateSlot::Caller) => self.execute_status_builtin(cmd, Executor::execute_caller),
            Some(CmdLateSlot::Jobs) => {
                // rubash#2397: inner roll (execute_cmd.c:4810-4813);
                // the outer `command` dispatch already rolled `Other`.
                self.note_builtin_dispatch(crate::shell::BuiltinDispatchClass::Jobs);
                self.execute_status_builtin(cmd, Executor::execute_jobs)
            }
            Some(CmdLateSlot::Disown) => self.execute_status_builtin(cmd, Executor::execute_disown),
            Some(CmdLateSlot::Wait) => self.execute_status_builtin(cmd, Executor::execute_wait),
            Some(CmdLateSlot::Fg) => {
                self.execute_job_control_builtin(cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)
            }
            Some(CmdLateSlot::Bg) => {
                self.execute_job_control_builtin(cmd, crate::builtins::fg_bg::JobControlBuiltin::Bg)
            }
            Some(CmdLateSlot::Suspend) => {
                self.execute_status_builtin(cmd, Executor::execute_suspend)
            }
            Some(CmdLateSlot::History) => {
                self.execute_status_builtin(cmd, Executor::execute_history)
            }
            Some(CmdLateSlot::Bind) => self.execute_status_builtin(cmd, Executor::execute_bind),
            Some(CmdLateSlot::Fc) => self.execute_status_builtin(cmd, Executor::execute_fc),
            Some(CmdLateSlot::Complete) => self.execute_completion_builtin_status(
                cmd,
                crate::builtins::complete::CompletionBuiltin::Complete,
            ),
            Some(CmdLateSlot::Compgen) => self.execute_completion_builtin_status(
                cmd,
                crate::builtins::complete::CompletionBuiltin::Compgen,
            ),
            Some(CmdLateSlot::Compopt) => self.execute_completion_builtin_status(
                cmd,
                crate::builtins::complete::CompletionBuiltin::Compopt,
            ),
            Some(CmdLateSlot::Trap) => self.execute_status_builtin(cmd, Executor::execute_trap),
            Some(CmdLateSlot::Type) => {
                // niubash #108: always the buffered path — see
                // command_dispatch_late.rs. Command substitution is not an
                // explicit redirect, so the old gate leaked `$(type -t ls)`
                // output to the process stdout.
                self.execute_status_builtin(cmd, Executor::execute_type_redirected)
            }
            Some(CmdLateSlot::Test) => self.execute_test_builtin(cmd, false),
            Some(CmdLateSlot::TestBracket) => self.execute_test_builtin(cmd, true),
            Some(CmdLateSlot::Dirname) => self.execute_fastpath_or_external(
                cmd,
                "dirname",
                Executor::try_execute_dirname_fast_path,
                |env_vars, name| {
                    super::execution_misc::record_fastpath_command_resolution(env_vars, name)
                },
            ),
            Some(CmdLateSlot::Basename) => self.execute_fastpath_or_external(
                cmd,
                "basename",
                Executor::try_execute_basename_fast_path,
                |env_vars, name| {
                    super::execution_misc::record_fastpath_command_resolution(env_vars, name)
                },
            ),
            Some(CmdLateSlot::Uname) => {
                // Registry-driven stand-down, as in the simple
                // dispatch (ScriptYield/FailClosed, #2104).
                self.execute_identity_tool_or_external(cmd, "uname")
            }
            Some(CmdLateSlot::Arch) => self.execute_identity_tool_or_external(cmd, "arch"),
            Some(CmdLateSlot::Stty) => self.execute_console_mode_tool_or_external(cmd),
            _ => self.execute_external(cmd),
        }
    }
}
