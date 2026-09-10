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
                    .map(|value| remove_parameter_pattern(value, &pattern, operation, self.extglob_enabled()))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }

        if is_special_parameter_name(var_name) {
            return Some(remove_parameter_pattern(
                &self.expand_parameter_named_value(var_name),
                &pattern,
                operation,
                self.extglob_enabled(),
            ));
        }

        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| remove_parameter_pattern(value, &pattern, operation, self.extglob_enabled()))
                    .unwrap_or_default(),
            );
        }

        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(remove_parameter_pattern(
                &value,
                &pattern,
                operation,
                self.extglob_enabled(),
            ));
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
                            .map(|value| remove_parameter_pattern(&value, &pattern, operation, self.extglob_enabled()))
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, var_name)
                    })
                    .unwrap_or_default(),
            );
        }

        if is_shell_name(var_name) {
            let value = self.parameter_pattern_scalar_value(var_name).unwrap_or_default();
            return Some(remove_parameter_pattern(
                &value,
                &pattern,
                operation,
                self.extglob_enabled(),
            ));
        }

        None
    }

    /// nocasematch shopt state for pattern substitution (GNU subst.c applies
    /// FNMATCH_IGNCASE in match_upattern when nocasematch is set).
    pub(in crate::executor) fn nocasematch_enabled(&self) -> bool {
        crate::builtins::shopt::option_enabled(&self.env_vars, "nocasematch")
    }

    /// extglob shopt state for pattern removal (GNU subst.c match_upattern
    /// passes FNM_EXTMATCH when the extglob option is on).
    pub(in crate::executor) fn extglob_enabled(&self) -> bool {
        crate::builtins::shopt::option_enabled(&self.env_vars, "extglob")
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
        // GNU subst.c resolves a bare array name to one element, not the whole
        // array (get_var_and_type -> VT_ARRAYVAR): associative arrays read key
        // "0" (assoc_cell), indexed arrays read element [0] (array_cell). Expanding
        // the raw storage marker here leaks ([FOO]=BAR) where GNU prints the
        // element value or empty when key "0" is absent.
        if is_marked_var(&self.env_vars, ASSOC_VARS, &resolved) {
            return Some(assoc_value_at(value, "0").unwrap_or_default());
        }

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
        // Decode quotes before embedded expansion so quoted glob
        // metacharacters stay marked, but mask nested braced parameters
        // first so the decoder does not tag glob chars inside an inner
        // expansion (keeps ? a glob in the inner removal).
        let mut masked = String::with_capacity(pattern.len());
        let mut rest = pattern;
        let mut slots: Vec<String> = Vec::new();
        while let Some(pos) = rest.find("${") {
            masked.push_str(&rest[..pos]);
            let after = &rest[pos + 2..];
            match matching_parameter_brace(after) {
                Some(end) => {
                    slots.push(format!("${{{}}}", &after[..end]));
                    masked.push('\x1c');
                    masked.push_str(&(slots.len() - 1).to_string());
                    rest = &after[end + 1..];
                }
                None => {
                    masked.push_str("${");
                    rest = after;
                }
            }
        }
        masked.push_str(rest);

        let decoded = decode_parameter_pattern_quotes(&masked).replace('\x1b', "");

        let mut restored = String::with_capacity(decoded.len());
        let mut rest = decoded.as_str();
        while let Some(pos) = rest.find('\x1c') {
            restored.push_str(&rest[..pos]);
            let digits: String = rest[pos + 1..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            if digits.is_empty() {
                restored.push('\x1c');
                rest = &rest[pos + 1..];
                continue;
            }
            let index: usize = digits.parse().unwrap_or(0);
            if let Some(slot) = slots.get(index) {
                restored.push_str(slot);
            }
            rest = &rest[pos + 1 + digits.len()..];
        }
        restored.push_str(rest);

        let expanded = self.expand_embedded_parameters_preserving_escaped_single_quotes(&restored);
        // Quoted glob metacharacters remain pattern literals. Preserve the
        // escape for the parameter matcher instead of exposing a raw marker.
        expanded.replace('\x11', "\\")
    }

    pub(in crate::executor) fn assoc_subscript_key(&self, key: &str) -> String {
        // GNU subst.c expand_subscript_string runs the subscript through
        // word expansion where a backslash quotes the next character:
        // `\$` yields a LITERAL `$` and never opens a substitution
        // (census-array2.sh V1: `A["x,\$(echo uname)"]=v` stores the key
        // `x,$(echo uname)`, it must not execute `echo uname` nor truncate
        // at the bracket). Convert `\$` to the $ marker BEFORE expansion;
        // expand_embedded_parameters restores it verbatim without treating
        // the `$` as an expansion start. Unescaped `$(cmd)` subscripts
        // still expand, matching GNU.
        let protected = if key.contains("\\$") {
            key.replace("\\$", "\u{1f}")
        } else {
            key.to_string()
        };
        let mut expanded = self
            .expand_embedded_parameters(&protected)
            // Double-quote quote removal (subst.c): a backslash keeps its
            // special meaning only before $ ` " \ and newline; the raw
            // subscript path stores `\$` literally and must shed it.
            .replace("\\$", "$")
            .replace("\\\"", "\"")
            .replace("\\'", "'");
        loop {
            let trimmed = expanded.trim_matches('\x1d');
            let stripped = strip_matching_quotes(trimmed);
            if stripped == trimmed {
                return stripped.trim_matches('\x1d').to_string();
            }
            expanded = stripped.to_string();
        }
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

        // Integer/uppercase/lowercase attributes transform the stored value
        // exactly like the declare assignment path does (arrayfunc.c). This
        // applies to both indexed and associative arrays.
        let value = if is_marked_var(&self.env_vars, INTEGER_VARS, array_name) {
            match self.eval_arithmetic_expansion_value(&value) {
                Some(evaluated) => evaluated.to_string(),
                None => value,
            }
        } else {
            value
        };
        let value = if is_marked_var(&self.env_vars, UPPERCASE_VARS, array_name) {
            value.to_uppercase()
        } else if is_marked_var(&self.env_vars, LOWERCASE_VARS, array_name) {
            value.to_lowercase()
        } else {
            value
        };

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

        // GNU evaluates indexed-array subscripts arithmetically at assignment
        // time (subst.c/eval_arith_subscript): ${a[$(echo 42)]=x} lands at
        // index 42 instead of being dropped as an unparseable literal key.
        // The @ and * subscripts are expansion operators, not arithmetic
        // operands, and keep their existing handling.
        let subscript = self.expand_arithmetic_special_parameters(&key);
        if matches!(subscript.as_str(), "@" | "*") || subscript.trim().is_empty() {
            return false;
        }
        let Some(index) = self.eval_arithmetic_expansion_value(&subscript) else {
            return false;
        };
        let Ok(index) = usize::try_from(index) else {
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