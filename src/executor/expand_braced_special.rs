use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_special_or_indirect_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        match name {
            "#" => return Some(self.positional_params.len().to_string()),
            "@" | "*" => return Some(self.positional_params.join(" ")),
            "?" => return Some(self.exit_code.to_string()),
            "$" => return Some(std::process::id().to_string()),
            "!" => return Some(self.last_background_pid_value()),
            "-" => return Some(self.shell_option_flags()),
            "0" => return Some(self.script_name_value()),
            _ => {}
        }

        if let Ok(index) = name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .cloned()
                    .unwrap_or_default(),
            );
        }

        let indirect_name = name.strip_prefix('!')?;
        if let Some((var_name, transform)) = parse_parameter_transform(name) {
            if let Some(value) = self.indirect_parameter_transform(var_name, transform) {
                return Some(value);
            }
        }
        if let Some(value) = self.indirect_pattern_removal(indirect_name) {
            return Some(value);
        }

        if let Some(array_name) = indirect_name
            .strip_suffix("[@]")
            .or_else(|| indirect_name.strip_suffix("[*]"))
        {
            let storage_name = self.resolved_variable_name(array_name);
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        if storage_name
                            .as_deref()
                            .is_some_and(|name| is_marked_var(&self.env_vars, ASSOC_VARS, name))
                        {
                            assoc_keys(&value).join(" ")
                        } else {
                            array_indices(&value).join(" ")
                        }
                    })
                    .unwrap_or_default(),
            );
        }

        if let Some(prefix) = indirect_name
            .strip_suffix('*')
            .or_else(|| indirect_name.strip_suffix('@'))
        {
            let mut names: Vec<&str> = self
                .env_vars
                .keys()
                .map(String::as_str)
                .filter(|name| name.starts_with(prefix))
                .collect();
            names.sort_unstable();
            return Some(names.join(" "));
        }

        if indirect_name == "#" {
            return Some(self.positional_params.last().cloned().unwrap_or_default());
        }

        if is_shell_name(indirect_name) {
            if let Some(target_name) = self.nameref_target_name(indirect_name) {
                return Some(target_name);
            }
        }

        let target_name = if let Ok(index) = indirect_name.parse::<usize>() {
            self.positional_params
                .get(index.saturating_sub(1))
                .cloned()
                .unwrap_or_default()
        } else {
            self.env_vars
                .get(indirect_name)
                .cloned()
                .unwrap_or_default()
        };

        Some(self.expand_parameter_named_value(&target_name))
    }
}
