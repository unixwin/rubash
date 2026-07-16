use super::*;

impl Executor {
    pub(in crate::executor) fn indirect_parameter_transform(
        &self,
        name: &str,
        transform: ParameterTransform,
    ) -> Option<String> {
        let indirect_name = name.strip_prefix('!')?;
        let ref_name = indirect_name
            .strip_suffix("[@]")
            .or_else(|| indirect_name.strip_suffix("[*]"));
        if ref_name.is_none() {
            let target_name = self.env_vars.get(indirect_name)?;
            if transform == ParameterTransform::Assignment {
                return Some(self.parameter_assignment_transform(target_name));
            }
            if transform == ParameterTransform::Attributes {
                return Some(self.parameter_attribute_transform(target_name));
            }
            if transform == ParameterTransform::KeyValueQuoted {
                return Some(self.parameter_key_value_transform(target_name, true));
            }
            if transform == ParameterTransform::KeyValueSplit {
                return Some(self.parameter_key_value_transform(target_name, false));
            }
            let value = self
                .array_element_parameter_value(target_name)
                .or_else(|| {
                    self.env_vars.get(target_name).and_then(|value| {
                        if is_array_storage(value)
                            || is_marked_array_var(&self.env_vars, target_name)
                        {
                            array_value_at(value, 0)
                        } else {
                            Some(value.clone())
                        }
                    })
                })
                .unwrap_or_default();
            return Some(self.apply_parameter_transform_value(&value, transform));
        }
        let ref_name = ref_name?;
        let target_name = self.env_vars.get(ref_name)?;
        let value = if let Some(array_expr) = target_name
            .strip_suffix("[@]")
            .or_else(|| target_name.strip_suffix("[*]"))
        {
            self.env_vars
                .get(array_expr)
                .and_then(|value| array_value_at(value, 0))
                .unwrap_or_default()
        } else {
            self.env_vars
                .get(target_name)
                .and_then(|value| {
                    if is_array_storage(value) || is_marked_array_var(&self.env_vars, target_name) {
                        array_value_at(value, 0)
                    } else {
                        Some(value.clone())
                    }
                })
                .unwrap_or_default()
        };
        Some(self.apply_parameter_transform_value(&value, transform))
    }

    pub(in crate::executor) fn expand_parameter_pattern_removal(
        &self,
        var_name: &str,
        pattern: &str,
        operation: PatternRemoval,
    ) -> Option<String> {
        let pattern = self.expand_parameter_pattern_word(pattern);
        if matches!(var_name, "@" | "*") {
            return Some(
                self.positional_params
                    .iter()
                    .map(|value| remove_parameter_pattern(value, &pattern, operation))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }

        if is_special_parameter_name(var_name) {
            return Some(remove_parameter_pattern(
                &self.expand_parameter_named_value(var_name),
                &pattern,
                operation,
            ));
        }

        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| remove_parameter_pattern(value, &pattern, operation))
                    .unwrap_or_default(),
            );
        }

        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(remove_parameter_pattern(&value, &pattern, operation));
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
                            .map(|value| remove_parameter_pattern(&value, &pattern, operation))
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, var_name)
                    })
                    .unwrap_or_default(),
            );
        }

        if is_shell_name(var_name) {
            return Some(
                self.parameter_pattern_scalar_value(var_name)
                    .map(|value| remove_parameter_pattern(&value, &pattern, operation))
                    .unwrap_or_default(),
            );
        }

        None
    }

    pub(in crate::executor) fn parameter_pattern_scalar_value(&self, name: &str) -> Option<String> {
        if is_special_parameter_name(name) {
            return Some(self.expand_parameter_named_value(name));
        }

        if let Some(value) = self.dynamic_parameter_value(name) {
            return Some(value);
        }

        let resolved = self.resolved_variable_name(name)?;
        let value = self.env_vars.get(&resolved)?;
        if is_marked_var(&self.env_vars, ARRAY_VARS, &resolved) {
            return Some(
                array_value_at(value, 0)
                    .or_else(|| assoc_value_at(value, "0"))
                    .unwrap_or_default(),
            );
        }

        Some(value.clone())
    }

    pub(in crate::executor) fn expand_parameter_pattern_word(&self, pattern: &str) -> String {
        let pattern = self.expand_embedded_parameters_preserving_escaped_single_quotes(pattern);
        decode_parameter_pattern_quotes(&pattern)
    }

    pub(in crate::executor) fn assoc_subscript_key(&self, key: &str) -> String {
        let expanded = self.expand_embedded_parameters(key);
        strip_matching_quotes(&expanded).to_string()
    }

    pub(in crate::executor) fn apply_array_element_parameter_assignment(
        &mut self,
        expression: &str,
        value: String,
    ) -> bool {
        let Some((array_name, key)) = parse_array_subscript(expression) else {
            return false;
        };
        let Some(array_name) = self.resolved_variable_name(array_name) else {
            return false;
        };
        let array_name = array_name.as_str();
        if !is_shell_name(array_name)
            || is_marked_var(&self.env_vars, READONLY_VARS, array_name)
            || is_noassign_bash_array(array_name)
        {
            return false;
        }

        if is_marked_var(&self.env_vars, ASSOC_VARS, array_name) {
            let key = self.assoc_subscript_key(key);
            let current = self.env_vars.get(array_name).cloned().unwrap_or_default();
            let mut entries = assoc_entries(&current);
            if let Some((_, entry_value)) = entries
                .iter_mut()
                .rev()
                .find(|(entry_key, _)| entry_key == &key)
            {
                *entry_value = value;
            } else {
                entries.push((key, value));
            }
            self.env_vars
                .insert(array_name.to_string(), format_assoc_storage(entries));
            return true;
        }

        let Some(index) = key.parse::<usize>().ok() else {
            return false;
        };
        let current = self.env_vars.get(array_name).cloned().unwrap_or_default();
        let mut entries = indexed_array_entries(&current);
        entries.insert(index, value);
        self.env_vars.insert(
            array_name.to_string(),
            format_indexed_array_storage(entries),
        );
        mark_env_name(&mut self.env_vars, ARRAY_VARS, array_name);
        true
    }
}
