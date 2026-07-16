use super::*;

impl Executor {
    pub(in crate::executor) fn is_brace_expand_enabled(&self) -> bool {
        crate::builtins::set::shell_option_enabled(&self.env_vars, "braceexpand")
    }
    pub(in crate::executor) fn expand_word_mut(&mut self, word: &str) -> String {
        self.apply_parameter_assignment_expansions_in_word(word);

        if let Some(word) = word.strip_prefix('\x1b') {
            return self.expand_embedded_parameters_mut(word);
        }

        if let Some(word) = word.strip_prefix('\x1d') {
            return self.expand_quoted_parameter_word_mut(word);
        }

        if let Some((raw_name, value)) = word.split_once('=') {
            let name = self.expand_embedded_parameters_mut(raw_name);
            let (base_name, _) = assignment_name_and_append(&name);
            if raw_name.contains('$')
                && !raw_name.contains(['{', '(', ')', '}'])
                && is_shell_name(base_name)
            {
                let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
                let value = tilde_expand::strip_assignment_quote_marker(value);
                if let Some(prepared) = self.expand_escaped_indirect_parameter_literal(value) {
                    return format!("{name}={}", unescape_remaining_shell_escapes(&prepared));
                }
                let expanded = self.expand_embedded_parameters_mut(value);
                if !quoted
                    && !expanded.contains('=')
                    && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
                        || expanded.starts_with("~/"))
                {
                    return format!("{name}={}", self.expand_assignment_tilde(&expanded));
                }

                return format!("{name}={expanded}");
            }
        }

        if let Some((name, value)) = split_assignment_word(word) {
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
            let expanded = self.expand_embedded_parameters_mut(value);
            if !quoted
                && !expanded.contains('=')
                && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
                    || expanded.starts_with("~/"))
            {
                return format!("{name}={}", self.expand_assignment_tilde(&expanded));
            }

            return format!("{name}={expanded}");
        }

        if let Some(expression) = word
            .strip_prefix("$((")
            .and_then(|rest| rest.strip_suffix("))"))
        {
            if let Some(value) = self.eval_arithmetic_command_value(expression) {
                return value.to_string();
            }
        }

        if word.contains("$((") || word.contains("$[") {
            return self.expand_embedded_parameters_mut(word);
        }

        if let Some(source) = word
            .strip_prefix("$(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            if command_substitution_spans_whole_word(word) {
                return self.expand_command_substitution_mut(source);
            }
        }

        // Embedded $() substitutions may contain full command lists or
        // compound commands, so use the mutable path that can execute an AST.
        if word.contains("$(") {
            return self.expand_embedded_parameters_mut(word);
        }

        self.expand_word(word)
    }

    pub(in crate::executor) fn expand_parameter_named_value(&self, name: &str) -> String {
        match name {
            "#" => return self.positional_params.len().to_string(),
            "@" | "*" => return self.positional_params.join(" "),
            "?" => return self.exit_code.to_string(),
            "$" => return std::process::id().to_string(),
            "!" => return self.last_background_pid_value(),
            "-" => return self.shell_option_flags(),
            "0" => return self.script_name_value(),
            _ => {}
        }

        if let Ok(index) = name.parse::<usize>() {
            return self
                .positional_params
                .get(index.saturating_sub(1))
                .cloned()
                .unwrap_or_default();
        }

        if is_shell_name(name) {
            return self
                .dynamic_parameter_value(name)
                .or_else(|| {
                    self.shell_variable_value(name)
                        .map(|value| shell_safe_value(&value))
                })
                .unwrap_or_default();
        }

        String::new()
    }

    pub(in crate::executor) fn parse_parameter_substring<'a>(
        &self,
        name: &'a str,
    ) -> Option<(&'a str, isize, Option<isize>)> {
        let (var_name, rest) = name.split_once(':')?;
        if var_name.is_empty() || matches!(rest.chars().next(), Some('=' | '+' | '?')) {
            return None;
        }
        if rest.starts_with('-') {
            return None;
        }

        let (offset, length) = rest.split_once(':').unwrap_or((rest, ""));
        let offset = offset.trim_start();
        if offset.is_empty() && length.is_empty() {
            return None;
        }

        let offset = if offset.is_empty() {
            0
        } else {
            self.eval_parameter_substring_offset(offset)?
        };
        let length = if length.is_empty() {
            None
        } else {
            Some(self.eval_parameter_substring_offset(length)?)
        };

        Some((var_name, offset, length))
    }

    pub(in crate::executor) fn eval_parameter_substring_offset(
        &self,
        value: &str,
    ) -> Option<isize> {
        let expression = value
            .strip_prefix("$((")
            .and_then(|inner| inner.strip_suffix("))"))
            .or_else(|| {
                value
                    .strip_prefix('(')
                    .and_then(|inner| inner.strip_suffix(')'))
            })
            .unwrap_or(value)
            .trim();
        let expression = self.expand_arithmetic_special_parameters(expression);
        let evaluated = eval_conditional_arith_value(&expression, &self.env_vars)?;
        isize::try_from(evaluated).ok()
    }
}
