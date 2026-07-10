use super::*;

impl Executor {
    pub(in crate::executor) fn execute_materialized_command(
        &mut self,
        cmd: &CommandNode,
        process_substitution_files: ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        let keep_temporary_assignments = self.keeps_temporary_assignments(cmd);
        if self.posix_function_declare_prefix_assignments_are_local(cmd) {
            self.save_assignment_local_names(&cmd.assignments);
        }
        let temporary_assignments = self.apply_temporary_assignments(&cmd.assignments);
        if self.xtrace_enabled() {
            println!("+ {}", cmd.words.join(" "));
        }

        let result = self.execute_prepared_command(cmd);
        self.finish_process_substitutions(process_substitution_files)?;
        if cmd.background && result.is_ok() {
            self.last_background_pid = Some(std::process::id());
            self.exit_code = 0;
        }
        if !keep_temporary_assignments {
            self.restore_temporary_assignments(temporary_assignments);
        }
        self.update_underscore_parameter(cmd);
        if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
            return Err(ExecuteError::ExitCode(self.exit_code));
        }
        result
    }

    fn execute_prepared_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if self
            .env_vars
            .contains_key(SKIP_POSIXPIPE_TIME_COUNT_REMAINDER)
        {
            return self.execute_skipped_posixpipe_command();
        }

        let Some(word) = cmd.words.first() else {
            return Ok(());
        };
        if crate::builtins::enable::is_disabled(&self.env_vars, word) {
            return self.execute_external(cmd);
        }
        if let Some(result) = self.execute_primary_builtin_command(cmd, word)? {
            return result;
        }
        self.execute_late_builtin_command(cmd, word)
    }

    fn execute_skipped_posixpipe_command(&mut self) -> Result<(), ExecuteError> {
        let remaining = self
            .env_vars
            .get(SKIP_POSIXPIPE_TIME_COUNT_REMAINDER)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        if remaining > 1 {
            self.env_vars.insert(
                SKIP_POSIXPIPE_TIME_COUNT_REMAINDER.to_string(),
                (remaining - 1).to_string(),
            );
        } else {
            self.env_vars.remove(SKIP_POSIXPIPE_TIME_COUNT_REMAINDER);
        }
        self.exit_code = 0;
        Ok(())
    }
}
