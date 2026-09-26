use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_special_or_indirect_parameter(
        &self,
        name: &str,
        unquoted: bool,
    ) -> Option<String> {
        match name {
            "#" => return Some(self.shell_state.positional_params.len().to_string()),
            // GNU string_list_dollar_star: `*` joins with IFS[0] in scalar
            // contexts; `@` stays space-joined.
            "@" => return Some(self.shell_state.positional_params.join(" ")),
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
                self.shell_state
                    .positional_params
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

        let indirect_name = match self.indirect_parameter_body(name) {
            Some(body) => body,
            // posix mode: `!` before `?`/`#` is not indirect
            // (subst.c:122 VALID_INDIR_PARAM) — `${!?}`/`${!#}` expand the
            // `!` parameter itself; the error path reports it when unset.
            None if name.starts_with('!') => {
                return Some(self.parameter_operator_value("!").unwrap_or_default());
            }
            None => return None,
        };
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
            // GNU subst.c string_list_pos_params over the key list from
            // arrayfunc.c array_keys:
            //   *  -> string_list_dollar_star, IFS[0] join (empty IFS joins
            //         with the empty string) in EVERY context;
            //   @  -> dollar_star IFS[0] join for unquoted command words
            //         ("separated by the first character of $IFS for later
            //         splitting"), but dollar_at (elements quoted,
            //         space-joined, never split) inside double quotes /
            //         here-docs and on assignment RHS (PF_ASSIGNRHS).
            // The returned string then undergoes the caller's normal
            // split/glob pass, which reproduces the GNU observable result.
            let ifs_first = self.ifs_first_char_separator();
            let separator = if indirect_name.ends_with("[*]") {
                ifs_first
            } else if !unquoted || self.inside_assignment_rhs.get() || ifs_first.is_empty() {
                " ".to_string()
            } else {
                ifs_first
            };
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        if let Some(resolved) = storage_name.as_deref().filter(|name| {
                            is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
                        }) {
                            assoc_keys(&value, assoc_nbuckets(&self.shell_state.env_vars, resolved))
                                .join(&separator)
                        } else {
                            array_indices(&value).join(&separator)
                        }
                    })
                    .unwrap_or_default(),
            );
        }

        if let Some(prefix) = indirect_name
            .strip_suffix('*')
            .or_else(|| indirect_name.strip_suffix('@'))
        {
            // GNU param_expand lists shell_variables only (subst.c
            // parameter_brace_expand_indir); invalid-name environment
            // entries live in the separate invisible invalid_env table
            // (variables.c:3307) and never match (niubash issue #102).
            // GNU subst.c:9978-10007 parameter_brace_expand: the prefix list
            // comes from all_variables_matching_prefix, which vapply()s
            // sort_variables — strcmp order (variables.c:4250-4262). `@`
            // uses string_list_dollar_at and `*` string_list_dollar_star;
            // unquoted both join with IFS[0] (' ' when unset/empty) and the
            // caller's field split reproduces the per-name fields.
            let mut names: Vec<&str> = self
                .shell_state
                .env_vars
                .keys()
                .map(String::as_str)
                .filter(|name| is_shell_name(name) && name.starts_with(prefix))
                .collect();
            names.sort_unstable();
            return Some(names.join(&self.ifs_first_char_separator()));
        }

        if indirect_name == "#" {
            return Some(
                self.shell_state
                    .positional_params
                    .last()
                    .cloned()
                    .unwrap_or_default(),
            );
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
            self.shell_state
                .positional_params
                .get(index.saturating_sub(1))
                .cloned()
                .unwrap_or_default()
        } else if let Some((base, sub)) = indirect_name
            .split_once('[')
            .and_then(|(base, rest)| rest.strip_suffix(']').map(|sub| (base, sub)))
            .filter(|(base, _)| is_shell_name(base))
        {
            // GNU subst.c:7883-7935 parameter_brace_expand_indir: for
            // `${!name[sub]}` the ELEMENT value becomes the indirect name
            // — the ksh93 nameref-cell shortcut above applies to a bare
            // name only. The base is resolved through namerefs
            // (find_variable in array_variable_part), so `declare -n m=a`
            // reads a[sub]; an unresolvable base was already reported as
            // "invalid indirect expansion" by the word error scan and
            // expands empty here.
            let element = match self.resolved_variable_name(base) {
                Some(resolved) => {
                    let base_is_array =
                        is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, &resolved)
                            || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved)
                            || self.parameter_array_storage(&resolved).is_some();
                    if base_is_array {
                        self.array_element_parameter_value(&format!("{resolved}[{sub}]"))
                    } else if sub == "0" {
                        // A scalar is element [0] of an implicit array
                        // (array_value, arrayfunc.c).
                        self.shell_variable_value(&resolved)
                    } else {
                        None
                    }
                }
                None => None,
            };
            element.unwrap_or_default()
        } else {
            self.shell_state
                .env_vars
                .get(indirect_name)
                .cloned()
                .unwrap_or_default()
        };
        // GNU subst.c:7955 parameter_brace_expand_indir re-expands the
        // indirect target through parameter_brace_expand_word, whose
        // find_variable follows namerefs: an indirect name landing on a
        // nameref cell expands the RESOLVED target (`indir=ref`,
        // `declare -n ref=arr` -> `${!indir}` reads arr[0]), not the
        // cell's stored text. The ksh93 name-of shortcut above applies
        // only when `name` itself is the nameref.
        let target_name = self
            .resolved_variable_name(&target_name)
            .unwrap_or(target_name);

        // GNU subst.c:7883-7935 parameter_brace_expand_indir: the indirect
        // target is itself expanded as a variable reference, so a
        // `name[@]`/`name[*]` target (`aref='assoc[@]'`) expands to ALL
        // element values — space-joined for `@`, IFS[0]-joined for `*`
        // (string_list_dollar_at / string_list_dollar_star).
        if let Some(base) = target_name
            .strip_suffix("[@]")
            .or_else(|| target_name.strip_suffix("[*]"))
        {
            if let Some(resolved) = self.resolved_variable_name(base) {
                if is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, &resolved)
                    || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved)
                    || self.parameter_array_storage(&resolved).is_some()
                {
                    let separator = if target_name.ends_with("[*]") {
                        self.ifs_first_char_separator()
                    } else {
                        " ".to_string()
                    };
                    return Some(
                        self.parameter_array_storage(&resolved)
                            .map(|value| {
                                if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved)
                                {
                                    // GNU assoc_reference order follows the
                                    // hash table's bucket order, not
                                    // insertion order (hashlib.c).
                                    let nbuckets =
                                        assoc_nbuckets(&self.shell_state.env_vars, &resolved);
                                    bash_assoc_order(&assoc_entries(&value), nbuckets)
                                        .into_iter()
                                        .map(|(_, (_, entry_value))| entry_value)
                                        .collect::<Vec<_>>()
                                        .join(&separator)
                                } else {
                                    indexed_array_entries(&value)
                                        .into_values()
                                        .collect::<Vec<_>>()
                                        .join(&separator)
                                }
                            })
                            .unwrap_or_default(),
                    );
                }
            }
        }
        if let Some(value) = self.array_element_parameter_value(&target_name) {
            return Some(value);
        }

        // GNU chk_atstar (subst.c:7922): a target value of `@` or `*`
        // re-expands as $@/$*; scalar context joins with a space for @ and
        // IFS[0] for * (string_list_dollar_at / string_list_dollar_star).
        match target_name.as_str() {
            "@" => return Some(self.shell_state.positional_params.join(" ")),
            "*" => {
                return Some(
                    self.shell_state
                        .positional_params
                        .join(&self.ifs_first_char_separator()),
                )
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
