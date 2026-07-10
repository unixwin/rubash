use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_indexed_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        if name == "DIRSTACK[@]" || name == "DIRSTACK[*]" {
            return Some(crate::builtins::pushd::stack_words(&self.env_vars));
        }
        if let Some(index) = name
            .strip_prefix("DIRSTACK[")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|index| self.dirstack_subscript(index))
        {
            return Some(
                crate::builtins::pushd::stack_value(&self.env_vars, index).unwrap_or_default(),
            );
        }
        if let Some(array_name) = name.strip_prefix('#').and_then(|name| {
            name.strip_suffix("[@]")
                .or_else(|| name.strip_suffix("[*]"))
        }) {
            if array_name == "GROUPS" {
                return Some(self.groups_words().len().to_string());
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        if is_marked_array_var(&self.env_vars, array_name)
                            || is_array_storage(&value)
                        {
                            self.array_length(array_name)
                        } else {
                            1
                        }
                    })
                    .unwrap_or(0)
                    .to_string(),
            );
        }
        if let Some(var_name) = name.strip_prefix('#') {
            return Some(self.expand_braced_length_parameter(var_name));
        }
        if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
            return Some(self.expand_braced_substring_parameter(var_name, offset, length));
        }
        self.array_element_parameter_value(name)
    }

    fn expand_braced_length_parameter(&self, var_name: &str) -> String {
        if matches!(var_name, "@" | "*") {
            return self.positional_params.len().to_string();
        }
        if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
            return self
                .expand_parameter_named_value(var_name)
                .chars()
                .count()
                .to_string();
        }
        if let Some((array_name, index)) = parse_array_integer_subscript(var_name) {
            return self
                .env_vars
                .get(array_name)
                .and_then(|value| {
                    resolve_indexed_array_subscript(value, index)
                        .and_then(|index| array_value_at(value, index))
                })
                .map(|value| value.chars().count().to_string())
                .unwrap_or_else(|| "0".to_string());
        }
        if let Some((array_name, index)) = parse_array_numeric_subscript(var_name) {
            return self
                .env_vars
                .get(array_name)
                .and_then(|value| array_value_at(value, index))
                .map(|value| value.chars().count().to_string())
                .unwrap_or_else(|| "0".to_string());
        }
        if let Some((array_name, key)) = parse_array_subscript(var_name) {
            if self.is_assoc_parameter_array(array_name) {
                let key = self.assoc_subscript_key(key);
                return self
                    .parameter_array_storage(array_name)
                    .and_then(|value| assoc_value_at(&value, &key))
                    .map(|value| value.chars().count().to_string())
                    .unwrap_or_else(|| "0".to_string());
            }
        }
        if let Some(value) = self.dynamic_parameter_value(var_name) {
            return value.chars().count().to_string();
        }
        self.env_vars
            .get(var_name)
            .map(|value| {
                if value.starts_with('(') && value.ends_with(')') {
                    self.array_length(var_name).to_string()
                } else {
                    value.chars().count().to_string()
                }
            })
            .unwrap_or_else(|| "0".to_string())
    }

    fn expand_braced_substring_parameter(
        &self,
        var_name: &str,
        offset: isize,
        length: Option<isize>,
    ) -> String {
        if matches!(var_name, "@" | "*") {
            return positional_parameter_substring(&self.positional_params, offset, length)
                .join(" ");
        }
        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return self
                .parameter_array_storage(array_name)
                .map(|value| {
                    array_parameter_slice(
                        &value,
                        offset,
                        length.and_then(|length| usize::try_from(length).ok()),
                    )
                    .join(" ")
                })
                .unwrap_or_default();
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return parameter_substring(&value, offset, length);
        }
        if is_shell_name(var_name) {
            return self
                .env_vars
                .get(var_name)
                .map(|value| parameter_substring(value, offset, length))
                .unwrap_or_default();
        }
        String::new()
    }
}
