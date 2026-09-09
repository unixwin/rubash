use super::*;

#[derive(Debug, Eq, PartialEq)]
pub(in crate::executor) struct AssignmentExpansionResult {
    pub(in crate::executor) value: String,
    pub(in crate::executor) substitution_status: Option<i32>,
    pub(in crate::executor) arithmetic_error: bool,
    pub(in crate::executor) arithmetic_nonfatal_error: bool,
}

impl Executor {
    pub(in crate::executor) fn expand_assignment_value_result(
        &mut self,
        value: &str,
    ) -> AssignmentExpansionResult {
        self.last_command_substitution_status.set(None);
        let expanded = self.expand_assignment_value(value);
        let substitution_status = self.last_command_substitution_status.get();
        self.last_command_substitution_status.set(None);
        let arithmetic_error = self.arithmetic_expansion_error.replace(false);
        let arithmetic_nonfatal_error = self.arithmetic_nonfatal_error.replace(false);
        AssignmentExpansionResult {
            value: expanded,
            substitution_status,
            arithmetic_error,
            arithmetic_nonfatal_error,
        }
    }

    /// Raw double quotes surviving in a token value are single-quote DATA at
    /// this point: remove_shell_quotes already consumed the active ones, so a
    /// remaining `"` can only come from a single-quoted segment (GNU keeps it
    /// literal in the assigned value). The downstream embedded-parameter
    /// re-scan would re-process it as syntax, so carry those quotes with the
    /// internal DATA_DOUBLE_QUOTE marker across expansion and restore them on
    /// the way out (assignment_expansion hoist/restore contract).
    pub(in crate::executor) fn expand_assignment_value(&mut self, value: &str) -> String {
        // Only hoist when no command-substitution payload is present: quotes
        // inside a $()/backtick body are syntax for the nested parse, not data.
        if !value.contains('"')
            || value.contains('`')
            || value.contains("$(")
            || contains_command_substitution_payload(value)
        {
            return self.expand_assignment_value_inner(value);
        }
        const DQ_DATA: &str = "\u{E001}";
        let expanded = self.expand_assignment_value_inner(&value.replace('"', DQ_DATA));
        expanded.replace(DQ_DATA, "\"")
    }

    /// GNU subst.c:4357 expand_string_assignment (reached with
    /// W_ASSIGNMENT from subst.c:11432): each unquoted element value of a
    /// compound assignment undergoes the assignment tilde pass (leading
    /// `~` and `~` after `:`). Quoted elements stay literal, and tilde
    /// text introduced by parameter expansion is never re-expanded because
    /// this pass sees the raw element text (array.tests: aa=([0]=~/a:~/b)
    /// stores the expanded paths while bb=([0]="~/a:~/b") stays literal).
    pub(in crate::executor) fn expand_tilde_in_compound_assignment(&self, value: &str) -> String {
        let Some(inner) = value
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
        else {
            return value.to_string();
        };

        let mut elements: Vec<String> = Vec::new();
        for token in split_compound_element_words(inner) {
            elements.push(self.expand_compound_element_tilde(&token));
        }
        format!("({})", elements.join(" "))
    }

    fn expand_compound_element_tilde(&self, token: &str) -> String {
        const DQ_DATA: &str = "\u{E001}";
        let (prefix, element) = if token.starts_with('[') {
            match token.find("]=") {
                Some(offset) => (&token[..offset + 2], &token[offset + 2..]),
                None => ("", token),
            }
        } else {
            ("", token)
        };
        if element.starts_with('\'')
            || element.starts_with('"')
            || element.starts_with(DQ_DATA)
        {
            return token.to_string();
        }
        if !tilde_expand::assignment_value_needs_tilde_expansion(element, true) {
            return token.to_string();
        }
        format!(
            "{prefix}{}",
            tilde_expand::expand_assignment_tilde_value(element, &self.home_value(), true)
        )
    }

