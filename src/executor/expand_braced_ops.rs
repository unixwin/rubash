use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_operator_or_array_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        if let Some((var_name, word)) = name.split_once(":=") {
            if self
                .parameter_operator_value(var_name)
                .is_some_and(|value| !value.is_empty())
            {
                return Some(
                    self.parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default(),
                );
            }
            return Some(self.expand_parameter_word(word));
        }
        if let Some((var_name, word)) = name.split_once(":-") {
            if self
                .parameter_operator_value(var_name)
                .is_some_and(|value| !value.is_empty())
            {
                return Some(
                    self.parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default(),
                );
            }
            return Some(self.expand_parameter_word(word));
        }
        if let Some((var_name, word)) = name.split_once(":+") {
            if self
                .parameter_operator_value(var_name)
                .is_some_and(|value| !value.is_empty())
            {
                return Some(self.expand_parameter_word(word));
            }
            return Some(String::new());
        }
        if let Some((var_name, word)) = name.split_once('=') {
            return Some(
                self.parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_parameter_word(word)),
            );
        }
        if let Some((var_name, word)) = name.split_once('+') {
            if self.parameter_operator_value(var_name).is_some() {
                return Some(self.expand_parameter_word(word));
            }
            return Some(String::new());
        }
        if let Some((array_name, index)) = parse_array_integer_subscript(name) {
            if array_name == "GROUPS" {
                let Ok(index) = usize::try_from(index) else {
                    return Some(String::new());
                };
                return Some(self.group_value_at(index).unwrap_or_default());
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .and_then(|value| {
                        resolve_indexed_array_subscript(&value, index)
                            .and_then(|index| array_value_at(&value, index))
                    })
                    .map(normalize_array_expanded_value)
                    .unwrap_or_default(),
            );
        }
        if let Some((var_name, word)) = name.split_once('-') {
            return Some(
                self.parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_parameter_word(word)),
            );
        }
        if let Some((array_name, default)) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .and_then(|array_name| array_name.split_once('-').map(|_| (array_name, "")))
        {
            return Some(
                self.parameter_array_storage(array_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| self.join_array_parameter_values(&value, name))
                    .unwrap_or_else(|| default.to_string()),
            );
        }
        if let Some((array_expr, default)) = name.split_once('-') {
            if let Some(array_name) = array_expr
                .strip_suffix("[@]")
                .or_else(|| array_expr.strip_suffix("[*]"))
            {
                return Some(
                    self.parameter_array_storage(array_name)
                        .filter(|value| !value.is_empty())
                        .map(|value| self.join_array_parameter_values(&value, array_expr))
                        .unwrap_or_else(|| default.to_string()),
                );
            }
            return Some(
                self.shell_variable_value(array_expr)
                    .filter(|value| !value.is_empty() && !is_array_storage(value))
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| default.to_string()),
            );
        }
        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
        {
            if array_name == "GROUPS" {
                return Some(self.groups_words().join(" "));
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| self.join_array_parameter_values(&value, name))
                    .unwrap_or_default(),
            );
        }
        if let Some((array_name, index)) = parse_array_numeric_subscript(name) {
            if array_name == "GROUPS" {
                return Some(self.group_value_at(index).unwrap_or_default());
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .and_then(|value| array_value_at(&value, index))
                    .map(normalize_array_expanded_value)
                    .unwrap_or_default(),
            );
        }
        if let Some((array_name, key)) = parse_array_subscript(name) {
            if self.is_assoc_parameter_array(array_name) {
                let key = self.assoc_subscript_key(key);
                return Some(
                    self.parameter_array_storage(array_name)
                        .and_then(|value| assoc_value_at(&value, &key))
                        .unwrap_or_default(),
                );
            }
            if let Some(value) = self.array_element_parameter_value(name) {
                return Some(normalize_array_expanded_value(value));
            }
        }
        None
    }
}
