use super::*;

impl Executor {
    pub(in crate::executor) fn update_underscore_parameter(&mut self, cmd: &CommandNode) {
        if let Some(value) = cmd.words.last() {
            self.env_vars.insert("_".to_string(), value.clone());
            set_process_env("_", value);
        }
    }

    pub(in crate::executor) fn removes_unquoted_null_word(
        &self,
        cmd: &CommandNode,
        index: usize,
    ) -> bool {
        if cmd.words.first().is_some_and(|word| word == "[[") {
            return false;
        }

        cmd.word_kinds
            .get(index)
            .is_some_and(|kind| *kind == TokenKind::Variable)
    }

    pub(in crate::executor) fn splits_unquoted_expanded_word(
        &self,
        cmd: &CommandNode,
        index: usize,
        expanded: &str,
    ) -> bool {
        let unquoted_variable = cmd
            .word_kinds
            .get(index)
            .is_some_and(|kind| *kind == TokenKind::Variable);
        let unquoted_command_substitution = cmd
            .words
            .get(index)
            .is_some_and(|word| word_has_unquoted_command_substitution(word));
        let unquoted_indirect_name_list = cmd
            .words
            .get(index)
            .is_some_and(|word| word_is_unquoted_indirect_name_list(word));

        ((unquoted_variable && expanded.contains(['\n', '\t']))
            || (unquoted_command_substitution && expanded.contains(char::is_whitespace))
            || (unquoted_indirect_name_list && expanded.contains(char::is_whitespace)))
            && expanded.split_whitespace().nth(1).is_some()
    }

    pub(in crate::executor) fn expand_for_word_values_result(
        &self,
        word: &str,
    ) -> Result<Vec<String>, String> {
        if let Some(values) = self.array_at_word_values(word) {
            return Ok(values);
        }
        if self.is_brace_expand_enabled() && !word.contains("${") {
            let braced = crate::expand::braces::expand_braces(word);
            if braced.len() > 1 {
                let values = braced
                    .into_iter()
                    .map(|word| self.expand_for_brace_word_values(&word))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .flatten()
                    .collect();
                return Ok(values);
            }
        }

        self.expand_for_brace_word_values(word)
    }

    fn expand_for_brace_word_values(&self, word: &str) -> Result<Vec<String>, String> {
        let expanded = self.expand_word(word);
        if for_word_has_unquoted_expansion(word) {
            return Ok(expanded.split_whitespace().map(str::to_string).collect());
        }
        // Apply glob expansion for for-loop words
        match glob::pathname_expand_word(&expanded, &self.env_vars) {
            glob::PathnameExpansion::Matches(matches) => Ok(matches),
            glob::PathnameExpansion::NoMatch => Ok(vec![expanded]),
            glob::PathnameExpansion::Fail(pattern) => Err(pattern),
        }
    }

    pub(in crate::executor) fn field_split_values(&self, value: &str) -> Vec<String> {
        field_split_values_with_ifs(value, self.env_vars.get("IFS").map(String::as_str))
    }

    pub(in crate::executor) fn expand_escaped_indirect_parameter_literal(
        &self,
        value: &str,
    ) -> Option<String> {
        let marker = "\\${$";
        let start = value.find(marker)?;
        let mut output = String::new();
        output.push_str(&value[..start]);
        let mut index = start + marker.len();
        let rest = &value[index..];
        let mut name = String::new();
        for ch in rest.chars() {
            if !is_shell_name_char(ch) {
                break;
            }
            name.push(ch);
            index += ch.len_utf8();
        }
        if name.is_empty() {
            return None;
        }
        let tail = &value[index..];
        let end = tail.find('}')?;
        let resolved = self.expand_embedded_parameters(&format!("${name}"));
        output.push_str("${");
        output.push_str(&resolved);
        output.push_str(&tail[..end]);
        output.push('}');
        output.push_str(&tail[end + 1..]);
        Some(output)
    }
}

fn word_is_unquoted_indirect_name_list(word: &str) -> bool {
    let Some(inner) = word
        .strip_prefix("${!")
        .and_then(|word| word.strip_suffix('}'))
    else {
        return false;
    };

    inner
        .strip_suffix("[@]")
        .or_else(|| inner.strip_suffix("[*]"))
        .is_some_and(|name| !name.is_empty())
        || inner
            .strip_suffix('*')
            .or_else(|| inner.strip_suffix('@'))
            .is_some_and(|prefix| !prefix.is_empty())
}
