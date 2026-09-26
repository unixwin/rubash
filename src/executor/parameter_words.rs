use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

impl Executor {
    pub(in crate::executor) fn expand_parameter_word(&self, word: &str) -> String {
        // TODO(subst.c/parse.y): The `word` half of ${parameter:-word},
        // ${parameter:=word}, and ${parameter+word} has quote-aware expansion
        // flags. This covers tilde2.tests while the lexer still discards most
        // quote state.
        let expanded = self.expand_embedded_parameters(word);
        // GNU subst.c:4807 dequote_string: quote removal ran inside the
        // expansion pass, so `'`/`"` left in its output are
        // expansion-produced data, not syntax. Mark them \x17/\x18 before
        // decode_parameter_word_quotes, whose quote rules would otherwise
        // re-read a data `"` as an opener and drop it plus everything after
        // (array6.sub: `X${dbg-'x"'}Y` -> `Ax"Y`, not `AxY`).
        let expanded = expanded.replace('"', "\u{18}").replace('\'', "\u{17}");
        let expanded = unescape_remaining_shell_escapes(&decode_parameter_word_quotes(&expanded));
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.shell_state.env_vars, false)
    }

    // Alternate word of the `-`/`+`/`:-`/`:+` operators. GNU expands the
    // rhs like an unquoted word (subst.c parameter_brace_expand_rhs with
    // quoted == 0) while keeping quoted/escaped whitespace out of field
    // splitting (posixexp2 37). The walk-time variant in
    // expand_embedded_parameters_alternate_mut applies Unquoted quote
    // removal, the whitespace sentinel, and unquoted backslash rules in
    // one pass, so expansions inside quoted regions see the quote state
    // and their own whitespace stays protected (more-exp ${B:-"$A"}).
    pub(in crate::executor) fn expand_alternate_parameter_word(&mut self, word: &str) -> String {
        let expanded = self.expand_embedded_parameters_alternate_mut(word);
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.shell_state.env_vars, false)
    }

    pub(in crate::executor) fn expand_parameter_word_mut(&mut self, word: &str) -> String {
        let expanded = unescape_remaining_shell_escapes(&decode_parameter_word_quotes(
            &self.expand_embedded_parameters_mut(word),
        ));
        tilde_expand::expand_assignment_tilde_value(&expanded, &self.shell_state.env_vars, false)
    }

    /// GNU subst.c pos_params (3745) + string_list_pos_params (3030):
    /// `${*:offset:length}` selects a range of positional parameters and
    /// joins them with IFS[0] (string_list_dollar_star, subst.c:2902),
    /// while `${@:offset:length}` joins with a space (string_list). The
    /// offset==0 case prepends $0 (dollar_vars[0]) per pos_params:3759.
    /// expand_braced_substring_parameter always joins with a space, so
    /// intercept `*`/`@` here to apply the correct separator.
    fn expand_star_at_substring(
        &self,
        var_name: &str,
        offset: isize,
        length: Option<isize>,
    ) -> String {
        let selected = positional_parameter_substring_with_zero(
            &self.shell_state.positional_params,
            &self.script_name_value(),
            offset,
            length,
        );
        if var_name == "*" {
            let ifs = self
                .shell_state
                .env_vars
                .get("IFS")
                .cloned()
                .unwrap_or_else(|| " \t\n".to_string());
            match ifs.chars().next() {
                Some(separator) => selected.join(&separator.to_string()),
                None => selected.concat(),
            }
        } else {
            selected.join(" ")
        }
    }

    /// GNU subst.c:9990-9993 + param_expand: for a list operand
    /// (`name[@]`, `name[*]`, `@`, `*`) the `-`/`:-`/`+`/`:+` set/null test
    /// applies to the operand's string form — string_list_dollar_at joins
    /// [@]/@ with " ", string_list_dollar_star joins [*]/* with IFS[0] — so
    /// `"<${A[*]:-X}>"` under IFS='' with A=('' '') joins to "" and yields
    /// `X`, while `"${A[@]:-Y}"` joins to " " and expands (array22.sub).
    /// Returns Some((joined_word, non_empty)) for list operands.
    fn list_operand_joined_word(&self, var_name: &str) -> Option<(String, bool)> {
        let (values, is_at) = self.braced_operator_list_values(var_name)?;
        let separator = if is_at {
            " ".to_string()
        } else {
            self.ifs_first_char_separator()
        };
        let non_empty = !values.is_empty();
        Some((values.join(&separator), non_empty))
    }

    pub(in crate::executor) fn expand_quoted_parameter_word(&self, word: &str) -> String {
        // TODO(subst.c/parse.y): Quoted parameter expansion should carry
        // CTLESC/CTLQUOTEMARK state from the parser. This preserves the
        // tilde2.tests distinction that quoted default/alternate words do not
        // perform tilde expansion.
        let Some(name) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return self.expand_embedded_parameters(word);
        };
        if !braced_parameter_spans_whole_word(word) {
            return self.expand_embedded_parameters(word);
        }

        if let Some((var_name, default)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":-")
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        return joined;
                    }
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                default,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    default,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        )
                    });
            }
        }

        if let Some((var_name, alternate)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":+")
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        return unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    alternate,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        );
                    }
                    return String::new();
                }
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                alternate,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return String::new();
            }
        }

        if let Some((var_name, error_word)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":?")
        {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                return self.expand_embedded_parameters(error_word);
            }
        }

        if let Some((var_name, error_word)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '?')
        {
            // GNU subst.c parameter_brace_expand: `${#?}` is the length of
            // `$?`, not `$#` with the `?` error operator. Only the bare `#?`
            // form (no error word) is the length-of-special case; `${#?word}`
            // stays the `?` operator.
            if is_parameter_error_name(var_name) && !(var_name == "#" && error_word.is_empty()) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(error_word));
            }
        }

        if let Some((var_name, word)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":=")
        {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(word));
            }
        }

        if let Some((var_name, word)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '=')
        {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_embedded_parameters(word));
            }
        }

        if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
            // `${#:offset}` / `${#?:0}` slice the special parameter value,
            // not a length expansion. expand_braced_substring_parameter does
            // not resolve special-parameter names, so fetch the value here.
            if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
                let value = self.expand_parameter_named_value(var_name);
                return parameter_substring(&value, offset, length);
            }
            if matches!(var_name, "*" | "@") {
                return self.expand_star_at_substring(var_name, offset, length);
            }
            return self.expand_braced_substring_parameter(var_name, offset, length);
        }

        if name.starts_with('#') && !hash_is_special_param_with_operator(name) {
            if let Some(value) = self.expand_braced_indexed_parameter(name) {
                return value;
            }
        }

        if let Some((array_name, _)) = parse_array_subscript(name) {
            // GNU valid_array_reference only treats NAME[...] as a subscript
            // when NAME is a valid identifier; pattern words like `z//[^;]`
            // or `z/#[^;][^;]` must not produce a spurious bad-array-
            // subscript diagnostic here (new-exp8.sub).
            if is_shell_name(array_name) {
                if let Some(value) = self.array_element_parameter_value(name) {
                    return shell_safe_value(&value);
                }
            }
        }

        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))
        {
            return self
                .parameter_array_storage(array_name)
                .map(|value| self.join_array_parameter_values(&value, name))
                .unwrap_or_default();
        }

        if let Some((var_name, alternate)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '+')
        {
            if is_parameter_error_name(var_name) {
                if let Some((_, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        return unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    alternate,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        );
                    }
                    return String::new();
                }
                if self.parameter_operator_value(var_name).is_some() {
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                alternate,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return String::new();
            }
        }

        if let Some((var_name, default)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '-')
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        return joined;
                    }
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters(
                            &decode_double_quotes_in_quoted_parameter_word(
                                default,
                                self.posix_mode_enabled(),
                                false,
                            ),
                        ),
                        SubstitutionQuoteContext::DoubleQuoted,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        unescape_parameter_operator_result(
                            &self.expand_embedded_parameters(
                                &decode_double_quotes_in_quoted_parameter_word(
                                    default,
                                    self.posix_mode_enabled(),
                                    false,
                                ),
                            ),
                            SubstitutionQuoteContext::DoubleQuoted,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        )
                    });
            }
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(name, false) {
            return value;
        }

        self.expand_word(word)
    }

    // GNU param_expand: the word of ${var-word}/${var:=word}/${var+word}/...
    // undergoes tilde expansion only in an unquoted expansion context and only
    // when the word itself starts bare (an explicitly quoted `~` stays
    // literal). Tilde applies at the word start, not after colons.
    fn tilde_expand_operator_word(&self, word: &str, context: SubstitutionQuoteContext) -> String {
        if !matches!(context, SubstitutionQuoteContext::Unquoted) {
            return word.to_string();
        }
        if word.starts_with('"') || word.starts_with('\'') || word.starts_with('\\') {
            return word.to_string();
        }
        tilde_expand::expand_assignment_tilde_value(word, &self.shell_state.env_vars, false)
    }

    /// GNU parameter_brace_expand_word (subst.c:7663): the `word` half of
    /// `${var op word}` is expanded under the word's own quote context. The
    /// double-quote sentence decode (slashify_in_quotes port) applies inside
    /// "${...}" and in here-document bodies (Q_HERE_DOCUMENT behaves like a
    /// dq context for ${} words); for an unquoted ${...} the rhs keeps its
    /// original
    /// quote syntax so the walker's quote removal strips it (`o=${x-' '}`
    /// stores a space; `${f-'$HOME'}` keeps `$HOME` unexpanded).
    fn decode_operator_word_for_context(
        &self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        if matches!(
            context,
            SubstitutionQuoteContext::DoubleQuoted | SubstitutionQuoteContext::HereDocument
        ) {
            decode_double_quotes_in_quoted_parameter_word(
                word,
                self.posix_mode_enabled(),
                matches!(context, SubstitutionQuoteContext::HereDocument),
            )
        } else {
            word.to_string()
        }
    }

    pub(in crate::executor) fn expand_quoted_parameter_word_mut(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        // GNU param_expand resolves one `${}` expansion once: memoize
        // array-element fetches for the duration of this word so repeated
        // helper-layer reads (operator set-checks, error probes) do not
        // re-run subscript side effects (AEPV_MEMO). Whole-word `\x1d`-marked
        // `${}` forms reach here without passing a walker `${` arm, so a
        // frame is needed at this entry — but only when the word is one
        // `${}` span; a multi-`${}` word must keep per-`${}` scoping, which
        // the walker's `${` arms provide (push-if-empty shares nested `${}`s
        // with the outer scope).
        let _memo_frame = if braced_parameter_spans_whole_word(word) {
            Some(crate::executor::expand_braced_indices::AepvMemoFrame::new())
        } else {
            None
        };
        // Bash 5.3 (parser.h FUNSUB_CHAR): a whitespace-led `${ command; }` /
        // `${|command;}` word is a nofork command substitution, not a
        // parameter form. The operator split_once parsing below would treat
        // the body's first `=` as ${name=word} assignment syntax and expand
        // the rest of the body as literal text (comsub22.sub: a multi-line
        // funsub in double quotes printed the body instead of executing it).
        // Route the whole word through the embedded walker, which collects
        // the funsub span and executes it in the current shell.
        // Bash 5.3 (parser.h FUNSUB_CHAR): a whitespace-led `${ command; }` /
        // `${|command;}` word is a nofork command substitution, not a
        // parameter form. The operator split_once parsing below would treat
        // the body's first `=` as ${name=word} assignment syntax and expand
        // the rest of the body as literal text (comsub22.sub: a multi-line
        // funsub in double quotes printed the body instead of executing it).
        // Route the whole word through the embedded walker, which collects
        // the funsub span and executes it in the current shell. Nested
        // funsubs inside an outer parameter form stay on the operator path.
        if crate::executor::parameter_core::word_contains_current_shell_command_substitution(word)
            && crate::executor::parameter_core::funsub_span_is_top_level(word)
        {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        }
        // In POSIX mode, a double-quoted `${...}` may close at a `}` inside
        // the apparent word when a single quote is literal (Interp 221).
        // Expand that braced head separately, then continue with the suffix.
        if matches!(context, SubstitutionQuoteContext::DoubleQuoted) && self.posix_mode_enabled() {
            if let Some(rest) = word.strip_prefix("${") {
                if let Some(close) = matching_parameter_brace_in_context(rest, true, true) {
                    if close + 1 < rest.len() {
                        let braced_end = 2 + close + 1;
                        let head =
                            self.expand_quoted_parameter_word_mut(&word[..braced_end], context);
                        let tail = self.expand_embedded_parameters_mut_with_context(
                            &word[braced_end..],
                            context,
                        );
                        return format!("{head}{tail}");
                    }
                }
            }
        }

        let Some(name) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        };
        // The whole-word span check must use the same quote rules the lexer
        // used to build the word. In POSIX mode inside double quotes the
        // Interp 221 big hammer closes `${...}` at the first `}` (single
        // quotes are literal), so a body with an unbalanced `'` such as
        // `${IFS+"'"x ~ x'}` still closes at the final `}`; the non-POSIX
        // scan leaves that `}` quote-hidden and mis-routes the word to the
        // embedded walker (posixexp2 case 28).
        let spans_whole_word = if matches!(context, SubstitutionQuoteContext::DoubleQuoted)
            && self.posix_mode_enabled()
        {
            braced_parameter_spans_whole_word_in_context(word, true, true)
        } else {
            braced_parameter_spans_whole_word(word)
        };
        if !spans_whole_word {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        }

        // GNU subst.c:10272-10288 (parameter_brace_expand): a quote
        // terminating the parameter name matches no operator arm and lands
        // on the `bad substitution` default. Mirror of the same check in
        // expand_braced_parameter_word for the `\x1d`/whole-word entry.
        if crate::executor::expand_word::braced_name_ends_on_quote(name) {
            eprintln!(
                "{}{}: bad substitution",
                self.diagnostic_prefix(),
                crate::executor::expand_word::bad_substitution_display(word)
            );
            self.shell_state.parameter_bad_substitution.set(true);
            return String::new();
        }

        // This `\x1d`-quoted word IS one `${}` fragment: record site [0]
        // so the `:=`/`-=` operator set-checks dedup subscript side
        // effects against the pre-scan (SUB_RES_XPASS). An active site
        // means the enclosing fragment already named it — keep it.
        let _site_guard = (!crate::executor::expand_braced_indices::sub_site_active())
            .then(|| crate::executor::expand_braced_indices::SubSiteGuard::new(0));

        if let Some((var_name, default)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":-")
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        return joined;
                    }
                    let default = self.tilde_expand_operator_word(default, context);
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters_mut_with_context(
                            &self.decode_operator_word_for_context(&default, context),
                            context,
                        ),
                        context,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let default = self.tilde_expand_operator_word(default, context);
                        unescape_parameter_operator_result(
                            &self.expand_embedded_parameters_mut_with_context(
                                &self.decode_operator_word_for_context(&default, context),
                                context,
                            ),
                            context,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        )
                    });
            }
        }

        if let Some((var_name, alternate)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":+")
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, _)) = self.list_operand_joined_word(var_name) {
                    if !joined.is_empty() {
                        let alternate = self.tilde_expand_operator_word(alternate, context);
                        return unescape_parameter_operator_result(
                            &self.expand_embedded_parameters_mut_with_context(
                                &self.decode_operator_word_for_context(&alternate, context),
                                context,
                            ),
                            context,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        );
                    }
                    return String::new();
                }
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    let alternate = self.tilde_expand_operator_word(alternate, context);
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters_mut_with_context(
                            &self.decode_operator_word_for_context(&alternate, context),
                            context,
                        ),
                        context,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return String::new();
            }
        }

        if let Some((var_name, error_word)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":?")
        {
            if is_parameter_error_name(var_name) {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                let error_word = self.tilde_expand_operator_word(error_word, context);
                return self.expand_embedded_parameters_mut_with_context(&error_word, context);
            }
        }

        if let Some((var_name, error_word)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '?')
        {
            // GNU subst.c parameter_brace_expand: `${#?}` is the length of
            // `$?`, not `$#` with the `?` error operator. Only the bare `#?`
            // form (no error word) is the length-of-special case; `${#?word}`
            // stays the `?` operator.
            if is_parameter_error_name(var_name) && !(var_name == "#" && error_word.is_empty()) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let error_word = self.tilde_expand_operator_word(error_word, context);
                        self.expand_embedded_parameters_mut_with_context(&error_word, context)
                    });
            }
        }

        if let Some((var_name, word)) =
            super::expand_braced_ops::split_once_outside_subscript_str(name, ":=")
        {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let word = self.tilde_expand_operator_word(word, context);
                        // GNU parameter_brace_expand_word sets
                        // expand_no_split_dollar_star for op == '='
                        // (subst.c:4487), which includes `:=`. This makes
                        // unquoted $* with null IFS join with IFS[0] inside
                        // the value (exp11.sub ${c=${*/}}).
                        let old =
                            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
                        let result =
                            self.expand_embedded_parameters_mut_with_context(&word, context);
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
                        result
                    });
            }
        }

        if let Some((var_name, word)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '=')
        {
            if is_parameter_error_name(var_name) {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let word = self.tilde_expand_operator_word(word, context);
                        // GNU parameter_brace_expand_word sets
                        // expand_no_split_dollar_star for op == '='
                        // (subst.c:4487). This makes unquoted $* with null
                        // IFS join with IFS[0] inside the value
                        // (exp11.sub ${c=${*/}}).
                        let old =
                            super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
                        let result =
                            self.expand_embedded_parameters_mut_with_context(&word, context);
                        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
                        result
                    });
            }
        }

        if let Some((var_name, offset, length)) = self.parse_parameter_substring_mut(name) {
            // `${#:offset}` / `${#?:0}` slice the special parameter value,
            // not a length expansion. expand_braced_substring_parameter does
            // not resolve special-parameter names, so fetch the value here.
            if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
                let value = self.expand_parameter_named_value(var_name);
                return parameter_substring(&value, offset, length);
            }
            if matches!(var_name, "*" | "@") {
                return self.expand_star_at_substring(var_name, offset, length);
            }
            return self.expand_braced_substring_parameter(var_name, offset, length);
        }

        if name.starts_with('#') && !hash_is_special_param_with_operator(name) {
            if let Some(value) = self.expand_braced_indexed_parameter(name) {
                return value;
            }
        }

        if let Some((array_name, _)) = parse_array_subscript(name) {
            // GNU valid_array_reference only treats NAME[...] as a subscript
            // when NAME is a valid identifier; pattern words like `z//[^;]`
            // or `z/#[^;][^;]` must not produce a spurious bad-array-
            // subscript diagnostic here (new-exp8.sub).
            if is_shell_name(array_name) {
                if let Some(value) = self.array_element_parameter_value(name) {
                    return shell_safe_value(&value);
                }
            }
        }

        if let Some(array_name) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
            .filter(|array_name| is_shell_name(array_name))
        {
            return self
                .parameter_array_storage(array_name)
                .map(|value| self.join_array_parameter_values(&value, name))
                .unwrap_or_default();
        }

        if let Some((var_name, alternate)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '+')
        {
            if is_parameter_error_name(var_name) {
                if let Some((_, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        let alternate = self.tilde_expand_operator_word(alternate, context);
                        let decoded = self.decode_operator_word_for_context(&alternate, context);
                        let expanded =
                            self.expand_embedded_parameters_mut_with_context(&decoded, context);
                        return unescape_parameter_operator_result(
                            &expanded,
                            context,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        );
                    }
                    return String::new();
                }
                if self.parameter_operator_value(var_name).is_some() {
                    let alternate = self.tilde_expand_operator_word(alternate, context);
                    let decoded = self.decode_operator_word_for_context(&alternate, context);
                    let expanded =
                        self.expand_embedded_parameters_mut_with_context(&decoded, context);
                    let final_value = unescape_parameter_operator_result(
                        &expanded,
                        context,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                    return final_value;
                }
                return String::new();
            }
        }

        if let Some((var_name, default)) =
            super::expand_braced_ops::split_once_outside_subscript(name, '-')
        {
            if is_parameter_error_name(var_name) {
                if let Some((joined, non_empty)) = self.list_operand_joined_word(var_name) {
                    if non_empty {
                        return joined;
                    }
                    let default = self.tilde_expand_operator_word(default, context);
                    return unescape_parameter_operator_result(
                        &self.expand_embedded_parameters_mut_with_context(
                            &self.decode_operator_word_for_context(&default, context),
                            context,
                        ),
                        context,
                        self.shell_state.env_vars.get("IFS").map(String::as_str),
                    );
                }
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| {
                        let default = self.tilde_expand_operator_word(default, context);
                        unescape_parameter_operator_result(
                            &self.expand_embedded_parameters_mut_with_context(
                                &self.decode_operator_word_for_context(&default, context),
                                context,
                            ),
                            context,
                            self.shell_state.env_vars.get("IFS").map(String::as_str),
                        )
                    });
            }
        }

        if let Some(value) = self.expand_braced_pattern_or_transform_parameter(name) {
            return value;
        }

        if let Some(value) = self.expand_braced_special_or_indirect_parameter(
            name,
            matches!(context, SubstitutionQuoteContext::Unquoted),
        ) {
            return value;
        }

        self.expand_word(word)
    }

    /// Expand a `=`/`:=` alternate for assignment. Outside double quotes the
    /// alternate is fully quote-removed (`a\ b` -> `a b`, posixexp2 case 35).
    /// Inside double quotes `\` only escapes $, `, ", \, and newline; any
    /// other `\X` is literal data and survives into the assigned value
    /// (`"${v=a\ b}"` assigns `a\ b`, posixexp2 case 36). Inside double
    /// quotes single quotes are data, not delimiters, so `$var` inside
    /// `'...'` is expanded (`"${fox='$foo'}"` assigns `'bar'`,
    /// more-exp.tests:112). Double quotes in the alternate are removed
    /// by `decode_double_quotes_in_quoted_parameter_word` (matching the
    /// `+`/`-` operator path), so `"${und="foo"}"` assigns `foo`
    /// (new-exp.tests:33).
    fn expand_assignment_alternate_mut(&mut self, value: &str, double_quoted: bool) -> String {
        // GNU parameter_brace_expand_word sets expand_no_split_dollar_star
        // for op == '=' (subst.c:4487), which includes `:=`. This makes
        // unquoted $* with null IFS join with IFS[0] inside the value
        // (exp11.sub ${c=${*/}}).
        let old = super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.get());
        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(true));
        let result = self.expand_assignment_alternate_mut_inner(value, double_quoted);
        super::expand_braced_replacement::ASSIGNMENT_RHS.with(|f| f.set(old));
        result
    }

    fn expand_assignment_alternate_mut_inner(
        &mut self,
        value: &str,
        double_quoted: bool,
    ) -> String {
        if !double_quoted {
            return self.expand_parameter_word_mut(value);
        }
        // 0x0e is unused by every other sentinel layer (the lexer's
        // PARAM_NAME_END_MARKER is 0x13); protect/restore is local to this
        // function, so the two never interact.
        const PROTECTED_LITERAL_BACKSLASH: char =
            crate::executor::markers::PARAM_WORD_BACKSLASH_GUARD;
        let chars: Vec<char> = value.chars().collect();
        let mut protected = String::with_capacity(value.len());
        let mut index = 0usize;
        while index < chars.len() {
            if chars[index] == '\\' {
                if let Some(next) = chars.get(index + 1).copied() {
                    if !matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                        protected.push(PROTECTED_LITERAL_BACKSLASH);
                        protected.push(next);
                        index += 2;
                        continue;
                    }
                }
            }
            protected.push(chars[index]);
            index += 1;
        }
        // Remove double quotes from the alternate (matching the `+`/`-`
        // operator path which calls decode_double_quotes_in_quoted_parameter_word).
        let decoded = decode_double_quotes_in_quoted_parameter_word(
            &protected,
            self.posix_mode_enabled(),
            false,
        );
        // Use DoubleQuoted context so single quotes are treated as data
        // (not quote delimiters), matching GNU's expand_string_for_rhs
        // behavior inside double quotes. Use unescape_parameter_operator_result
        // (not decode_parameter_word_quotes) so single quotes survive as data.
        let expanded = self.expand_embedded_parameters_mut_with_context(
            &decoded,
            SubstitutionQuoteContext::DoubleQuoted,
        );
        let unescaped = unescape_parameter_operator_result(
            &expanded,
            SubstitutionQuoteContext::DoubleQuoted,
            self.shell_state.env_vars.get("IFS").map(String::as_str),
        );
        unescaped.replace(PROTECTED_LITERAL_BACKSLASH, "\\")
    }

    pub(in crate::executor) fn apply_parameter_assignment_expansions_in_word(
        &mut self,
        word: &str,
    ) {
        // Assignment alternates are quote-removed with the quote rules of
        // the region their `${...}` sits in: inside double quotes `\` only
        // escapes $, `, ", \, and newline, so `"${v=a\ b}"` assigns `a\ b`
        // (posixexp2 case 36), while an unquoted `${v=a\ b}` assigns `a b`
        // and then field-splits (case 35). Parameter bodies inside a
        // command substitution span do NOT inherit the surrounding word's
        // quote context — they are applied later, when the inner command's
        // own words expand (`"x $(printf '%s ' ${v=a\ b})"` assigns `a b`).
        let quoted_word = word.starts_with(STORAGE_WORD_PREFIX);
        // When this word IS the `${}` fragment currently being evaluated
        // (a `${name}` body re-entered through expand_word_mut), its single
        // `${` occurrence inherits the enclosing fragment's site rather
        // than re-keying on the synthetic string (SUB_RES_XPASS docs).
        let inherit_site = braced_parameter_spans_whole_word(word)
            && crate::executor::expand_braced_indices::sub_site_active();
        // The prefix scanned for quote/CS context always runs from the word
        // start, so state skipped-over bodies (e.g. inside a command
        // substitution) still counts toward the next body's context.
        let mut consumed = 0usize;
        // Top-level `${` ordinal — the same fragments in the same order as
        // the expansion walker's `${` arms see them (command-substitution
        // bodies do not reach either scan's arm).
        let mut frag_index = 0usize;
        while let Some(rel) = word[consumed..].find("${") {
            let start = consumed + rel;
            let (in_double, inside_cs) = scan_word_prefix_quote_state(&word[..start], quoted_word);
            let body_start = start + 2;
            let Some(end) = matching_parameter_brace(&word[body_start..]) else {
                break;
            };
            if inside_cs {
                // Skip the whole body; the inner expansion applies it.
                consumed = body_start + end + 1;
                continue;
            }
            let inner = &word[body_start..body_start + end];
            let _site_guard = (!inherit_site).then(|| {
                let guard = crate::executor::expand_braced_indices::SubSiteGuard::new(frag_index);
                frag_index += 1;
                guard
            });
            self.apply_parameter_assignment_expansion_with_context(inner, in_double);
            consumed = body_start + end + 1;
        }
    }

    fn apply_parameter_assignment_expansion_with_context(
        &mut self,
        inner: &str,
        double_quoted: bool,
    ) {
        // GNU subst.c: a failed := assignment is an expand_word_error that
        // aborts the whole word expansion (DISCARD at top level), so once a
        // failure is latched the remaining expansions are skipped — this also
        // prevents the duplicate diagnostic when the word is re-expanded.
        if self.parameter_assignment_failure.get() {
            return;
        }
        // GNU subst.c param_expand: `=`/`:=` are parameter operators only at
        // the top level of the `${...}` body; inside `[...]` they belong to
        // the array subscript's arithmetic (nameref10.sub:
        // `${x[i=0$(...)]}` expands element i=0 -- it does not assign
        // through `x[i` and does not expand the subscript text as the
        // operator word, which would run the embedded $(...) twice).
        if let Some((name, value)) =
            super::expand_braced_ops::split_once_outside_subscript_str(inner, ":=")
        {
            if self
                .parameter_operator_value(name)
                .is_some_and(|value| !value.is_empty())
            {
                return;
            }
            let value = self.expand_assignment_alternate_mut(value, double_quoted);
            // GNU parameter_brace_assign resolves the subscript AGAIN for
            // the assignment target (array_expand_index on the
            // assign_array_element path), distinct from the set-test's
            // array_variable_part evaluation — so `${b[i++]:=z}` stores
            // at the SECOND index. The marker path keeps this evaluation
            // out of the set-test's cross-pass memo.
            let _assign_site =
                crate::executor::expand_braced_indices::SubSiteGuard::new(usize::MAX);
            if self.apply_array_element_parameter_assignment(name, value.clone()) {
                return;
            }
            if self.apply_indirect_parameter_assignment(name, value.clone()) {
                return;
            }
            if !is_shell_name(name) {
                return;
            }
            if !self.apply_shell_assignment(name, value) {
                self.parameter_assignment_failure.set(true);
            }
            return;
        }

        if let Some((name, value)) =
            super::expand_braced_ops::split_once_outside_subscript(inner, '=')
        {
            if self.parameter_operator_value(name).is_some() {
                return;
            }
            let value = self.expand_assignment_alternate_mut(value, double_quoted);
            // Same assign-target re-evaluation as `:=` above.
            let _assign_site =
                crate::executor::expand_braced_indices::SubSiteGuard::new(usize::MAX);
            if self.apply_array_element_parameter_assignment(name, value.clone()) {
                return;
            }
            if self.apply_indirect_parameter_assignment(name, value.clone()) {
                return;
            }
            if !is_shell_name(name) {
                return;
            }
            if !self.apply_shell_assignment(name, value) {
                self.parameter_assignment_failure.set(true);
            }
        }
    }

    fn apply_indirect_parameter_assignment(&mut self, name: &str, value: String) -> bool {
        let Some(indirect_name) = name.strip_prefix('!') else {
            return false;
        };
        if self.nameref_target_name(indirect_name).is_some() {
            return false;
        }
        let Some(target_name) = self.shell_state.env_vars.get(indirect_name).cloned() else {
            return false;
        };
        if self.apply_array_element_parameter_assignment(&target_name, value.clone()) {
            return true;
        }
        if !is_shell_name(&target_name) {
            return false;
        }
        self.apply_shell_assignment(&target_name, value);
        true
    }
}

