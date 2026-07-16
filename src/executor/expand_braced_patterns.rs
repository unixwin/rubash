use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_pattern_or_transform_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        if let Some(value) = self.expand_braced_pattern_parameter(name) {
            return Some(value);
        }
        if let Some(value) = self.expand_braced_transform_parameter(name) {
            return Some(value);
        }
        if let Some(value) = self.expand_braced_case_parameter(name) {
            return Some(value);
        }
        self.expand_braced_replacement_parameter(name)
    }

    fn expand_braced_pattern_parameter(&self, name: &str) -> Option<String> {
        if let Some((var_name, _pattern)) = name.split_once("##*/") {
            return Some(
                self.parameter_pattern_scalar_value(var_name)
                    .as_deref()
                    .and_then(|value| value.rsplit('/').next())
                    .map(|basename| {
                        if var_name == "THIS_SH" && basename == "rubash-wrapper" {
                            "bash"
                        } else {
                            basename
                        }
                    })
                    .unwrap_or_default()
                    .to_string(),
            );
        }
        if let Some((var_name, pattern)) = name.split_once("##") {
            return Some(self.expand_prefix_pattern_parameter(
                var_name,
                pattern,
                PatternRemoval::LongestPrefix,
                MatchLength::Longest,
            ));
        }
        if let Some((var_name, pattern)) = name.split_once('#') {
            return Some(self.expand_prefix_pattern_parameter(
                var_name,
                pattern,
                PatternRemoval::ShortestPrefix,
                MatchLength::Shortest,
            ));
        }
        if let Some((var_name, pattern)) = name.split_once("%%") {
            return Some(self.expand_suffix_pattern_parameter(
                var_name,
                pattern,
                PatternRemoval::LongestSuffix,
                MatchLength::Longest,
            ));
        }
        if let Some((var_name, pattern)) = name.split_once('%') {
            return Some(self.expand_suffix_pattern_parameter(
                var_name,
                pattern,
                PatternRemoval::ShortestSuffix,
                MatchLength::Shortest,
            ));
        }
        None
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
                        &self.expand_embedded_parameters(pattern),
                        match_length,
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
                        &self.expand_embedded_parameters(pattern),
                        match_length,
                    )
                })
                .unwrap_or_default();
        }
        String::new()
    }

    fn expand_braced_transform_parameter(&self, name: &str) -> Option<String> {
        let (var_name, transform) = parse_parameter_transform(name)?;
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
        if transform == ParameterTransform::Prompt {
            return Some(self.parameter_prompt_transform(var_name));
        }
        if let Some(value) = self.indirect_parameter_transform(var_name, transform) {
            return Some(value);
        }
        if matches!(var_name, "@" | "*") {
            return Some(
                self.positional_params
                    .iter()
                    .map(|value| apply_parameter_transform(value, transform))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| apply_parameter_transform(value, transform))
                    .unwrap_or_default(),
            );
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(apply_parameter_transform(&value, transform));
        }
        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        array_values(&value)
                            .into_iter()
                            .map(|value| apply_parameter_transform(&value, transform))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default(),
            );
        }
        if is_shell_name(var_name) {
            let Some(name) = self.resolved_variable_name(var_name) else {
                return Some(String::new());
            };
            if let Some(value) = self.env_vars.get(&name) {
                if is_marked_var(&self.env_vars, ASSOC_VARS, &name) {
                    return Some(
                        assoc_value_at(value, "0")
                            .map(|value| apply_parameter_transform(&value, transform))
                            .unwrap_or_default(),
                    );
                }
                if is_marked_array_var(&self.env_vars, &name) || is_array_storage(value) {
                    return Some(
                        array_value_at(value, 0)
                            .map(|value| apply_parameter_transform(&value, transform))
                            .unwrap_or_default(),
                    );
                }
                return Some(apply_parameter_transform(value, transform));
            }
            return Some(String::new());
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
                        array_values(value)
                            .into_iter()
                            .map(|value| apply_parameter_case_mod(&value, operation, &pattern))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default(),
            );
        }
        if is_shell_name(var_name) {
            return Some(
                self.env_vars
                    .get(var_name)
                    .map(|value| apply_parameter_case_mod(value, operation, &pattern))
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
                        array_values(value)
                            .into_iter()
                            .map(|value| apply_parameter_case_mod(&value, operation, pattern))
                            .collect::<Vec<_>>()
                            .join(" ")
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
