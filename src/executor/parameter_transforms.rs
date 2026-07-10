use super::*;

impl Executor {
    pub(in crate::executor) fn parameter_assignment_transform(&self, name: &str) -> String {
        if let Some(array_name) = name
            .strip_suffix("[*]")
            .or_else(|| name.strip_suffix("[@]"))
        {
            let Some(array_name) = self.resolved_variable_name(array_name) else {
                return String::new();
            };
            return self.array_assignment_transform(&array_name);
        }

        if let Some((array_name, index)) = parse_array_numeric_subscript(name) {
            let Some(array_name) = self.resolved_variable_name(array_name) else {
                return String::new();
            };
            let Some(value) = self
                .env_vars
                .get(&array_name)
                .and_then(|value| array_value_at(value, index))
            else {
                return String::new();
            };
            let array_flag = if is_marked_var(&self.env_vars, ASSOC_VARS, &array_name) {
                "-A"
            } else {
                "-a"
            };
            return format!(
                "declare {array_flag} {array_name}={}",
                shell_single_quote_assignment_value(&value)
            );
        }

        if let Some((array_name, key)) = parse_array_subscript(name) {
            let Some(array_name) = self.resolved_variable_name(array_name) else {
                return String::new();
            };
            if !is_marked_var(&self.env_vars, ASSOC_VARS, &array_name) {
                return String::new();
            }
            let key = self.assoc_subscript_key(key);
            let Some(value) = self
                .env_vars
                .get(&array_name)
                .and_then(|value| assoc_value_at(value, &key))
            else {
                return String::new();
            };
            return format!(
                "declare -A {array_name}={}",
                shell_single_quote_assignment_value(&value)
            );
        }

        let Some(name) = self.resolved_variable_name(name) else {
            return String::new();
        };
        let name = name.as_str();

        if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
            if let Some(value) = self
                .env_vars
                .get(name)
                .and_then(|value| assoc_value_at(value, "0"))
            {
                return format!(
                    "declare -A {name}={}",
                    shell_single_quote_assignment_value(&value)
                );
            }
            return format!("declare -A {name}");
        }

        if self
            .env_vars
            .get(name)
            .is_some_and(|value| is_array_storage(value))
            || is_marked_array_var(&self.env_vars, name)
        {
            return self
                .env_vars
                .get(name)
                .and_then(|value| array_value_at(value, 0))
                .map(|value| {
                    format!(
                        "declare -a {name}={}",
                        shell_single_quote_assignment_value(&value)
                    )
                })
                .unwrap_or_else(|| format!("declare -a {name}"));
        }

        if !is_shell_name(name) {
            return String::new();
        }

        let Some(value) = self.env_vars.get(name) else {
            return String::new();
        };

        let rendered = shell_single_quote_assignment_value(value);
        let readonly = is_marked_var(&self.env_vars, READONLY_VARS, name);
        let exported = is_marked_var(&self.env_vars, EXPORTED_VARS, name);
        let integer = is_marked_var(&self.env_vars, INTEGER_VARS, name);
        let uppercase = is_marked_var(&self.env_vars, UPPERCASE_VARS, name);
        let lowercase = is_marked_var(&self.env_vars, LOWERCASE_VARS, name);