/// GNU subst.c parameter_brace_expand: `${#` followed by one of the operator
/// characters `-`, `+`, `=`, `?` and a word makes `#` the special parameter
/// (positional parameter count) with that operator, not a length prefix.
/// `${#-}` / `${#?}` (operator char alone before `}`) remain length of the
/// special parameter. A `:`-led operator (`:-`, `:+`, `:=`, `:?`) makes `#`
/// the parameter even with an empty word — `${#:-}` is `$#` under `:-`
/// (more-exp `recho ${#:-}` -> `0`), while a bare `${#:}` is still a bad
/// substitution. Returns true when the `#` length-prefix route should be
/// skipped so the operator splits downstream handle the form.
fn hash_is_special_param_with_operator(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('#') else {
        return false;
    };
    if let Some(colon_op) = rest.strip_prefix(':') {
        return colon_op
            .chars()
            .next()
            .is_some_and(|op| matches!(op, '-' | '+' | '=' | '?'));
    }
    let Some(op) = rest.chars().next() else {
        return false;
    };
    matches!(op, '-' | '+' | '=' | '?') && rest.len() > op.len_utf8()
}

/// Walk the text before a `${...}` body and report (a) whether the body
/// sits inside double quotes and (b) whether it sits inside a command
/// substitution span. `quoted_word` marks words whose whole text is a
/// double-quoted region (the executor's `\x1d` marker). A `$(...)` span
/// starts a fresh quoting context: quoting inside it does not affect the
/// enclosing region, and it does not inherit the enclosing word's quotes
/// (`"x $(printf '%s ' ${v=a\ b})"` expands the body unquoted).
fn scan_word_prefix_quote_state(prefix: &str, quoted_word: bool) -> (bool, bool) {
    #[derive(Clone, Copy)]
    struct Frame {
        in_single: bool,
        in_double: bool,
    }
    let mut stack: Vec<Frame> = vec![Frame {
        in_single: false,
        in_double: quoted_word,
    }];
    let chars: Vec<char> = prefix.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let frame = *stack.last().expect("quote frame stack is never empty");
        if frame.in_single {
            if chars[index] == '\'' {
                stack.last_mut().unwrap().in_single = false;
            }
            index += 1;
            continue;
        }
        if frame.in_double {
            match chars[index] {
                '\\' => index += 2,
                '"' => stack.last_mut().unwrap().in_double = false,
                '$' if chars.get(index + 1) == Some(&'(') => {
                    stack.push(Frame {
                        in_single: false,
                        in_double: false,
                    });
                    index += 2;
                }
                _ => index += 1,
            }
            continue;
        }
        match chars[index] {
            '\'' => {
                stack.last_mut().unwrap().in_single = true;
                index += 1;
            }
            '"' => {
                stack.last_mut().unwrap().in_double = true;
                index += 1;
            }
            '`' => {
                // Old-style substitution: skip to the closing backtick.
                if let Some(close) = chars[index + 1..].iter().position(|c| *c == '`') {
                    index += close + 2;
                    continue;
                }
                break;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                stack.push(Frame {
                    in_single: false,
                    in_double: false,
                });
                index += 2;
                continue;
            }
            '\\' => {
                index += 2;
                continue;
            }
            ')' if stack.len() > 1 => {
                stack.pop();
                index += 1;
            }
            _ => {
                index += 1;
            }
        }
    }
    let top = *stack.last().expect("quote frame stack is never empty");
    (top.in_double, stack.len() > 1)
}

