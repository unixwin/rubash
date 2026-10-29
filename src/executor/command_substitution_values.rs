use super::command_registry::{comsub_front_slot, ComsubFrontSlot};
use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

impl Executor {
    /// Expand one command-substitution argument word, applying pathname
    /// expansion for unquoted words. GNU Bash runs expand_words (subst.c)
    /// on the substitution body's word list before dispatching the command:
    /// unquoted `*`, `?`, `[...]` patterns in the body undergo pathname
    /// expansion exactly like a top-level command's argument list. The
    /// specialised shortcuts below emulated only the parameter-expansion
    /// and field-splitting steps, leaving literal globs unexpanded
    /// (`x=$(echo *)` produced the literal star instead of the directory
    /// listing). This helper restores the missing final step.
    pub(in crate::executor) fn expand_command_substitution_arg_values(
        &self,
        word: &str,
    ) -> Vec<String> {
        self.expand_command_substitution_arg_values_quoted(word, false)
    }

    pub(in crate::executor) fn expand_command_substitution_arg_values_quoted(
        &self,
        word: &str,
        quoted: bool,
    ) -> Vec<String> {
        if let Some(values) = self.quoted_positional_at_word_values(word, None) {
            return values;
        }
        if let Some(values) = self.array_at_word_values(word) {
            return values;
        }
        let suppress_glob = quoted
            || word.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
            || word.starts_with(STORAGE_WORD_PREFIX);
        let expanded = strip_matching_quotes(&restore_command_substitution_output(
            &self.expand_word(word),
        ))
        .to_string();
        // The specialized substitution paths emulate GNU's expand_words for
        // the substitution body (subst.c): an unquoted parameter/command
        // substitution word is field-split on $IFS exactly like a plain
        // command's word list (nquote5.tests `$(echo $a)` with IFS=$'\001'
        // must hand echo three args whose space-joined output re-splits to
        // one field, not the literal \001 bytes).
        let values = if for_word_has_unquoted_expansion(word, None) {
            self.field_split_values(&expanded)
        } else {
            vec![expanded]
        };
        if suppress_glob {
            return values;
        }
        // Pathname expansion (subst.c expand_words -> pathname expansion):
        // each field of the unquoted word list is expanded independently.
        values
            .into_iter()
            .flat_map(|value| self.apply_command_substitution_pathname_expansion(&value))
            .collect()
    }

    /// Apply pathname expansion to one already-expanded word from a command
    /// substitution body. Returns the match list when the word is a pattern,
    /// or the word itself when it is not.
    pub(in crate::executor) fn apply_command_substitution_pathname_expansion(
        &self,
        word: &str,
    ) -> Vec<String> {
        match glob::pathname_expand_word(word, &self.shell_state.env_vars, &self.shell_state.cwd) {
            glob::PathnameExpansion::Matches(matches) => matches,
            glob::PathnameExpansion::NoMatch => vec![word.to_string()],
            glob::PathnameExpansion::Fail(_) => vec![word.to_string()],
        }
    }

    pub(in crate::executor) fn command_describe_substitution_output(
        &self,
        words: &[String],
    ) -> Option<String> {
        if words.first().map(String::as_str) != Some("command") {
            return None;
        }
        if words
            .iter()
            .any(|word| matches!(word.as_str(), "|" | ">" | ">>" | "<" | "2>" | "2>>" | "&>"))
        {
            return None;
        }
        let Some((mode, use_standard_path, first_name)) = parse_command_describe_args(&words[1..])
        else {
            return None;
        };

        let mut stdout = Vec::new();
        // rubash#704 R6.2: same GNU any-found rule as the direct path
        // (builtins/command.def:111-121): one resolved operand suffices for
        // status 0, and the command.def:96-97 no-operand early return makes
        // `$(command -v)` succeed with empty output.
        let mut any_found = false;
        let names = &words[1 + first_name..];
        if names.is_empty() {
            self.last_command_substitution_status.set(Some(0));
            return Some(String::new());
        }
        for name in names {
            let name = self.expand_word(name);
            match self.describe_name_with_io(&name, mode, use_standard_path, false, &mut stdout) {
                Ok(true) => any_found = true,
                // command.def:115-117: the per-operand "not found" diagnostic
                // belongs to -V only, and GNU routes it to the shell's stderr
                // even from inside a substitution (oracle probe:
                // `$(command -V echo nosuch)` captures the description while
                // `command: nosuch: not found` reaches stderr). eprintln! is
                // this file's established carrier for expansion-time stderr
                // (the `cat` shortcut above the describe call).
                Ok(false) => {
                    if mode == TypeDescribeMode::Verbose {
                        eprintln!("{}command: {name}: not found", self.diagnostic_prefix());
                    }
                }
                Err(_) => {}
            }
        }
        self.last_command_substitution_status
            .set(Some(if any_found { 0 } else { 1 }));
        Some(
            crate::executor::substitution_metadata::bytes_to_shell_text(&stdout)
                .trim_capture_terminator()
                .to_string(),
        )
    }

