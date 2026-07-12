use super::glob::pathname_expand_word;
use super::*;

impl Executor {
    pub(in crate::executor) fn report_command_heredoc_errors(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if cmd.subshell && command_has_unterminated_heredoc(cmd) {
            self.report_unterminated_subshell_heredoc(cmd);
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }
        if command_has_unterminated_heredoc(cmd) {
            self.report_unterminated_heredoc(cmd);
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_initial_command_node(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<Result<(), ExecuteError>> {
        if command_is_time_prefixed_compound(cmd) {
            return Some(self.execute_time_prefixed_compound_command(cmd));
        }
        if let Some(for_command) = &cmd.for_command {
            return Some(self.execute_for_command_with_redirects(for_command, cmd));
        }
        if let Some(select_command) = &cmd.select_command {
            return Some(self.execute_select_command(cmd, select_command));
        }
        if let Some(case_command) = &cmd.case_command {
            return Some(self.execute_case_command_with_redirects(cmd, case_command));
        }
        if let Some(coproc_cmd) = &cmd.coproc_command {
            return Some(self.execute_coproc_command(cmd, coproc_cmd));
        }
        if let Some(function_command) = &cmd.function_command {
            return Some(self.define_function(cmd, function_command));
        }
        None
    }

    pub(in crate::executor) fn execute_empty_words_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if command_has_no_effect(cmd) {
            return Ok(());
        }
        if let Some((name, message)) = self.parameter_assignment_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = 1;
            return Err(ExecuteError::ExitCode(1));
        }
        if let Some((name, message, status)) = self.parameter_expansion_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = status;
            return Err(ExecuteError::ExitCode(status));
        }
        let mut status = 0;
        for (name, value) in &cmd.assignments {
            let (expanded_value, substitution_status) =
                self.expand_assignment_value_with_status(value);
            if let Some(substitution_status) = substitution_status {
                status = substitution_status;
            }
            if !self.apply_shell_assignment(name, expanded_value) {
                status = 1;
            }
        }
        self.exit_code = status;
        if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
            return Err(ExecuteError::ExitCode(self.exit_code));
        }
        Ok(())
    }

    pub(in crate::executor) fn validate_command_parameter_expansions(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if let Some((name, message)) = self.parameter_assignment_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = 1;
            return Err(ExecuteError::ExitCode(1));
        }
        self.apply_parameter_assignment_expansions(cmd);
        if let Some((name, message, status)) = self.parameter_expansion_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = status;
            return Err(ExecuteError::ExitCode(status));
        }
        Ok(())
    }

    pub(in crate::executor) fn expand_command_words(&mut self, cmd: &CommandNode) -> CommandNode {
        let mut variable_expanded = cmd.clone();
        variable_expanded.words = cmd
            .words
            .iter()
            .enumerate()
            .flat_map(|(index, word)| self.expand_command_word(cmd, index, word))
            .collect();
        variable_expanded.word_kinds = Vec::new();

        let is_test_cmd = cmd.words.first().is_some_and(|w| w == "[[" || w == "[");
        if !is_test_cmd {
            variable_expanded.words = variable_expanded
                .words
                .into_iter()
                .flat_map(|word| match pathname_expand_word(&word, &self.env_vars) {
                    Some(matches) => matches,
                    None => vec![word],
                })
                .collect();
        }
        variable_expanded
    }

    fn expand_command_word(&mut self, cmd: &CommandNode, index: usize, word: &str) -> Vec<String> {
        if let Some(values) = self.array_at_word_values(word) {
            return values;
        }
        if let Some(values) = self.quoted_positional_at_word_values(word, cmd.word_kinds.get(index))
        {
            return values;
        }
        if self.is_brace_expand_enabled() && !word.contains("${") {
            let braced = crate::expand::braces::expand_braces(word);
            if braced.len() > 1 {
                return braced;
            }
        }
        let expanded = self.expand_word_mut(word);
        if expanded.is_empty() && self.removes_unquoted_null_word(cmd, index) {
            Vec::new()
        } else if self.splits_unquoted_expanded_word(cmd, index, &expanded) {
            self.field_split_values(&expanded)
        } else {
            vec![expanded]
        }
    }

    pub(in crate::executor) fn apply_alias_expansion_after_word_expansion(
        &mut self,
        variable_expanded: &CommandNode,
    ) -> CommandNode {
        let mut words = self.expand_aliases(&variable_expanded.words);
        if words != variable_expanded.words {
            words = words
                .into_iter()
                .map(|word| {
                    if word.starts_with('$') {
                        self.expand_word_mut(&word)
                    } else {
                        word
                    }
                })
                .collect();
        }
        CommandNode {
            words,
            ..variable_expanded.clone()
        }
    }

    pub(in crate::executor) fn execute_function_command_invocation(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<Result<(), ExecuteError>> {
        let function_name = cmd
            .words
            .first()
            .and_then(|word| self.function_name_for_command_word(word))?;
        let temporary_assignments = self.apply_temporary_assignments(&cmd.assignments);
        let applied_assignment_values = self.applied_temporary_assignment_values(&cmd.assignments);
        let old_posix_export_touched = self.env_vars.remove(POSIX_FUNCTION_EXPORT_TOUCHED);
        let result = self.execute_function(&function_name, &cmd.words[1..], cmd);
        if self.posix_mode_enabled() {
            self.restore_function_temporary_assignments(
                temporary_assignments,
                applied_assignment_values,
            );
        } else {
            self.restore_temporary_assignments(temporary_assignments);
        }
        restore_optional_env_var(
            &mut self.env_vars,
            POSIX_FUNCTION_EXPORT_TOUCHED,
            old_posix_export_touched,
        );
        Some(result)
    }

    pub(in crate::executor) fn execute_assignment_or_comment_command(
        &mut self,
        cmd: &CommandNode,
    ) -> bool {
        if self.execute_integer_assignment_suffix(cmd) || self.execute_assignment_words(cmd) {
            return true;
        }
        if self.execute_array_element_assignment(cmd) {
            return true;
        }
        if cmd.words.first().is_some_and(|word| word.starts_with('#')) {
            self.exit_code = 0;
            return true;
        }
        false
    }
}
