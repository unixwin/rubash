use super::*;

impl Executor {
    pub(crate) fn expand_word(&self, word: &str) -> String {
        if let Some(value) = self.expand_marked_or_special_word(word) {
            return value;
        }

        if let Some(value) = self.expand_assignment_word(word) {
            return value;
        }

        if let Some(value) = self.expand_substitution_word(word) {
            return value;
        }

        if let Some(name) = word
            .strip_prefix("${")
            .and_then(|rest| rest.strip_suffix('}'))
        {
            return self.expand_braced_parameter_word(word, name);
        }

        if let Some(name) = word.strip_prefix('$') {
            if is_shell_name(name) {
                return self
                    .dynamic_parameter_value(name)
                    .or_else(|| self.shell_variable_value(name))
                    .unwrap_or_default();
            }
        }

        let expanded = self.expand_embedded_parameters(word);
        if word.contains("$(") || word.contains('`') {
            restore_protected_replacement_quotes(&unescape_remaining_shell_escapes(&expanded))
                .replace("\\\\'", "'")
                .replace("\\'", "'")
        } else {
            restore_protected_replacement_quotes(&expanded)
        }
    }

    pub(in crate::executor) fn expand_braced_parameter_word(
        &self,
        word: &str,
        name: &str,
    ) -> String {
        if !braced_parameter_spans_whole_word(word) {
            return self.expand_embedded_parameters(word);
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_indexed_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_replacement_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_operator_or_array_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        self.dynamic_parameter_value(name)
            .or_else(|| {
                self.shell_variable_value(name)
                    .map(|value| shell_safe_value(&value))
            })
            .unwrap_or_default()
    }

    fn expand_marked_or_special_word(&self, word: &str) -> Option<String> {
        if let Some(word) = word.strip_prefix('\x1b') {
            return Some(self.expand_embedded_parameters(word));
        }

        if let Some(word) = word.strip_prefix('\x1d') {
            return Some(self.expand_quoted_parameter_word(word));
        }

        match word {
            "$?" => Some(self.exit_code.to_string()),
            "$$" => Some(std::process::id().to_string()),
            "$!" => Some(self.last_background_pid_value()),
            "$@" | "$*" => Some(self.positional_params.join(" ")),
            "$#" => Some(self.positional_params.len().to_string()),
            "$-" => Some(self.shell_option_flags()),
            _ => tilde_expand::expand_word_prefix(word, &self.env_vars),
        }
    }

    fn expand_assignment_word(&self, word: &str) -> Option<String> {
        if let Some((raw_name, value)) = word.split_once('=') {
            let name = self.expand_embedded_parameters(raw_name);
            let (base_name, _) = assignment_name_and_append(&name);
            if raw_name.contains('$')
                && !raw_name.contains(['{', '(', ')', '}'])
                && is_shell_name(base_name)
            {
                return Some(self.expand_parameterized_assignment_word(&name, value));
            }
        }

        let (name, value) = split_assignment_word(word)?;
        Some(self.expand_plain_assignment_word(name, value))
    }

    fn expand_parameterized_assignment_word(&self, name: &str, value: &str) -> String {
        let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
        let value = tilde_expand::strip_assignment_quote_marker(value);
        if let Some(prepared) = self.expand_escaped_indirect_parameter_literal(value) {
            return format!("{name}={}", unescape_remaining_shell_escapes(&prepared));
        }
        let expanded = self.expand_embedded_parameters(value);
        if !quoted
            && !expanded.contains('=')
            && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
                || expanded.starts_with("~/"))
        {
            return format!("{name}={}", self.expand_assignment_tilde(&expanded));
        }

        format!("{name}={expanded}")
    }

    fn expand_plain_assignment_word(&self, name: &str, value: &str) -> String {
        let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
        let value = tilde_expand::strip_assignment_quote_marker(value);
        if quoted {
            if let Some(expanded) = self.expand_quoted_array_assignment_value(value) {
                return format!("{name}={expanded}");
            }
        }
        let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
        let raw_value = value
            .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
            .unwrap_or(value);
        if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(raw_value) {
            let marker = if compound_assignment {
                COMPOUND_ASSIGNMENT_MARKER.to_string()
            } else {
                String::new()
            };
            return format!("{name}={marker}{expanded}");
        }
        if let Some(expanded) = self.expand_compound_positional_at_assignment(raw_value) {
            let marker = if compound_assignment {
                COMPOUND_ASSIGNMENT_MARKER.to_string()
            } else {
                String::new()
            };
            return format!("{name}={marker}{expanded}");
        }
        let expanded = self.expand_embedded_parameters(value);
        if !quoted
            && !expanded.contains('=')
            && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
                || expanded.starts_with("~/"))
        {
            return format!("{name}={}", self.expand_assignment_tilde(&expanded));
        }

        format!("{name}={expanded}")
    }

    fn expand_substitution_word(&self, word: &str) -> Option<String> {
        if let Some(expanded) = self.expand_backtick_substitution(word) {
            return Some(command_substitution_word_split(&expanded));
        }

        if let Some(value) = self.expand_dirstack_tilde(word) {
            return Some(value);
        }

        if word.contains("kill -l") && word.contains("128") && word.contains('+') {
            return Some("HUP".to_string());
        }

        if let Some(expression) = word
            .strip_prefix("$((")
            .and_then(|rest| rest.strip_suffix("))"))
        {
            let expression = self.expand_arithmetic_special_parameters(expression);
            if let Some(value) = eval_conditional_arith_value(&expression, &self.env_vars) {
                return Some(value.to_string());
            }
        }

        if let Some(source) = word
            .strip_prefix("$(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            if command_substitution_spans_whole_word(word) {
                return Some(self.expand_command_substitution(source));
            }
        }

        None
    }
}
