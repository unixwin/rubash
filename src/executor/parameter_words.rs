use super::*;

impl Executor {
    pub(in crate::executor) fn expand_parameter_word(&self, word: &str) -> String {
        // TODO(subst.c/parse.y): The `word` half of ${parameter:-word},
        // ${parameter:=word}, and ${parameter+word} has quote-aware expansion
        // flags. This covers tilde2.tests while the lexer still discards most
        // quote state.
        let expanded = decode_parameter_word_quotes(&self.expand_embedded_parameters(word));
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.home_value(), false)
    }

    pub(in crate::executor) fn expand_parameter_word_mut(&mut self, word: &str) -> String {
        let expanded = decode_parameter_word_quotes(&self.expand_embedded_parameters_mut(word));
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.home_value(), false)
    }

    pub(in crate::executor) fn expand_quoted_parameter_word(&self, word: &str) -> String {
        // TODO(subst.c/parse.y): Quoted parameter expansion should carry
        // CTLESC/CTLQUOTEMARK state from the parser. This preserves the
        // tilde2.tests distinction that quoted default/alternate words do not
        // perform tilde expansion.
        let Some(name) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return self.expand_embedded_parameters(word);
        };
        if !braced_parameter_spans_whole_word(word) {
            return self.expand_embedded_parameters(word);
        }

        if let Some((var_name, default)) = name.split_once(":-") {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(default));
            }
        }

        if let Some((var_name, alternate)) = name.split_once(":+") {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self.expand_embedded_parameters(alternate);
                }
                return String::new();
            }
        }

        if let Some((var_name, error_word)) = name.split_once(":?") {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                return self.expand_embedded_parameters(error_word);
            }
        }

        if let Some((var_name, error_word)) = name.split_once('?') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(error_word));
            }
        }

        if let Some((var_name, word)) = name.split_once(":=") {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(word));
            }
        }

        if let Some((var_name, word)) = name.split_once('=') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(word));
            }
        }

        if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
            return self.expand_braced_substring_parameter(var_name, offset, length);
        }

        if name.starts_with('#') {
            if let Some(value) = self.expand_braced_indexed_parameter(name) {
                return value;
            }
        }

        if let Some(value) = self.array_element_parameter_value(name) {
            return shell_safe_value(&value);
        }

        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))
        {
            return self
                .parameter_array_storage(array_name)
                .map(|value| self.join_array_parameter_values(&value, name))
                .unwrap_or_default();
        }

        if let Some((var_name, alternate)) = name.split_once('+') {
            if is_parameter_error_name(var_name) {
                if self.parameter_operator_value(var_name).is_some() {
                    return self.expand_embedded_parameters(alternate);
                }
                return String::new();
            }
        }

        if let Some((var_name, default)) = name.split_once('-') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(default));
            }
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        self.expand_word(word)
    }

    pub(in crate::executor) fn expand_quoted_parameter_word_mut(&mut self, word: &str) -> String {
        let Some(name) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return self.expand_embedded_parameters_mut(word);
        };
        if !braced_parameter_spans_whole_word(word) {
            return self.expand_embedded_parameters_mut(word);
        }

        if let Some((var_name, default)) = name.split_once(":-") {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters_mut(default));
            }
        }

        if let Some((var_name, alternate)) = name.split_once(":+") {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self.expand_embedded_parameters_mut(alternate);
                }
                return String::new();
            }
        }

        if let Some((var_name, error_word)) = name.split_once(":?") {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                return self.expand_embedded_parameters_mut(error_word);
            }
        }

        if let Some((var_name, error_word)) = name.split_once('?') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters_mut(error_word));
            }
        }

        if let Some((var_name, word)) = name.split_once(":=") {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters_mut(word));
            }
        }

        if let Some((var_name, word)) = name.split_once('=') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters_mut(word));
            }
        }

        if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
            return self.expand_braced_substring_parameter(var_name, offset, length);
        }

        if name.starts_with('#') {
            if let Some(value) = self.expand_braced_indexed_parameter(name) {
                return value;
            }
        }

        if let Some(value) = self.array_element_parameter_value(name) {
            return shell_safe_value(&value);
        }

        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))
        {
            return self
                .parameter_array_storage(array_name)
                .map(|value| self.join_array_parameter_values(&value, name))
                .unwrap_or_default();
        }

        if let Some((var_name, alternate)) = name.split_once('+') {
            if is_parameter_error_name(var_name) {
                if self.parameter_operator_value(var_name).is_some() {
                    return self.expand_embedded_parameters_mut(alternate);
                }
                return String::new();
            }
        }

        if let Some((var_name, default)) = name.split_once('-') {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters_mut(default));
            }
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        self.expand_word(word)
    }

    pub(in crate::executor) fn apply_parameter_assignment_expansions(&mut self, cmd: &CommandNode) {
        // TODO(subst.c): Assignment operators should be part of normal word
        // expansion. Rubash's word expansion is still immutable, so apply the
        // simple shell-name side effects before command dispatch.
        for word in &cmd.words[1..] {
            self.apply_parameter_assignment_expansions_in_word(word);
        }
    }

    pub(in crate::executor) fn apply_parameter_assignment_expansions_in_word(
        &mut self,
        word: &str,
    ) {
        let mut rest = word;
        while let Some(start) = rest.find("${") {
            rest = &rest[start + 2..];
            let Some(end) = matching_parameter_brace(rest) else {
                break;
            };
            let inner = &rest[..end];
            self.apply_parameter_assignment_expansion(inner);
            rest = &rest[end + 1..];
        }
    }

    pub(in crate::executor) fn apply_parameter_assignment_expansion(&mut self, inner: &str) {
        if let Some((name, value)) = inner.split_once(":=") {
            if self
                .parameter_operator_value(name)
                .is_some_and(|value| !value.is_empty())
            {
                return;
            }
            let value = self.expand_parameter_word_mut(value);
            if self.apply_array_element_parameter_assignment(name, value.clone()) {
                return;
            }
            if self.apply_indirect_parameter_assignment(name, value.clone()) {
                return;
            }
            if !is_shell_name(name) {
                return;
            }
            self.apply_shell_assignment(name, value);
            return;
        }

        if let Some((name, value)) = inner.split_once('=') {
            if self.parameter_operator_value(name).is_some() {
                return;
            }
            let value = self.expand_parameter_word_mut(value);
            if self.apply_array_element_parameter_assignment(name, value.clone()) {
                return;
            }
            if self.apply_indirect_parameter_assignment(name, value.clone()) {
                return;
            }
            if !is_shell_name(name) {
                return;
            }
            self.apply_shell_assignment(name, value);
        }
    }

    fn apply_indirect_parameter_assignment(&mut self, name: &str, value: String) -> bool {
        let Some(indirect_name) = name.strip_prefix('!') else {
            return false;
        };
        if self.nameref_target_name(indirect_name).is_some() {
            return false;
        }
        let Some(target_name) = self.env_vars.get(indirect_name).cloned() else {
            return false;
        };
        if self.apply_array_element_parameter_assignment(&target_name, value.clone()) {
            return true;
        }
        if !is_shell_name(&target_name) {
            return false;
        }
        self.apply_shell_assignment(&target_name, value);
        true
    }
}