        let mut flags = String::from("-");
        if integer {
            flags.push('i');
        }
        if readonly {
            flags.push('r');
        }
        if exported {
            flags.push('x');
        }
        if lowercase {
            flags.push('l');
        }
        if uppercase {
            flags.push('u');
        }
        if flags.len() > 1 {
            format!("declare {flags} {name}={rendered}")
        } else {
            format!("{name}={rendered}")
        }
    }

    pub(in crate::executor) fn parameter_attribute_transform(&self, name: &str) -> String {
        let base_name = parse_array_subscript(name)
            .map(|(array_name, _)| array_name)
            .unwrap_or(name);
        let Some(base_name) = self.resolved_variable_name(base_name) else {
            return String::new();
        };
        let base_name = base_name.as_str();
        if !is_shell_name(base_name) || !self.env_vars.contains_key(base_name) {
            return String::new();
        }

        let mut attrs = String::new();
        if is_marked_var(&self.env_vars, ASSOC_VARS, base_name) {
            attrs.push('A');
        } else if self
            .env_vars
            .get(base_name)
            .is_some_and(|value| is_array_storage(value))
            || is_marked_array_var(&self.env_vars, base_name)
        {
            attrs.push('a');
        }
        if is_marked_var(&self.env_vars, INTEGER_VARS, base_name) {
            attrs.push('i');
        }
        if is_marked_var(&self.env_vars, READONLY_VARS, base_name) {
            attrs.push('r');
        }
        if is_marked_var(&self.env_vars, EXPORTED_VARS, base_name) {
            attrs.push('x');
        }
        if is_marked_var(&self.env_vars, LOWERCASE_VARS, base_name) {
            attrs.push('l');
        }
        if is_marked_var(&self.env_vars, UPPERCASE_VARS, base_name) {
            attrs.push('u');
        }
        attrs
    }

    pub(in crate::executor) fn parameter_key_value_transform(
        &self,
        name: &str,
        quoted: bool,
    ) -> String {
        let array_name = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"));

        if let Some(array_name) = array_name {
            let Some(array_name) = self.resolved_variable_name(array_name) else {
                return String::new();
            };
            let Some(value) = self.env_vars.get(&array_name) else {
                return String::new();
            };
            if is_marked_var(&self.env_vars, ASSOC_VARS, &array_name) {
                return assoc_entries(value)
                    .into_iter()
                    .map(|(key, value)| format_key_value_transform_part(&key, &value, quoted))
                    .collect::<Vec<_>>()
                    .join(" ");
            }

            return indexed_array_entries(value)
                .into_iter()
                .map(|(index, value)| {
                    format_key_value_transform_part(&index.to_string(), &value, quoted)
                })
                .collect::<Vec<_>>()
                .join(" ");
        }

        if let Some((array_name, key)) = parse_array_subscript(name) {
            let Some(array_name) = self.resolved_variable_name(array_name) else {
                return String::new();
            };
            let Some(value) = self.env_vars.get(&array_name) else {
                return String::new();
            };
            if is_marked_var(&self.env_vars, ASSOC_VARS, &array_name) {
                let key = self.assoc_subscript_key(key);
                return assoc_value_at(value, &key)
                    .map(|value| shell_single_quote_assignment_value(&value))
                    .unwrap_or_default();
            }
            if let Ok(index) = key.parse::<usize>() {
                return array_value_at(value, index)
                    .map(|value| shell_single_quote_assignment_value(&value))
                    .unwrap_or_default();
            }
            return String::new();
        }

        let Some(name) = self.resolved_variable_name(name) else {
            return String::new();
        };
        if let Some(value) = self.env_vars.get(&name) {
            if is_marked_var(&self.env_vars, ASSOC_VARS, &name) {
                return assoc_value_at(value, "0")
                    .map(|value| shell_single_quote_assignment_value(&value))
                    .unwrap_or_default();
            }
            if is_marked_array_var(&self.env_vars, &name) || is_array_storage(value) {
                return array_value_at(value, 0)
                    .map(|value| shell_single_quote_assignment_value(&value))
                    .unwrap_or_default();
            }
        }

        self.parameter_error_value(&name)
            .map(|value| shell_single_quote_assignment_value(&value))
            .unwrap_or_default()
    }

    pub(in crate::executor) fn parameter_prompt_transform(&self, name: &str) -> String {
        let Some(value) = self.parameter_error_value(name) else {
            return String::new();
        };
        self.expand_prompt_parameters(&self.decode_prompt_string(strip_matching_quotes(&value)))
    }
}
