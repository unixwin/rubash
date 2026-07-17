use super::glob::{pathname_expand_word, PathnameExpansion};
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
        if let Some(if_command) = &cmd.if_command {
            return Some(self.execute_if_command_with_redirects(cmd, if_command));
        }
        if let Some(loop_command) = &cmd.loop_command {
            return Some(self.execute_loop_command_with_redirects(cmd, loop_command));
        }
        if let Some(subshell_command) = &cmd.subshell_command {
            return Some(self.execute_subshell_command_with_redirects(cmd, subshell_command));
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
        self.apply_no_output_builtin_redirects(cmd)?;
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

    pub(in crate::executor) fn expand_command_words(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<CommandNode, ExecuteError> {
        let mut variable_expanded = cmd.clone();
        let expanded_words = cmd
            .words
            .iter()
            .enumerate()
            .flat_map(|(index, word)| {
                let raw = cmd
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str());
                let suppress_glob =
                    word.starts_with('\x1b') || word.starts_with('\x1d') || raw_word_is_quoted(raw);
                self.expand_command_word(cmd, index, word, raw)
                    .into_iter()
                    .map(move |word| (word, suppress_glob))
            })
            .collect::<Vec<_>>();
        variable_expanded.words = expanded_words
            .iter()
            .map(|(word, _)| word.clone())
            .collect();
        variable_expanded.word_kinds = Vec::new();

        let is_test_cmd = cmd.words.first().is_some_and(|w| w == "[[" || w == "[");
        if !is_test_cmd {
            let mut words = Vec::new();
            for (word, suppress_glob) in expanded_words {
                if suppress_glob {
                    words.push(word);
                } else {
                    match pathname_expand_word(&word, &self.env_vars) {
                        PathnameExpansion::Matches(matches) => words.extend(matches),
                        PathnameExpansion::NoMatch => words.push(word),
                        PathnameExpansion::Fail(pattern) => {
                            self.report_failglob(&pattern);
                            return Err(ExecuteError::ExitCode(1));
                        }
                    }
                }
            }
            variable_expanded.words = words;
        }
        Ok(variable_expanded)
    }

    fn expand_command_word(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        word: &str,
        raw: Option<&str>,
    ) -> Vec<String> {
        if cmd
            .process_substitutions
            .iter()
            .any(|process| process.word_index == Some(index) && process.target == word)
        {
            return vec![word.to_string()];
        }
        if let Some(values) = self.array_at_word_values(word) {
            if word_is_unquoted_array_list_expansion(word) {
                return field_split_array_values_with_ifs(
                    values,
                    self.env_vars.get("IFS").map(String::as_str),
                );
            }
            return values;
        }
        if let Some(values) = self.quoted_positional_at_word_values(word, cmd.word_kinds.get(index))
        {
            return values;
        }
        if self.is_brace_expand_enabled() && !word.contains("${") {
            let braced = expand_braces_with_optional_raw(word, raw);
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

    pub(in crate::executor) fn report_failglob(&mut self, pattern: &str) {
        eprintln!("{}no match: {pattern}", self.diagnostic_prefix());
        self.exit_code = 1;
    }
}

pub(in crate::executor) fn expand_braces_with_optional_raw(
    word: &str,
    raw: Option<&str>,
) -> Vec<String> {
    if let Some(raw) = raw {
        if raw != word && !raw.contains("${") {
            let braced = crate::expand::braces::expand_braces(raw);
            if braced.len() > 1 {
                return braced
                    .into_iter()
                    .map(|word| crate::lexer::remove_shell_quotes(&word))
                    .collect();
            }
        }
    }

    crate::expand::braces::expand_braces(word)
}

fn raw_word_is_quoted(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '\'' | '"' => return true,
            '$' if chars.get(index + 1) == Some(&'\'') || chars.get(index + 1) == Some(&'"') => {
                return true;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                index = skip_raw_command_substitution(&chars, index + 2);
                continue;
            }
            '`' => {
                index = skip_raw_backtick(&chars, index + 1);
                continue;
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    false
}

fn skip_raw_command_substitution(chars: &[char], mut index: usize) -> usize {
    let mut depth = 1usize;
    while index < chars.len() {
        match chars[index] {
            '\'' => index = skip_raw_quote(chars, index + 1, '\''),
            '"' => index = skip_raw_quote(chars, index + 1, '"'),
            '`' => index = skip_raw_backtick(chars, index + 1),
            '$' if chars.get(index + 1) == Some(&'(') => {
                depth += 1;
                index += 2;
                continue;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index + 1;
                }
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    index
}

fn skip_raw_quote(chars: &[char], mut index: usize, quote: char) -> usize {
    while index < chars.len() {
        if chars[index] == '\\' && quote != '\'' {
            index += 2;
            continue;
        }
        if chars[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    index
}

fn skip_raw_backtick(chars: &[char], mut index: usize) -> usize {
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '`' {
            return index + 1;
        }
        index += 1;
    }
    index
}