pub(in crate::executor) fn decode_double_quotes_in_quoted_parameter_word(
    word: &str,
    posix: bool,
    heredoc: bool,
) -> String {
    let mut output = String::new();
    let chars = word.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut in_sq = false;
    while index < chars.len() {
        // POSIX mode keeps ' as a real single-quote delimiter inside
        // "${v op w}" (parse.y:4036 Austin Group interp 221), so a `'`
        // there opens/closes a quoted span — a `$'` only starts ANSI-C
        // quoting when the ' is not a span closer (GNU's LEX_WASDOL check
        // in parse_matched_pair only sees delimiters).
        if in_sq {
            if chars[index] == '\'' {
                in_sq = false;
            }
            output.push(chars[index]);
            index += 1;
            continue;
        }
        // GNU parse.y:4053-4070 (parse_matched_pair): inside a `${...}`
        // grouping construct a `$'...'` body is ansiexpanded at extraction
        // time and wrapped in sh_single_quote so the decoded bytes ride out
        // quote removal as data. Emit each decoded char in its data-carrier
        // form so the expansion walker never re-reads it as syntax; without
        // this the `'` is marked DATA_SQUOTE below and `$'\t'` inside
        // "${v-word}" leaks literal text (nquote.tests).
        if chars[index] == '$' && chars.get(index + 1) == Some(&'\'') && !heredoc {
            let mut body = String::new();
            let mut cursor = index + 2;
            let mut closed = false;
            while cursor < chars.len() {
                let ch = chars[cursor];
                if ch == '\\' && cursor + 1 < chars.len() {
                    body.push(ch);
                    body.push(chars[cursor + 1]);
                    cursor += 2;
                    continue;
                }
                if ch == '\'' {
                    closed = true;
                    cursor += 1;
                    break;
                }
                body.push(ch);
                cursor += 1;
            }
            if !closed {
                output.push('$');
                index += 1;
                continue;
            }
            index = cursor;
            for ch in crate::lexer::decode_ansi_c_quoted(&body).chars() {
                match ch {
                    '\'' => output.push(crate::executor::markers::DATA_SQUOTE),
                    '"' => output.push(crate::executor::markers::DATA_DQUOTE),
                    '\\' => output.push(crate::executor::markers::DATA_BACKSLASH),
                    '$' => output.push(DATA_DOLLAR),
                    '`' => output.push(crate::executor::markers::DATA_BACKTICK),
                    ' ' | '\t' | '\n' => {
                        output.push(crate::executor::markers::IFS_GLUE);
                        output.push(ch);
                    }
                    _ => output.push(ch),
                }
            }
            continue;
        }
        // A backslash escape outside a double-quote span survives quote
        // removal intact: the expansion pass turns it into protected data
        // (`\"` yields a literal quote, posixexp2 case 8). Dropping the
        // escaped quote here made the expansion output an unmarked bare
        // quote that later stages swallowed.
        if chars[index] == '\\'
            && index + 1 < chars.len()
            && (matches!(chars[index + 1], '$' | '`' | '"' | '\\' | '}' | '\n')
                // GNU parse.y dolbrace is POSIX-only: outside POSIX mode a
                // `'` inside "${var op word}" is literal data, so \' is an
                // escape pair producing a literal quote.
                || (!posix && chars[index + 1] == '\''))
        {
            // `\\` becomes the escaped-backslash marker (\x14) so the
            // expansion walker treats it as data, not as an escape for
            // the following character (GNU slashify_in_quotes: `\\` → `\`
            // but does NOT escape `$`; rhs-exp: `\\$selvecs` → `\&m68kcoff_vec`).
            if chars[index + 1] == '\\' {
                output.push(crate::executor::markers::DATA_BACKSLASH);
            } else if chars[index + 1] == '\'' {
                // GNU retains the backslash: `\'` inside a double-quoted
                // "${var op word}" survives quote removal as literal
                // backslash+quote (rhs-exp.tests `\'$selvecs\'`).
                output.push(crate::executor::markers::DATA_BACKSLASH);
                output.push(crate::executor::markers::DATA_SQUOTE);
            } else {
                output.push(chars[index]);
                output.push(chars[index + 1]);
            }
            index += 2;
            continue;
        }
        if chars[index] == '\'' && !posix && !heredoc {
            // Non-POSIX "${var op word}": ' is literal text, never an sq
            // opener. Emit it escaped so the expansion pass yields a data
            // quote and any following $( still expands (braces.tests).
            output.push(crate::executor::markers::DATA_SQUOTE);
            index += 1;
            continue;
        }
        if chars[index] == '\'' && posix && !heredoc {
            in_sq = true;
            output.push(chars[index]);
            index += 1;
            continue;
        }
        if chars[index] != '"' {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        index += 1;
        while index < chars.len() {
            match chars[index] {
                '"' => {
                    index += 1;
                    break;
                }
                '\\' if matches!(chars.get(index + 1), Some('\\' | '"' | '$' | '`' | '\n')) => {
                    let escaped = chars[index + 1];
                    index += 2;
                    match escaped {
                        '\n' => {}
                        // `\\` becomes the escaped-backslash marker (\x14)
                        // so the expansion walker treats it as data, not as
                        // an escape for the following character (GNU
                        // slashify_in_quotes: `\\` → `\` but does NOT escape
                        // `$`; rhs-exp: `"\\$selvecs"` → `\&m68kcoff_vec`).
                        '\\' => {
                            output.push(crate::executor::markers::DATA_BACKSLASH);
                        }
                        // Protect `$` and `` ` `` from re-expansion: inside
                        // double quotes `\$` and `\`` are literal data that
                        // must not trigger parameter/command substitution.
                        '$' => output.push(DATA_DOLLAR),
                        '`' => output.push(crate::executor::markers::DATA_BACKTICK),
                        _ => output.push(escaped),
                    }
                }
                // GNU expand_word_internal with Q_DOUBLE_QUOTES: backslash
                // before a char NOT in CBSDQUOTE ($ ` " \ newline) is removed
                // — only the char survives (rhs-exp.tests: `\p` → `p`,
                // `\'` → `'`).
                '\\' => {
                    if let Some(&next) = chars.get(index + 1) {
                        index += 2;
                        output.push(next);
                    } else {
                        output.push('\\');
                        index += 1;
                    }
                }
                ch => {
                    output.push(ch);
                    index += 1;
                }
            }
        }
    }
    output
}

fn unescape_double_quoted_backslashes(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        // The escaped-backslash marker (\x14) from decode_double_quotes...
        // resolves to a literal backslash here. The mut path already did
        // this via restore_protected_replacement_quotes; the non-mut path
        // reaches this function with \x14 still intact.
        if ch == crate::executor::markers::DATA_BACKSLASH {
            output.push('\\');
            continue;
        }
        if ch == '\\' {
            if let Some(next) = chars.peek().copied() {
                // `}` joins the escapable set because a double-quoted
                // ${...} alternate already lost one escaping level while
                // the body was extracted (subst.c): GNU gives `}z` for
                // `"${IFS+\}z}"` (posixexp2 cases 9/14/15).
                // `\\` is no longer in the set: decode_double_quotes...
                // already converted `\\` to \x14, so a bare `\\` reaching
                // here is two literal backslashes (from \x\x or expansion
                // data) and must NOT be collapsed to `\`.
                if matches!(next, '$' | '`' | '"' | '}' | '\n') {
                    chars.next();
                    if next != '\n' {
                        output.push(next);
                    }
                    continue;
                }
            }
        }
        output.push(ch);
    }
    output
}