    fn expand_assignment_value_inner(&mut self, value: &str) -> String {
        // The verbatim single-element fast path is only for storage-shaped
        // values without expansions: a compound value containing a
        // parameter expansion (e.g. (${!xx})) must reach the compound
        // expander below or the expansion text lands in the array as a
        // literal element (new-exp4.sub Case05).
        if !value.contains("$(") && !value.contains('`') && !value.contains('$') {
            if let Some(array_value) = normalize_single_element_array_assignment(value) {
                return array_value;
            }
        }

        let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
        let value = tilde_expand::strip_assignment_quote_marker(value);
        let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
        let value = value
            .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
            .unwrap_or(value);
        // GNU subst.c:4357 expand_string_assignment (W_ASSIGNMENT,
        // subst.c:11432): unquoted element values of a compound assignment
        // undergo the assignment tilde pass on the RAW element text, before
        // parameter expansion, so tilde text produced by $params is never
        // re-expanded (array.tests: aa=([0]=~/a:~/b) expands both segments
        // while w=([0]=~/a [1]=$p) keeps $p's result literal). Quoted
        // elements stay literal; quoted whole-RHS values skip the pass.
        let tilde_value = if compound_assignment
            && !quoted
            && value.starts_with('(')
            && value.ends_with(')')
        {
            std::borrow::Cow::Owned(self.expand_tilde_in_compound_assignment(value))
        } else {
            std::borrow::Cow::Borrowed(value)
        };
        let value: &str = &tilde_value;
        if value.contains("\\$(") {
            let literal = if quoted {
                strip_matching_quotes(value)
            } else {
                value
            };
            return unescape_remaining_shell_escapes(literal);
        }
        if quoted && value.contains(":$((") {
            return self.expand_quoted_prompt_arithmetic_assignment(value);
        }
        let value = if quoted && (value.contains("$(") || value.contains('`')) {
            strip_matching_quotes(value)
        } else {
            value
        };
        if quoted {
            if let Some(expanded) = self.expand_quoted_array_assignment_value(value) {
                return expanded;
            }
        }
        if compound_assignment
            && value.starts_with('(')
            && value.ends_with(')')
            && !value.contains('$')
            && !value.contains('`')
        {
            return format!("{COMPOUND_ASSIGNMENT_MARKER}{value}");
        }
        if !quoted && !compound_assignment {
            if let Some(expanded) = self.expand_fast_assignment_value(value) {
                return expanded;
            }
        }
        self.apply_parameter_assignment_expansions_in_word(value);
        if let Some(expanded) = self.expand_compound_positional_at_assignment(value, quoted) {
            if compound_assignment {
                return format!("{COMPOUND_ASSIGNMENT_MARKER}{expanded}");
            }
            return expanded;
        }
        if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(value) {
            if compound_assignment {
                return format!("{COMPOUND_ASSIGNMENT_MARKER}{expanded}");
            }
            return expanded;
        }

        if !compound_assignment && !value.starts_with("$((") && !value.starts_with("$[") {
            if let Some(source) = value
                .strip_prefix("$(")
                .and_then(|rest| rest.strip_suffix(')'))
                // GNU expands each $() span in the RHS separately ("$(a)$(b)"
                // concatenates two substitution outputs, subst.c string
                // extraction never spans across substitutions). Keep the
                // single-substitution fast path only for words that are
                // exactly one $() group; multi-span values fall through to
                // expand_mixed_command_substitution_assignment (issue
                // niubash#71).
                .filter(|_| command_substitution_spans_whole_word(value))
            {
                let result = self.expand_command_substitution_mut_typed_with_context(
                    source,
                    if quoted {
                        SubstitutionQuoteContext::DoubleQuoted
                    } else {
                        SubstitutionQuoteContext::Unquoted
                    },
                );
                return result.assignment_text();
            }
        }

        if !compound_assignment {
            if let Some(output) = self.expand_backtick_substitution_typed(value, quoted) {
                return output.assignment_text();
            }
            if let Some(separator) = value.find('=') {
                let (prefix, rhs) = value.split_at(separator);
                if is_shell_name(prefix) {
                    if let Some(output) = self.expand_backtick_substitution_typed(&rhs[1..], quoted)
                    {
                        return format!("{prefix}={}", output.assignment_text());
                    }
                }
            }
            if let Some(expanded) = self.expand_mixed_command_substitution_assignment(value) {
                return expanded;
            }
        }

        if let Some(expanded) = self.expand_backtick_substitution(value) {
            return expanded;
        }

        let expanded_value = self.expand_embedded_parameters_mut(value);
        let expanded = if quoted {
            // Prompt transforms consume Bash's `\!` and `\#` escapes after
            // parameter expansion. Keep those two quoted backslashes until
            // `${var@P}` reaches prompt_expansion; ordinary shell escapes
            // still undergo the normal assignment quote-removal pass.
            {
                let mut restored = preserve_prompt_escapes(&expanded_value).replace('\x11', "");
                if value.contains(['\x16', '\x17', '\x18']) {
                    restored = restored
                        .replace('\x16', "'")
                        .replace('\x17', "'")
                        .replace('\x18', "\"")
                        .replace("\\'", "'");
                }
                restored
            }
        } else {
            // GNU strips quote syntax that parameter expansion introduced into
            // an unquoted assignment RHS (`v=${IFS+'}'z}` stores `}z`). Quotes
            // inside protected substitution payloads are data, so leave those
            // values alone. Escaped-quote markers (\x17 from \' and \x18 from
            // \" in the source word) are DATA quotes: parse.y records a
            // backslash-escaped quote as a quoted literal that survives quote
            // removal into the stored value (`x=a\'b` stores `a'b`). Hoist the
            // markers out of the quote-removal pass so the data quotes they
            // become are not re-stripped as syntax, then restore them.
            const DATA_SINGLE_QUOTE: &str = "\u{E000}";
            const DATA_DOUBLE_QUOTE: &str = "\u{E001}";
            let hoisted_value = value
                .replace('\x17', DATA_SINGLE_QUOTE)
                .replace('\x18', DATA_DOUBLE_QUOTE);
            let expanded_value = self.expand_embedded_parameters_mut(&hoisted_value);
            let stripped = if expanded_value.contains(['\'', '"'])
                && !contains_command_substitution_payload(&expanded_value)
            {
                crate::lexer::remove_shell_quotes(&expanded_value)
            } else {
                expanded_value.clone()
            };
            unescape_remaining_shell_escapes(&stripped)
                .replace(DATA_SINGLE_QUOTE, "'")
                .replace(DATA_DOUBLE_QUOTE, "\"")
        };
        let mut expanded = decode_command_substitution_payload(&expanded);
        if expanded.contains("<(") || expanded.contains(">(") {
            if let Ok(materialized) = self.materialize_assignment_process_substitutions(&expanded) {
                expanded = materialized;
            }
        }
        if value.starts_with('(') && value.ends_with(')') {
            if compound_assignment {
                return format!("{COMPOUND_ASSIGNMENT_MARKER}{expanded}");
            }
            return expanded;
        }
        if value.contains('=') {
            return expanded;
        }

        if quoted {
            return expanded;
        }

        // TODO(subst.c/variables.c): Bash's assignment-word expansion has a
        // special tilde pass on RHS prefixes and selected colon-separated
        // path positions. Keep it centralized here until Rubash ports the
        // `expand_string_assignment`/SHELL_VAR path more directly.
        self.expand_assignment_tilde(&expanded)
    }

