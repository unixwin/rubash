use super::storage::quote_assoc_display_key;
use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::NamerefResolution;
use crate::executor::{
    assoc_hash_ordered_entries, assoc_hash_ordered_values, assoc_keys, assoc_nbuckets,
    eval_conditional_arith_value_with_writes, IndexedSubscript, SubscriptSource,
    DECLARED_UNSET_VARS, NAMEREF_VARS,
};

impl Executor {
    pub(in crate::executor) fn indexed_array_stack(&self, name: &str) -> Vec<String> {
        match name {
            "PIPESTATUS" => return self.pipestatus_values(),
            "FUNCNAME" => {
                let mut stack = self.shell_state.function_name_stack.clone();
                if !stack.is_empty() && stack.last().map(String::as_str) != Some("main") {
                    stack.push("main".to_string());
                }
                return stack;
            }
            "BASH_ARGC" => return self.shell_state.bash_argc_stack.clone(),
            "BASH_ARGV" => return self.shell_state.bash_argv_stack.clone(),
            "BASH_LINENO" => return self.bash_lineno_view(),
            "BASH_SOURCE" => return self.shell_state.bash_source_stack.clone(),
            _ => {}
        }
        self.shell_state
            .env_vars
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

        let Some(value) = self.shell_state.env_vars.get(name) else {
            // GNU array_var_assignment (subst.c:8680): a declared-unset array
            // (invisible cell or no value) renders `declare -<flags> name`
            // with no `=()` body, keeping the full attribute string.
            let flags = self.variable_assignment_flags(name, true);
            return format!("declare -{flags} {name}");
        };

        let flags = self.variable_assignment_flags(name, true);
        // GNU array_var_assignment (subst.c:8690-8691): a declared-unset
        // (invisible) array that still has a value cell drops the `=()` body
        // just like a missing cell. Rubash stores declared-unset arrays in
        // env_vars with a marker, so check the marker here.
        if is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, name) {
            return format!("declare -{flags} {name}");
        }
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) {
            let entries =
                assoc_hash_ordered_entries(value, assoc_nbuckets(&self.shell_state.env_vars, name));
            if entries.is_empty() {
                // GNU array_var_assignment (subst.c:8693-8697): a set-but-empty
                // array gets `=()` (val == 0 but var_isset); only invisible/unset
                // arrays drop the body.
                return format!("declare -{flags} {name}=()");
            }
            let rendered = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "[{}]={}",
                        quote_assoc_display_key(&key),
                        quote_array_value(&value)
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            return format!("declare -{flags} {name}=({rendered} )");
        }

        if is_marked_array_var(&self.shell_state.env_vars, name) || is_array_storage(value) {
            let rendered = indexed_array_entries(value)
                .into_iter()
                .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
                .collect::<Vec<_>>()
                .join(" ");
            if rendered.is_empty() {
                // GNU array_var_assignment (subst.c:8693-8697): a set-but-empty
                // array gets `=()` (val == 0 but var_isset); only invisible/unset
                // arrays drop the body (new-exp15 uses the scalar ${foo@A} form
                // which goes through string_var_assignment, not this path).
                return format!("declare -{flags} {name}=()");
            }
            return format!("declare -{flags} {name}=({rendered})");
        }

        String::new()
    }

    pub(in crate::executor) fn array_element_parameter_value(
        &self,
        expression: &str,
    ) -> Option<String> {
        // GNU param_expand resolves the subscript once per `${}` expansion;
        // the memo frame pushed by the embedded-parameter walkers dedups the
        // repeated identical fetches the `&self` helper layers make, so
        // `$((i++))` side effects run exactly once (AEPV_MEMO docs).
        if let Some(hit) = crate::executor::expand_braced_indices::aepv_memo_lookup(expression) {
            return hit;
        }
        let result = self.array_element_parameter_value_uncached(expression);
        crate::executor::expand_braced_indices::aepv_memo_store(expression, result.clone());
        result
    }

    fn array_element_parameter_value_uncached(&self, expression: &str) -> Option<String> {
        let (array_name, key) = parse_array_subscript(expression)?;

        let storage_name = self.resolved_variable_name(array_name)?;
        let storage = self.parameter_array_storage(array_name).unwrap_or_default();
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &storage_name) {
            // GNU parameters.c assoc_reference: a literal * subscript means
            // all elements, not the key "*"; a quoted * joins with IFS[0]
            // (string_list_pos_params). Expanded subscripts such as
            // assoc[$key] still look up the literal key (assoc13).
            if key == "*" {
                return Some(
                    assoc_hash_ordered_values(
                        &storage,
                        assoc_nbuckets(&self.shell_state.env_vars, &storage_name),
                    )
                    .join(&self.ifs_first_char_separator()),
                );
            }
            let key = self.assoc_subscript_key(key);
            return assoc_value_at(&storage, &key);
        }
        // Use expand_arithmetic_special_parameters for array subscripts so that
        // $- expands to 0 (not shell flags) in arithmetic contexts. See array.tests line 60.
        // For $((...)) subscripts, expand_arithmetic_special_parameters would
        // evaluate the arithmetic in a cloned env (losing side effects like
        // count++). Instead, detect $((...)) and evaluate directly with
        // eval_conditional_arith_value_with_writes which captures side effects.
        let key = if key
            .strip_prefix("$((")
            .is_some_and(|rest| rest.strip_suffix("))").is_some())
        {
            // The resolved subscript text dedups across the layered `${}`
            // passes (SUB_RES_XPASS): the `:=` pre-scan, the assignment
            // apply, and the real expansion all resolve this same raw key.
            let memo_key = crate::executor::expand_braced_indices::sub_site_key(key);
            if let Some(hit) = memo_key
                .as_ref()
                .and_then(crate::executor::expand_braced_indices::sub_res_lookup)
            {
                hit
            } else {
                let expr = key
                    .strip_prefix("$((")
                    .unwrap()
                    .strip_suffix("))")
                    .unwrap()
                    .trim();
                // Still expand special parameters ($#, $-) inside the expression.
                let expr = expr
                    .replace("$#", &self.shell_state.positional_params.len().to_string())
                    .replace("$-", "0");
                let overlaid =
                    crate::executor::expand_braced_indices::env_vars_with_pending_subscript_writes(
                        &self.shell_state.env_vars,
                    );
                let (result, writes) = eval_conditional_arith_value_with_writes(&expr, &overlaid);
                if !writes.is_empty() {
                    crate::executor::expand_braced_indices::PENDING_SUBSCRIPT_WRITES.with(|w| {
                        w.borrow_mut().extend(writes);
                    });
                }
                match result {
                    Some(v) => {
                        let resolved = v.to_string();
                        if let Some(key) = memo_key {
                            crate::executor::expand_braced_indices::sub_res_store(
                                key,
                                resolved.clone(),
                            );
                        }
                        resolved
                    }
                    None => return None,
                }
            }
        } else {
            // GNU subst.c array_variable_part expands the subscript in a
            // double-quoted context: `"x"` loses its quotes before evalexp,
            // while `'x'` survives as literal text (sq is data in dq context)
            // and evalexp then rejects it as a string operand — `a[' ']`
            // fails "' ': operand expected" where `a[" "]` resolves to 0.
            let expanded = self.expand_arithmetic_special_parameters(key);
            if expanded.len() >= 2 && expanded.starts_with('"') && expanded.ends_with('"') {
                expanded[1..expanded.len() - 1].to_string()
            } else {
                expanded
            }
        };
        if key.trim() == "*" || key.trim() == "@" {
            return None;
        }
        // GNU subst.c array_variable_part -> array_expand_index -> evalexp:
        // the expanded subscript text is evaluated under no-expand rules, so
        // a surviving `$name`/`$(...)` fails "operand expected" (expr.c
        // evalerror aborts the command list) rather than being stored or
        // silently treated as index 0. The `key` above already received the
        // single expand_subscript_string pass, so it is Protected data here.
        let index = match self.eval_indexed_subscript_deferred(SubscriptSource::Protected(&key)) {
            IndexedSubscript::Index(index) => index,
            // `${a[]}`: GNU subst.c reports "bad substitution" for the
            // expansion, which parameter_errors turns into the dropped word;
            // keep the existing silent-failure shape here.
            IndexedSubscript::Empty => return None,
            IndexedSubscript::Error => return None,
        };
        // GNU arrayfunc.c:1582-1583 INDEX_ERROR(): a negative subscript that
        // still resolves negative (empty/unset array) prints err_badarraysub
        // and returns NULL — the expansion is empty but the command still
        // runs. This is the VALUE expansion path (not ${#arr[bad]} length
        // expansion which returns &expand_wdesc_error at subst.c:9955 and
        // abandons the command), so we must NOT set arithmetic_nonfatal_error.
        let Some(index) = resolve_indexed_array_subscript(&storage, index) else {
            eprintln!(
                "{}{}: bad array subscript",
                self.diagnostic_prefix(),
                array_name
            );
            return None;
        };
        let result = array_value_at(&storage, index);
        result
    }

    pub(in crate::executor) fn array_length(&self, name: &str) -> usize {
        if name == "GROUPS" {
            return self.groups_words().len();
        }
        self.parameter_array_storage(name)
            .map(|value| array_values(&value).len())
            .unwrap_or(0)
    }

    /// GNU assoc_subrange (assoc.c:244): on an associative array the
    /// substring offset is a 1-based position into the hash-ordered word
    /// list — `${a[*]:0}` and `${a[*]:1}` both start at the first element,
    /// `:2:1` returns the second element — and verify_substring_values
    /// (subst.c:8437) resolves a negative offset against n+1. Indexed
    /// arrays keep array_subrange's index-based semantics.
    pub(in crate::executor) fn array_subscript_range_values(
        &self,
        array_name: &str,
        offset: isize,
        length: Option<usize>,
    ) -> Option<Vec<String>> {
        let storage = self.parameter_array_storage(array_name)?;
        let resolved = self
            .resolved_variable_name(array_name)
            .unwrap_or_else(|| array_name.to_string());
        if !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved) {
            // GNU subst.c:9064 get_var_and_type resolves / on
            // a scalar to VT_VARIABLE, so parameter_brace_substring
            // (subst.c:9083-9099) applies the offset/length to the string
            // value itself rather than slicing an element list.
            let is_array = is_marked_array_var(&self.shell_state.env_vars, &resolved)
                || is_array_storage(&storage);
            if !is_array {
                return Some(vec![crate::executor::parameter_substring(
                    &storage,
                    offset,
                    length.map(|length| length as isize),
                )]);
            }
            return Some(array_parameter_slice(&storage, offset, length));
        }
        let values = assoc_hash_ordered_values(
            &storage,
            assoc_nbuckets(&self.shell_state.env_vars, &resolved),
        );
        let count = values.len() as i128;
        let start = if offset < 0 {
            offset as i128 + count + 1
        } else {
            offset as i128
        };
        if start < 0 || start > count {
            return Some(Vec::new());
        }
        let skip = (start - 1).max(0) as usize;
        Some(
            values
                .into_iter()
                .skip(skip)
                .take(length.unwrap_or(usize::MAX))
                .collect(),
        )
    }

    pub(in crate::executor) fn array_at_word_values(&self, word: &str) -> Option<Vec<String>> {
        let quoted_array_word =
            (word.starts_with('"') && word.ends_with('"')) || word.starts_with(STORAGE_WORD_PREFIX);
        let word = word
            .strip_prefix('"')
            .and_then(|word| word.strip_suffix('"'))
            .unwrap_or(word);
        let word = word.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(word);
        if let Some(values) = self.array_transform_word_values(word, quoted_array_word) {
            return Some(values);
        }
        if let Some(values) = self.array_pattern_word_values(word, quoted_array_word) {
            return Some(values);
        }
        if !quoted_array_word {
            if let Some((name, offset, length)) =
                crate::executor::parameter_ops::whole_word_braced_parameter_body(word)
                    .and_then(|name| self.parse_parameter_substring(name))
            {
                if let Some(array_name) = name
                    .strip_suffix("[@]")
                    .or_else(|| name.strip_suffix("[*]"))
                {
                    return self.array_subscript_range_values(
                        array_name,
                        offset,
                        length.and_then(|length| usize::try_from(length).ok()),
                    );
                }
            }
        }
        if quoted_array_word {
            if let Some((name, offset, length)) =
                crate::executor::parameter_ops::whole_word_braced_parameter_body(word)
                    .and_then(|name| self.parse_parameter_substring(name))
            {
                if let Some(indirect_name) = name.strip_prefix('!') {
                    let target_expr = self.shell_state.env_vars.get(indirect_name)?;
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
                    return self.array_subscript_range_values(
                        array_name,
                        offset,
                        length.and_then(|length| usize::try_from(length).ok()),
                    );
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
                    .shell_state
                    .env_vars
                    .keys()
                    .map(String::as_str)
                    // GNU param_expand lists shell_variables only; invalid-name
                    // environment entries live in the invisible invalid_env
                    // table (variables.c:3307) and never match (issue #102).
                    .filter(|name| is_shell_name(name) && name.starts_with(prefix))
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
                    .shell_state
                    .env_vars
                    .keys()
                    .map(String::as_str)
                    // Same invalid_env exclusion as the @ form above (issue #102).
                    .filter(|name| is_shell_name(name) && name.starts_with(prefix))
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
                if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &storage_name) {
                    return Some(assoc_keys(
                        &storage,
                        assoc_nbuckets(&self.shell_state.env_vars, &storage_name),
                    ));
                }
                return Some(array_indices(&storage));
            }
            if let Some(name) = word
                .strip_prefix("${!")
                .and_then(|word| word.strip_suffix("[*]}"))
            {
                let storage_name = self.resolved_variable_name(name)?;
                let storage = self.parameter_array_storage(name)?;
                let keys = if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &storage_name) {
                    assoc_keys(
                        &storage,
                        assoc_nbuckets(&self.shell_state.env_vars, &storage_name),
                    )
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
        // GNU variables.c find_variable_nameref: an unbraced `$ref` word
        // whose nameref cell is an array-at reference (`arr[@]`) expands the
        // referenced array's elements -- quoted `[@]` yields one word per
        // element and `[*]` a single joined word (nameref18.sub
        // `recho "$ref"` yields argv[1..3] = <1> <2> <3>). A BRACED
        // `"${ref}"` stays scalar (parameter_brace_expand reads it through
        // the scalar name path), so only the unbraced form qualifies. A
        // trailing PARAM_NAME_END_MARKER is the quote-boundary marker the
        // lexer leaves on a quoted unbraced `$name` (`"$ref"` arrives as
        // `$ref\x13`), and it counts as quoting for the [*] join.
        let quoted_array_word = quoted_array_word || word.ends_with('\u{13}');
        let bare_name = word
            .strip_prefix('$')
            .filter(|name| !name.starts_with('{'))
            .map(|name| name.trim_end_matches('\u{13}'))
            .filter(|name| is_shell_name(name));
        if let Some(name) = bare_name {
            if let NamerefResolution::Target(target) = self.nameref_resolution(name) {
                if let Some(array_name) = target.strip_suffix("[@]") {
                    if let Some(storage) = self.parameter_array_storage(array_name) {
                        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name) {
                            return Some(assoc_hash_ordered_values(
                                &storage,
                                assoc_nbuckets(&self.shell_state.env_vars, array_name),
                            ));
                        }
                        return Some(array_values(&storage));
                    }
                } else if let Some(array_name) = target.strip_suffix("[*]") {
                    if let Some(storage) = self.parameter_array_storage(array_name) {
                        let values =
                            if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name) {
                                assoc_hash_ordered_values(
                                    &storage,
                                    assoc_nbuckets(&self.shell_state.env_vars, array_name),
                                )
                            } else {
                                array_values(&storage)
                            };
                        if quoted_array_word {
                            return Some(vec![values.join(&self.ifs_first_char_separator())]);
                        }
                        return Some(values);
                    }
                }
            }
        }
        // The [@]/[*] element list requires the word to be ONE `${}` span;
        // a trailing `[@]}` on a glued multi-expansion word (`${x}${a[@]}`)
        // must not admit it (whole_word_braced_parameter_body — see
        // iquote.sub `"${del:0:1}${a#d}"`).
        if !crate::executor::parameter_ops::braced_parameter_spans_whole_word(word) {
            return None;
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
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) {
            return Some(assoc_hash_ordered_values(
                &storage,
                assoc_nbuckets(&self.shell_state.env_vars, name),
            ));
        }
        Some(array_values(&storage))
    }

    fn array_transform_word_values(
        &self,
        word: &str,
        quoted_array_word: bool,
    ) -> Option<Vec<String>> {
        let (var_name, transform) =
            crate::executor::parameter_ops::whole_word_braced_parameter_body(word)
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
            // The unquoted result goes through the caller's
            // field_split_array_values_with_ifs — matching GNU, where
            // quote_escapes (subst.c:8705) leaves whitespace bare so the
            // kvpair string splits while its quotes stay literal data.
            return Some(vec![self.parameter_key_value_transform(var_name, true)]);
        }
        if transform == ParameterTransform::KeyValueSplit {
            let values = self.array_key_value_split_transform_values(array_name)?;
            if quoted_array_word && starred {
                // GNU array_transform (subst.c:8869): @k hands the kv word
                // list to string_list_pos_params(itype, list, qflags), so a
                // quoted `*` joins every key/value word with IFS[0] into a
                // single word (dollar_star) while `@` stays per-word.
                return Some(vec![values.join(&self.ifs_first_char_separator())]);
            }
            return Some(values);
        }
        if !array_value_transform_splits_words(transform) {
            return None;
        }
        let storage = self.parameter_array_storage(array_name);
        let values = if transform == ParameterTransform::Attributes {
            // GNU array_transform (subst.c:8856-8862): `arr[@]@a` on a
            // DECLARED-but-never-assigned array (array_cell == NULL)
            // returns var_attribute_string once — the attribute list
            // describes the variable itself. An assigned-but-empty array
            // (`foo=()`) has a real cell, so the 'a' transform maps over
            // its (empty) element list and yields nothing.
            let resolved = self.resolved_variable_name(array_name).unwrap_or_default();
            // DECLARED_UNSET marks the never-assigned cell (`declare -a x`),
            // matching GNU's array_cell(v) == NULL; `x=()` allocates a real
            // (empty) cell and clears the marker.
            let null_cell = storage.is_none()
                || is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, &resolved);
            if null_cell {
                let attrs = self.parameter_attribute_transform(array_name);
                if attrs.is_empty() {
                    Vec::new()
                } else {
                    vec![attrs]
                }
            } else {
                let count = match storage.as_ref() {
                    Some(storage)
                        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved) =>
                    {
                        assoc_hash_ordered_values(
                            storage,
                            assoc_nbuckets(&self.shell_state.env_vars, &resolved),
                        )
                        .len()
                    }
                    Some(storage) => array_values(storage).len(),
                    None => 0,
                };
                (0..count)
                    .map(|_| self.parameter_attribute_transform(array_name))
                    .collect()
            }
        } else {
            let storage = storage?;
            if is_marked_var(
                &self.shell_state.env_vars,
                ASSOC_VARS,
                &self.resolved_variable_name(array_name).unwrap_or_default(),
            ) {
                assoc_hash_ordered_values(
                    &storage,
                    assoc_nbuckets(
                        &self.shell_state.env_vars,
                        &self.resolved_variable_name(array_name).unwrap_or_default(),
                    ),
                )
            } else {
                array_values(&storage)
            }
            .into_iter()
            .map(|value| self.apply_parameter_transform_value(&value, transform))
            .collect::<Vec<_>>()
        };
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
        let inner = crate::executor::parameter_ops::whole_word_braced_parameter_body(word)?;

        // array_modified_word_values only handles `name[@]`/`name[*]`
        // targets; validate that before expanding the pattern/replacement
        // text. Expanding first runs expansion side effects
        // (`${a[$((i++))],,}` evaluated the subscript) on a probe that then
        // returns None — GNU expands the `${}` body exactly once.
        let array_target = |var_name: &str| var_name.ends_with("[@]") || var_name.ends_with("[*]");

        if let Some((var_name, pattern, operation)) = parse_indirect_pattern_removal(inner) {
            if !array_target(var_name) {
                return None;
            }
            let pattern = self.expand_parameter_pattern_word(pattern);
            return self.array_modified_word_values(var_name, quoted_array_word, |value| {
                remove_parameter_pattern(value, &pattern, operation, self.extglob_enabled())
            });
        }

        if let Some((var_name, pattern, replacement, global)) = parse_parameter_replacement(inner) {
            if !array_target(var_name) {
                return None;
            }
            let pattern = self.expand_parameter_pattern_word(pattern);
            let replacement = self.expand_patsub_replacement_text(replacement);
            return self.array_modified_word_values(var_name, quoted_array_word, |value| {
                self.replace_patsub_pattern(value, &pattern, &replacement, global)
            });
        }

        if let Some((var_name, operation, pattern)) = parse_parameter_case_mod(inner) {
            if !array_target(var_name) {
                return None;
            }
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
            &self.shell_state.env_vars,
            ASSOC_VARS,
            &self.resolved_variable_name(array_name).unwrap_or_default(),
        ) {
            assoc_hash_ordered_values(
                &storage,
                assoc_nbuckets(
                    &self.shell_state.env_vars,
                    &self.resolved_variable_name(array_name).unwrap_or_default(),
                ),
            )
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
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &storage_name) {
            return Some(
                assoc_hash_ordered_entries(
                    &storage,
                    assoc_nbuckets(&self.shell_state.env_vars, &storage_name),
                )
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
        let indirect_name = crate::executor::parameter_ops::whole_word_braced_parameter_body(word)
            .and_then(|body| body.strip_prefix('!'))?;
        // A nameref indirection yields the referenced NAME itself, not the
        // target's value (GNU parameter_brace_expand_indir subst.c:7896
        // returns the nameref cell verbatim); leave those to the scalar
        // path.
        if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, indirect_name) {
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
    pub(in crate::executor) fn resolve_indirect_target_expr(
        &self,
        indirect_name: &str,
    ) -> Option<String> {
        if let Ok(index) = indirect_name.parse::<usize>() {
            return self
                .shell_state
                .positional_params
                .get(index.saturating_sub(1))
                .cloned();
        }
        if !is_shell_name(indirect_name) {
            return None;
        }
        self.shell_state.env_vars.get(indirect_name).cloned()
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
                    self.shell_state.positional_params.clone()
                } else {
                    field_split_positional_values_with_ifs(
                        self.shell_state.positional_params.clone(),
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    )
                });
            }
            "*" => {
                return Some(if quoted_array_word {
                    vec![self
                        .shell_state
                        .positional_params
                        .join(&self.ifs_first_char_separator())]
                } else {
                    field_split_positional_values_with_ifs(
                        self.shell_state.positional_params.clone(),
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
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
            self.shell_state.env_vars.get("IFS").map(String::as_str),
        ))
    }

    pub(in crate::executor) fn ifs_first_char_separator(&self) -> String {
        match self.shell_state.env_vars.get("IFS") {
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
            // GNU list_transform (subst.c:8906) applies @a per element, so
            // `${arr[@]@a}` yields one attribute string per element.
            | ParameterTransform::Attributes
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
