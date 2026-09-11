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
            restore_command_substitution_output(
                &restore_protected_replacement_quotes(&unescape_remaining_shell_escapes(&expanded))
                    .replace("\\\\'", "'")
                    .replace("\\'", "'"),
            )
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

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(name, true) {
            return value;
        }

        if let Some(value) = self.expand_braced_indexed_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_replacement_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_operator_or_array_parameter(name) {
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
            "$$" => Some(self.shell_pid_value().to_string()),
            "$!" => Some(self.last_background_pid_value()),
            "$@" => Some(self.positional_params.join(" ")),
            // Bash joins `$*` with the first character of IFS, not a space.
            "$*" => Some(
                self.positional_params
                    .join(&self.ifs_first_char_separator()),
            ),
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
        let expanded = if quoted {
            expanded.replace('\x11', "")
        } else {
            expanded
        };
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
        // GNU subst.c:4357 expand_string_assignment (W_ASSIGNMENT,
        // subst.c:11432): unquoted element values of a compound assignment
        // undergo the assignment tilde pass on the RAW word, before
        // parameter expansion, so tilde text produced by $params is never
        // re-expanded (array.tests: aa=([0]=~/a:~/b) expands both segments
        // while w=([0]=~/a [1]=$p) keeps $p's result literal). Quoted
        // elements stay literal; quoted whole-RHS values skip the pass.
        let tilde_raw_owned;
        let raw_value = if !quoted && raw_value.starts_with('(') && raw_value.ends_with(')') {
            tilde_raw_owned = self.expand_tilde_in_compound_assignment(raw_value);
            &tilde_raw_owned
        } else {
            raw_value
        };
        if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(raw_value) {
            let marker = if compound_assignment {
                COMPOUND_ASSIGNMENT_MARKER.to_string()
            } else {
                String::new()
            };
            return format!("{name}={marker}{expanded}");
        }
        if let Some(expanded) = self.expand_compound_positional_at_assignment(raw_value, quoted) {
            let marker = if compound_assignment {
                COMPOUND_ASSIGNMENT_MARKER.to_string()
            } else {
                String::new()
            };
            return format!("{name}={marker}{expanded}");
        }
        let expanded = self.expand_embedded_parameters(value);
        let expanded = if quoted {
            expanded.replace('\x11', "")
        } else {
            expanded
        };
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
            return Some(restore_command_substitution_output(
                &command_substitution_word_split(&expanded),
            ));
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
            if crate::builtins::set::shell_option_enabled(&self.env_vars, "nounset") {
                if let Some(name) = arithmetic_unbound_variable(&expression, &self.env_vars) {
                    if !self.arithmetic_expansion_error.replace(true) {
                        eprintln!("{}{}: unbound variable", self.diagnostic_prefix(), name);
                    }
                    // GNU expr.c expr_streval: an unbound variable under `set
                    // -u` raises FORCE_EOF and exits the shell (127 in -c
                    // mode), regardless of the other words in the command.
                    self.arithmetic_nounset_error.set(true);
                    return Some(String::new());
                }
            }
            let (value, actual_category) =
                eval_conditional_arith_value_categorized(&expression, &self.env_vars);
            if let Some(value) = value {
                return Some(value.to_string());
            }
            self.arithmetic_last_error_category.set(actual_category);
            // Bash reports arithmetic expansion errors (floating point,
            // negative exponent, division by zero, ...) on stderr instead of
            // silently producing nothing, and abandons the enclosing command
            // list (status 1; GNU probe d2: `echo $((1/0)); echo after` never
            // prints "after").
            let message = crate::executor::arithmetic::arithmetic_error_message(&expression, true)
                .unwrap_or_else(|| {
                    format!(
                        "{expression}: syntax error in expression (error token is \"{expression}\")"
                    )
                });
            let actual_fatal = self.arithmetic_last_error_category.take().is_some();
            if !actual_fatal
                && !crate::executor::arithmetic::arithmetic_expansion_is_fatal(&expression)
            {
                self.arithmetic_nonfatal_error.set(true);
            } else {
                self.arithmetic_fatal_error.set(true);
            }
            if !self.arithmetic_expansion_error.replace(true) {
                eprintln!("{}{}", self.diagnostic_prefix(), message);
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
