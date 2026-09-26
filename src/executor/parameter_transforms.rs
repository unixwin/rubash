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
                if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, resolved_name)
                    || is_marked_array_var(&self.shell_state.env_vars, resolved_name)
                    || self
                        .shell_state
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
                .shell_state
                .env_vars
                .get(&array_name)
                .and_then(|value| array_value_at(value, index))
            else {
                // GNU string_var_assignment (subst.c:8645): a declared-unset
                // variable renders `declare -<flags> name` with no `=` body.
                let flags = self.variable_assignment_flags(&array_name, false);
                return format!("declare {flags} {array_name}");
            };
            let array_flag = if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &array_name) {
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
            if !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &array_name) {
                return String::new();
            }
            let key = self.assoc_subscript_key(key);
            let Some(value) = self
                .shell_state
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

        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) {
            // GNU string_var_assignment on an assoc cell emits the full
            // var_attribute_string flag set (subst.c:8712), not just -A.
            let flags = self.variable_assignment_flags(name, true);
            if let Some(value) = self
                .shell_state
                .env_vars
                .get(name)
                .and_then(|value| assoc_value_at(value, "0"))
            {
                return format!("declare -{flags} {name}={}", shell_reusable_quote(&value));
            }
            return format!("declare -{flags} {name}");
        }

        if self
            .shell_state
            .env_vars
            .get(name)
            .is_some_and(|value| is_array_storage(value))
            || is_marked_array_var(&self.shell_state.env_vars, name)
        {
            let flags = self.variable_assignment_flags(name, true);
            return self
                .shell_state
                .env_vars
                .get(name)
                .and_then(|value| array_value_at(value, 0))
                .map(|value| format!("declare -{flags} {name}={}", shell_reusable_quote(&value)))
                .unwrap_or_else(|| format!("declare -{flags} {name}"));
        }

        let readonly = is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name);
        let exported = is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, name);
        let integer = is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, name);
        let uppercase = is_marked_var(&self.shell_state.env_vars, UPPERCASE_VARS, name);
        let lowercase = is_marked_var(&self.shell_state.env_vars, LOWERCASE_VARS, name);

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

        match self.shell_state.env_vars.get(name) {
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
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) {
                flags.push('A');
            } else {
                flags.push('a');
            }
        }
        if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, name) {
            flags.push('i');
        }
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name) {
            flags.push('r');
        }
        if is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, name) {
            flags.push('x');
        }
        if is_marked_var(&self.shell_state.env_vars, LOWERCASE_VARS, name) {
            flags.push('l');
        }
        if is_marked_var(&self.shell_state.env_vars, UPPERCASE_VARS, name) {
            flags.push('u');
        }
        flags
    }

    pub(in crate::executor) fn parameter_attribute_transform(&self, name: &str) -> String {
        // GNU array_transform -> list_transform (subst.c:8856+): `arr[@]@a`
        // maps the attribute string over the ELEMENT list — an allocated
        // array with N elements yields N copies (dollar_at space-joins
        // inside quotes, dollar_star joins with IFS[0]), so an
        // allocated-but-empty array (`foo=()`) yields nothing at all. A
        // declared-but-never-assigned array (array_cell == NULL) takes the
        // special case and reports the attributes once. The allocated cell
        // is distinguished by an env value without the DECLARED_UNSET
        // marker.
        let star_suffix = name
            .strip_suffix("[@]")
            .map(|base| (base, false))
            .or_else(|| name.strip_suffix("[*]").map(|base| (base, true)));
        if let Some((base, starred)) = star_suffix {
            let resolved = self
                .resolved_variable_name(base)
                .unwrap_or_else(|| base.to_string());
            let allocated = self.shell_state.env_vars.contains_key(&resolved)
                && !is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, &resolved);
            if allocated {
                let storage = self.parameter_array_storage(&resolved);
                let count = storage
                    .as_ref()
                    .map(|storage| {
                        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved) {
                            assoc_hash_ordered_values(
                                storage,
                                assoc_nbuckets(&self.shell_state.env_vars, &resolved),
                            )
                            .len()
                        } else {
                            array_values(storage).len()
                        }
                    })
                    .unwrap_or(0);
                let attrs = self.parameter_attribute_transform(&resolved);
                if count == 0 || attrs.is_empty() {
                    return String::new();
                }
                let sep = if starred {
                    self.ifs_first_char_separator()
                } else {
                    " ".to_string()
                };
                return vec![attrs; count].join(&sep);
            }
        }
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
        let has_cell = self.shell_state.env_vars.contains_key(base_name)
            || is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, base_name)
            || is_marked_var(&self.shell_state.env_vars, READONLY_VARS, base_name)
            || is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, base_name)
            || is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name)
            || is_marked_var(&self.shell_state.env_vars, UPPERCASE_VARS, base_name)
            || is_marked_var(&self.shell_state.env_vars, LOWERCASE_VARS, base_name)
            || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name)
            || is_marked_array_var(&self.shell_state.env_vars, base_name);
        if !has_cell {
            return String::new();
        }

        // GNU var_attribute_string (builtins/setattr.def:421-457) emits the
        // flags in this fixed order: a A f i n r t x c l u.
        let mut attrs = String::new();
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, base_name) {
            attrs.push('A');
        } else if self
            .shell_state
            .env_vars
            .get(base_name)
            .is_some_and(|value| is_array_storage(value))
            || is_marked_array_var(&self.shell_state.env_vars, base_name)
        {
            attrs.push('a');
        }
        if self.shell_state.functions.contains_key(base_name) {
            attrs.push('f');
        }
        if is_marked_var(&self.shell_state.env_vars, INTEGER_VARS, base_name) {
            attrs.push('i');
        }
        if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, base_name) {
            attrs.push('n');
        }
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, base_name) {
            attrs.push('r');
        }
        if is_marked_var(&self.shell_state.env_vars, TRACE_VARS, base_name) {
            attrs.push('t');
        }
        if is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, base_name) {
            attrs.push('x');
        }
        if is_marked_var(&self.shell_state.env_vars, CAPCASE_VARS, base_name) {
            attrs.push('c');
        }
        if is_marked_var(&self.shell_state.env_vars, LOWERCASE_VARS, base_name) {
            attrs.push('l');
        }
        if is_marked_var(&self.shell_state.env_vars, UPPERCASE_VARS, base_name) {
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
            let Some(value) = self.shell_state.env_vars.get(&array_name) else {
                return String::new();
            };
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &array_name) {
                // GNU assoc_to_kvpair (assoc.c:346) appends a space after
                // every `key "value"` element — including the last — while
                // the indexed array_to_kvpair (array.c:896) only separates,
                // and @k's string_list_pos_params path has no trailing pad.
                let joined = assoc_hash_ordered_entries(
                    value,
                    assoc_nbuckets(&self.shell_state.env_vars, &array_name),
                )
                .into_iter()
                .map(|(key, value)| format_key_value_transform_part(&key, &value, quoted))
                .collect::<Vec<_>>()
                .join(" ");
                return if quoted && !joined.is_empty() {
                    format!("{joined} ")
                } else {
                    joined
                };
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
            let Some(value) = self.shell_state.env_vars.get(&array_name) else {
                return String::new();
            };
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &array_name) {
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
        if let Some(value) = self.shell_state.env_vars.get(&name) {
            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &name) {
                return assoc_value_at(value, "0")
                    .map(|value| shell_reusable_quote(&value))
                    .unwrap_or_default();
            }
            if is_marked_array_var(&self.shell_state.env_vars, &name) || is_array_storage(value) {
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
