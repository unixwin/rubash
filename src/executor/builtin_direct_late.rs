use super::builtin_direct::command_node_from_args;
use super::*;

impl Executor {
    pub(in crate::executor) fn execute_builtin_direct_late(
        &mut self,
        args: &[String],
        name: &str,
    ) -> Result<(), ExecuteError> {
        match name {
            "pushd" => {
                let command = command_node_from_args(args);
                self.exit_code = self
                    .execute_stack_builtin(&command, crate::builtins::pushd::StackBuiltin::Pushd)?;
                Ok(())
            }
            "popd" => {
                let command = command_node_from_args(args);
                self.exit_code = self
                    .execute_stack_builtin(&command, crate::builtins::pushd::StackBuiltin::Popd)?;
                Ok(())
            }
            "dirs" => {
                let command = command_node_from_args(args);
                self.exit_code = self
                    .execute_stack_builtin(&command, crate::builtins::pushd::StackBuiltin::Dirs)?;
                Ok(())
            }
            "type" => {
                if self.execute_type_with_disabled_builtin_state(&args[1..])? {
                    return Ok(());
                }
                self.exit_code = self.execute_type(&args[1..]);
                Ok(())
            }
            "test" => {
                self.exit_code = crate::builtins::test::execute(&args[1..], false, &self.env_vars)?;
                Ok(())
            }
            "[" => {
                self.exit_code = crate::builtins::test::execute(&args[1..], true, &self.env_vars)?;
                Ok(())
            }
            "let" => {
                self.exit_code = self.execute_let(&args[1..]);
                Ok(())
            }
            "umask" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_umask(&command)?;
                Ok(())
            }
            "ulimit" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_ulimit(&command)?;
                Ok(())
            }
            "read" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_read(&command);
                Ok(())
            }
            "mapfile" | "readarray" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_mapfile(&command);
                Ok(())
            }
            "times" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_times(&command)?;
                Ok(())
            }
            "caller" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_caller(&command)?;
                Ok(())
            }
            "jobs" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_jobs(&command)?;
                Ok(())
            }
            "disown" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_disown(&command)?;
                Ok(())
            }
            "wait" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_wait(&command)?;
                Ok(())
            }
            "fg" => {
                let command = command_node_from_args(args);
                self.exit_code =
                    self.execute_fg_bg(&command, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                Ok(())
            }
            "bg" => {
                let command = command_node_from_args(args);
                self.exit_code =
                    self.execute_fg_bg(&command, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                Ok(())
            }
            "suspend" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_suspend(&command)?;
                Ok(())
            }
            "history" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_history(&command)?;
                Ok(())
            }
            "bind" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_bind(&command)?;
                Ok(())
            }
            "fc" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_fc(&command)?;
                Ok(())
            }
            "complete" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_completion_builtin(
                    &command,
                    crate::builtins::complete::CompletionBuiltin::Complete,
                )?;
                Ok(())
            }
            "compgen" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_completion_builtin(
                    &command,
                    crate::builtins::complete::CompletionBuiltin::Compgen,
                )?;
                Ok(())
            }
            "compopt" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_completion_builtin(
                    &command,
                    crate::builtins::complete::CompletionBuiltin::Compopt,
                )?;
                Ok(())
            }
            "time" => {
                self.execute_time_command(&args[1..])?;
                Ok(())
            }
            "trap" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_trap(&command)?;
                Ok(())
            }
            "shift" => self.execute_shift(&args[1..]),
            _ => {
                let mut stderr = Vec::new();
                writeln!(
                    &mut stderr,
                    "{}builtin: {name}: not a shell builtin",
                    self.diagnostic_prefix()
                )?;
                self.write_default_stderr(&stderr)?;
                self.exit_code = 1;
                Ok(())
            }
        }
    }
}
