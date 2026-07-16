use super::*;

impl Executor {
    /// Execute an AST
    pub fn execute_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        self.set_current_line(cmd);
        self.set_current_command(cmd);
        self.report_command_heredoc_errors(cmd)?;

        if let Some(result) = self.execute_initial_command_node(cmd) {
            return result;
        }

        if cmd.words.is_empty() {
            return self.execute_empty_words_command(cmd);
        }

        self.validate_command_parameter_expansions(cmd)?;

        if self.execute_parser_level_alias(cmd)? {
            return Ok(());
        }

        let expanded = self.expand_command_words(cmd)?;
        let cmd = self.apply_alias_expansion_after_word_expansion(&expanded);

        if self.execute_alias_expanded_syntax(&cmd)? {
            return Ok(());
        }

        if let Some(result) = self.execute_function_command_invocation(&cmd) {
            return result;
        }

        if self.execute_assignment_or_comment_command(&cmd) {
            return Ok(());
        }

        let (materialized_cmd, process_substitution_files) =
            self.command_with_process_substitution_files(&cmd)?;
        self.execute_materialized_command(&materialized_cmd, process_substitution_files)
    }
}
