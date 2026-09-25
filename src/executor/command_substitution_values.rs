use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

impl Executor {
    pub(in crate::executor) fn command_substitution_pipeline_filter(
        &self,
        words: &[String],
        input: &str,
    ) -> Option<(String, i32)> {
        match words.first().map(String::as_str)? {
            "sort" => {
                // Only bare `sort` and `sort -u` are represented inline; any
                // other flag (`-r`, `-n`, ...) or a file operand must run the
                // real sort instead of being silently ignored.
                let mut unique = false;
                for word in &words[1..] {
                    match self.expand_word(word).as_str() {
                        "-u" => unique = true,
                        _ => return None,
                    }
                }
                let mut lines = input.lines().map(str::to_string).collect::<Vec<_>>();
                lines.sort();
                if unique {
                    lines.dedup();
                }
                let mut output = lines.join("\n");
                if !output.is_empty() {
                    output.push('\n');
                }
                Some((output, 0))
            }
            "sed" => {
                let args = words[1..]
                    .iter()
                    .map(|word| {
                        self.expand_word(word)
                            .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
                            .replace(DATA_DOLLAR, "$")
                            .replace(crate::executor::markers::CTLESC, "")
                    })
                    .collect::<Vec<_>>();
                apply_simple_sed_args(input, &args).map(|output| (output, 0))
            }
            "tr" => {
                let args = words[1..]
                    .iter()
                    .map(|word| {
                        self.expand_word(word)
                            .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
                            .replace(DATA_DOLLAR, "$")
                    })
                    .collect::<Vec<_>>();
                if args.len() != 2 {
                    return None;
                }
                if matches!(args[0].as_str(), "\\n" | "\n") {
                    Some((input.replace('\n', &args[1]), 0))
                } else if crate::executor::pipeline_exec::inline_expand_tr_set(&args[0]).is_some()
                    && crate::executor::pipeline_exec::inline_expand_tr_set(&args[1]).is_some()
                {
                    Some((
                        crate::executor::pipeline_exec::translate_tr(input, &args[0], &args[1]),
                        0,
                    ))
                } else {
                    // Syntax the inline fast path cannot represent must run
                    // the real external `tr`.
                    None
                }
            }
            "head" => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                // Byte mode has no line-count representation; the real head
                // must run instead of silently emitting whole lines.
                if args.iter().any(|a| a == "-c" || a == "--bytes") {
                    return None;
                }
                let count = crate::executor::pipeline_exec::head_line_count(&args).unwrap_or(10);
                Some((input.split_inclusive('\n').take(count).collect(), 0))
            }
            "grep" => {
                // The inline fast path only represents the exact two-word
                // form `grep PATTERN`. Flags (`-c`, `-n`, ...), `--`, and
                // file operands change semantics this arm cannot model, and
                // treating them as the pattern produced silently empty or
                // wrong captures; fall back to the real parser, which runs
                // the actual grep (its stderr is not swallowed there).
                if words.len() != 2 {
                    return None;
                }
                let raw_pattern = words[1].as_str();
                if raw_pattern.starts_with('-') && raw_pattern.len() > 1 {
                    return None;
                }
                let pattern = self.expand_word(raw_pattern);
                let mut output = String::new();
                let mut matched = false;
                for line in input.split_inclusive('\n') {
                    let comparable = line.strip_suffix('\n').unwrap_or(line);
                    if crate::executor::simple_grep_pattern_matches(comparable, &pattern) {
                        matched = true;
                        output.push_str(line);
                        if !line.ends_with('\n') {
                            output.push('\n');
                        }
                    }
                }
                Some((output, i32::from(!matched)))
            }
            "wc" => {
                let option = words.get(1).map(String::as_str).unwrap_or("-l");
                let value = match option {
                    "-c" => input.as_bytes().len(),
                    "-l" => input.bytes().filter(|byte| *byte == b'\n').count(),
                    _ => return None,
                };
                Some((format!("{value}\n"), 0))
            }
            "tail" => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                // Byte mode has no line-count representation; the real tail
                // must run instead of silently emitting whole lines.
                if args.iter().any(|a| a == "-c" || a == "--bytes") {
                    return None;
                }
                let count = crate::executor::pipeline_exec::head_line_count(&args).unwrap_or(10);
                let lines = input.split_inclusive('\n').collect::<Vec<_>>();
                let start = lines.len().saturating_sub(count);
                Some((lines[start..].concat(), 0))
            }
            "uniq" => {
                // No uniq flags (`-d`, `-u`, `-c`, ...) or operands are
                // represented inline; anything beyond bare `uniq` must run
                // the real uniq rather than silently ignoring the flag.
                if words.len() != 1 {
                    return None;
                }
                let mut output = String::new();
                let mut previous = None;
                for line in input.split_inclusive('\n') {
                    let comparable = line.strip_suffix('\n').unwrap_or(line);
                    if previous != Some(comparable) {
                        output.push_str(line);
                    }
                    previous = Some(comparable);
                }
                Some((output, 0))
            }
            _ => {
                let cmd_name = self.expand_word(&words[0]);
                let expanded_args: Vec<String> =
                    words[1..].iter().map(|w| self.expand_word(w)).collect();
                use std::io::Write;
                use std::process::Stdio;
                let program = find_user_command(&cmd_name, &self.shell_state.env_vars)?;
                let (dev_args, dev_ops) = self.materialize_dev_fd_operands(
                    &expanded_args,
                    crate::executor::dev_fd_operands::DevOperandStdin::Payload(
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(input),
                    ),
                    crate::executor::dev_fd_operands::DevOperandStdout::Capture,
                );
                let (mut process, _) = external_command_for_named_program(
                    &program,
                    Some(&cmd_name),
                    &dev_args,
                    &self.shell_state.env_vars,
                );
                self.apply_child_environment(&mut process);
                let mut child = process
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()?;
                child
                    .stdin
                    .as_mut()?
                    .write_all(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(input),
                    )
                    .ok()?;
                let mut output = child.wait_with_output().ok()?;
                output.stdout.extend(self.collect_dev_fd_capture(&dev_ops));
                Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout)
                        .trim_capture_terminator()
                        .to_string(),
                    output.status.code().unwrap_or(1),
                ))
            }
        }
    }

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
        match glob::pathname_expand_word(word, &self.shell_state.env_vars) {
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
        let mut status = 0;
        for name in &words[1 + first_name..] {
            let name = self.expand_word(name);
            match self.describe_name_with_io(&name, mode, use_standard_path, false, &mut stdout) {
                Ok(true) => {}
                Ok(false) => status = 1,
                Err(_) => status = 1,
            }
        }
        self.last_command_substitution_status.set(Some(status));
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
            return Some(self.shell_state.positional_params.clone());
        }
        if word == "$@" && kind.map_or(true, |kind| *kind == TokenKind::Word) {
            return Some(self.shell_state.positional_params.clone());
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
                        return Some(self.shell_state.positional_params.clone());
                    }
                    if indirect == "*" {
                        return Some(vec![self
                            .shell_state
                            .positional_params
                            .join(&self.ifs_first_char_separator())]);
                    }
                    if is_shell_name(indirect) {
                        if let Some(target) =
                            self.shell_state.env_vars.get(indirect).map(String::as_str)
                        {
                            if target == "@" {
                                return Some(self.shell_state.positional_params.clone());
                            }
                            if target == "*" {
                                return Some(vec![self
                                    .shell_state
                                    .positional_params
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
                            return Some(self.shell_state.positional_params.clone());
                        }
                        return Some(vec![self
                            .shell_state
                            .positional_params
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
                        &self.shell_state.positional_params,
                        &self.script_name_value(),
                        offset,
                        length,
                    ));
                }
                if var_name == "*" {
                    let values = positional_parameter_substring_with_zero(
                        &self.shell_state.positional_params,
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
            let mut values = vec!["set".to_string(), "--".to_string()];
            values.extend(
                self.shell_state
                    .positional_params
                    .iter()
                    .map(|value| shell_single_quote_assignment_value(value)),
            );
            values
        } else {
            self.shell_state
                .positional_params
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
            .positional_params
            .iter()
            .map(|value| modify(value))
            .collect::<Vec<_>>();
        if quoted && name == "*" {
            return Some(vec![values.join(&self.ifs_first_char_separator())]);
        }
        Some(values)
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
                            self.shell_state.positional_params.clone()
                        }
                        QuotedPositionalAtSegment::ArrayAt(name, _) => self
                            .array_subscript_range_values(name, 0, None)
                            .unwrap_or_default(),
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
        let array_name = expression
            .strip_suffix("[@]")
            .or_else(|| expression.strip_suffix("[*]"))
            .unwrap_or_default();
        let ordered = if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, array_name) {
            assoc_hash_ordered_values(
                value,
                assoc_nbuckets(&self.shell_state.env_vars, array_name),
            )
        } else {
            array_values(value)
        };
        let values = ordered
            .into_iter()
            .map(normalize_array_expanded_value)
            .collect::<Vec<_>>();
        self.join_expanded_array_values(values, expression)
    }

    pub(in crate::executor) fn join_expanded_array_values(
        &self,
        values: Vec<String>,
        expression: &str,
    ) -> String {
        if expression.ends_with("[*]") {
            // GNU dollar_star (subst.c string_list_dollar_star): IFS[0], the
            // empty string when IFS is set empty, a space when IFS is unset.
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
            self.diagnostic_prefix_for_line(warning_line)
        );
    }

    pub(in crate::executor) fn run_external_command_substitution(
        &self,
        words: &[String],
    ) -> Option<String> {
        words.first()?;
        let stdio = self.command_substitution_words_and_stdio(words)?;
        if matches!(
            stdio.expanded_words.first().map(String::as_str),
            Some("tty") | Some("/bin/tty") | Some("/usr/bin/tty")
        ) {
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
        // not the shell's "fc" builtin. Also respect "enable -n".
        let first_word = stdio
            .expanded_words
            .first()
            .map(String::as_str)
            .unwrap_or("");
        if is_shell_builtin_name(first_word)
            && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, first_word)
        {
            return None;
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
        let (mut process, _) = external_command_for_named_program(
            &program,
            Some(&stdio.expanded_words[0]),
            &dev_args,
            &self.shell_state.env_vars,
        );

        self.apply_child_environment(&mut process);
        // GNU subst.c:7143 command_substitute forks sharing the parent's
        // fd 0: feed the child the unread tail of FUNCTION_STDIN and, after
        // it runs, drain the caller's cursor to EOF.
        let mut piped_stdin: Option<Vec<u8>> = None;
        if let Some(stdin_path) = stdio.stdin_path {
            let file = File::open(stdin_path).ok()?;
            process.stdin(Stdio::from(file));
        } else if let Some(input) = self.function_stdin_remaining() {
            process.stdin(Stdio::piped());
            piped_stdin =
                Some(crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input));
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
        } else {
            process.stderr(Stdio::piped());
        }
        let mut spawned = process.spawn().ok()?;
        if let Some(input) = piped_stdin.as_deref() {
            if let Some(mut child_stdin) = spawned.stdin.take() {
                use std::io::Write;
                let _ = child_stdin.write_all(input);
            }
        }
        let mut output = spawned.wait_with_output().ok()?;
        output.stdout.extend(self.collect_dev_fd_capture(&dev_ops));
        if piped_stdin.is_some() {
            if let Some(text) = self.shell_state.env_vars.get(FUNCTION_STDIN) {
                self.comsub_stdin_writeback
                    .set(Some((text.len(), Self::function_stdin_fingerprint(text))));
            }
        }
        let status = output.status.code().unwrap_or(1);
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
        Some(shell_path_to_windows(&expanded, &self.shell_state.env_vars))
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
        Some(self.expand_command_substitution_mut_typed_with_context(
            &source,
            if quoted {
                SubstitutionQuoteContext::DoubleQuoted
            } else {
                SubstitutionQuoteContext::Unquoted
            },
        ))
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
        Some(
            self.expand_command_substitution_readback_with_context(
                &source,
                SubstitutionQuoteContext::Unquoted,
            )
            .text_lossy(),
        )
    }

    pub(in crate::executor) fn expand_dirstack_tilde(&self, word: &str) -> Option<String> {
        // TODO(subst.c/builtins/pushd.def): Bash performs directory-stack
        // tilde expansion during word expansion. This implements ~N and ~-N
        // for upstream dstack2.tests.
        let rest = word.strip_prefix('~')?;
        if rest.is_empty() || rest.starts_with('/') {
            return None;
        }

        let (from_right, digits) = if let Some(digits) = rest.strip_prefix('-') {
            (true, digits)
        } else {
            (false, rest)
        };
        if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }

        let value = digits.parse::<usize>().ok()?;
        let stack = crate::builtins::pushd::load_stack(&self.shell_state.env_vars);
        let index = if from_right {
            if value < stack.len() {
                stack.len() - 1 - value
            } else {
                return Some(word.to_string());
            }
        } else {
            value
        };
        stack.get(index).cloned().or_else(|| Some(word.to_string()))
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
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    if redirect.append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    options.open(&redirect.path)
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
                    return None;
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
fn decode_protected_ifs_chars(text: &str) -> String {
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