// Post-expansion backslash handling for a ${op word} alternate. Quote
// removal already ran on the raw alternate (decode_double_quotes...); the
// expansion result itself is data and only needs its remaining escapes
// resolved (GNU subst.c never quote-removes expansion results).
pub(in crate::executor) fn unescape_parameter_operator_result(
    word: &str,
    context: SubstitutionQuoteContext,
    ifs: Option<&str>,
) -> String {
    let unescaped = if matches!(context, SubstitutionQuoteContext::DoubleQuoted) {
        unescape_double_quoted_backslashes(word)
    } else {
        unescape_remaining_shell_escapes(word)
    };
    // GNU parameter_brace_expand carries `quoted` into the operator word's
    // expansion (subst.c): inside "${name op word}" the whole result is
    // quote-protected, so its IFS characters are data for field splitting --
    // `"${x:-$(echo "foo bar")}"` stays one word (exp.tests:222).
    if matches!(context, SubstitutionQuoteContext::DoubleQuoted) {
        crate::executor::command_substitution_values::protect_ifs_field_chars(&unescaped, ifs)
    } else {
        unescaped
    }
}

#[cfg(test)]
mod scanner_tests {
    use super::scan_word_prefix_quote_state;

    #[test]
    fn cs_span_in_plain_word() {
        let word = "A: $(printf '<%s> ' ${w=a\\ b}) | x";
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], false);
        assert!(cs, "plain-word CS body must be inside_cs, dq={dq}");
        assert!(!dq);
    }

    #[test]
    fn cs_span_in_quoted_word() {
        let word = format!(
            "{}{}",
            crate::executor::markers::STORAGE_WORD_PREFIX_STR,
            "A: $(printf '<%s> ' ${w=a\\ b}) | x"
        );
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], true);
        assert!(cs);
        assert!(!dq, "CS-local quoting, not the outer dquote");
    }

    #[test]
    fn direct_dquote_body() {
        let word = format!(
            "{}{}",
            crate::executor::markers::STORAGE_WORD_PREFIX_STR,
            "${v=a\\ b}"
        );
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], true);
        assert!(dq);
        assert!(!cs);
    }

    #[test]
    fn cs_closes_and_next_body_is_outer() {
        let word = format!(
            "{}{}",
            crate::executor::markers::STORAGE_WORD_PREFIX_STR,
            "A: $(f) ${v=a\\ b}"
        );
        let start = word.find("${").unwrap();
        let (dq, cs) = scan_word_prefix_quote_state(&word[..start], true);
        assert!(!cs, "body after the CS span is outer");
        assert!(dq);
    }
}
