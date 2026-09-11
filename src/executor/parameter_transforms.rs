use super::*;

impl Executor {
    pub(in crate::executor) fn parameter_assignment_transform(&self, name: &str) -> String {
        if let Some(array_name) = name
            .strip_suffix("[*]")
            .or_else(|| name.strip_suffix("[@]"))
        {
            // GNU get_var_and_type routes [@]/[*] through VT_ARRAYVAR only
            // when the variable actually is an array; a scalar variable falls
            // through to the string path, so ${VAR1[@]@A} renders the scalar
            // form (new-exp.tests new-exp15: `declare -rl VAR1`).
            let resolved = self.resolved_variable_name(array_name);
            if let Some(resolved_name) = resolved.as_deref() {
                if is_marked_var(&self.env_vars, ASSOC_VARS, resolved_name)
                    || is_marked_array_var(&self.env_vars, resolved_name)
                    || self
                        .env_vars
                        .get(resolved_name)
                        .is_some_and(|value| is_array_storage(value))
                {
                    return self.array_assignment_transform(resolved_name);
                }
            }
            return self.scalar_assignment_transform(array_name);
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
                // GNU string_var_assignment (subst.c:8645): a declared-unset
                // variable renders `declare -<flags> name` with no `=` body.
                let flags = self.variable_assignment_flags(&array_name, false);
                return format!("declare {flags} {array_name}");
            };
            let array_flag = if is_marked_var(&self.env_vars, ASSOC_VARS, &array_name) {
                "-A"
            } else {
                "-a"
            };
            return format!(
                "declare {array_flag} {array_name}={}",
                shell_reusable_quote(&value)
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
                return format!("declare -A {array_name}");
            };
            return format!("declare -A {array_name}={}", shell_reusable_quote(&value));
        }

        self.scalar_assignment_transform(name)
    }

    /// GNU string_var_assignment (subst.c:8645) for a scalar variable: the
    /// flags come from the variable cell in var_attribute_string order
    /// (builtins/setattr.def:415 — a/A, f, i, n, r, t, x, c, l, u; the i, r,
    /// x, l, u subset applies here), the value part is sh_quote_reusable, and
    /// a declared-unset variable keeps its attributes but loses the `=value`
    /// body. A variable with no attributes at all renders `name=value`.
    fn scalar_assignment_transform(&self, raw_name: &str) -> String {
        let Some(name) = self.resolved_variable_name(raw_name) else {
            return String::new();
        };
        let name = name.as_str();
        if !is_shell_name(name) {
            return String::new();
        }

        if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
            if let Some(value) = self
                .env_vars
                .get(name)
                .and_then(|value| assoc_value_at(value, "0"))
            {
                return format!("declare -A {name}={}", shell_reusable_quote(&value));
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
                .map(|value| format!("declare -a {name}={}", shell_reusable_quote(&value)))
                .unwrap_or_else(|| format!("declare -a {name}"));
        }

        let readonly = is_marked_var(&self.env_vars, READONLY_VARS, name);
        let exported = is_marked_var(&self.env_vars, EXPORTED_VARS, name);
        let integer = is_marked_var(&self.env_vars, INTEGER_VARS, name);
        let uppercase = is_marked_var(&self.env_vars, UPPERCASE_VARS, name);
        let lowercase = is_marked_var(&self.env_vars, LOWERCASE_VARS, name);

        let mut flags = String::new();
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

        match self.env_vars.get(name) {
            Some(value) => {
                let rendered = shell_reusable_quote(value);
                if flags.is_empty() {
                    format!("{name}={rendered}")
                } else {
                    format!("declare -{flags} {name}={rendered}")
                }
            }
            None => {
                // Declared-unset: attributes survive, the value does not
                // (subst.c:8652 val == NULL); with no attributes the whole
                // expansion disappears (string_transform returns NULL).
                if flags.is_empty() {
                    String::new()
                } else {
                    format!("declare -{flags} {name}")
                }
            }
        }
    }

    /// var_attribute_string attribute letters for an array-typed variable:
    /// the array attribute comes first (a indexed, A associative), then
    /// i, r, x, l, u in GNU's setattr.def order.
    pub(in crate::executor) fn variable_assignment_flags(
        &self,
        name: &str,
        array_typed: bool,
    ) -> String {
        let mut flags = String::new();
        if array_typed {
            if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
                flags.push('A');
            } else {
                flags.push('a');
            }
        }
        if is_marked_var(&self.env_vars, INTEGER_VARS, name) {
            flags.push('i');
        }
        if is_marked_var(&self.env_vars, READONLY_VARS, name) {
            flags.push('r');
        }
        if is_marked_var(&self.env_vars, EXPORTED_VARS, name) {
            flags.push('x');
        }
        if is_marked_var(&self.env_vars, LOWERCASE_VARS, name) {
            flags.push('l');
        }
        if is_marked_var(&self.env_vars, UPPERCASE_VARS, name) {
            flags.push('u');
        }
        flags
    }

    pub(in crate::executor) fn parameter_attribute_transform(&self, name: &str) -> String {
        let base_name = parse_array_subscript(name)
            .map(|(array_name, _)| array_name)
            .unwrap_or(name);
        let Some(base_name) = self.resolved_variable_name(base_name) else {
            return String::new();
        };
        let base_name = base_name.as_str();
        if !is_shell_name(base_name) {
            return String::new();
        }
        // GNU string_transform('a', v, 0) reports the attributes of any
        // variable cell that exists, including a declared-unset one
        // (var_attribute_string); a name with no variable cell at all yields
        // no output (string_transform returns NULL when v == 0).
        let has_cell = self.env_vars.contains_key(base_name)
            || is_marked_var(&self.env_vars, DECLARED_UNSET_VARS, base_name)
            || is_marked_var(&self.env_vars, READONLY_VARS, base_name)
            || is_marked_var(&self.env_vars, EXPORTED_VARS, base_name)
            || is_marked_var(&self.env_vars, INTEGER_VARS, base_name)
            || is_marked_var(&self.env_vars, UPPERCASE_VARS, base_name)
            || is_marked_var(&self.env_vars, LOWERCASE_VARS, base_name)
            || is_marked_var(&self.env_vars, ASSOC_VARS, base_name)
            || is_marked_array_var(&self.env_vars, base_name);
        if !has_cell {
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
                return assoc_hash_ordered_entries(value)
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
                    .map(|value| shell_reusable_quote(&value))
                    .unwrap_or_default();
            }
            if let Ok(index) = key.parse::<usize>() {
                return array_value_at(value, index)
                    .map(|value| shell_reusable_quote(&value))
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
                    .map(|value| shell_reusable_quote(&value))
                    .unwrap_or_default();
            }
            if is_marked_array_var(&self.env_vars, &name) || is_array_storage(value) {
                return array_value_at(value, 0)
                    .map(|value| shell_reusable_quote(&value))
                    .unwrap_or_default();
            }
        }

        self.parameter_error_value(&name)
            .map(|value| shell_reusable_quote(&value))
            .unwrap_or_default()
    }

    pub(in crate::executor) fn apply_parameter_transform_value(
        &self,
        value: &str,
        transform: ParameterTransform,
    ) -> String {
        if transform == ParameterTransform::Prompt {
            return self.expand_prompt_parameters(
                &self.decode_prompt_string(strip_matching_quotes(value)),
            );
        }
        apply_parameter_transform(value, transform)
    }
}