    pub(in crate::executor) fn quoted_positional_at_word_values(
        &self,
        word: &str,
        kind: Option<&TokenKind>,
    ) -> Option<Vec<String>> {
        let quoted_positional_word =
            (word.starts_with('"') && word.ends_with('"')) || word.starts_with(STORAGE_WORD_PREFIX);
        let word = word
            .strip_prefix('"')
            .and_then(|word| word.strip_suffix('"'))
            .unwrap_or(word);
        let word = word.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(word);
        if word == "${@}" {
            return Some(self.shell_state.positional_window_cloned());
        }
        if word == "$@" && kind.map_or(true, |kind| *kind == TokenKind::Word) {
            return Some(self.shell_state.positional_window_cloned());
        }
        if let Some(name) = whole_word_braced_parameter_body(word) {
            // A quoted indirect reference whose target is @ or * expands with
            // the same word-boundary rules as "${@}" / "${*}" (GNU subst.c
            // parameter_brace_expand_indir re-expands the target name in the
            // caller's quote context): "${!foo}" with foo=@ yields one word
            // per positional parameter, foo=* one joined word. Unquoted
            // indirect references keep today's join-and-split path.
            if quoted_positional_word {
                if let Some(indirect) = name.strip_prefix('!') {
                    // GNU subst.c parameter_brace_expand_indir +
                    // parameter_brace_transform: `${!X@T}` / `${!X[@]@T}` /
                    // `${!X[*]@T}` resolve the indirection to a LIST when the
                    // target is `arr[@]`/`arr[*]` (or, for `${!arr[@]@T}` on
                    // an array, its keys — arrayfunc.c array_keys), then the
                    // @T transform applies to each element (list_transform).
                    if let Some((ref_name, transform)) = parse_parameter_transform(name) {
                        if let Some(ind) = ref_name.strip_prefix('!') {
                            let subscripted = ind
                                .strip_suffix("[@]")
                                .map(|base| (base, false))
                                .or_else(|| ind.strip_suffix("[*]").map(|base| (base, true)));
                            let resolved_list: Option<(Vec<String>, bool)> = (|| {
                                if let Some((base, starred)) = subscripted {
                                    let resolved = self.resolved_variable_name(base)?;
                                    let is_assoc = is_marked_var(
                                        &self.shell_state.env_vars,
                                        ASSOC_VARS,
                                        &resolved,
                                    );
                                    let is_array = is_assoc
                                        || is_marked_array_var(
                                            &self.shell_state.env_vars,
                                            &resolved,
                                        )
                                        || self
                                            .shell_state
                                            .env_vars
                                            .get(&resolved)
                                            .is_some_and(|value| is_array_storage(value));
                                    if is_array {
                                        let storage = self.shell_state.env_vars.get(&resolved)?;
                                        let keys = if is_assoc {
                                            assoc_keys(
                                                storage,
                                                assoc_nbuckets(
                                                    &self.shell_state.env_vars,
                                                    &resolved,
                                                ),
                                            )
                                        } else {
                                            array_indices(storage)
                                        };
                                        return Some((keys, starred));
                                    }
                                    // Scalar X with [@]/[*]: the subscript
                                    // collapses to X[0] (array_variable on a
                                    // non-array yields element 0), so the
                                    // indirection target is X's value.
                                    let target = self.shell_state.env_vars.get(&resolved)?;
                                    let arr =
                                        target.strip_suffix("[@]").map(|a| (a, false)).or_else(
                                            || target.strip_suffix("[*]").map(|a| (a, true)),
                                        )?;
                                    let resolved_arr = self.resolved_variable_name(arr.0)?;
                                    let storage = self.shell_state.env_vars.get(&resolved_arr)?;
                                    let values = if is_marked_var(
                                        &self.shell_state.env_vars,
                                        ASSOC_VARS,
                                        &resolved_arr,
                                    ) {
                                        assoc_hash_ordered_values(
                                            storage,
                                            assoc_nbuckets(
                                                &self.shell_state.env_vars,
                                                &resolved_arr,
                                            ),
                                        )
                                    } else {
                                        array_values(storage)
                                    };
                                    Some((values, arr.1))
                                } else {
                                    // `${!X@T}` — indirection target is X's
                                    // value; a `arr[@]`/`arr[*]` target
                                    // expands to the element list.
                                    let resolved = self.resolved_variable_name(ind)?;
                                    let target = self.shell_state.env_vars.get(&resolved)?;
                                    let arr =
                                        target.strip_suffix("[@]").map(|a| (a, false)).or_else(
                                            || target.strip_suffix("[*]").map(|a| (a, true)),
                                        )?;
                                    let resolved_arr = self.resolved_variable_name(arr.0)?;
                                    let storage = self.shell_state.env_vars.get(&resolved_arr)?;
                                    let values = if is_marked_var(
                                        &self.shell_state.env_vars,
                                        ASSOC_VARS,
                                        &resolved_arr,
                                    ) {
                                        assoc_hash_ordered_values(
                                            storage,
                                            assoc_nbuckets(
                                                &self.shell_state.env_vars,
                                                &resolved_arr,
                                            ),
                                        )
                                    } else {
                                        array_values(storage)
                                    };
                                    Some((values, arr.1))
                                }
                            })(
                            );
                            if let Some((elements, starred)) = resolved_list {
                                let transformed = elements
                                    .iter()
                                    .map(|value| {
                                        self.apply_parameter_transform_value(value, transform)
                                    })
                                    .collect::<Vec<_>>();
                                if starred {
                                    // Quoted `*` joins with IFS[0]
                                    // (string_list_pos_params dollar_star).
                                    return Some(vec![
                                        transformed.join(&self.ifs_first_char_separator())
                                    ]);
                                }
                                return Some(transformed);
                            }
                        }
                    }
                    if indirect == "@" {
                        return Some(self.shell_state.positional_window_cloned());
                    }
                    if indirect == "*" {
                        return Some(vec![self
                            .shell_state
                            .positional_window()
                            .join(&self.ifs_first_char_separator())]);
                    }
                    if is_shell_name(indirect) {
                        if let Some(target) =
                            self.shell_state.env_vars.get(indirect).map(String::as_str)
                        {
                            if target == "@" {
                                return Some(self.shell_state.positional_window_cloned());
                            }
                            if target == "*" {
                                return Some(vec![self
                                    .shell_state
                                    .positional_window()
                                    .join(&self.ifs_first_char_separator())]);
                            }
                        }
                    }
                }
            }
            // GNU subst.c param_expand: when the parameter itself is $@ or
            // $* and it is set, the `-`/`:-` operator just uses the
            // parameter's value (TEMP), which carries W_DOLLARAT for $@. In
            // double quotes that means one field per positional parameter
            // (like `"$@"`), and $* joins with IFS[0] (like `"$*"`). The
            // String-based operator path collapses these to a single joined
            // field, so intercept here and return the per-parameter fields.
            // This covers the `for` loop's word-list expansion path, which
            // does not go through expand_command_word.
            if quoted_positional_word {
                let (var_name, _alternate, use_when_set, require_non_empty) =
                    if let Some((var_name, alternate)) = name.split_once(":+") {
                        (var_name, alternate, true, true)
                    } else if let Some((var_name, alternate)) = name.split_once('+') {
                        (var_name, alternate, true, false)
                    } else if let Some((var_name, alternate)) = name.split_once(":-") {
                        (var_name, alternate, false, true)
                    } else if let Some((var_name, alternate)) = name.split_once('-') {
                        (var_name, alternate, false, false)
                    } else {
                        ("", "", false, false)
                    };
                if var_name == "@" || var_name == "*" {
                    let value = self.parameter_operator_value(var_name);
                    let word_used = if use_when_set {
                        value.is_some()
                            && (!require_non_empty || !value.unwrap_or_default().is_empty())
                    } else {
                        value.is_none()
                            || (require_non_empty && value.unwrap_or_default().is_empty())
                    };
                    if !word_used {
                        if var_name == "@" {
                            return Some(self.shell_state.positional_window_cloned());
                        }
                        return Some(vec![self
                            .shell_state
                            .positional_window()
                            .join(&self.ifs_first_char_separator())]);
                    }
                }
            }
            if let Some(values) =
                self.positional_transform_word_values(name, quoted_positional_word)
            {
                return Some(values);
            }
            if let Some(values) = self.positional_modified_word_values(name, quoted_positional_word)
            {
                return Some(values);
            }
            if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
                if var_name == "@" {
                    return Some(positional_parameter_substring_with_zero(
                        self.shell_state.positional_window(),
                        &self.script_name_value(),
                        offset,
                        length,
                    ));
                }
                if var_name == "*" {
                    let values = positional_parameter_substring_with_zero(
                        self.shell_state.positional_window(),
                        &self.script_name_value(),
                        offset,
                        length,
                    );
                    if quoted_positional_word {
                        return Some(vec![values.join(&self.ifs_first_char_separator())]);
                    }
                    return Some(values);
                }
            }
        }
        None
    }

    pub(in crate::executor) fn word_is_unquoted_positional_modified_list_expansion(
        &self,
        word: &str,
    ) -> bool {
        if word.starts_with('"') || word.starts_with('\'') || word.starts_with(STORAGE_WORD_PREFIX)
        {
            return false;
        }
        let Some(inner) = whole_word_braced_parameter_body(word) else {
            return false;
        };
        self.positional_modified_base_name(inner)
            .or_else(|| parse_parameter_transform(inner).map(|(name, _)| name))
            .or_else(|| {
                self.parse_parameter_substring(inner)
                    .map(|(name, _, _)| name)
            })
            .is_some_and(|name| matches!(name, "@" | "*"))
    }

    pub(in crate::executor) fn word_is_unquoted_positional_list_expansion(
        &self,
        word: &str,
    ) -> bool {
        if word.starts_with('"') || word.starts_with('\'') || word.starts_with(STORAGE_WORD_PREFIX)
        {
            return false;
        }
        let Some(inner) = whole_word_braced_parameter_body(word) else {
            return false;
        };
        matches!(inner, "@" | "*")
    }

    fn positional_transform_word_values(&self, name: &str, quoted: bool) -> Option<Vec<String>> {
        let (var_name, transform) = parse_parameter_transform(name)?;
        if !matches!(var_name, "@" | "*") {
            return None;
        }

        let values = if transform == ParameterTransform::Assignment {
            // GNU subst.c @A form: with zero positional parameters there
            // is no `set --` header at all — "${@@A}" expands to zero
            // words and "${*@A}" to a single empty word (rubash#1269).
            if self.shell_state.positional_window().is_empty() {
                if var_name == "*" && quoted {
                    return Some(vec![String::new()]);
                }
                return Some(vec![]);
            }
            let mut values = vec!["set".to_string(), "--".to_string()];
            values.extend(
                self.shell_state
                    .positional_window()
                    .iter()
                    .map(|value| shell_single_quote_assignment_value(value)),
            );
            values
        } else {
            self.shell_state
                .positional_window()
                .iter()
                .map(|value| self.apply_parameter_transform_value(value, transform))
                .collect::<Vec<_>>()
        };

        if var_name == "*" && transform != ParameterTransform::Assignment {
            // GNU string_list_pos_params (subst.c:3030-3057): an unquoted `*`
            // uses dollar_star (IFS[0]) except when IFS is set empty, where
            // Posix interp 888 falls back to dollar_at (space separator); the
            // joined word is then field-split like any unquoted expansion
            // (W_SPLITSPACE). The split runs over list_quote_escapes-protected
            // elements (subst.c:3014), so each element survives verbatim: one
            // word per positional (exp10.sub `${*@Q}` with `set -- ' A ' ' B '`).
            let ifs_set_empty = self
                .shell_state
                .env_vars
                .get("IFS")
                .is_some_and(|value| value.is_empty());
            if ifs_set_empty && !quoted {
                return Some(values);
            }
            let separator = if ifs_set_empty {
                " ".to_string()
            } else {
                self.ifs_first_char_separator()
            };
            return Some(vec![values.join(&separator)]);
        }
        if quoted && var_name == "*" {
            if transform == ParameterTransform::Assignment {
                let mut rendered = String::from("set -- ");
                rendered.push_str(
                    &values[2..]
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(&self.ifs_first_char_separator()),
                );
                return Some(vec![rendered]);
            }
            return Some(vec![values.join(&self.ifs_first_char_separator())]);
        }

        Some(values)
    }

    fn positional_modified_word_values(&self, name: &str, quoted: bool) -> Option<Vec<String>> {
        // positional_modified_values only handles `@`/`*` targets; validate
        // before expanding the pattern/replacement text — expanding first
        // runs expansion side effects (`${a[$((i++))],,}` evaluated the
        // subscript) on a probe that then returns None. GNU expands the
        // `${}` body exactly once.
        let positional_target = |var_name: &str| matches!(var_name, "@" | "*");

        if let Some((var_name, pattern, operation)) = parse_indirect_pattern_removal(name) {
            if !positional_target(var_name) {
                return None;
            }
            let pattern = self.expand_parameter_pattern_word(pattern);
            return self.positional_modified_values(var_name, quoted, |value| {
                remove_parameter_pattern(value, &pattern, operation, self.extglob_enabled())
            });
        }

        if let Some((var_name, pattern, replacement, global)) = parse_parameter_replacement(name) {
            if !positional_target(var_name) {
                return None;
            }
            let pattern = self.expand_parameter_pattern_word(pattern);
            let replacement = self.expand_patsub_replacement_text(replacement);
            return self.positional_modified_values(var_name, quoted, |value| {
                self.replace_patsub_pattern(value, &pattern, &replacement, global)
            });
        }

        if let Some((var_name, operation, pattern)) = parse_parameter_case_mod(name) {
            if !positional_target(var_name) {
                return None;
            }
            let pattern = self.expand_embedded_parameters(pattern);
            return self.positional_modified_values(var_name, quoted, |value| {
                apply_parameter_case_mod(value, operation, &pattern)
            });
        }

        None
    }

    fn positional_modified_base_name<'a>(&self, name: &'a str) -> Option<&'a str> {
        parse_indirect_pattern_removal(name)
            .map(|(name, _, _)| name)
            .or_else(|| parse_parameter_replacement(name).map(|(name, _, _, _)| name))
            .or_else(|| parse_parameter_case_mod(name).map(|(name, _, _)| name))
    }

    fn positional_modified_values<F>(
        &self,
        name: &str,
        quoted: bool,
        modify: F,
    ) -> Option<Vec<String>>
    where
        F: Fn(&str) -> String,
    {
        if !matches!(name, "@" | "*") {
            return None;
        }
        let values = self
            .shell_state
            .positional_window()
            .iter()
            .map(|value| modify(value))
            .collect::<Vec<_>>();
        if quoted && name == "*" {
            return Some(vec![values.join(&self.ifs_first_char_separator())]);
        }
        Some(values)
    }

    /// Resolve an `@`-slice segment inner body (`a[@]:off[:len]` or
    /// `@:off[:len]`) to the sliced element list (rubash#1011). Same helpers
    /// as the standalone paths — `array_subscript_range_values` (what
    /// `array_at_word_values` uses for `"${a[@]:1:2}"`) and
    /// `positional_parameter_substring_with_zero` (what
    /// `quoted_positional_at_word_values` uses for `"${@:2:2}"`) — so
    /// embedded and standalone slices share byte-identical slicing
    /// semantics (GNU subst.c:9050 parameter_brace_substring).
    pub(in crate::executor) fn at_slice_segment_values(&self, inner: &str) -> Vec<String> {
        let Some((var_name, offset, length)) = self.parse_parameter_substring(inner) else {
            return Vec::new();
        };
        if var_name == "@" {
            return positional_parameter_substring_with_zero(
                self.shell_state.positional_window(),
                &self.script_name_value(),
                offset,
                length,
            );
        }
        if let Some(base) = var_name.strip_suffix("[@]") {
            if is_shell_name(base) {
                return self
                    .array_subscript_range_values(base, offset, length)
                    .unwrap_or_default();
            }
        }
        Vec::new()
    }

    /// Per-element values of an `AtModified` segment (rubash#1456): the
    /// segment body is `<target><op>` where the admission gate already
    /// proved the target is `@` or `name[@]` and the operator is one of
    /// the pattern/replacement/case forms. Resolution reuses the exact
    /// helpers the whole-word forms use — `positional_modified_word_values`
    /// for `@` (quoted `@` semantics: the operator applies per parameter)
    /// and `array_pattern_word_values` for `name[@]` — so the per-element
    /// results are identical to the standalone `"${...}"` expansion; only
    /// the enclosing word's affix attachment differs (handled by the
    /// segment expander).
    pub(in crate::executor) fn at_modified_segment_values(&self, inner: &str) -> Vec<String> {
        if inner.starts_with('@') {
            return self
                .positional_modified_word_values(inner, true)
                .unwrap_or_default();
        }
        self.array_pattern_word_values(&format!("${{{inner}}}"), true)
            .unwrap_or_default()
    }

    pub(in crate::executor) fn quoted_positional_at_word_values_with_raw(
        &self,
        word: &str,
        raw: Option<&str>,
        kind: Option<&TokenKind>,
    ) -> Option<Vec<String>> {
        // The raw-word scan is self-guarding: it returns None unless the raw
        // word has a top-level quoted "$@"/"${@}" segment, so whole-word
        // `${...}` forms (including `${@:off:len}`) fall through to the
        // cooked-word path below. Gating on `word.starts_with("${")` wrongly
        // skipped mixed words like `${foo}"$@"`, collapsing the positional
        // words into one field under a null IFS (array6.sub
        // `recho ${foo}"$@"` with IFS=).
        if let Some(values) = raw.and_then(|raw| {
            quoted_positional_at_segments(raw, &self.shell_state.env_vars, &|name| {
                self.nameref_target_name(name)
            })
            .map(|segments| {
                expand_quoted_positional_at_segments(
                    &segments,
                    self.shell_state.env_vars.get("IFS").map(String::as_str),
                    |segment| match segment {
                        QuotedPositionalAtSegment::PositionalAt(_) => {
                            self.shell_state.positional_window_cloned()
                        }
                        QuotedPositionalAtSegment::ArrayAt(name, _) => self
                            .array_subscript_range_values(name, 0, None)
                            .unwrap_or_default(),
                        QuotedPositionalAtSegment::ArrayAtSlice(inner, _)
                        | QuotedPositionalAtSegment::PositionalAtSlice(inner, _) => {
                            self.at_slice_segment_values(inner)
                        }
                        QuotedPositionalAtSegment::AtModified(inner, _) => {
                            self.at_modified_segment_values(inner)
                        }
                        // GNU string_list_dollar_star: `[*]` joins the
                        // elements with IFS[0] into one word.
                        QuotedPositionalAtSegment::ArrayStar(name, _) => vec![self
                            .array_subscript_range_values(name, 0, None)
                            .unwrap_or_default()
                            .join(&self.ifs_first_char_separator())],
                        QuotedPositionalAtSegment::Literal { .. } => Vec::new(),
                    },
                    |literal| self.expand_embedded_parameters(literal),
                )
            })
        }) {
            return Some(values);
        }

        self.quoted_positional_at_word_values(word, kind)
    }

    pub(in crate::executor) fn join_array_parameter_values(
        &self,
        value: &str,
        expression: &str,
    ) -> String {
        // GNU assoc.c / hashlib.c iterate `${name[@]}` over hash-slot order,
        // not insertion order. Every other array-expansion path already routes
        // declared associative variables through assoc_hash_ordered_values; this
        // shared join helper was the one still using raw insertion order
        // (assoc4.sub: `"at|${i[@]}"` gave `at|fooq  barq ` instead of
        // `at| barq  fooq`, while the standalone `"${i[@]}"` and unquoted
        // `${i[@]}` forms were already correct).
        //
        // `expression` is the already-stripped parameter name (`foo[@]`,
        // `foo[*]`, or a transform expression), so the `[@]`/`[*]` suffix is
        // present directly - there is no leading `${` to remove first.
        //
        // Perf: the indexed branch borrows the content-addressed memo
        // (`indexed_array_entries_cached`, same discipline as the assoc parse
        // memo) instead of re-scanning the storage string per join; the
        // storage decode already yields final element text, so the
        // `normalize_array_expanded_value` pass is the identity and is
        // skipped rather than cloned through a second `Vec`. The `[*]`
        // separator (`subst.c:2902 string_list_dollar_star` over
        // `subst.c:2864 ifs_firstchar`) is looked up once, outside any
        // per-element loop.
        let array_name = expression
            .strip_suffix("[@]")
            .or_else(|| expression.strip_suffix("[*]"))
            .unwrap_or_default();
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name) {
            // Assoc: hash-slot order (assoc.c/hashlib.c hash_search buckets);
            // `assoc_hash_ordered_values` already borrows its parse memo, and
            // its elements are final text — no normalize clone.
            let ordered = assoc_hash_ordered_values(
                value,
                assoc_nbuckets(&self.shell_state.env_vars, array_name),
            );
            return self.join_expanded_array_values(ordered, expression);
        }
        // Indexed: memoised index-order entries (GNU array.c array_max_index
        // order); borrow the values, no intermediate `Vec<String>` clone.
        let entries = indexed_array_entries_cached(value);
        if expression.ends_with("[*]") {
            let separator = self.ifs_first_char_separator();
            let parts: Vec<&str> = entries.values().map(String::as_str).collect();
            parts.join(&separator)
        } else {
            let parts: Vec<&str> = entries.values().map(String::as_str).collect();
            parts.join(" ")
        }
    }

    pub(in crate::executor) fn join_expanded_array_values(
        &self,
        values: Vec<String>,
        expression: &str,
    ) -> String {
        if expression.ends_with("[*]") {
            // GNU dollar_star (subst.c:2902 string_list_dollar_star): IFS[0], the
            // empty string when IFS is set empty, a space when IFS is unset
            // (subst.c:2864 ifs_firstchar).
            return values.join(&self.ifs_first_char_separator());
        }
        values.join(" ")
    }

    pub(in crate::executor) fn report_command_substitution_heredoc_warning(
        &self,
        source: &str,
        command: &CommandNode,
    ) {
        let current_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .unwrap_or_else(|| command.line.unwrap_or(1));
        let start_line = command.line.unwrap_or(1);
        // GNU make_cmd.c:627: `lineno` is the line_number at the time
        // gather_here_documents was called (the line where `<<EOF` appeared),
        // and internal_warning's prefix uses the current line_number (after
        // make_here_document read the body, i.e. the EOF/delimiter line).
        // Use start_line (which accounts for leading newlines in the comsub
        // source) as the base, not current_line (which is the outer command
        // line, not the comsub body start line).
        let warning_line = start_line + source.lines().count().saturating_sub(1);
        let delimiter = command.heredoc_delimiter.as_deref().unwrap_or("");
        eprintln!(
            "{}warning: here-document at line {start_line} delimited by end-of-file (wanted `{delimiter}')",
            self.error_prefix_for_line(warning_line)
        );
    }

    pub(in crate::executor) fn run_external_command_substitution(
        &self,
        words: &[String],
    ) -> Option<String> {
        words.first()?;
        let stdio = self.command_substitution_words_and_stdio(words)?;
        // The front face's claim keys are the registry's COMSUB_FRONT
        // projection (rubash#2036 S4), slash forms included.
        if stdio
            .expanded_words
            .first()
            .and_then(|word| comsub_front_slot(word))
            == Some(ComsubFrontSlot::Tty)
        {
            let silent = stdio
                .expanded_words
                .iter()
                .skip(1)
                .any(|arg| arg == "-s" || arg == "--silent" || arg == "--quiet");
            self.last_command_substitution_status.set(Some(1));
            return Some(if silent {
                String::new()
            } else {
                "not a tty".to_string()
            });
        }
        // Don't route shell builtins through the external-command path.
        // On Windows, "fc" resolves to system32\fc.exe (file compare),
        // not the shell's "fc" builtin. Also respect "enable -n". The
        // hidden late builtins (uname/arch, rubash#154 identity tools)
        // must fall through the same way or `$(uname -s)` would run
        // whatever external sits on PATH instead of the engine builtin.
        let first_word = stdio
            .expanded_words
            .first()
            .map(String::as_str)
            .unwrap_or("");
        if (self.routes_as_builtin(first_word)
            || crate::executor::command_dispatch_late::is_hidden_late_builtin(first_word))
            && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, first_word)
        {
            return None;
        }
        // #800A: the same inline stand-in the simple-command path uses
        // (external_file_builtins::external_tput). The harvest assets pay
        // ~32 `$(tput …)` calls per source, and without this arm each one
        // spawned the real child (~17ms host, ~300ms on the harvest
        // runner). Admission is the byte-exact whitelist; a non-admitted
        // shape (option forms, unlisted TERM, unknown caps) falls through
        // to the external spawner below, unchanged. Like the tty arm this
        // &self path records no hash-table entry (`$(tty)` has the same
        // introspection face).
        #[cfg(windows)]
        if comsub_front_slot(first_word) == Some(ComsubFrontSlot::Tput) {
            if let Some(outcome) = self.admitted_tput_outcome_for_words(&stdio.expanded_words) {
                let (output, status) = match outcome {
                    crate::executor::tput_emulation::TputOutcome::Bytes(bytes) => {
                        (bytes.into_owned(), 0)
                    }
                    // ncurses: an absent capability prints nothing, exits 1
                    // (verified `TERM=vt100 tput setaf 1`).
                    crate::executor::tput_emulation::TputOutcome::Absent => (String::new(), 1),
                };
                self.last_command_substitution_status.set(Some(status));
                return Some(output);
            }
        }
        let Some(program) = find_user_command(&stdio.expanded_words[0], &self.shell_state.env_vars)
        else {
            if stdio.expanded_words.first().map(String::as_str) == Some("mktemp") {
                return None;
            }
            // Not an external command: let the full-execution fallback run
            // the source (functions, builtins, compound commands).
            return None;
        };
        let dev_stdin = if let Some(stdin_path) = &stdio.stdin_path {
            crate::executor::dev_fd_operands::DevOperandStdin::Path(stdin_path.clone())
        } else if let Some(input) = self.function_stdin_remaining() {
            crate::executor::dev_fd_operands::DevOperandStdin::Payload(
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
            )
        } else {
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable
        };
        let dev_stdout = match &stdio.stdout_redirect {
            Some(redirect) => {
                crate::executor::dev_fd_operands::DevOperandStdout::Path(redirect.path.clone())
            }
            None => crate::executor::dev_fd_operands::DevOperandStdout::Capture,
        };
        let (dev_args, dev_ops) =
            self.materialize_dev_fd_operands(&stdio.expanded_words[1..], dev_stdin, dev_stdout);
        // rubash#1905: synthetic /proc operands become spawn-time
        // snapshot files for a child without its own /proc layer; the
        // record lives until the substitution's output is collected
        // (function scope end).
        let (proc_args, _proc_ops) = crate::executor::proc_operands::materialize_proc_operands(
            &dev_args,
            Some(&stdio.expanded_words[0]),
            Some(program.as_path()),
            &self.shell_state.env_vars,
        );
        let (mut process, _) = external_command_for_named_program(
            &program,
            Some(&stdio.expanded_words[0]),
            &proc_args,
            &self.shell_state.env_vars,
            self.bash_argv0_env_supplied(&[]),
        );

        self.apply_child_environment(&mut process);
        // GNU subst.c:7143 command_substitute forks sharing the parent's
        // fd 0: feed the child the unread tail of FUNCTION_STDIN and, after
        // it runs, drain the caller's cursor to EOF.
        let mut piped_stdin: Option<Vec<u8>> = None;
        let mut fd_table_text_stdin = false;
        if let Some(stdin_path) = stdio.stdin_path {
            let file = File::open(stdin_path).ok()?;
            process.stdin(Stdio::from(file));
        } else if self.fd_table.is_closed(0)
            || (self.fd_table.has_entry(0) && !self.fd_table.is_open_for_read(0))
        {
            // rubash#1877: mirror apply_external_stdin_redirect's tail —
            // an exec-installed closed/write-only fd 0 gives the forked
            // child no readable descriptor (rubash#1102), never the
            // process's ambient stdin.
            process.stdin(crate::executor::external_setup::closed_stdin());
        } else if let Some(
            FdReadEndpoint::File(file_fd) | FdReadEndpoint::CoprocStdout { fd: file_fd, .. },
        ) = self.fd_table.read_endpoint(0)
        {
            // rubash#1877: GNU subst.c:7143 command_substitute forks, so
            // the child shares fd 0's open file description — after
            // `exec < file` an external command inside $( ) reads from
            // the shell's current offset and advances it for the next
            // reader. duplicate_handle shares the file pointer exactly
            // like the generic spawn path (apply_external_stdin_redirect,
            // rubash#1102/#1103); this arm must win over the
            // FUNCTION_STDIN buffer below for the same reason.
            let dup = crate::fd::duplicate_handle(file_fd.handle).ok()?;
            process.stdin(Stdio::from(crate::fd::handle_to_file(dup)));
        } else if let Some(input) = self.function_stdin_remaining() {
            process.stdin(Stdio::piped());
            piped_stdin =
                Some(crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input));
        } else if let Some(bytes) = self.virtual_fd_stdin_remaining_bytes(0) {
            // Buffered fd-0 endpoint (heredoc/herestring text or a
            // process-substitution stream staged by `exec`): the forked
            // child drains the stream, so feed the unread remainder and
            // consume the parent's cursor once it has run.
            process.stdin(Stdio::piped());
            piped_stdin = Some(bytes);
            fd_table_text_stdin = true;
        }
        if let Some(redirect) = &stdio.stdout_redirect {
            let file = open_command_substitution_redirect(redirect).ok()?;
            process.stdout(Stdio::from(file));
        } else {
            process.stdout(Stdio::piped());
        }
        if let Some(redirect) = &stdio.stderr_redirect {
            let file = open_command_substitution_redirect(redirect).ok()?;
            process.stderr(Stdio::from(file));
        } else if self.command_substitution_stderr_is_process_stderr() {
            // GNU subst.c:7143 command_substitute: the forked child shares
            // the parent's fd 2 — only fd 1 is replaced by the capture
            // pipe, fd 2 is never captured (subst.c:7149). Inherit it live:
            // piping it into wait_with_output's buffer dropped the bytes
            // wholesale, so a spawned child's diagnostics inside $( )
            // vanished while its status survived (rubash#2086).
            process.stderr(Stdio::inherit());
        } else {
            // fd 2 is rebound (a file, a stdout snapshot), closed, or
            // captured by an enclosing pipeline stage in this context: a
            // bare spawn has no handle for that binding. Decline to the
            // full-execution path, whose stdio planner realizes fd 2 from
            // the fd table (the same fall-through every other
            // inexpressible shape in this route takes).
            return None;
        }
        // rubash#438: /dev/fd/N operand words stay literal on Unix; pin the
        // substitution pipe read ends at their numbers in the child.
        #[cfg(unix)]
        crate::executor::dev_fd_operands::attach_unix_dev_fd_operands(&mut process, &dev_ops);
        process.current_dir(&self.shell_state.cwd);
        suppress_orphan_console(&mut process);
        let mut spawned = process.spawn().ok()?;
        adopt_orphan_console(spawned.id());
        if let Some(input) = piped_stdin.as_deref() {
            if let Some(mut child_stdin) = spawned.stdin.take() {
                use std::io::Write;
                let _ = child_stdin.write_all(input);
            }
        }
        let mut output = spawned.wait_with_output().ok()?;
        output.stdout.extend(self.collect_dev_fd_capture(&dev_ops));
        if piped_stdin.is_some() && !fd_table_text_stdin {
            if let Some(text) = self.shell_state.env_vars.get(FUNCTION_STDIN) {
                self.comsub_stdin_writeback
                    .set(Some((text.len(), Self::function_stdin_fingerprint(text))));
            }
        }
        if fd_table_text_stdin {
            // The child drained the buffered fd-0 stream (shared-offset
            // semantics, redir.c): the parent's cursor moves to EOF.
            self.fd_table.drain_input_to_eof(0);
        }
        let status = crate::executor::wait_status::process_exit_status(&output.status);
        if stdio.expanded_words.first().map(String::as_str) == Some("mktemp")
            && status != 0
            && !stdio.had_redirect
        {
            return None;
        }
        self.last_command_substitution_status.set(Some(status));
        if stdio.stdout_redirect.is_some() {
            return Some(String::new());
        }
        Some(
            bytes_to_shell_text(&output.stdout)
                .trim_capture_terminator()
                .to_string(),
        )
    }

    /// True when this executor's fd 2 is the ambient process stderr: the
    /// seeded default endpoint, not closed, and not shadowed by an
    /// executor-level stderr capture (a pipeline-stage body installs one,
    /// and its buffered routing — not a live handle — owns fd 2 there).
    /// Only then does `Stdio::inherit()` name exactly the object GNU's
    /// substitution fork leaves on the child's fd 2 (subst.c:7143).
    fn command_substitution_stderr_is_process_stderr(&self) -> bool {
        !self.fd_table.is_closed(2)
            && self.stderr_capture.is_none()
            && matches!(
                self.fd_table.output_endpoint(2),
                Some(FdWriteEndpoint::Stderr)
            )
    }

    fn command_substitution_words_and_stdio(
        &self,
        words: &[String],
    ) -> Option<CommandSubstitutionStdio> {
        let mut stdio = CommandSubstitutionStdio::default();
        let mut index = 0;
        while index < words.len() {
            match words[index].as_str() {
                "|" => return None,
                "<" => {
                    stdio.stdin_path =
                        Some(self.command_substitution_redirect_path(words.get(index + 1)?)?);
                    stdio.had_redirect = true;
                    index += 2;
                }
                ">" | "1>" | ">|" | "1>|" => {
                    stdio.stdout_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: false,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                ">>" | "1>>" => {
                    stdio.stdout_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: true,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                "2>" | "2>|" => {
                    stdio.stderr_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: false,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                "2>>" => {
                    stdio.stderr_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: true,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                word => {
                    stdio
                        .expanded_words
                        .extend(self.expand_command_substitution_arg_values(word));
                    index += 1;
                }
            }
        }
        (!stdio.expanded_words.is_empty()).then_some(stdio)
    }

    fn command_substitution_redirect_path(&self, target: &str) -> Option<PathBuf> {
        let expanded = strip_matching_quotes(&self.expand_word(target)).to_string();
        // G-02: comsub redirect opens anchor at the session cwd.
        Some(self.session_path(shell_path_to_windows(&expanded, &self.shell_state.env_vars)))
    }

    pub(in crate::executor) fn expand_backtick_substitution_typed(
        &mut self,
        word: &str,
        quoted: bool,
    ) -> Option<SubstitutionOutput> {
        if !backtick_substitution_spans_whole_word(word) {
            return None;
        }
        let source =
            decode_backtick_substitution_source(word.strip_prefix('`')?.strip_suffix('`')?);
        // GNU keeps backquote bodies verbatim (parse.y:3877
        // parse_matched_pair) — no print_comsub canonicalization — so their
        // diagnostics number the raw body lines under the same (S - 1) base
        // as `$()` bodies (probes b1/b2, rubash#201).
        // The ambient backtick window lasts through the expansion: the
        // body's parse and diagnostics render as GNU's parse_and_execute
        // child (whom "command substitution", subst.c:7413), and a body
        // parse error latches the backtick companion so the enclosing
        // command recovers (issue #515).
        let saved_backtick_context = self.backtick_body_diagnostic.replace(true);
        let output = super::command_substitution::with_comsub_raw_text(|| {
            self.expand_command_substitution_mut_typed_with_context(
                &source,
                if quoted {
                    SubstitutionQuoteContext::DoubleQuoted
                } else {
                    SubstitutionQuoteContext::Unquoted
                },
            )
        });
        self.backtick_body_diagnostic.set(saved_backtick_context);
        Some(output)
    }

    pub(in crate::executor) fn expand_backtick_substitution(&self, word: &str) -> Option<String> {
        // TODO(subst.c): Backquote command substitution should invoke the
        // parser and run a subshell. This reuses the same in-process command
        // substitution bridge as `$()`.
        if !backtick_substitution_spans_whole_word(word) {
            return None;
        }
        let source =
            decode_backtick_substitution_source(word.strip_prefix('`')?.strip_suffix('`')?);
        // Same ambient backtick window as the typed path (issue #515).
        let saved_backtick_context = self.backtick_body_diagnostic.replace(true);
        let expanded = super::command_substitution::with_comsub_raw_text(|| {
            self.expand_command_substitution_readback_with_context(
                &source,
                SubstitutionQuoteContext::Unquoted,
            )
        });
        self.backtick_body_diagnostic.set(saved_backtick_context);
        Some(expanded.text_lossy())
    }

    pub(in crate::executor) fn expand_dirstack_tilde(&self, word: &str) -> Option<String> {
        // GNU general.c:1120 bash_special_tilde_expansions +
        // builtins/pushd.def:620 get_dirstack_from_string. The shared
        // implementation lives in expand/tilde (the plain-word and
        // assignment paths resolve the same prefixes there): `~+N` and
        // bare `~N` count from the stack top, `~-N` from the bottom, and
        // a `/tail` glues onto the resolved directory (rubash#1244: this
        // copy handled only `~N`/`~-N` with no tail, so `~+1` and
        // `~-0/sub` stayed literal). Some(word) verbatim when the prefix
        // is a special form that does not resolve (out of range); None
        // when it is not a special prefix at all, so the caller falls
        // through to the `~user` lookup.
        match crate::expand::tilde::tilde::special_tilde_expansion(word, &self.shell_state.env_vars)
        {
            Some(Some(value)) => Some(value),
            Some(None) => Some(word.to_string()),
            None => None,
        }
    }

    pub(in crate::executor) fn dirstack_subscript(&self, index: &str) -> Option<usize> {
        if let Ok(index) = index.parse::<usize>() {
            return Some(index);
        }

        if index == "NDIRS" {
            return self
                .shell_state
                .env_vars
                .get("NDIRS")
                .and_then(|value| value.parse::<usize>().ok())
                .or_else(|| {
                    Some(
                        crate::builtins::pushd::load_stack(&self.shell_state.env_vars)
                            .len()
                            .saturating_sub(1),
                    )
                });
        }

        let (name, rhs) = index.split_once('-')?;
        if name != "NDIRS" {
            return None;
        }
        let rhs = rhs.parse::<usize>().ok()?;
        let ndirs = self
            .shell_state
            .env_vars
            .get("NDIRS")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_else(|| {
                crate::builtins::pushd::load_stack(&self.shell_state.env_vars)
                    .len()
                    .saturating_sub(1)
            });
        ndirs.checked_sub(rhs)
    }
}

fn decode_backtick_substitution_source(source: &str) -> String {
    decode_old_style_backtick_source(source)
        .replace(crate::executor::markers::DATA_BACKTICK, "`")
        .replace(crate::executor::markers::CTLESC, "")
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
}

#[derive(Default)]
struct CommandSubstitutionStdio {
    expanded_words: Vec<String>,
    stdin_path: Option<PathBuf>,
    stdout_redirect: Option<CommandSubstitutionRedirect>,
    stderr_redirect: Option<CommandSubstitutionRedirect>,
    had_redirect: bool,
}

struct CommandSubstitutionRedirect {
    path: PathBuf,
    append: bool,
}

fn open_command_substitution_redirect(redirect: &CommandSubstitutionRedirect) -> io::Result<File> {
    if redirect.append {
        return open_redirect_append(&redirect.path);
    }
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&redirect.path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QuotedPositionalAtSegment {
    /// Literal text. `quoted` marks text that came from inside a double-quoted
    /// span (GNU subst.c add_quoted_char): its IFS characters are quote-
    /// protected data, so they attach to the adjacent word and never field-
    /// split. Text collected between quoted spans is `quoted == false` and
    /// keeps the historic unquoted-literal split behavior.
    Literal { text: String, quoted: bool },
    /// `quoted` distinguishes a `"$@"`/`"${@}"` span from a bare unquoted
    /// `$@`/`${@}`: quoted elements' IFS characters are quote-protected data
    /// (GNU CTLESC, subst.c:12273+ word splitting skips them); unquoted
    /// elements field-split like any other unquoted expansion result.
    PositionalAt(bool),
    /// A `"${name[@]}"`/`${name[@]}` span: expands with the same affix rules
    /// as `"$@"`/`$@` — one word per element, affixes attach to the
    /// first/last word (GNU subst.c array_at word-list handling in
    /// expand_word_internal). `quoted` carries the same split rule as
    /// `PositionalAt`.
    ArrayAt(String, bool),
    /// An `@`-slice span — `"${name[@]:off[:len]}"`/`${name[@]:off[:len]}`
    /// or `"${@:off[:len]}"`/`${@:off[:len]}`: GNU subst.c:9050
    /// parameter_brace_substring (VT_ARRAYVAR/VT_POSPARMS arms,
    /// subst.c:9101-9136) keeps the @-list word boundaries
    /// (array_subrange/string_list_pos_params), so the slice fans out
    /// exactly like the unsliced list with affixes on the first/last
    /// element word. Carries the inner `${...}` body plus the `quoted`
    /// split rule; resolution reuses the standalone-slice helpers.
    ArrayAtSlice(String, bool),
    /// A `"${@:off[:len]}"`/`${@:off[:len]}` span: positional slice of the
    /// same GNU parameter_brace_substring arm. Same fan-out contract as
    /// `ArrayAtSlice`.
    PositionalAtSlice(String, bool),
    /// A `"${name[@]<op>}"`/`${<op>}` span with a per-element operator —
    /// pattern removal (`%`/`%%`/`#`/`##`), pattern replacement (`/`),
    /// or case modification (`^`/`,`): GNU subst.c:10119-10128
    /// parameter_brace_expand runs chk_atstar (subst.c:7635-7639) before
    /// the operator switch, so the @-list word boundaries survive the
    /// operator and each element is transformed separately
    /// (subst.c:9777 parameter_brace_expand's per-element operator loop);
    /// affixes attach to the first/last element word exactly like the
    /// unsliced list (rubash#1456). Carries the inner `${...}` body plus
    /// the `quoted` split rule; resolution reuses the whole-word
    /// per-element operator helpers.
    AtModified(String, bool),
    /// A `"${name[*]}"`-class span reached through indirection (`${!name}`
    /// whose value is `arr[*]`, or a `$ref` nameref to `arr[*]`): GNU
    /// joins the elements with IFS[0] into one word (subst.c
    /// string_list_dollar_star), keeping the same affix rules.
    ArrayStar(String, bool),
}

/// `${!name}` where `name` holds `arr[@]`/`arr[*]`: the indirect expansion
/// is the referenced array's element list (subst.c param_expand indirect
/// name -> array_at), not the literal text. Returns (array_name, is_star).
fn indirect_array_target(
    env_vars: &std::collections::HashMap<String, String>,
    name: &str,
) -> Option<(String, bool)> {
    if !is_shell_name(name) {
        return None;
    }
    let target = env_vars.get(name)?;
    if let Some(base) = target.strip_suffix("[@]") {
        return Some((base.to_string(), false));
    }
    target
        .strip_suffix("[*]")
        .map(|base| (base.to_string(), true))
}

/// An unbraced `$name` whose nameref cell is `arr[@]`/`arr[*]` expands to
/// the referenced array's elements (GNU find_variable_nameref — the
/// whole-word `recho "$ref"` case in nameref18.sub; the same word-list
/// expansion applies inside composite words).
fn nameref_array_target(
    nameref_target: &dyn Fn(&str) -> Option<String>,
    name: &str,
) -> Option<(String, bool)> {
    let target = nameref_target(name)?;
    if let Some(base) = target.strip_suffix("[@]") {
        return Some((base.to_string(), false));
    }
    target
        .strip_suffix("[*]")
        .map(|base| (base.to_string(), true))
}

/// Word-list target of an `@`-slice span for the segment scanners below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtSliceTarget {
    /// `name[@]:off[:len]` — slice of array `name`'s elements.
    Array,
    /// `@:off[:len]` — slice of the positional parameters.
    Positional,
}

/// Validate an `${...}` inner body as an `@`-slice word-list source for the
/// raw-word segment scanners (`name[@]:off[:len]` or `@:off[:len]`, rubash#1011).
///
/// GNU anchor — subst.c:10119-10128 parameter_brace_expand calls chk_atstar
/// (subst.c:7635-7639) on every valid array reference BEFORE the operator
/// switch, so a quoted `[@]`/`@` marks quoted_dollar_at even with the `:`
/// substring operator present; subst.c:9050 parameter_brace_substring then
/// keeps the @-list word boundaries (VT_ARRAYVAR/VT_POSPARMS arms,
/// subst.c:9101-9136: array_subrange/string_list_pos_params). The slice
/// therefore splits the enclosing word exactly like the unsliced list, with
/// affixes on the first/last element word.
///
/// Admission is a whitelist (rubash#117 discipline): anything this declines
/// stays on the pre-existing path byte-for-byte. Declined: `[*]` slices
/// (GNU string_list_dollar_star joins those into one word, which the String
/// path already produces), `!`-indirect forms (different GNU arm,
/// subst.c:10017 parameter_brace_expand_indir/keys), operator-looking rests
/// (`:-`/`:=`/`:+`/`:?` belong to the operator paths, never the substring
/// path — parse_parameter_substring rejects them the same way), and spans
/// whose slice region contains `$(` or a backtick (offsets evaluated by the
/// `&self` segment resolver cannot execute command substitutions; those
/// stay on the `&mut` generic path).
fn at_slice_segment_body(inner: &str) -> Option<AtSliceTarget> {
    if let Some(rest) = inner.strip_prefix("@:") {
        return at_slice_rest_admitted(rest).then_some(AtSliceTarget::Positional);
    }
    let (name, rest) = inner.split_once("[@]:")?;
    if !is_shell_name(name) {
        return None;
    }
    at_slice_rest_admitted(rest).then_some(AtSliceTarget::Array)
}

/// Validate an `${...}` inner body as a per-element-operator word-list
/// source for the segment scanners (`name[@]%pat`, `name[@]/pat/repl`,
/// `name[@]^^`, `@%pat`, ...; rubash#1456).
///
/// GNU anchor — subst.c:10119-10128 parameter_brace_expand calls
/// chk_atstar (subst.c:7635-7639) BEFORE the operator switch, so a quoted
/// `[@]`/`@` target keeps quoted_dollar_at and the operator is applied
/// per element (subst.c:9777 parameter_brace_expand's array loop), never
/// to the space-joined string the generic path produces.
///
/// Admission is a whitelist (rubash#117 discipline), mirroring
/// `at_slice_segment_body`: the body must parse through one of the three
/// pure text operator parsers the whole-word helpers themselves use, and
/// its target must be exactly `@` or `name[@]`. Declined: `[*]` targets
/// (GNU joins those into one word — the String path already does),
/// `:-`-family operators (no per-element loop in GNU; the word stays a
/// single value on the braced-alternate path), `${name[@]@Q}`-style
/// transforms (different resolver), `!`-indirect targets, and bodies
/// containing `$(` or a backtick anywhere (the `&self` segment resolver
/// cannot execute command substitutions; those stay on the generic path).
fn at_modified_segment_body(inner: &str) -> bool {
    if inner.contains("$(") || inner.contains('`') {
        return false;
    }
    let target_ok = |var_name: &str| {
        var_name == "@"
            || var_name
                .strip_suffix("[@]")
                .is_some_and(|name| is_shell_name(name))
    };
    if let Some((var_name, _, _)) = parse_indirect_pattern_removal(inner) {
        return target_ok(var_name);
    }
    if let Some((var_name, _, _, _)) = parse_parameter_replacement(inner) {
        return target_ok(var_name);
    }
    if let Some((var_name, _, _)) = parse_parameter_case_mod(inner) {
        return target_ok(var_name);
    }
    false
}

/// Shared admission for the offset/length region after `@:`/`[@]:` — see
/// `at_slice_segment_body` for the rationale per clause.
fn at_slice_rest_admitted(rest: &str) -> bool {
    if rest.is_empty() {
        return false;
    }
    if matches!(rest.as_bytes()[0], b'=' | b'+' | b'?' | b'-') {
        return false;
    }
    if rest.contains("$(") || rest.contains('`') {
        return false;
    }
    true
}

fn quoted_positional_at_segments(
    raw: &str,
    env_vars: &std::collections::HashMap<String, String>,
    nameref_target: &dyn Fn(&str) -> Option<String>,
) -> Option<Vec<QuotedPositionalAtSegment>> {
    let chars = raw.chars().collect::<Vec<_>>();
    // Map char indices to byte offsets so `${...}` bodies can be skipped with
    // the canonical `matching_parameter_brace` scanner (parameter_ops.rs),
    // which honors quotes, nested expansions, and bracket patterns.
    let char_to_byte: Vec<usize> = raw.char_indices().map(|(offset, _)| offset).collect();
    let mut segments = Vec::new();
    let mut literal_start = 0usize;
    let mut index = 0usize;
    let mut saw_positional_at = false;
    // Unquoted `(`/`)` depth: a compound-assignment body `name=( ... )`
    // (W_COMPASSIGN) and a `$(...)` command substitution both keep their
    // contents out of the OUTER word's segment structure — the compound
    // body expands its own elements via expand_compound_array_assignment
    // (arrayfunc.c:557) and the comsub body is a separate parse. Scanning
    // `$@`/`[@]` inside them as top-level segments would wrongly split the
    // enclosing word (`declare -al ar=(${ar[@]})` must stay one word).
    let mut paren_depth = 0usize;

    while index < chars.len() {
        if paren_depth > 0 {
            match chars[index] {
                '\\' => index += 2,
                '\'' => index = skip_single_quote(&chars, index + 1)?,
                '"' => index = skip_double_quote(&chars, index + 1)?,
                '$' if chars.get(index + 1) == Some(&'\'') => {
                    index = skip_single_quote(&chars, index + 2)?
                }
                '$' if chars.get(index + 1) == Some(&'{') => {
                    let body_start = index + 2;
                    if let Some(&body_start_byte) = char_to_byte.get(body_start) {
                        if let Some(end_byte) = matching_parameter_brace(&raw[body_start_byte..]) {
                            let close_byte = body_start_byte + end_byte;
                            index = raw[..=close_byte].chars().count();
                            continue;
                        }
                    }
                    index = chars.len();
                }
                '(' => {
                    paren_depth += 1;
                    index += 1;
                }
                ')' => {
                    paren_depth -= 1;
                    index += 1;
                }
                _ => index += 1,
            }
            continue;
        }
        match chars[index] {
            '"' => {
                let Some(end) = skip_double_quote(&chars, index + 1) else {
                    return None;
                };
                let body = &chars[index + 1..end];
                if body == "$@".chars().collect::<Vec<_>>().as_slice() {
                    push_quoted_positional_literal_segment(
                        &mut segments,
                        &chars[literal_start..index],
                        false,
                    )?;
                    segments.push(QuotedPositionalAtSegment::PositionalAt(true));
                    saw_positional_at = true;
                    index = end + 1;
                    literal_start = index;
                    continue;
                }
                // A quoted `"${name[@]}"` span is a word-list source just
                // like `"$@"`: affixes attach to the first/last element word.
                if body.first() == Some(&'$')
                    && body.get(1) == Some(&'{')
                    && body.len() > 6
                    && body[body.len() - 4..] == ['[', '@', ']', '}']
                {
                    let array_name: String = body[2..body.len() - 4].iter().collect();
                    if is_shell_name(&array_name) {
                        push_quoted_positional_literal_segment(
                            &mut segments,
                            &chars[literal_start..index],
                            false,
                        )?;
                        segments.push(QuotedPositionalAtSegment::ArrayAt(array_name, true));

                        saw_positional_at = true;
                        index = end + 1;
                        literal_start = index;
                        continue;
                    }
                }
                // A quoted span whose body embeds `$@`/`${@}` around other
                // text splits into literal/positional segments: affixes attach
                // to the first/last positional word (GNU subst.c expands the
                // quoted span into a word list whose boundary words carry the
                // surrounding quoted text; expand_word_internal 11723-11808).
                if let Some(body_segments) =
                    quoted_body_positional_at_segments(body, env_vars, nameref_target)
                {
                    push_quoted_positional_literal_segment(
                        &mut segments,
                        &chars[literal_start..index],
                        false,
                    )?;
                    segments.extend(body_segments);
                    saw_positional_at = true;
                    index = end + 1;
                    literal_start = index;
                    continue;
                }
                index = end + 1;
                continue;
            }
            '\'' => {
                index = skip_single_quote(&chars, index + 1)?;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'\'') => {
                index = skip_single_quote(&chars, index + 2)?;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'{') => {
                // Skip `${...}` bodies: a `"$@"` inside a parameter expansion
                // default/alternate word (e.g. `${undef-"$@"}`) is part of the
                // expansion, not a top-level quoted positional-at segment.
                // GNU parse.y extracts the braced body as one unit; the `}`
                // closes the expansion and any text after it is separate.
                // Unquoted `${@}` / `${name[@]}` is a word-list source just
                // like the quoted forms: GNU expands `[@]`/`$@` to one word
                // per element even unquoted (subst.c param_expand — the
                // W_DOLLARAT list survives field splitting), with affixes
                // attached to the first/last element word (array6.sub
                // `recho ${foo}$@` under IFS=).
                let body_start = index + 2;
                if let Some(&body_start_byte) = char_to_byte.get(body_start) {
                    if let Some(end_byte) = matching_parameter_brace(&raw[body_start_byte..]) {
                        let close_byte = body_start_byte + end_byte;
                        let body_end = index + 2 + raw[body_start_byte..close_byte].chars().count();
                        let inner = &chars[index + 2..body_end];
                        if inner == ['@'] {
                            push_quoted_positional_literal_segment(
                                &mut segments,
                                &chars[literal_start..index],
                                false,
                            )?;
                            segments.push(QuotedPositionalAtSegment::PositionalAt(false));

                            saw_positional_at = true;
                            index = raw[..=close_byte].chars().count();
                            literal_start = index;
                            continue;
                        }
                        if inner.first() == Some(&'!') {
                            // `${!name}` where name's VALUE is `arr[@]` /
                            // `arr[*]` is the array element list (subst.c
                            // param_expand indirect expansion), e.g.
                            // indir='arr[@]' in nameref18.sub.
                            let ind_name: String = inner[1..].iter().collect();
                            if let Some((base, star)) = indirect_array_target(env_vars, &ind_name) {
                                push_quoted_positional_literal_segment(
                                    &mut segments,
                                    &chars[literal_start..index],
                                    false,
                                )?;
                                segments.push(if star {
                                    QuotedPositionalAtSegment::ArrayStar(base, false)
                                } else {
                                    QuotedPositionalAtSegment::ArrayAt(base, false)
                                });
                                saw_positional_at = true;
                                index = raw[..=close_byte].chars().count();
                                literal_start = index;
                                continue;
                            }
                        }
                        if inner.len() > 3 && inner[inner.len() - 3..] == ['[', '@', ']'] {
                            let array_name: String = inner[..inner.len() - 3].iter().collect();
                            if is_shell_name(&array_name) {
                                push_quoted_positional_literal_segment(
                                    &mut segments,
                                    &chars[literal_start..index],
                                    false,
                                )?;
                                segments
                                    .push(QuotedPositionalAtSegment::ArrayAt(array_name, false));
                                saw_positional_at = true;
                                index = raw[..=close_byte].chars().count();
                                literal_start = index;
                                continue;
                            }
                        }
                        // `@`-slice word-list source (rubash#1011): `inner`
                        // is already brace-aware delimited by
                        // matching_parameter_brace above, so the validator
                        // sees the true span. Unquoted slices field-split
                        // like the unsliced list (same `quoted == false`
                        // contract as `ArrayAt` above).
                        if let Some(target) =
                            at_slice_segment_body(&inner.iter().collect::<String>())
                        {
                            push_quoted_positional_literal_segment(
                                &mut segments,
                                &chars[literal_start..index],
                                false,
                            )?;
                            let inner_body: String = inner.iter().collect();
                            segments.push(match target {
                                AtSliceTarget::Array => {
                                    QuotedPositionalAtSegment::ArrayAtSlice(inner_body, false)
                                }
                                AtSliceTarget::Positional => {
                                    QuotedPositionalAtSegment::PositionalAtSlice(inner_body, false)
                                }
                            });
                            saw_positional_at = true;
                            index = raw[..=close_byte].chars().count();
                            literal_start = index;
                            continue;
                        }
                        // Per-element-operator word-list source
                        // (`pre${arr[@]%.txt}`, rubash#1456): same
                        // chk_atstar anchor as the slice gate above;
                        // unquoted results field-split like the unsliced
                        // list (`quoted == false` contract).
                        let inner_body: String = inner.iter().collect();
                        if at_modified_segment_body(&inner_body) {
                            push_quoted_positional_literal_segment(
                                &mut segments,
                                &chars[literal_start..index],
                                false,
                            )?;
                            segments.push(QuotedPositionalAtSegment::AtModified(inner_body, false));
                            saw_positional_at = true;
                            index = raw[..=close_byte].chars().count();
                            literal_start = index;
                            continue;
                        }
                        index = raw[..=close_byte].chars().count();
                        continue;
                    }
                }
                // Unterminated `${...}`: skip to end.
                index = chars.len();
                continue;
            }
            '$' if chars
                .get(index + 1)
                .is_some_and(|ch| is_shell_name_start(*ch)) =>
            {
                // Unbraced `$name` whose nameref cell is `arr[@]`/`arr[*]`
                // expands to the referenced array's element list
                // (find_variable_nameref; nameref18.sub `recho $ref`).
                let mut end = index + 1;
                while end < chars.len() && is_shell_name_char(chars[end]) {
                    end += 1;
                }
                let name: String = chars[index + 1..end].iter().collect();
                if let Some((base, star)) = nameref_array_target(nameref_target, &name) {
                    push_quoted_positional_literal_segment(
                        &mut segments,
                        &chars[literal_start..index],
                        false,
                    )?;
                    segments.push(if star {
                        QuotedPositionalAtSegment::ArrayStar(base, false)
                    } else {
                        QuotedPositionalAtSegment::ArrayAt(base, false)
                    });
                    saw_positional_at = true;
                    index = end;
                    literal_start = index;
                    continue;
                }
                index += 1;
            }
            '$' if chars.get(index + 1) == Some(&'@') => {
                // Bare unquoted `$@` inside a mixed word: same word-list
                // source as `"$@"`.
                push_quoted_positional_literal_segment(
                    &mut segments,
                    &chars[literal_start..index],
                    false,
                )?;
                segments.push(QuotedPositionalAtSegment::PositionalAt(false));

                saw_positional_at = true;
                index += 2;
                literal_start = index;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                // Command substitution body: its `$@`/`"` contents belong
                // to the inner parse, not this word's segments.
                paren_depth += 1;
                index += 2;
                continue;
            }
            '(' => {
                paren_depth += 1;
                index += 1;
                continue;
            }

            '\\' => {
                index += 2;
                continue;
            }
            _ => index += 1,
        }
    }

    if !saw_positional_at {
        return None;
    }

    push_quoted_positional_literal_segment(&mut segments, &chars[literal_start..], false)?;
    Some(segments)
}

/// Split the body of one double-quoted span around its unescaped `$@` /
/// `${@}` occurrences. Returns None when the body has no positional-at token
/// (the caller leaves the span as ordinary text) or when the body contains
/// anything this narrow path must not guess at: other `${...}` operator
/// forms, backticks, or backslash escapes.
fn quoted_body_positional_at_segments(
    body: &[char],
    env_vars: &std::collections::HashMap<String, String>,
    nameref_target: &dyn Fn(&str) -> Option<String>,
) -> Option<Vec<QuotedPositionalAtSegment>> {
    let mut segments = Vec::new();
    let mut piece_start = 0usize;
    let mut index = 0usize;
    let mut saw_positional_at = false;

    while index < body.len() {
        match body[index] {
            '`' | '\\' => return None,
            '$' if body.get(index + 1) == Some(&'{') => {
                if body.get(index + 2) == Some(&'@') && body.get(index + 3) == Some(&'}') {
                    push_body_piece(&mut segments, &body[piece_start..index])?;
                    segments.push(QuotedPositionalAtSegment::PositionalAt(true));
                    saw_positional_at = true;
                    index += 4;
                    piece_start = index;
                } else if let Some(close) = body[index + 2..].iter().position(|ch| *ch == '}') {
                    // `${name[@]}` word-list source inside a larger quoted
                    // body (GNU subst.c: the `[@]` subscript produces one
                    // word per element inside double quotes).
                    let token = &body[index + 2..index + 2 + close];
                    if token.first() == Some(&'!') {
                        let ind_name: String = token[1..].iter().collect();
                        if let Some((base, star)) = indirect_array_target(env_vars, &ind_name) {
                            push_body_piece(&mut segments, &body[piece_start..index])?;
                            segments.push(if star {
                                QuotedPositionalAtSegment::ArrayStar(base, true)
                            } else {
                                QuotedPositionalAtSegment::ArrayAt(base, true)
                            });
                            saw_positional_at = true;
                            index += 2 + close + 1;
                            piece_start = index;
                            continue;
                        }
                    }
                    if token.len() > 3 && token[token.len() - 3..] == ['[', '@', ']'] {
                        let name: String = token[..token.len() - 3].iter().collect();
                        if is_shell_name(&name) {
                            push_body_piece(&mut segments, &body[piece_start..index])?;
                            segments.push(QuotedPositionalAtSegment::ArrayAt(name, true));

                            saw_positional_at = true;
                            index += 2 + close + 1;
                            piece_start = index;
                            continue;
                        }
                    }
                    // GNU subst.c:11730-11808 (expand_word_internal, the
                    // double-quote case): the whole quoted span is one
                    // recursively-expanded unit in which `$@`/`${@}` is the
                    // word-list split point and every other `${name...}`
                    // parameter is ordinary affix text around it — it stays
                    // part of the current literal piece and the segment
                    // expander expands it. Skip the unit brace-aware so a
                    // `$@` inside an operator word (`${a:-$@}`) belongs to
                    // that expansion, never to the top-level split
                    // (rubash#258 QRK_EMPTPPWRD: `"${v}$@${v}"` is 0 fields
                    // with no positional parameters and one word per
                    // parameter otherwise).
                    let rest: String = body[index + 2..].iter().collect();
                    match super::parameter_ops::matching_parameter_brace(&rest) {
                        Some(end_byte) => {
                            // `@`-slice word-list source inside the quoted
                            // body (`"A|${a[@]:1:2}|B"`, rubash#1011): the
                            // brace-aware true inner keeps the @-list
                            // boundaries (subst.c:9050
                            // parameter_brace_substring), so affixes attach
                            // to the first/last element word. The naive `}`
                            // above may sit inside a nested expansion in the
                            // offset (`${a[@]:${i}}`), hence the re-scan —
                            // the shape gate below runs on the true span.
                            let true_inner = &rest[..end_byte];
                            if let Some(target) = at_slice_segment_body(true_inner) {
                                push_body_piece(&mut segments, &body[piece_start..index])?;
                                segments.push(match target {
                                    AtSliceTarget::Array => {
                                        QuotedPositionalAtSegment::ArrayAtSlice(
                                            true_inner.to_string(),
                                            true,
                                        )
                                    }
                                    AtSliceTarget::Positional => {
                                        QuotedPositionalAtSegment::PositionalAtSlice(
                                            true_inner.to_string(),
                                            true,
                                        )
                                    }
                                });
                                saw_positional_at = true;
                                index += 2 + rest[..=end_byte].chars().count();
                                piece_start = index;
                                continue;
                            }
                            // Per-element-operator word-list source
                            // inside the quoted body
                            // (`"pre${arr[@]%.txt}"`, rubash#1456): same
                            // brace-aware true inner, same chk_atstar
                            // anchor as the slice gate above.
                            if at_modified_segment_body(true_inner) {
                                push_body_piece(&mut segments, &body[piece_start..index])?;
                                segments.push(QuotedPositionalAtSegment::AtModified(
                                    true_inner.to_string(),
                                    true,
                                ));
                                saw_positional_at = true;
                                index += 2 + rest[..=end_byte].chars().count();
                                piece_start = index;
                                continue;
                            }
                            index += 2 + rest[..=end_byte].chars().count();
                        }
                        None => return None,
                    }
                } else {
                    return None;
                }
            }
            '$' if body.get(index + 1) == Some(&'(') => {
                // GNU subst.c: a `$@`/`${@}` inside `$(...)`/`$((...))`
                // belongs to the inner substitution's own expansion, not a
                // top-level positional word-list source — `"A=$(( $@ ))"`
                // expands the arith in place (array17.sub).
                index = crate::lexer::skip_parenthesized_unit_corrected(body, index + 1)
                    .unwrap_or(body.len());
            }
            '$' if body
                .get(index + 1)
                .is_some_and(|ch| is_shell_name_start(*ch)) =>
            {
                // `"$ref"` inside a larger quoted body: a nameref cell of
                // `arr[@]`/`arr[*]` is still a word-list source.
                let mut end = index + 1;
                while end < body.len() && is_shell_name_char(body[end]) {
                    end += 1;
                }
                let name: String = body[index + 1..end].iter().collect();
                if let Some((base, star)) = nameref_array_target(nameref_target, &name) {
                    push_body_piece(&mut segments, &body[piece_start..index])?;
                    segments.push(if star {
                        QuotedPositionalAtSegment::ArrayStar(base, true)
                    } else {
                        QuotedPositionalAtSegment::ArrayAt(base, true)
                    });
                    saw_positional_at = true;
                    index = end;
                    piece_start = index;
                    continue;
                }
                index += 1;
            }
            '$' if body.get(index + 1) == Some(&'@') => {
                push_body_piece(&mut segments, &body[piece_start..index])?;
                segments.push(QuotedPositionalAtSegment::PositionalAt(true));
                saw_positional_at = true;
                index += 2;
                piece_start = index;
            }
            _ => index += 1,
        }
    }

    if !saw_positional_at {
        return None;
    }
    push_body_piece(&mut segments, &body[piece_start..])?;
    Some(segments)
}

fn push_body_piece(segments: &mut Vec<QuotedPositionalAtSegment>, chars: &[char]) -> Option<()> {
    if chars.is_empty() {
        return Some(());
    }
    if chars.contains(&'`') || chars.contains(&'\\') {
        return None;
    }
    let raw = chars.iter().collect::<String>();
    segments.push(QuotedPositionalAtSegment::Literal {
        text: crate::lexer::remove_shell_quotes(&raw),
        quoted: true,
    });
    Some(())
}

fn push_quoted_positional_literal_segment(
    segments: &mut Vec<QuotedPositionalAtSegment>,
    chars: &[char],
    quoted: bool,
) -> Option<()> {
    if chars.is_empty() {
        return Some(());
    }
    let raw = chars.iter().collect::<String>();
    if raw.contains(['`', '\\']) {
        return None;
    }
    segments.push(QuotedPositionalAtSegment::Literal {
        text: crate::lexer::remove_shell_quotes(&raw),
        quoted,
    });
    Some(())
}

/// Mark every IFS character in `text` with the `\x1c` protection carrier so
/// field splitting treats it as data — the `\x1c` carrier stands in for the
/// CTLESC protection GNU gives quoted expansion text (subst.c:12273+ word
/// splitting skips quoted separators). Existing `\x1c` pairs pass through.
pub(in crate::executor) fn protect_ifs_field_chars(text: &str, ifs: Option<&str>) -> String {
    let ifs = ifs.unwrap_or(" \t\n");
    if ifs.is_empty() || !text.contains(|ch| ifs.contains(ch)) {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == crate::executor::markers::IFS_GLUE {
            output.push(ch);
            if let Some(next) = chars.next() {
                output.push(next);
            }
            continue;
        }
        if ifs.contains(ch) {
            output.push(crate::executor::markers::IFS_GLUE);
        }
        output.push(ch);
    }
    output
}

/// Decode `\x1c` protection pairs back to their literal characters for the
/// no-splitting case (IFS explicitly empty).
pub(in crate::executor) fn decode_protected_ifs_chars(text: &str) -> String {
    if !text.contains(crate::executor::markers::IFS_GLUE) {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == crate::executor::markers::IFS_GLUE {
            if let Some(next) = chars.next() {
                output.push(next);
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn expand_quoted_positional_at_segments<F, R>(
    segments: &[QuotedPositionalAtSegment],
    ifs: Option<&str>,

    resolve_list: R,
    expand_literal: F,
) -> Vec<String>
where
    F: Fn(&str) -> String,
    R: Fn(&QuotedPositionalAtSegment) -> Vec<String>,
{
    // Each produced word carries a keep-empty flag: GNU drops empty fields
    // from unquoted expansion but keeps an empty word that a quoted part
    // contributed (had_quoted_null, subst.c:12026-12035).
    let mut words: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut current_present = false;
    let mut current_has_quoted = false;
    let mut saw_positional_at = false;
    let mut saw_non_empty_expansion = false;

    for (segment_index, segment) in segments.iter().enumerate() {
        match segment {
            QuotedPositionalAtSegment::Literal { text, quoted } => {
                // GNU word splitting (subst.c:10649+ word_split) applies to
                // expansion-produced text only — literal characters in the
                // word are never split. Protecting the IFS characters of the
                // RAW literal before expansion keeps e.g. `+"$@"` literal
                // under IFS='+' while expansion-produced separators inside
                // the literal's own `$v` text still split (more-exp
                // `recho + "$@"` / `+"$@"` -> `+`).
                let expanded = if *quoted {
                    expand_literal(text)
                } else {
                    expand_literal(&protect_ifs_field_chars(text, ifs))
                };
                if !expanded.is_empty() {
                    saw_non_empty_expansion = true;
                }
                if *quoted {
                    // Quoted literal affixes are data: their IFS characters
                    // attach verbatim and never field-split (GNU: `"  $@  "`
                    // keeps its spaces on the first/last positional word).
                    current_has_quoted = true;
                    current.push_str(&protect_ifs_field_chars(&expanded, ifs));
                } else if segment_index > 0
                    && matches!(
                        segments.get(segment_index - 1),
                        Some(QuotedPositionalAtSegment::PositionalAt(_))
                            | Some(QuotedPositionalAtSegment::ArrayAt(..))
                            | Some(QuotedPositionalAtSegment::ArrayAtSlice(..))
                            | Some(QuotedPositionalAtSegment::PositionalAtSlice(..))
                            | Some(QuotedPositionalAtSegment::AtModified(..))
                    )
                {
                    // Only an unquoted literal directly after $@ emulates the
                    // GNU unquoted-suffix rule where trailing IFS whitespace
                    // terminates the field instead of joining the next one
                    // (e.g. `"$@"$space`).
                    current.push_str(expanded.trim_end_matches([' ', '\t', '\n']));
                } else {
                    current.push_str(&expanded);
                }
                current_present = true;
            }
            QuotedPositionalAtSegment::PositionalAt(quoted)
            | QuotedPositionalAtSegment::ArrayAt(_, quoted)
            | QuotedPositionalAtSegment::ArrayAtSlice(_, quoted)
            | QuotedPositionalAtSegment::PositionalAtSlice(_, quoted)
            | QuotedPositionalAtSegment::AtModified(_, quoted)
            | QuotedPositionalAtSegment::ArrayStar(_, quoted) => {
                saw_positional_at = true;
                let mut values = resolve_list(segment);

                if values.is_empty() {
                    continue;
                }
                saw_non_empty_expansion = true;
                current_has_quoted |= *quoted;
                if *quoted {
                    for value in &mut values {
                        *value = protect_ifs_field_chars(value, ifs);
                    }
                } else if !ifs.is_some_and(|ifs| ifs.is_empty()) {
                    // GNU string_list_dollar_at (subst.c:2957): an UNQUOTED
                    // $@/${arr[@]} does not splice one word per element —
                    // the elements join with the first IFS character (a
                    // space when IFS is unset) into the word's single
                    // expansion string, and word_split runs over the whole
                    // string, so junction empties count exactly like a
                    // scalar split (rubash#1714: a delimiter-only element
                    // contributes two empty fields, an empty element one).
                    // Splicing per element and splitting each produced
                    // word separately loses precisely those junction
                    // fields (`x${arr[@]}y`, `for x in ${arr[@]}`). A null
                    // IFS keeps the per-element splice below: no joining
                    // and no splitting happen there.
                    let separator = match ifs {
                        Some(ifs) => ifs
                            .chars()
                            .next()
                            .map(|ch| ch.to_string())
                            .unwrap_or_default(),
                        None => " ".to_string(),
                    };
                    current.push_str(&values.join(&separator));
                    current_present = true;
                    continue;
                }

                current.push_str(&values[0]);
                current_present = true;

                if values.len() > 1 {
                    let keep = current_has_quoted;
                    words.push((std::mem::take(&mut current), keep));
                    current_has_quoted = *quoted;
                    for value in &values[1..values.len() - 1] {
                        words.push((value.clone(), *quoted));
                    }
                    current.push_str(&values[values.len() - 1]);
                }
            }
        }
    }

    if current_present {
        // GNU expand_word_internal (subst.c 12026-12052): a word containing a
        // quoted $@ whose every part expands empty produces NO word at all
        // (quoted_dollar_at beats the quoted-null retention). Only when some
        // part actually expanded (or positional params exist) is the word kept.
        if saw_positional_at && !saw_non_empty_expansion {
            return Vec::new();
        }
        words.push((current, current_has_quoted));
    }

    // GNU subst.c word splitting: the unquoted portions of each produced
    // word field-split on IFS; \x1c-marked (quoted-sourced) IFS characters
    // stay data. `$@$@` splits `def ghi` while `$@"$@"` keeps `ca b`
    // (exp9.sub).
    words
        .into_iter()
        .flat_map(|(word, keep_empty)| {
            if word.is_empty() {
                return if keep_empty { vec![word] } else { Vec::new() };
            }
            if ifs.is_some_and(|ifs| ifs.is_empty()) {
                return vec![decode_protected_ifs_chars(&word)];
            }
            field_split_values_with_ifs(&word, ifs)
        })
        .collect()
}

fn skip_double_quote(chars: &[char], mut index: usize) -> Option<usize> {
    while index < chars.len() {
        match chars[index] {
            '"' => return Some(index),
            '\\' => index += 2,
            '$' if chars.get(index + 1) == Some(&'{') => {
                // GNU parse_matched_pair (parse.y:3877): a `${...}` inside
                // double quotes nests its own quoting — a `"` inside the
                // expansion word does not close the outer span
                // (`"${1-"$@"}"`).
                let sub: String = chars[index + 2..].iter().collect();
                match matching_parameter_brace(&sub) {
                    Some(end_byte) => {
                        index += 2 + sub[..end_byte].chars().count() + 1;
                    }
                    None => return None,
                }
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                // `$(...)`/`$((...))` nests its own quoting as well: a `"`
                // inside the substitution body does not close the outer
                // span (parse.y parse_comsub).
                index = crate::lexer::skip_parenthesized_unit_corrected(chars, index + 1)
                    .unwrap_or(chars.len());
            }
            _ => index += 1,
        }
    }
    None
}

fn skip_single_quote(chars: &[char], mut index: usize) -> Option<usize> {
    while index < chars.len() {
        if chars[index] == '\'' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}
