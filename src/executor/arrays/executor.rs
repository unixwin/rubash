use super::*;
use crate::executor::{assoc_hash_ordered_entries, assoc_hash_ordered_values, assoc_keys, NAMEREF_VARS};

impl Executor {
    pub(in crate::executor) fn indexed_array_stack(&self, name: &str) -> Vec<String> {
        match name {
            "PIPESTATUS" => return self.pipestatus_values(),
            "FUNCNAME" => {
                let mut stack = self.function_name_stack.clone();
                if !stack.is_empty() && stack.last().map(String::as_str) != Some("main") {
                    stack.push("main".to_string());
                }
                return stack;
            }
            "BASH_ARGC" => return self.bash_argc_stack.clone(),
            "BASH_ARGV" => return self.bash_argv_stack.clone(),
            "BASH_LINENO" => return self.bash_lineno_view(),
            "BASH_SOURCE" => return self.bash_source_stack.clone(),
            _ => {}
        }
        self.env_vars
            .get(name)
            .map(|value| array_values(value))
            .unwrap_or_default()
    }

    pub(in crate::executor) fn array_assignment_transform(&self, name: &str) -> String {
        if name == "PIPESTATUS" {
            let rendered = self
                .pipestatus_values()
                .into_iter()
                .enumerate()
                .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
                .collect::<Vec<_>>()
                .join(" ");
            return format!("declare -a {name}=({rendered})");
        }

        let Some(value) = self.env_vars.get(name) else {
            // GNU array_var_assignment (subst.c:8680): a declared-unset array
            // (invisible cell or no value) renders `declare -<flags> name`
            // with no `=()` body, keeping the full attribute string.
            let flags = self.variable_assignment_flags(name, true);
            return format!("declare -{flags} {name}");
        };

        let flags = self.variable_assignment_flags(name, true);
        if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
            let entries = assoc_hash_ordered_entries(value);
            if entries.is_empty() {
                return format!("declare -{flags} {name}");
            }
            let rendered = entries
                .into_iter()
                .map(|(key, value)| {
                    format!("[{}]={}", quote_assoc_key(&key), quote_array_value(&value))
                })
                .collect::<Vec<_>>()
                .join(" ");
            return format!("declare -{flags} {name}=({rendered} )");
        }

        if is_marked_array_var(&self.env_vars, name) || is_array_storage(value) {
            let rendered = indexed_array_entries(value)
                .into_iter()
                .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
                .collect::<Vec<_>>()
                .join(" ");
            if rendered.is_empty() {
                // GNU: a set-but-empty array also drops the `=()` body
                // (new-exp15: `declare -ia foo=()` -> `declare -ai foo`).
                return format!("declare -{flags} {name}");
            }
            return format!("declare -{flags} {name}=({rendered})");
        }

