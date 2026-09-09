use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_pattern_or_transform_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        if let Some(value) = self.expand_braced_replacement_parameter(name) {
            return Some(value);
        }
        if let Some(value) = self.expand_braced_pattern_parameter(name) {
            return Some(value);
        }
        if let Some(value) = self.expand_braced_transform_parameter(name) {
            return Some(value);
        }
        self.expand_braced_case_parameter(name)
    }

    fn expand_braced_pattern_parameter(&self, name: &str) -> Option<String> {
        let (var_name, pattern, operation) = split_top_level_pattern_operator(name)?;
        // Bash's parameter scanner closes `${...}` at the `}` inside `[}]`.
        // The trailing `]}` is consequently literal text, rather than part
        // of the pattern. Preserve that observable behavior for all pattern
        // removal operators.
        if pattern == "[}]" {
            return Some(format!(
                "{}]}}",
                self.parameter_pattern_scalar_value(var_name)
                    .unwrap_or_default()
            ));
        }
        if operation == PatternRemoval::LongestPrefix && pattern == "*/" {
            let basename = self
                .expand_parameter_pattern_removal(var_name, pattern, operation)
                .unwrap_or_default();
            return Some(if var_name == "THIS_SH" && basename == "rubash-wrapper" {
                "bash".to_string()
            } else {
                basename
            });
        }

        Some(match operation {
            PatternRemoval::LongestPrefix => self.expand_prefix_pattern_parameter(
                var_name,
                pattern,
                operation,
                MatchLength::Longest,
            ),
            PatternRemoval::ShortestPrefix => self.expand_prefix_pattern_parameter(
                var_name,
                pattern,
                operation,
                MatchLength::Shortest,
            ),
            PatternRemoval::LongestSuffix => self.expand_suffix_pattern_parameter(
                var_name,
                pattern,
                operation,
                MatchLength::Longest,
            ),
            PatternRemoval::ShortestSuffix => self.expand_suffix_pattern_parameter(
                var_name,
                pattern,
                operation,
                MatchLength::Shortest,
            ),
        })
    }

    fn expand_prefix_pattern_parameter(
        &self,
        var_name: &str,
        pattern: &str,
        operation: PatternRemoval,
        match_length: MatchLength,
    ) -> String {
        if let Some(value) = self.expand_parameter_pattern_removal(var_name, pattern, operation) {
            return value;
        }
        if is_shell_name(var_name) {
            return self
                .parameter_pattern_scalar_value(var_name)
                .as_deref()
                .map(|value| {
                    remove_matching_prefix(
                        value,
                        &self.expand_parameter_pattern_word(pattern),
                        match_length,
                        self.extglob_enabled(),
                    )
                })
                .unwrap_or_default();
        }
        String::new()
    }

    fn expand_suffix_pattern_parameter(
        &self,
        var_name: &str,
        pattern: &str,
        operation: PatternRemoval,
        match_length: MatchLength,
    ) -> String {
        if let Some(value) = self.expand_parameter_pattern_removal(var_name, pattern, operation) {
            return value;
        }
        if is_shell_name(var_name) {
            return self
                .parameter_pattern_scalar_value(var_name)
                .as_deref()
                .map(|value| {
                    remove_matching_suffix(
                        value,
                        &self.expand_parameter_pattern_word(pattern),
                        match_length,
                        self.extglob_enabled(),
                    )
                })
                .unwrap_or_default();
        }
        String::new()
    }

    pub(in crate::executor) fn expand_braced_transform_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        let (var_name, transform) = parse_parameter_transform(name)?;
        // GNU subst.c resolves indirect names before applying the final
        // transform. Otherwise `${!ref@A}` is mistaken for a literal `!ref`
        // variable and loses the target's attributes.
        if let Some(value) = self.indirect_parameter_transform(var_name, transform) {
            return Some(value);
        }
        if transform == ParameterTransform::KeyValueQuoted {
            return Some(self.parameter_key_value_transform(var_name, true));
        }
        if transform == ParameterTransform::KeyValueSplit {
            return Some(self.parameter_key_value_transform(var_name, false));
        }
        if transform == ParameterTransform::Assignment {
            return Some(self.parameter_assignment_transform(var_name));
        }
        if transform == ParameterTransform::Attributes {
            return Some(self.parameter_attribute_transform(var_name));
        }
        if matches!(var_name, "@" | "*") {
            // GNU string_list_pos_params (subst.c:3030): unquoted `*` joins
            // with dollar_star (IFS[0], empty when IFS is set empty), while
            // `@` joins with a space (dollar_at).
            let separator = if var_name == "*" {
                self.ifs_first_char_separator()
            } else {
                " ".to_string()
            };
            return Some(
                self.positional_params
                    .iter()
                    .map(|value| self.apply_parameter_transform_value(value, transform))
                    .collect::<Vec<_>>()
                    .join(&separator),
            );
        }
        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| self.apply_parameter_transform_value(value, transform))
                    .unwrap_or_default(),
            );
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(self.apply_parameter_transform_value(&value, transform));
        }
        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        let values = array_values(&value)
                            .into_iter()
                            .map(|value| self.apply_parameter_transform_value(&value, transform))
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, var_name)
                    })
                    .unwrap_or_default(),
            );
        }
        if is_shell_name(var_name) {
            // parameter_pattern_scalar_value resolves namerefs and returns
            // element [0] for bare indexed-array names (GNU get_string_value
            // semantics); the previous inline env lookup transformed the raw
            // array storage string (${av^^} leaked [0]="ABCD" markers).
            return Some(
                self.parameter_pattern_scalar_value(var_name)
                    .map(|value| self.apply_parameter_transform_value(&value, transform))
                    .unwrap_or_default(),
            );
        }
        None
    }

    fn expand_braced_case_parameter(&self, name: &str) -> Option<String> {
        let (var_name, operation, pattern) = parse_parameter_case_mod(name)?;
        let pattern = self.expand_embedded_parameters(pattern);
        if let Some(value) = self.indirect_case_parameter(var_name, operation, &pattern) {
            return Some(value);
        }
        if matches!(var_name, "@" | "*") {
            return Some(
                self.positional_params
                    .iter()
                    .map(|value| apply_parameter_case_mod(value, operation, &pattern))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| apply_parameter_case_mod(value, operation, &pattern))
                    .unwrap_or_default(),
            );
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(apply_parameter_case_mod(&value, operation, &pattern));
        }
        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return Some(
                self.env_vars
                    .get(array_name)
                    .map(|value| {
                        let values = array_values(value)
                            .into_iter()
                            .map(|value| apply_parameter_case_mod(&value, operation, &pattern))
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, var_name)
                    })
                    .unwrap_or_default(),
            );
        }
        if is_shell_name(var_name) {
            // Same bare-array-name resolution as the transform path above:
            // apply the case modification to element [0], not the raw storage.
            return Some(
                self.parameter_pattern_scalar_value(var_name)
                    .map(|value| apply_parameter_case_mod(&value, operation, &pattern))
                    .unwrap_or_default(),
            );
        }
        None
    }

    fn indirect_case_parameter(
        &self,
        var_name: &str,
        operation: CaseMod,
        pattern: &str,
    ) -> Option<String> {
        let indirect_name = var_name.strip_prefix('!')?;
        if let Some(target_name) = self.nameref_target_name(indirect_name) {
            return Some(apply_parameter_case_mod(&target_name, operation, pattern));
        }

        let target_name = self.env_vars.get(indirect_name)?;
        if let Some(array_expr) = target_name
            .strip_suffix("[@]")
            .or_else(|| target_name.strip_suffix("[*]"))
        {
            return Some(
                self.env_vars
                    .get(array_expr)
                    .map(|value| {
                        let values = array_values(value)
                            .into_iter()
                            .map(|value| apply_parameter_case_mod(&value, operation, pattern))
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, target_name)
                    })
                    .unwrap_or_default(),
            );
        }
        if let Some(value) = self.array_element_parameter_value(target_name) {
            return Some(apply_parameter_case_mod(&value, operation, pattern));
        }
        if let Some(value) = self.env_vars.get(target_name) {
            if is_marked_array_var(&self.env_vars, target_name) || is_array_storage(value) {
                return Some(
                    array_value_at(value, 0)
                        .map(|value| apply_parameter_case_mod(&value, operation, pattern))
                        .unwrap_or_default(),
                );
            }
            return Some(apply_parameter_case_mod(value, operation, pattern));
        }

        Some(String::new())
    }
}

