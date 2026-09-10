use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_special_or_indirect_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        match name {
            "#" => return Some(self.positional_params.len().to_string()),
            // GNU string_list_dollar_star: `*` joins with IFS[0] in scalar
            // contexts; `@` stays space-joined.
            "@" => return Some(self.positional_params.join(" ")),
            "*" => return Some(self.positional_params_star_joined()),
            "?" => return Some(self.exit_code.to_string()),
            "$" => return Some(self.shell_pid_value().to_string()),
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

        // Bash permits a parameter name to be another expansion, as in
        // `${$1}` and `${$(($2 + 2))}`. Resolve that name before the final
        // lookup; eval-heavy helpers such as bashdb's getopts_long depend on
        // preserving this positional indirection and its empty sentinel.
        if name.starts_with('$') {
            let target_name = self.expand_embedded_parameters(name);
            if target_name != name {
                return Some(self.expand_parameter_named_value(&target_name));
            }
        }

        let indirect_name = name.strip_prefix('!')?;
        // GNU param_expand (subst.c): a `!` immediately followed by an
        // operator character is the `$!` parameter with that operator
        // applied (`${!-ok 27}` -> "ok 27", `${!:-posparams}`), not an
        // indirect expansion. Fall through to the operator family, which
        // resolves the base `!` through parameter_operator_value.
        if matches!(indirect_name.chars().next(), Some('-' | '=' | '+' | ':')) {
            return None;
        }
        if has_indirect_parameter_word_operator(name) {
            return None;
        }
        if self.parse_parameter_substring(name).is_some() {
            return None;
        }
        if parse_parameter_replacement(name).is_some() {
            return None;
        }
        if parse_parameter_case_mod(name).is_some() {
            return None;
        }
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

        // GNU subst.c parameter_brace_expand_indir: the target may itself be
        // a special parameter, so the bang-question form expands the exit
        // status first and indirects through the result (posixexp2: with
        // status 0 it resolves to the shell name).
        if indirect_name == "?" {
            let target = self.exit_code.to_string();
            return Some(self.expand_parameter_named_value(&target));
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

        if let Some(value) = self.array_element_parameter_value(&target_name) {
            return Some(value);
        }

        // GNU chk_atstar (subst.c:7922): a target value of `@` or `*`
        // re-expands as $@/$*; scalar context joins with a space for @ and
        // IFS[0] for * (string_list_dollar_at / string_list_dollar_star).
        match target_name.as_str() {
            "@" => return Some(self.positional_params.join(" ")),
            "*" => {
                return Some(self.positional_params.join(&self.ifs_first_char_separator()))
            }
            _ => {}
        }

        // GNU parameter_brace_expand_word (subst.c:7955) re-expands the
        // target as a parameter: a value ending in `[@]`/`[*]` joins its
        // elements in scalar context (array_value AV_ALLOWALL branch,
        // arrayfunc.c:1513-1564), a bare array name reads element [0], and
        // a scalar reads its value cell (parameter_pattern_scalar_value).
        if (target_name.ends_with("[@]") || target_name.ends_with("[*]"))
            && Self::is_valid_indirect_array_reference(&target_name)
        {
            let values = self.indirect_target_values(&target_name);
            return Some(values.join(&self.ifs_first_char_separator()));
        }
        if target_name.is_empty() {
            return Some(String::new());
        }
        // indirect_target_values decodes a bare array name to element [0]
        // even when the ARRAY_VARS marker is missing (implicit
        // `name=(...)` assignments store array text unmarked); special
        // parameters still fall through to the parameter resolution.
        let mut target_values = self.indirect_target_values(&target_name);
        if target_values.len() == 1 {
            return Some(target_values.remove(0));
        }
        if target_values.len() > 1 {
            return Some(target_values.join(&self.ifs_first_char_separator()));
        }
        return Some(
            self.parameter_pattern_scalar_value(&target_name)
                .unwrap_or_default(),
        );
    }
}
