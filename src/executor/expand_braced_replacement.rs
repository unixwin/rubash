use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_replacement_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        let (var_name, pattern, replacement, global) = parse_parameter_replacement(name)?;
        let pattern = self.expand_parameter_pattern_word(pattern);
        let replacement = decode_parameter_replacement_quotes(
            &self.expand_embedded_parameters_preserving_escaped_single_quotes(replacement),
        );
        if matches!(var_name, "@" | "*") {
            return Some(
                self.positional_params
                    .iter()
                    .map(|value| replace_parameter_pattern(value, &pattern, &replacement, global))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| replace_parameter_pattern(value, &pattern, &replacement, global))
                    .unwrap_or_default(),
            );
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(replace_parameter_pattern(
                &value,
                &pattern,
                &replacement,
                global,
            ));
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
                            .map(|value| {
                                replace_parameter_pattern(&value, &pattern, &replacement, global)
                            })
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, var_name)
                    })
                    .unwrap_or_default(),
            );
        }
        if is_shell_name(var_name) {
            return Some(
                self.dynamic_parameter_value(var_name)
                    .or_else(|| self.env_vars.get(var_name).cloned())
                    .map(|value| replace_parameter_pattern(&value, &pattern, &replacement, global))
                    .unwrap_or_default(),
            );
        }
        None
    }
}