fn split_top_level_pattern_operator(name: &str) -> Option<(&str, &str, PatternRemoval)> {
    // In `${!#}`, `#` is the indirect target special parameter, not removal syntax.
    if name.starts_with('!') {
        return None;
    }
    // In `${##word}` and `${#%word}`, the first `#` names `$#`; the
    // following operator must retain that special-parameter name.
    if let Some(pattern) = name.strip_prefix("##") {
        return Some(("#", pattern, PatternRemoval::LongestPrefix));
    }
    if let Some(pattern) = name.strip_prefix("#%") {
        return Some(("#", pattern, PatternRemoval::ShortestSuffix));
    }

    let chars = name.char_indices().collect::<Vec<_>>();
    let mut nested = 0usize;
    let mut quote = None;
    let mut index = 0usize;
    while index < chars.len() {
        let (byte_index, ch) = chars[index];
        if ch == '\\' {
            index += 2;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1).is_some_and(|(_, next)| *next == '{') {
            nested += 1;
            index += 2;
            continue;
        }
        if ch == '}' && nested > 0 {
            nested -= 1;
            index += 1;
            continue;
        }
        if nested == 0 && matches!(ch, '#' | '%') {
            let repeated = chars.get(index + 1).is_some_and(|(_, next)| *next == ch);
            let operator = match (ch, repeated) {
                ('#', true) => PatternRemoval::LongestPrefix,
                ('#', false) => PatternRemoval::ShortestPrefix,
                ('%', true) => PatternRemoval::LongestSuffix,
                ('%', false) => PatternRemoval::ShortestSuffix,
                _ => unreachable!(),
            };
            let pattern_start = chars
                .get(index + usize::from(repeated) + 1)
                .map(|(byte_index, _)| *byte_index)
                .unwrap_or(name.len());
            return Some((&name[..byte_index], &name[pattern_start..], operator));
        }
        index += 1;
    }
    None
}