    fn expand_mixed_command_substitution_assignment(&mut self, value: &str) -> Option<String> {
        // GNU parse.y:4096-4125 rewrites $"..." into an ordinary
        // double-quoted string at parse time (locale_expand is the identity
        // without a translation catalog), so every later expansion pass sees
        // plain "..." (subst.c:1626-1649 re-decodes at quote removal). The
        // mixed-substitution splitter runs on raw word text where the $"
        // prefix would otherwise survive as a literal dollar plus a quoted
        // span (y=$"A$(echo B)C" stored `$"ABC` instead of `ABC`).
        let value = normalize_dollar_double_quotes(value);
        let spans = scan_substitution_spans(&value);
        if spans.is_empty() {
            return None;
        }
        let mut word = ExpandedWord::default();
        let mut cursor = 0usize;
        for span in spans {
            let raw = value.get(span.start..span.end)?;
            let prefix = self.expand_embedded_parameters_mut(value.get(cursor..span.start)?);
            word.append_literal(&prefix, true);
            let output = if let Some(source) = raw
                .strip_prefix("$(")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                self.expand_command_substitution_mut_typed_with_context(source, span.context)
            } else if raw.starts_with('`') {
                self.expand_backtick_substitution_typed(
                    raw,
                    matches!(span.context, SubstitutionQuoteContext::DoubleQuoted),
                )?
            } else {
                return None;
            };
            word.append_substitution(output);
            cursor = span.end;
        }
        let suffix = self.expand_embedded_parameters_mut(value.get(cursor..)?);
        word.append_literal(&suffix, true);
        self.last_command_substitution_status.set(word.status);
        Some(word.materialize_lossy_at_boundary())
    }

    fn expand_fast_assignment_value(&mut self, value: &str) -> Option<String> {
        if let Some(expression) = value
            .strip_prefix("$((")
            .and_then(|rest| rest.strip_suffix("))"))
            .filter(|expression| !expression.contains("${"))
        {
            let Some(value) = self.eval_arithmetic_command_value(expression) else {
                // GNU expr.c raises evalerror from the actual evaluation, so
                // the recorded real-environment category decides fatality.
                // A fresh-environment re-evaluation would lose state-dependent
                // errors like `x+=2` on a declared integer, and a `set -u`
                // unbound variable must stay fatal even though a fresh
                // environment would happily evaluate it as 0.
                let actual_fatal = self.arithmetic_last_error_category.take().is_some()
                    || self.arithmetic_nounset_error.get();
                if !actual_fatal
                    && !crate::executor::arithmetic::arithmetic_expansion_is_fatal(expression)
                {
                    self.arithmetic_nonfatal_error.set(true);
                }
                if self.arithmetic_nounset_error.get() {
                    // `set -u` unbound is script-fatal (command_prepare turns
                    // the recorded flag into ExitCode). Returning an empty
                    // value here stops the slower assignment expanders from
                    // re-processing the `$(( ))` text as a command
                    // substitution, which produced a spurious
                    // `b: command not found` (issue #67).
                    return Some(String::new());
                }
                return None;
            };
            return Some(self.expand_assignment_tilde_if_needed(value.to_string()));
        }

        let parameter = value.strip_prefix('$')?;
        if parameter.len() != 1 {
            return None;
        }

        let expanded = match parameter.as_bytes()[0] {
            b'0' => self.script_name_value(),
            b'1'..=b'9' => {
                let index = usize::from(parameter.as_bytes()[0] - b'0' - 1);
                self.positional_params
                    .get(index)
                    .cloned()
                    .unwrap_or_default()
            }
            // Assignment RHS joins $* / $@ with the first IFS character
            // (GNU subst.c string_list_dollar_star / string_list_dollar_at
            // under W_ASSIGNRHS; expand_no_split_dollar_star, Posix interp
            // 888). IFS unset joins with space, IFS empty joins with
            // nothing.
            b'@' | b'*' => self.positional_params.join(&self.ifs_first_char_separator()),
            b'#' => self.positional_params.len().to_string(),
            b'?' => self.exit_code.to_string(),
            b'$' => self.shell_pid_value().to_string(),
            b'!' => self.last_background_pid_value(),
            b'-' => self.shell_option_flags(),
            _ => return None,
        };
        Some(self.expand_assignment_tilde_if_needed(expanded))
    }

    fn expand_assignment_tilde_if_needed(&self, value: String) -> String {
        if value.contains('=')
            || !tilde_expand::assignment_value_needs_tilde_expansion(&value, true)
            || (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) == Some("1")
                && !value.starts_with("~/"))
        {
            return value;
        }

        self.expand_assignment_tilde(&value)
    }

    fn expand_quoted_prompt_arithmetic_assignment(&mut self, value: &str) -> String {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum QuoteMode {
            None,
            Single,
            Double,
        }

        let mut output = String::with_capacity(value.len());
        let mut segment = String::new();
        let mut mode = QuoteMode::None;

        for ch in value.chars() {
            match (mode, ch) {
                (QuoteMode::None, '\'') => {
                    output.push_str(&self.expand_embedded_parameters_mut(&segment));
                    segment.clear();
                    mode = QuoteMode::Single;
                }
                (QuoteMode::None, '"') => {
                    output.push_str(&self.expand_embedded_parameters_mut(&segment));
                    segment.clear();
                    mode = QuoteMode::Double;
                }
                (QuoteMode::Single, '\'') => {
                    output.push_str(&segment);
                    segment.clear();
                    mode = QuoteMode::None;
                }
                (QuoteMode::Double, '"') => {
                    output.push_str(&self.expand_embedded_parameters_mut(&segment));
                    segment.clear();
                    mode = QuoteMode::None;
                }
                _ => segment.push(ch),
            }
        }

        if mode == QuoteMode::Single {
            output.push_str(&segment);
        } else {
            output.push_str(&self.expand_embedded_parameters_mut(&segment));
        }

        preserve_prompt_escapes(&output)
    }

    pub(in crate::executor) fn expand_compound_positional_at_assignment(
        &self,
        value: &str,
        quoted: bool,
    ) -> Option<String> {
        let inner = value.strip_prefix('(')?.strip_suffix(')')?;
        let mut changed = false;
        let mut values = Vec::new();
        for token in split_storage_words(inner) {
            let token = unquote_storage_value(&token);
            if token.strip_prefix('\x1d') == Some("${@}") || token == "$@" {
                changed = true;
                values.extend(
                    self.positional_params
                        .iter()
                        .map(|value| quote_array_value(value)),
                );
            } else if let Some(array_name) = token
                .strip_prefix('\x1d')
                .and_then(|token| token.strip_prefix("${"))
                .and_then(|token| token.strip_suffix("[@]}"))
            {
                if let Some(storage) = self.parameter_array_storage(array_name) {
                    changed = true;
                    values.extend(
                        array_values(&storage)
                            .iter()
                            .map(|value| quote_array_value(value)),
                    );
                } else {
                    values.push(quote_array_value(""));
                }
            } else if let Some(indirect_name) = token
                .strip_prefix('\x1d')
                .and_then(|token| token.strip_prefix("${"))
                .and_then(|token| token.strip_suffix('}'))
                .and_then(|name| name.strip_prefix('!'))
            {
                // GNU compound assignment of quoted "${!ref}": a direct
                // array reference in the braced name is the KEYS expansion
                // (arrayfunc.c array_keys), anything else is indirection
                // through the target value (new-exp9.sub / new-exp4.sub
                // Case06-08).
                match self.indirect_compound_assignment_values(indirect_name, true) {
                    Some(mut expanded) => {
                        changed = true;
                        values.append(&mut expanded);
                    }
                    None => values.push(quote_array_value(&token)),
                }
            } else if let Some(indirect_name) = token
                .strip_prefix("${")
                .and_then(|token| token.strip_suffix('}'))
                .and_then(|name| name.strip_prefix('!'))
            {
                // The lexer strips the token's quotes and marks the whole
                // quoted-RHS value, so the value-level flag decides between
                // the quoted (joined) and unquoted (field split) semantics
                // (new-exp4.sub Case05-08).
                match self.indirect_compound_assignment_values(indirect_name, quoted) {
                    Some(mut expanded) => {
                        changed = true;
                        values.append(&mut expanded);
                    }
                    None => values.push(quote_array_value(&token)),
                }
            } else if let Some(name) = token
                .strip_prefix('\x1d')
                .and_then(|token| token.strip_prefix("${"))
                .and_then(|token| token.strip_suffix('}'))
            {
                if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
                    if var_name == "@" {
                        changed = true;
                        values.extend(
                            positional_parameter_substring(&self.positional_params, offset, length)
                                .iter()
                                .map(|value| quote_array_value(value)),
                        );
                        continue;
                    }
                    if let Some(array_name) = var_name
                        .strip_suffix("[@]")
                        .or_else(|| var_name.strip_suffix("[*]"))
                    {
                        if let Some(storage) = self.parameter_array_storage(array_name) {
                            changed = true;
                            values.extend(
                                array_parameter_slice(
                                    &storage,
                                    offset,
                                    length.and_then(|length| usize::try_from(length).ok()),
                                )
                                .iter()
                                .map(|value| quote_array_value(value)),
                            );
                            continue;
                        }
                    }
                }
                values.push(quote_array_value(&token));
            } else {
                values.push(quote_array_value(&token));
            }
        }
        changed.then(|| format!("({})", values.join(" ")))
    }

    /// Compound-assignment element values for a `"${!ref}"` token (GNU
    /// parameter_brace_expand_indir plus the array-assignment element
    /// splitting): a direct array reference in the braced name expands as
    /// KEYS (arrayfunc.c array_keys), any other value is indirection
    /// through the target parameter -- `@`/`*` targets keep their
    /// $@/$* word semantics, `[@]`/`[*]` targets expand the array values,
    /// and scalars read their value cell (field split when the token is
    /// unquoted). Returns None for unrecognized names so the token stays
    /// literal.
    fn indirect_compound_assignment_values(
        &self,
        indirect_name: &str,
        quoted: bool,
    ) -> Option<Vec<String>> {
        if let Some(array_name) = indirect_name
            .strip_suffix("[@]")
            .or_else(|| indirect_name.strip_suffix("[*]"))
        {
            if is_shell_name(array_name) {
                let storage_name = self.resolved_variable_name(array_name)?;
                let storage = self.parameter_array_storage(array_name)?;
                let keys = if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
                    assoc_keys(&storage)
                } else {
                    array_indices(&storage)
                };
                if indirect_name.ends_with("[*]") {
                    return Some(vec![quote_array_value(
                        &keys.join(&self.ifs_first_char_separator()),
                    )]);
                }
                return Some(
                    keys.into_iter()
                        .map(|key| quote_array_value(&key))
                        .collect(),
                );
            }
        }

        // A nameref indirection yields the referenced NAME itself, not the
        // target value (GNU parameter_brace_expand_indir subst.c:7896).
        if is_marked_var(&self.env_vars, NAMEREF_VARS, indirect_name) {
            return None;
        }
        let target_expr = self.resolve_indirect_target_expr(indirect_name)?;
        match target_expr.as_str() {
            "@" => {
                return Some(
                    self.positional_params
                        .iter()
                        .map(|value| quote_array_value(value))
                        .collect(),
                )
            }
            "*" => {
                return Some(vec![quote_array_value(
                    &self.positional_params.join(&self.ifs_first_char_separator()),
                )])
            }
            _ => {}
        }
        let starred = target_expr.ends_with("[*]");
        if starred || target_expr.ends_with("[@]") {
            let values = self.indirect_target_values(&target_expr);
            if starred {
                if quoted {
                    return Some(vec![quote_array_value(
                        &values.join(&self.ifs_first_char_separator()),
                    )]);
                }
                return Some(
                    field_split_array_values_with_ifs(
                        values,
                        self.env_vars.get("IFS").map(String::as_str),
                    )
                    .into_iter()
                    .map(|value| quote_array_value(&value))
                    .collect(),
                );
            }
            return Some(
                values
                    .into_iter()
                    .map(|value| quote_array_value(&value))
                    .collect(),
            );
        }
        // Same bare-array decoding as the word path: implicit
        // `name=(...)` storage may be unmarked, so prefer
        // indirect_target_values before the parameter resolution.
        let mut target_values = self.indirect_target_values(&target_expr);
        let scalar = if target_values.len() == 1 {
            target_values.remove(0)
        } else if target_values.len() > 1 {
            target_values.join(&self.ifs_first_char_separator())
        } else {
            self.parameter_pattern_scalar_value(&target_expr)
                .unwrap_or_default()
        };
        if quoted {
            return Some(vec![quote_array_value(&scalar)]);
        }
        Some(
            field_split_values_with_ifs(&scalar, self.env_vars.get("IFS").map(String::as_str))
                .into_iter()
                .map(|value| quote_array_value(&value))
                .collect(),
        )
    }

    pub(in crate::executor) fn expand_unquoted_parameter_compound_assignment(
        &self,
        value: &str,
    ) -> Option<String> {
        let inner = value.strip_prefix('(')?.strip_suffix(')')?.trim();
        let unquoted_inner = strip_matching_quotes(inner);
        let parameter = if unquoted_inner == inner {
            inner
        } else {
            &unquoted_inner
        };
        let value = if let Some(name) = single_unquoted_parameter_name(parameter) {
            self.shell_variable_value(name).unwrap_or_default()
        } else if let Some(name) = parameter
            .strip_prefix("${")
            .and_then(|name| name.strip_suffix('}'))
        {
            let name = name.replace("\\\"", "\"").replace("\\'", "'");
            self.array_element_parameter_value(&name)?
        } else {
            return None;
        };
        let values =
            field_split_values_with_ifs(&value, self.env_vars.get("IFS").map(String::as_str))
                .into_iter()
                .map(|value| {
                    format!(
                        "{ARRAY_FIELD_SPLIT_MARKER}{}",
                        quote_compound_field_value(&value)
                    )
                })
                .collect::<Vec<_>>();
        Some(format!("({})", values.join(" ")))
    }

    pub(in crate::executor) fn expand_quoted_array_assignment_value(
        &self,
        value: &str,
    ) -> Option<String> {
        let value = value.strip_prefix('\x1d').unwrap_or(value);
        let name = value.strip_prefix("${")?.strip_suffix('}')?;
        let array_name = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))?;
        self.parameter_array_storage(array_name)
            .map(|value| self.join_array_parameter_values(&value, name))
    }

    pub(in crate::executor) fn expand_assignment_value_with_status(
        &mut self,
        value: &str,
    ) -> (String, Option<i32>) {
        let result = self.expand_assignment_value_result(value);
        (result.value, result.substitution_status)
    }
}