        String::new()
    }

    pub(in crate::executor) fn array_element_parameter_value(
        &self,
        expression: &str,
    ) -> Option<String> {
        let (array_name, key) = parse_array_subscript(expression)?;

        let storage_name = self.resolved_variable_name(array_name)?;
        let storage = self.parameter_array_storage(array_name).unwrap_or_default();
        if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
            // GNU parameters.c assoc_reference: a literal * subscript means
            // all elements, not the key "*"; a quoted * joins with IFS[0]
            // (string_list_pos_params). Expanded subscripts such as
            // assoc[$key] still look up the literal key (assoc13).
            if key == "*" {
                return Some(
                    assoc_hash_ordered_values(&storage).join(&self.ifs_first_char_separator()),
                );
            }
            let key = self.assoc_subscript_key(key);
            return assoc_value_at(&storage, &key);
        }
        // Use expand_arithmetic_special_parameters for array subscripts so that
        // $- expands to 0 (not shell flags) in arithmetic contexts. See array.tests line 60.
        let key =
            strip_matching_quotes(&self.expand_arithmetic_special_parameters(key)).to_string();
        if key.trim() == "*" || key.trim() == "@" {
            return None;
        }
        let Some(index) = eval_conditional_arith_value(&key, &self.env_vars) else {
            self.arithmetic_nonfatal_error.set(true);
            eprintln!(
                "{}{}: bad array subscript",
                self.diagnostic_prefix(),
                array_name
            );
            return None;
        };
        let Some(index) = resolve_indexed_array_subscript(&storage, index) else {
            self.arithmetic_nonfatal_error.set(true);
            eprintln!(
                "{}{}: bad array subscript",
                self.diagnostic_prefix(),
                array_name
            );
            return None;
        };
        array_value_at(&storage, index)
    }

    pub(in crate::executor) fn array_length(&self, name: &str) -> usize {
        if name == "GROUPS" {
            return self.groups_words().len();
        }
        self.parameter_array_storage(name)
            .map(|value| array_values(&value).len())
            .unwrap_or(0)
    }

    pub(in crate::executor) fn array_at_word_values(&self, word: &str) -> Option<Vec<String>> {
        let quoted_array_word =
            (word.starts_with('"') && word.ends_with('"')) || word.starts_with('\x1d');
        let word = word
            .strip_prefix('"')
            .and_then(|word| word.strip_suffix('"'))
            .unwrap_or(word);
        let word = word.strip_prefix('\x1d').unwrap_or(word);
        if let Some(values) = self.array_transform_word_values(word, quoted_array_word) {
            return Some(values);
        }
        if let Some(values) = self.array_pattern_word_values(word, quoted_array_word) {
            return Some(values);
        }
        if !quoted_array_word {
            if let Some((name, offset, length)) = word
                .strip_prefix("${")
                .and_then(|word| word.strip_suffix('}'))
                .and_then(|name| self.parse_parameter_substring(name))
            {
                if let Some(array_name) = name
                    .strip_suffix("[@]")
                    .or_else(|| name.strip_suffix("[*]"))
                {
                    return self.parameter_array_storage(array_name).map(|value| {
                        array_parameter_slice(
                            &value,
                            offset,
                            length.and_then(|length| usize::try_from(length).ok()),
                        )
                    });
                }
            }
        }
        if quoted_array_word {
            if let Some((name, offset, length)) = word
                .strip_prefix("${")
                .and_then(|word| word.strip_suffix('}'))
                .and_then(|name| self.parse_parameter_substring(name))
            {
                if let Some(indirect_name) = name.strip_prefix('!') {
                    let target_expr = self.env_vars.get(indirect_name)?;
                    let expands_as_array = target_expr.ends_with("[@]")
                        || (!quoted_array_word && target_expr.ends_with("[*]"));
                    if expands_as_array {
                        return Some(slice_array_values(
                            self.indirect_target_values(target_expr),
                            offset,
                            length.and_then(|length| usize::try_from(length).ok()),
                        ));
                    }
                }
                if let Some(array_name) = name.strip_suffix("[@]") {
                    if array_name == "GROUPS" {
                        return Some(slice_array_values(
                            self.groups_words(),
                            offset,
                            length.and_then(|length| usize::try_from(length).ok()),
                        ));
                    }
                    return self.parameter_array_storage(array_name).map(|value| {
                        array_parameter_slice(
                            &value,
                            offset,
                            length.and_then(|length| usize::try_from(length).ok()),
                        )
                    });
                }
            }
            if let Some(values) = self.indirect_array_reference_word_values(word, true) {
                return Some(values);
            }
            if let Some(prefix) = word
                .strip_prefix("${!")
                .and_then(|word| word.strip_suffix("@}"))
            {
                let mut names = self
                    .env_vars
                    .keys()
                    .map(String::as_str)
                    .filter(|name| name.starts_with(prefix))
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                names.sort_unstable();
                return Some(names);
            }
            if let Some(prefix) = word
                .strip_prefix("${!")
                .and_then(|word| word.strip_suffix("*}"))
            {
                let mut names = self
                    .env_vars
                    .keys()
                    .map(String::as_str)
                    .filter(|name| name.starts_with(prefix))
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                names.sort_unstable();
                return Some(vec![names.join(&self.ifs_first_char_separator())]);
            }
            if let Some(name) = word
                .strip_prefix("${!")
                .and_then(|word| word.strip_suffix("[@]}"))
            {
                let storage_name = self.resolved_variable_name(name)?;
                let storage = self.parameter_array_storage(name)?;
                if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
                    return Some(assoc_keys(&storage));
                }
                return Some(array_indices(&storage));
            }
            if let Some(name) = word
                .strip_prefix("${!")
                .and_then(|word| word.strip_suffix("[*]}"))
            {
                let storage_name = self.resolved_variable_name(name)?;
                let storage = self.parameter_array_storage(name)?;
                let keys = if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
                    assoc_keys(&storage)
                } else {
                    array_indices(&storage)
                };
                return Some(vec![keys.join(&self.ifs_first_char_separator())]);
            }
        }
        if let Some(values) = self.indirect_array_reference_word_values(word, quoted_array_word) {
            return Some(values);
        }
        if quoted_array_word && word == "${GROUPS[*]}" {
            return Some(vec![self
                .groups_words()
                .join(&self.ifs_first_char_separator())]);
        }
        let name = word.strip_prefix("${").and_then(|word| {
            if quoted_array_word {
                word.strip_suffix("[@]}")
            } else {
                word.strip_suffix("[@]}")
                    .or_else(|| word.strip_suffix("[*]}"))
            }
        })?;
        if name == "GROUPS" {
            return Some(self.groups_words());
        }
        let storage = self.parameter_array_storage(name)?;
        if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
            return Some(assoc_hash_ordered_values(&storage));
        }
        Some(array_values(&storage))
    }

    fn array_transform_word_values(
        &self,
        word: &str,
        quoted_array_word: bool,
    ) -> Option<Vec<String>> {
        let (var_name, transform) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
            .and_then(parse_parameter_transform)?;
        let (array_name, starred) = var_name
            .strip_suffix("[@]")
            .map(|name| (name, false))
            .or_else(|| var_name.strip_suffix("[*]").map(|name| (name, true)))?;
        if transform == ParameterTransform::Assignment {
            let value = self.parameter_assignment_transform(var_name);
            if quoted_array_word {
                return Some(split_array_assignment_transform_words(&value));
            }
            return Some(vec![value]);
        }
        if transform == ParameterTransform::KeyValueQuoted {
            return Some(vec![self.parameter_key_value_transform(var_name, true)]);
        }
        if transform == ParameterTransform::KeyValueSplit {
            return self.array_key_value_split_transform_values(array_name);
        }
        if !array_value_transform_splits_words(transform) {
            return None;
        }
        let storage = self.parameter_array_storage(array_name)?;
        let values = if is_marked_var(
            &self.env_vars,
            ASSOC_VARS,
            &self.resolved_variable_name(array_name).unwrap_or_default(),
        ) {
            assoc_hash_ordered_values(&storage)
        } else {
            array_values(&storage)
        }
        .into_iter()
        .map(|value| self.apply_parameter_transform_value(&value, transform))
        .collect::<Vec<_>>();
        if quoted_array_word && starred {
            // GNU string_list_pos_params (subst.c:3030): a quoted `*` joins
            // with dollar_star (IFS[0]); an unquoted `*` stays a per-element
            // word list that the caller field-splits (W_SPLITSPACE).
            return Some(vec![values.join(&self.ifs_first_char_separator())]);
        }
        Some(values)
    }

    fn array_pattern_word_values(
        &self,
        word: &str,
        quoted_array_word: bool,
    ) -> Option<Vec<String>> {
        let inner = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))?;

        if let Some((var_name, pattern, operation)) = parse_indirect_pattern_removal(inner) {
            let pattern = self.expand_parameter_pattern_word(pattern);
            return self.array_modified_word_values(var_name, quoted_array_word, |value| {
                remove_parameter_pattern(value, &pattern, operation, self.extglob_enabled())
            });
        }

        if let Some((var_name, pattern, replacement, global)) = parse_parameter_replacement(inner) {
            let pattern = self.expand_parameter_pattern_word(pattern);
            let replacement = self.expand_patsub_replacement_text(replacement);
            return self.array_modified_word_values(var_name, quoted_array_word, |value| {
                self.replace_patsub_pattern(value, &pattern, &replacement, global)
            });
        }

        if let Some((var_name, operation, pattern)) = parse_parameter_case_mod(inner) {
            let pattern = self.expand_embedded_parameters(pattern);
            return self.array_modified_word_values(var_name, quoted_array_word, |value| {
                apply_parameter_case_mod(value, operation, &pattern)
            });
        }

        None
    }

    fn array_modified_word_values<F>(
        &self,
        var_name: &str,
        quoted_array_word: bool,
        modify: F,
    ) -> Option<Vec<String>>
    where
        F: Fn(&str) -> String,
    {
        let (array_name, starred) = var_name
            .strip_suffix("[@]")
            .map(|name| (name, false))
            .or_else(|| var_name.strip_suffix("[*]").map(|name| (name, true)))?;
        let storage = self.parameter_array_storage(array_name)?;
        let values = if is_marked_var(
            &self.env_vars,
            ASSOC_VARS,
            &self.resolved_variable_name(array_name).unwrap_or_default(),
        ) {
            assoc_hash_ordered_values(&storage)
        } else {
            array_values(&storage)
        }
        .into_iter()
        .map(|value| modify(&value))
        .collect::<Vec<_>>();
        if quoted_array_word && starred {
            return Some(vec![values.join(&self.ifs_first_char_separator())]);
        }
        Some(values)
    }

    fn array_key_value_split_transform_values(&self, array_name: &str) -> Option<Vec<String>> {
        let storage_name = self.resolved_variable_name(array_name)?;
        let storage = self.parameter_array_storage(array_name)?;
        if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
            return Some(
                assoc_hash_ordered_entries(&storage)
                    .into_iter()
                    .flat_map(|(key, value)| [key, value])
                    .collect(),
            );
        }
        Some(
            indexed_array_entries(&storage)
                .into_iter()
                .flat_map(|(index, value)| [index.to_string(), value])
                .collect(),
        )
    }

    fn indirect_array_reference_word_values(
        &self,
        word: &str,
        quoted_array_word: bool,
    ) -> Option<Vec<String>> {
        let indirect_name = word
            .strip_prefix("${!")
            .and_then(|word| word.strip_suffix('}'))?;
        // A nameref indirection yields the referenced NAME itself, not the
        // target's value (GNU parameter_brace_expand_indir subst.c:7896
        // returns the nameref cell verbatim); leave those to the scalar
        // path.
        if is_marked_var(&self.env_vars, NAMEREF_VARS, indirect_name) {
            return None;
        }
        let target_expr = self.resolve_indirect_target_expr(indirect_name)?;
        self.indirect_target_word_values(&target_expr, quoted_array_word)
    }

    /// GNU parameter_brace_find_indir (subst.c:7839): the word after `!`
    /// is expanded first -- a positional number reads that parameter (so
    /// ${!1} sees the function's own arguments) and a shell name reads its
    /// value. The result is the parameter expression the indirection
    /// points at.
    pub(in crate::executor) fn resolve_indirect_target_expr(&self, indirect_name: &str) -> Option<String> {
        if let Ok(index) = indirect_name.parse::<usize>() {
            return self
                .positional_params
                .get(index.saturating_sub(1))
                .cloned();
        }
        if !is_shell_name(indirect_name) {
            return None;
        }
        self.env_vars.get(indirect_name).cloned()
    }

    /// GNU chk_atstar (subst.c:7922) plus the array-indirection branch of
    /// parameter_brace_expand_indir (subst.c:7945-7958): the target value
    /// is re-expanded as a parameter. An `@`-target keeps $@ word
    /// semantics, a `*`-target keeps $* semantics, and a value ending in
    /// `[@]`/`[*]` expands the array's values as elements (new-exp9.sub).
    /// Quoted `[@]` yields one word per element; unquoted results are
    /// field split like the corresponding $@/$* word. Quoted `[*]` joins
    /// with IFS[0] into a single word (string_list_dollar_star).
    fn indirect_target_word_values(
        &self,
        target_expr: &str,
        quoted_array_word: bool,
    ) -> Option<Vec<String>> {
        match target_expr {
            "@" => {
                // Unquoted $@ is field split like any unquoted expansion
                // (new-exp.tests:196 `recho ${!foo}` with foo=@ splits the
                // `b c` parameter under the default IFS); quoted "$@" keeps
                // one word per parameter verbatim.
                return Some(if quoted_array_word {
                    self.positional_params.clone()
                } else {
                    field_split_positional_values_with_ifs(
                        self.positional_params.clone(),
                        self.env_vars.get("IFS").map(String::as_str),
                    )
                });
            }
            "*" => {
                return Some(if quoted_array_word {
                    vec![self.positional_params.join(&self.ifs_first_char_separator())]
                } else {
                    field_split_positional_values_with_ifs(
                        self.positional_params.clone(),
                        self.env_vars.get("IFS").map(String::as_str),
                    )
                });
            }
            _ => {}
        }
        let starred = target_expr.ends_with("[*]");
        if !starred && !target_expr.ends_with("[@]") {
            return None;
        }
        let values = self.indirect_target_values(target_expr);
        if starred {
            if quoted_array_word {
                return Some(vec![values.join(&self.ifs_first_char_separator())]);
            }
        } else if quoted_array_word {
            return Some(values);
        }
        Some(field_split_array_values_with_ifs(
            values,
            self.env_vars.get("IFS").map(String::as_str),
        ))
    }

    pub(in crate::executor) fn ifs_first_char_separator(&self) -> String {
        match self.env_vars.get("IFS") {
            Some(ifs) => ifs
                .chars()
                .next()
                .map(|separator| separator.to_string())
                .unwrap_or_default(),
            None => " ".to_string(),
        }
    }
}

fn array_value_transform_splits_words(transform: ParameterTransform) -> bool {
    matches!(
        transform,
        ParameterTransform::Quote
            | ParameterTransform::Escape
            | ParameterTransform::Prompt
            | ParameterTransform::Upper
            | ParameterTransform::UpperFirst
            | ParameterTransform::Lower
    )
}

fn split_array_assignment_transform_words(value: &str) -> Vec<String> {
    let mut parts = value.splitn(3, char::is_whitespace);
    let first = parts.next().unwrap_or_default();
    if first.is_empty() {
        return Vec::new();
    }
    let Some(second) = parts.next() else {
        return vec![first.to_string()];
    };
    let Some(rest) = parts.next() else {
        return vec![first.to_string(), second.to_string()];
    };
    vec![first.to_string(), second.to_string(), rest.to_string()]
}