/// Split a compound assignment body into element tokens, treating single
/// quotes, double quotes and the hoisted DQ_DATA marker as quoting, so a
/// quoted space (`("a b"` hoisted to `(\u{E001}a b\u{E001}`) stays inside its
/// token. Tokens keep every character verbatim; only unquoted whitespace
/// separates elements.
fn split_compound_element_words(value: &str) -> Vec<String> {
    const DQ_DATA: char = '\u{E001}';
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            token.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !single => {
                token.push(ch);
                escaped = true;
            }
            '\'' if !double => {
                single = !single;
                token.push(ch);
            }
            '"' if !single => {
                double = !double;
                token.push(ch);
            }
            DQ_DATA if !single => {
                double = !double;
                token.push(ch);
            }
            ch if ch.is_whitespace() && !single && !double => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            ch => token.push(ch),
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn preserve_prompt_escapes(value: &str) -> String {
    const PROTECTED_PROMPT_ESCAPE: char = '\x15';
    let mut preserved = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && matches!(chars.peek(), Some('!' | '#')) {
            preserved.push(PROTECTED_PROMPT_ESCAPE);
            preserved.push(chars.next().expect("peeked prompt escape"));
        } else {
            preserved.push(ch);
        }
    }
    preserved.replace(PROTECTED_PROMPT_ESCAPE, "\\")
}

/// Quote-aware rewrite of $"..." into "..." for assignment expansion
/// (GNU parse.y:4096). Only a `$` immediately followed by an opening double
/// quote is rewritten: the scan tracks single- and double-quoted spans so a
/// `$` inside them ("a$"x", '...$"...') keeps its literal meaning, and
/// backslash escapes are passed through untouched.
fn normalize_dollar_double_quotes(value: &str) -> std::borrow::Cow<'_, str> {
    let needs_rewrite = value
        .chars()
        .zip(value.chars().skip(1))
        .any(|(ch, next)| ch == '$' && next == '"');
    if !needs_rewrite {
        return std::borrow::Cow::Borrowed(value);
    }
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            output.push('\\');
            output.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' if !in_double => {
                in_single = !in_single;
                output.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                output.push(ch);
            }
            '$' if !in_single && !in_double && chars.peek() == Some(&'"') => {
                // Drop the locale-quote dollar; the quote stays.
            }
            _ => output.push(ch),
        }
    }
    std::borrow::Cow::Owned(output)
}
