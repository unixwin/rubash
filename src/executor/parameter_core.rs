use super::*;
use crate::executor::assignment_expansion::{hoist_data_double_quotes, hoist_data_single_quotes};
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    pub(in crate::executor) fn is_brace_expand_enabled(&self) -> bool {
        crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "braceexpand")
    }
    pub(in crate::executor) fn expand_word_mut(&mut self, word: &str) -> String {
        self.expand_word_mut_with_context(word, SubstitutionQuoteContext::Unquoted)
    }

    /// Typed boundary for substitution-bearing words; ordinary expansion keeps
    /// the existing String API until its caller can preserve fragment provenance.
    pub(in crate::executor) fn expand_word_mut_typed_with_context(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<ExpandedWord> {
        if let Some(output) = self.expand_backtick_substitution_typed(
            word,
            matches!(context, SubstitutionQuoteContext::DoubleQuoted),
        ) {
            let mut expanded = ExpandedWord::default();
            expanded.append_substitution(output);
            return Some(expanded);
        }
        None
    }

    pub(in crate::executor) fn expand_word_mut_with_context(
        &mut self,
        word: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        // One cross-pass subscript-eval memo scope per word expansion:
        // the `:=` pre-scan, assignment apply, and real expansion of the
        // same `${}` fragment share its single GNU evaluation. `${}` body
        // expansions reaching here inside an enclosing word keep that
        // word's context so their nested sites stay under its path.
        let _xpass = crate::executor::expand_braced_indices::SubXpassFrame::new();
        let _wctx = crate::executor::expand_braced_indices::WordCtxGuard::new_if_absent();
        self.apply_parameter_assignment_expansions_in_word(word);

        if let Some(word) = word.strip_prefix(crate::executor::markers::QUOTED_WORD_PREFIX) {
            return self.expand_embedded_parameters_mut(word);
        }

        if let Some(word) = word.strip_prefix(STORAGE_WORD_PREFIX) {
            //  marks a quoted word; quoted defaults never tilde-expand.
            return self
                .expand_quoted_parameter_word_mut(word, SubstitutionQuoteContext::DoubleQuoted);
        }

        // GNU general.c:480 assignment(): whether a word is an assignment is
        // decided on the raw token — legal_variable_starter, then
        // legal_variable_chars, then an optional bracketed subscript. A `$`
        // outside a subscript bracket (`$x=v`, `a$b=v`, `A:$((i++)) i=v`) is
        // never an assignment name, so the word expands once as an ordinary
        // word. Expanding the name portion here to test it applied expansion
        // side effects ($((i++)), $(...) writes) for non-assignment words and
        // then discarded the result, so the word expanded twice below.
        // Static assignment words still route through split_assignment_word;
        // `name[$i]=v` reaches the same element-assignment result via the
        // single generic expansion.

        if let Some((name, value)) = split_assignment_word(word) {
            // GNU general.c:480 assignment() only marks an UNQUOTED token
            // W_ASSIGNMENT. The lexer signals the quoted case by placing
            // \x1c (QUOTED_ASSIGNMENT_VALUE) right after `=`
            // (word.rs mark_quoted_assignment_value): such a word is one
            // ordinary word — `a=` is literal text and the RHS is not an
            // assignment RHS. Taking the assignment path below would hoist
            // `'`-quotes inside `$(...)` bodies to SQ_DATA sentinels and
            // smuggle them into the nested parse as data (the PUA byte then
            // became a command name, rubash#117). Expand it as a plain word
            // with the caller's quote context instead.
            if let Some(rhs) = value.strip_prefix(tilde_expand::QUOTED_ASSIGNMENT_VALUE) {
                return self.expand_embedded_parameters_mut_with_context(
                    &format!("{name}={rhs}"),
                    context,
                );
            }
            let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
            let raw_value = value
                .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(value);
            // GNU subst.c:4357 expand_string_assignment (W_ASSIGNMENT,
            // subst.c:11432): unquoted element values of a compound
            // assignment undergo the assignment tilde pass on their RAW
            // text, before parameter expansion, so tilde text produced by
            // $params is never re-expanded (array.tests: `declare -a
            // n=([0]=~/a:~/b)` stores the expanded paths while
            // `n=([0]=~/a [1]=$p)` keeps $p's result literal). Quoted
            // elements stay literal.
            let tilde_raw_owned;
            let raw_value = if raw_value.starts_with('(') && raw_value.ends_with(')') {
                tilde_raw_owned = self.expand_tilde_in_compound_assignment(name, raw_value);
                &tilde_raw_owned
            } else {
                raw_value
            };
            if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(raw_value) {
                let marker = if compound_assignment {
                    COMPOUND_ASSIGNMENT_MARKER.to_string()
                } else {
                    String::new()
                };
                return format!("{name}={marker}{expanded}");
            }
            if let Some(expanded) = self.expand_compound_positional_at_assignment(raw_value, false)
            {
                let marker = if compound_assignment {
                    COMPOUND_ASSIGNMENT_MARKER.to_string()
                } else {
                    String::new()
                };
                return format!("{name}={marker}{expanded}");
            }
            // A CA-marked compound value without expansions must reach the
            // array storage verbatim: the expansion pass performs assignment
            // quote removal, which destroys the element quote grouping the
            // storage parser needs (declare -a e=([0]="x y") must keep one
            // element, GNU arrayfunc.c). Words containing $ or ` still take
            // the expansion path below. Unmarked compound values (including
            // the declare -a d='(...)' whole-single-quoted form) need a
            // parser-side marker instead; do NOT widen this guard, it would
            // suppress glob and brace expansion inside compound values.
            if compound_assignment && !value.contains('$') && !value.contains('`') {
                return format!("{name}={COMPOUND_ASSIGNMENT_MARKER}{raw_value}");
            }
            // Raw `"` surviving in the token value are single-quote DATA
            // (remove_shell_quotes consumed the active delimiters). Hoist
            // them across the embedded-parameter re-scan exactly like the
            // assignment-storage path, or an argument word shaped
            // `echo K='a"b'` loses the quote when this re-scan re-reads the
            // bare quote as a delimiter.
            const DQ_DATA: &'static str = crate::executor::markers::ASSIGN_DATA_DQUOTE_STR;
            // GNU arrayfunc.c:581 parse_string_to_word_list preserves the
            // W_QUOTED flag on each compound-assignment word; the expansion
            // pass expands words individually. Rubash expands the whole body
            // as one string, whose quote removal consumes `'` delimiters
            // and destroys single-quote word grouping (`foo=('a b' 1 "$v1" 2)`
            // would lose the `'a b'` boundary). Hoist `'` to a sentinel
            // before expansion and restore after, exactly as DQ_DATA does
            // for `"`.
            // \u{E303} is DATA_BACKTICK in assignment_expansion.rs; use a
            // free codepoint or the sentinel decodes as a backtick.
            const SQ_DATA: &'static str = crate::executor::markers::ASSIGN_HOISTED_SQUOTE_STR;
            // Quotes inside a `$(...)`/backtick body are syntax for the
            // nested parse (subst.c:7143 command_substitute re-parses the
            // body; parse.y parse_comsub PST_NOEXPAND keeps them out of the
            // outer pass), never data to hoist — the sentinel would reach
            // the comsub source verbatim. Same invariant as
            // expand_assignment_value_hoisting's `$(`-guard.
            // GNU arrayfunc.c:557 expand_compound_array_assignment tokenizes
            // the raw parenthesized text first; the preserve variant keeps
            // element quote syntax through the walker so the storage
            // tokenizer sees GNU's raw words, covering the `$(`/backtick
            // bodies the hoist guard excludes (assoc11.sub quote elements,
            // `d=(x $(echo 'y z') w)` assoc glue).
            let expanded = if compound_assignment {
                self.expand_compound_assignment_parameters_mut(&format!(
                    "{COMPOUND_ASSIGNMENT_MARKER}{raw_value}"
                ))
            } else {
                let needs_hoist = !raw_value.contains("$(") && !raw_value.contains('`');
                let hoisted = if needs_hoist {
                    hoist_data_single_quotes(&hoist_data_double_quotes(raw_value, DQ_DATA), SQ_DATA)
                } else {
                    raw_value.to_string()
                };
                crate::executor::assignment_expansion::restore_sq_content_markers(
                    self.expand_embedded_parameters_mut(&hoisted)
                        .replace(DQ_DATA, "\"")
                        .replace(SQ_DATA, "'"),
                )
            };
            // A compound `( ... )` RHS already took its per-element tilde
            // pass (assign_assoc_from_kvlist key/value split,
            // arrayfunc.c:630) — a whole-text `:`-tilde would wrongly
            // expand `~` inside key-position elements like `p:~/r` that
            // GNU leaves literal.
            if !compound_assignment
                && !expanded.contains('=')
                && tilde_expand::assignment_value_needs_tilde_expansion(raw_value, true)
                && (self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_POSIX_MODE")
                    .map(String::as_str)
                    != Some("1")
                    || expanded.starts_with("~/"))
            {
                return format!("{name}={}", self.expand_assignment_tilde(&expanded));
            }

            return format!("{name}={expanded}");
        }

        if let Some(expression) = word
            .strip_prefix("$((")
            .and_then(|rest| rest.strip_suffix("))"))
        {
            if let Some(value) = self.eval_arithmetic_expansion_value(expression) {
                return value.to_string();
            }
            // A command-list separator is GNU's recognition-failure path for
            // POSIX command substitution, not an arithmetic runtime error.
            if expression.contains(';') {
                let command_source = format!("({expression})");
                return self.expand_command_substitution_mut_with_context(&command_source, context);
            }
            let actual_fatal = self
                .shell_state
                .arithmetic_last_error_category
                .take()
                .is_some();
            if actual_fatal
                || crate::executor::arithmetic::arithmetic_expansion_is_fatal(expression)
            {
                self.shell_state.arithmetic_fatal_error.set(true);
                if !self.shell_state.arithmetic_expansion_error.replace(true) {
                    // GNU evalexp reports against the post-expansion string
                    // (expand_arith_string ran before it); the captured eval
                    // input echoes `$var` values, not the literal text.
                    let eval_input = self.arithmetic_last_eval_input.borrow().clone();
                    let display = if eval_input.is_empty() {
                        expression
                    } else {
                        eval_input.as_str()
                    };
                    let message = crate::executor::arithmetic::arithmetic_error_message(
                        display,
                        true,
                        &self.shell_state.env_vars,
                    )
                    .unwrap_or_else(|| {
                        format!(
                            "{display}: syntax error in expression (error token is \"{display}\")"
                        )
                    });
                    eprintln!("{}{}", self.diagnostic_prefix(), message);
                }
                return String::new();
            }
            // A `set -u` unbound-variable failure in the arithmetic context is
            // fatal in GNU Bash (expr.c expr_streval raises FORCE_EOF): the
            // diagnostic was already printed and the enclosing command list is
            // abandoned. Do not fall through to the command-substitution
            // retry below — GNU never re-parses the expansion text as a
            // command, and doing so produced a spurious
            // `b: command not found` (issue #67).
            if self.shell_state.arithmetic_nounset_error.get() {
                return String::new();
            }
            // GNU subst.c retries unrecognized arithmetic syntax as command substitution.
            // Keep the parenthesized command source so nested subshell delimiters
            // are balanced during the fallback parse.
            let command_source = format!("({expression})");
            self.expand_command_substitution_mut_with_context(&command_source, context);
        }

        // Current-shell forms are special `${...}` expansions, not ordinary
        // parameter names. GNU param_expand recognizes them before the generic
        // whole-word braced-parameter path. A funsub nested inside an outer
        // parameter form (`${word-${ echo x; }}`) stays on the parameter
        // path, whose alternate expansion executes it.
        if word_contains_current_shell_command_substitution(word) && funsub_span_is_top_level(word)
        {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        }

        // A whole-word `${...}` must go to the braced parameter expander.
        // Routing it through expand_embedded_parameters_mut re-collects the
        // same `${...}` and calls expand_word_mut again, recursing forever
        // (`echo ${foo:-$(echo x)}` overflowed the stack in comsub.tests).
        // The mutable expander preserves prompt/preexec side effects such as
        // Starship's `${var:$((var="$(cmd)",0)):0}` PS0 assignment.
        if word
            .strip_prefix("${")
            .and_then(|rest| rest.strip_suffix('}'))
            .is_some()
        {
            let posix_dquote = matches!(context, SubstitutionQuoteContext::DoubleQuoted)
                && self.posix_mode_enabled();
            let spans = if posix_dquote {
                braced_parameter_spans_whole_word_in_context(word, true, true)
            } else {
                braced_parameter_spans_whole_word(word)
            };
            if spans {
                return self.expand_quoted_parameter_word_mut(word, context);
            }
        }

        if word.contains("$((") || word.contains("$[") {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        }

        if let Some(source) = word
            .strip_prefix("$(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            if command_substitution_spans_whole_word(word) {
                return self
                    .expand_command_substitution_mut_typed_with_context(source, context)
                    .text_lossy();
            }
        }

        // Embedded $() substitutions may contain full command lists or
        // compound commands, so use the mutable path that can execute an AST.
        if word.contains("$(") {
            return self.expand_embedded_parameters_mut_with_context(word, context);
        }

        if word.contains('`') {
            if let Some(expanded) = self.expand_word_mut_typed_with_context(word, context) {
                return expanded.materialize_lossy_at_boundary();
            }
        }

        self.expand_word(word)
    }

    pub(in crate::executor) fn expand_parameter_named_value(&self, name: &str) -> String {
        match name {
            "#" => return self.shell_state.positional_params.len().to_string(),
            // GNU string_list_dollar_star (subst.c): `*` joins with IFS[0]
            // in scalar contexts (assignments, quoted joins); the same rule
            // the unbraced `$*` walker path applies. `@` stays space-joined.
            "@" => return self.shell_state.positional_params.join(" "),
            "*" => return self.positional_params_star_joined(),
            "?" => return self.exit_code.to_string(),
            "$" => return self.shell_pid_value().to_string(),
            "!" => return self.last_background_pid_value(),
            "-" => return self.shell_option_flags(),
            "0" => return self.script_name_value(),
            _ => {}
        }

        if let Ok(index) = name.parse::<usize>() {
            return self
                .shell_state
                .positional_params
                .get(index.saturating_sub(1))
                .cloned()
                .unwrap_or_default();
        }

        if is_shell_name(name) {
            return self
                .dynamic_parameter_value(name)
                .or_else(|| {
                    self.shell_variable_value(name)
                        .map(|value| shell_safe_value(&value))
                })
                .unwrap_or_default();
        }

        String::new()
    }

    pub(in crate::executor) fn parse_parameter_substring<'a>(
        &self,
        name: &'a str,
    ) -> Option<(&'a str, isize, Option<isize>)> {
        // Use split_top_level_colon to skip `:` inside $(...), ${...}, and
        // quoted regions — a plain split_once(':') would match `$( : )` in
        // `${x+ab "$( : )"}` and misroute a `+`-operator word to the substring
        // path (quote.tests quote3.sub).
        let (var_name, rest, has_colon) = split_top_level_colon(name);
        if !has_colon {
            return None;
        }
        let var_name = var_name.trim_end();
        // GNU string_extract (parse.y) stops the variable name at the first
        // character in "#%^,:-=?+/@}" — so a name containing `/` (e.g.
        // `XPATH//` from `${XPATH//:/ }`) is a pattern substitution, not a
        // substring. Reject names with operator characters before attempting
        // substring parsing. `#` is excluded because it is a valid special
        // parameter name (${#} = positional param count).
        if var_name.is_empty()
            || var_name
                .chars()
                .any(|c| matches!(c, '/' | '%' | '^' | ',' | '~'))
            // GNU subst.c parameter_brace_expand: string_extract stops the
            // variable name at the first character in `#%^,:-=?+/@}`, so a
            // `:` inside an operator word (`${var+"a: b"}`) never reaches
            // the substring path — the operator parse owns it. Require a
            // valid parameter name before `:` (covers shell names, special
            // parameters, positional digits, `!name`, array subscripts).
            || !crate::executor::parameter_ops::is_parameter_error_name(var_name)
        {
            return None;
        }
        if matches!(rest.chars().next(), Some('=' | '+' | '?')) {
            return None;
        }
        if rest.starts_with('-') {
            return None;
        }

        // Split offset/length on a *top-level* `:` only: `${v:${w:-4}}` has
        // offset `${w:-4}` whose inner `:` is default-value syntax, not the
        // slice separator (Bash extracts nested `${...}` as one unit).
        let (offset_str, length_str, has_length) = split_top_level_colon(rest);
        let offset_str = offset_str.trim_start();
        if offset_str.is_empty() && length_str.is_empty() && !has_length {
            return None;
        }

        let offset = if offset_str.is_empty() {
            0
        } else {
            match self.eval_parameter_substring_offset(offset_str) {
                Some(value) => value,
                None => {
                    // GNU subst.c verify_substring_values: an invalid offset
                    // expression is an arithmetic error (expok == 0), not a
                    // "not a substring" result. parameter_brace_substring
                    // returns &expand_param_error, which propagates as an
                    // expansion error. Returning None here would let the
                    // caller fall through to non-substring handlers (e.g.
                    // ${#} length), producing wrong output.
                    if self
                        .shell_state
                        .arithmetic_last_error_category
                        .take()
                        .is_some()
                    {
                        self.report_substring_arithmetic_error(var_name, offset_str);
                        return Some((var_name, 0, Some(0)));
                    }
                    return None;
                }
            }
        };
        let length = if !has_length {
            None
        } else if length_str.is_empty() {
            Some(0)
        } else {
            match self.eval_parameter_substring_offset(length_str) {
                Some(value) => Some(value),
                None => {
                    if self
                        .shell_state
                        .arithmetic_last_error_category
                        .take()
                        .is_some()
                    {
                        self.report_substring_arithmetic_error(var_name, length_str);
                        return Some((var_name, 0, Some(0)));
                    }
                    return None;
                }
            }
        };

        Some((var_name, offset, length))
    }

    pub(in crate::executor) fn eval_parameter_substring_offset(
        &self,
        value: &str,
    ) -> Option<isize> {
        let expression = value
            .strip_prefix("$((")
            .and_then(|inner| inner.strip_suffix("))"))
            .or_else(|| {
                value
                    .strip_prefix('(')
                    .and_then(|inner| inner.strip_suffix(')'))
            })
            .unwrap_or(value)
            .trim();
        // GNU subst.c:11395-11404 expand_array_subscript (under Q_ARITH):
        // a `name[sub]` inside the offset/length expands its subscript once
        // and backslash-quotes the products (abstab), so `A[$k]` with
        // k=`$(echo %)` keys on the literal `$(echo %)`. Encoding the assoc
        // key FIRST matters: if the $-passes below ran on `A[$k2]` first,
        // the raw `$(echo %)` product would sit inside the brackets and the
        // encoder would execute it a second time.
        let expression = self.expand_substring_assoc_subscripts(&expression);
        let expression = self.expand_arithmetic_special_parameters(&expression);
        // Expand nested parameter expansions in the offset/length expression
        // first: `${v:${w:-4}}` has offset `${w:-4}` which must become `4`
        // before arithmetic evaluation (Bash evaluates the slice offset as
        // an arithmetic expression after parameter expansion).
        let expression = self.expand_embedded_parameters(&expression);
        let (evaluated, category) =
            eval_conditional_arith_value_categorized(&expression, &self.shell_state.env_vars);
        if evaluated.is_none() {
            self.shell_state
                .arithmetic_last_error_category
                .set(category);
            // Save the expanded expression so report_substring_arithmetic_error
            // can use it — GNU evalexp operates on the expanded text, so the
            // error token must come from the post-expansion form (e.g.
            // `${HOME:`echo }`}` → offset `}` not `` `echo }` ``).
            *self.arithmetic_last_error_expression.borrow_mut() = expression.to_string();
        }
        isize::try_from(evaluated?).ok()
    }

    /// `&self` counterpart of
    /// [`Executor::expand_arithmetic_assoc_subscripts`] for the substring
    /// offset/length scan: finds `name[sub]` references to associative
    /// variables and replaces the subscript with the opaque encoded key
    /// after one `expand_subscript_string` pass.
    fn expand_substring_assoc_subscripts(&self, expression: &str) -> String {
        let bytes = expression.as_bytes();
        if !bytes.contains(&b'[') {
            return expression.to_string();
        }
        let mut output = String::with_capacity(expression.len());
        let mut index = 0usize;
        while index < bytes.len() {
            let ch = bytes[index];
            if !(ch.is_ascii_alphabetic() || ch == b'_') {
                let next = expression[index..].chars().next().unwrap_or_default();
                output.push(next);
                index += next.len_utf8();
                continue;
            }
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &expression[start..index];
            if index < bytes.len()
                && bytes[index] == b'['
                && is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
            {
                if let Some(close) = expression[index..].find(']').map(|p| index + p) {
                    let raw = &expression[index + 1..close];
                    if !raw.is_empty() {
                        let key = self.expand_subscript_string(raw);
                        output.push_str(name);
                        output.push('[');
                        output.push_str(&crate::executor::arithmetic::encode_arithmetic_assoc_key(
                            &key,
                        ));
                        output.push(']');
                        index = close + 1;
                        continue;
                    }
                }
            }
            output.push_str(name);
        }
        output
    }

    pub(in crate::executor) fn parse_parameter_substring_mut<'a>(
        &mut self,
        name: &'a str,
    ) -> Option<(&'a str, isize, Option<isize>)> {
        let (var_name, rest, has_colon) = split_top_level_colon(name);
        if !has_colon {
            return None;
        }
        let var_name = var_name.trim_end();
        // GNU string_extract (parse.y) stops the variable name at the first
        // character in "#%^,:-=?+/@}" — so a name containing `/` (e.g.
        // `XPATH//` from `${XPATH//:/ }`) is a pattern substitution, not a
        // substring. Reject names with operator characters before attempting
        // substring parsing. `#` is excluded because it is a valid special
        // parameter name (${#} = positional param count).
        if var_name.is_empty()
            || var_name
                .chars()
                .any(|c| matches!(c, '/' | '%' | '^' | ',' | '~'))
            // GNU subst.c parameter_brace_expand: string_extract stops the
            // variable name at the first character in `#%^,:-=?+/@}`, so a
            // `:` inside an operator word (`${var+"a: b"}`) never reaches
            // the substring path — the operator parse owns it. Require a
            // valid parameter name before `:` (covers shell names, special
            // parameters, positional digits, `!name`, array subscripts).
            || !crate::executor::parameter_ops::is_parameter_error_name(var_name)
        {
            return None;
        }
        if matches!(rest.chars().next(), Some('=' | '+' | '?')) {
            return None;
        }
        if rest.starts_with('-') {
            return None;
        }

        let (offset_str, length_str, has_length) = split_top_level_colon(rest);
        let offset_str = offset_str.trim_start();
        if offset_str.is_empty() && length_str.is_empty() && !has_length {
            return None;
        }

        let offset = if offset_str.is_empty() {
            0
        } else {
            match self.eval_parameter_substring_offset_mut(offset_str) {
                Some(value) => value,
                None => {
                    // GNU subst.c verify_substring_values: an invalid offset
                    // expression is an arithmetic error (expok == 0), not a
                    // "not a substring" result. parameter_brace_substring
                    // returns &expand_param_error, which propagates as an
                    // expansion error. Returning None here would let the
                    // caller fall through to non-substring handlers (e.g.
                    // ${#} length), producing wrong output.
                    if self
                        .shell_state
                        .arithmetic_last_error_category
                        .take()
                        .is_some()
                    {
                        self.report_substring_arithmetic_error(var_name, offset_str);
                        return Some((var_name, 0, Some(0)));
                    }
                    return None;
                }
            }
        };
        let length = if !has_length {
            None
        } else if length_str.is_empty() {
            Some(0)
        } else {
            match self.eval_parameter_substring_offset_mut(length_str) {
                Some(value) => Some(value),
                None => {
                    if self
                        .shell_state
                        .arithmetic_last_error_category
                        .take()
                        .is_some()
                    {
                        self.report_substring_arithmetic_error(var_name, length_str);
                        return Some((var_name, 0, Some(0)));
                    }
                    return None;
                }
            }
        };

        Some((var_name, offset, length))
    }

    fn eval_parameter_substring_offset_mut(&mut self, value: &str) -> Option<isize> {
        let expression = value
            .strip_prefix("$((")
            .and_then(|inner| inner.strip_suffix("))"))
            .or_else(|| {
                value
                    .strip_prefix('(')
                    .and_then(|inner| inner.strip_suffix(')'))
            })
            .unwrap_or(value)
            .trim();
        // Same expand_array_subscript protection as the &self variant:
        // encode `name[sub]` assoc subscripts before the $-passes, so
        // expansion products inside the subscript stay verbatim keys.
        let expression = self.expand_arithmetic_assoc_subscripts(&expression, false);
        let expression = self.expand_arithmetic_special_parameters(&expression);
        let expression = self.expand_embedded_parameters_mut(&expression);
        let evaluated = self.eval_arithmetic_expansion_value(&expression);
        if evaluated.is_none() {
            // Save the expanded expression for report_substring_arithmetic_error
            // (same rationale as eval_parameter_substring_offset).
            *self.arithmetic_last_error_expression.borrow_mut() = expression.to_string();
        }
        isize::try_from(evaluated?).ok()
    }

    /// GNU subst.c parameter_brace_substring sets `this_command_name` to the
    /// parameter name before calling verify_substring_values → evalexp. When
    /// evalexp fails (expr.c evalerror), the diagnostic is formatted as
    /// `<varname>: <expression>: <msg> (error token is "<token>")` — the
    /// varname prefix comes from `this_command_name`, the expression from the
    /// offset/length text. Rubash mirrors this by prepending the varname to
    /// the standard `arithmetic_error_message` output.
    fn report_substring_arithmetic_error(&self, var_name: &str, expression: &str) {
        self.shell_state.arithmetic_fatal_error.set(true);
        if !self.shell_state.arithmetic_expansion_error.replace(true) {
            // Prefer the expanded expression saved by eval_parameter_substring_offset
            // — GNU evalexp runs after parameter/command substitution, so the
            // error token must come from the post-expansion form (e.g.
            // `${HOME:`echo }`}` → `}` not `` `echo }` ``).
            let saved = self.arithmetic_last_error_expression.borrow().clone();
            let expression = if saved.is_empty() {
                expression.to_string()
            } else {
                saved
            };
            let expression = expression.as_str();
            let mut message = crate::executor::arithmetic::arithmetic_error_message(
                expression,
                true,
                &self.shell_state.env_vars,
            )
            .unwrap_or_else(|| {
                format!(
                    "{expression}: syntax error in expression (error token is \"{expression}\")"
                )
            });
            // GNU expr.c: when the expression is entirely an operator with no
            // left operand (e.g. `${#:%}` where the offset is `%`), the parser
            // raises "arithmetic syntax error: operand expected" from expvalue
            // (expr.c:1120), not "arithmetic syntax error in expression" from
            // trailing input (expr.c:485). Rubash's arithmetic_error_message
            // checks trailing_input_token before trailing_operator_error, so a
            // bare operator is misclassified as trailing input. Detect the
            // parse-failed-entirely case (trailing token == entire expression)
            // and fix the message to match GNU.
            if message.contains("arithmetic syntax error in expression") {
                if let Some((token, _)) =
                    crate::executor::arithmetic::trailing_input_token(expression)
                {
                    if token.trim() == expression.trim() {
                        let command_context = self
                            .shell_state
                            .env_vars
                            .get("__RUBASH_IS_C")
                            .map(String::as_str)
                            != Some("1");
                        let operand_expected = if command_context {
                            "arithmetic syntax error: operand expected"
                        } else {
                            "syntax error: operand expected"
                        };
                        let display = expression.trim_start();
                        message =
                            format!("{display}: {operand_expected} (error token is \"{token}\")");
                    }
                }
            }
            eprintln!("{}{}: {}", self.diagnostic_prefix(), var_name, message);
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
    }
}

/// Splits a slice rest (`offset[:length]`) on the first *top-level* colon,
/// skipping `:` inside nested `${...}` groups: `${v:${w:-4}}` must split on
/// the colon after `v`, not on the `:` inside `${w:-4}`.
fn split_top_level_colon(input: &str) -> (&str, &str, bool) {
    // GNU subst.c skip_to_delim (subst.c:2198-2300) finds the offset/length
    // separator colon with full shell-syntax awareness. In arithmetic context
    // (SD_ARITHEXP, used by parameter_brace_substring) it additionally:
    //   - counts each top-level `?` so the *following* `:` is treated as the
    //     ternary's own separator, not the slice separator (subst.c:2254-2264);
    //   - skips a whole `(...)` group via extract_delimited_string
    //     (subst.c:2282-2296), so colons inside parens are literal data.
    // Without this, `${v:j?1:0:j}` split as offset `j?1` / length `0:j`
    // instead of GNU's offset `j?1:0` / length `j`.
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut skipcol = 0usize;
    let mut escaped = false;
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        let ch = bytes[index];
        if ch == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == b'$' && bytes.get(index + 1) == Some(&b'{') {
            brace_depth += 1;
            index += 2;
            continue;
        }
        if ch == b'}' && brace_depth > 0 {
            brace_depth -= 1;
            index += 1;
            continue;
        }
        if brace_depth == 0 {
            if ch == b'(' {
                paren_depth += 1;
                index += 1;
                continue;
            }
            if ch == b')' && paren_depth > 0 {
                paren_depth -= 1;
                index += 1;
                continue;
            }
            if paren_depth == 0 {
                if ch == b'?' {
                    skipcol += 1;
                    index += 1;
                    continue;
                }
                if ch == b':' {
                    if skipcol > 0 {
                        skipcol -= 1;
                        index += 1;
                        continue;
                    }
                    return (&input[..index], &input[index + 1..], true);
                }
            }
        }
        index += 1;
    }
    (input, "", false)
}

pub(in crate::executor) fn current_shell_command_substitution_span(word: &str) -> Option<&str> {
    let marker = "$";
    let mut search_start = 0usize;
    while let Some(relative) = word.get(search_start..)?.find(marker) {
        let start = search_start + relative;
        if word.as_bytes().get(start + 1) != Some(&b'{') {
            search_start = start + 1;
            continue;
        }
        let after_open = start + 2;
        let first = word.get(after_open..)?.chars().next()?;
        // Bash 5.3 param_expand (parser.h FUNSUB_CHAR) treats `${` followed
        // by whitespace or `|` as a foreground current-shell command
        // substitution (`${ command; }` / `${|command;}`), not a parameter
        // expansion. Every other character keeps the classic parameter
        // spelling (`${-3}`, `${#:}` stay `bad substitution` candidates).
        if !(first == '|' || first.is_whitespace()) {
            search_start = after_open;
            continue;
        }
        let bytes = word.as_bytes();
        let mut index = after_open;
        let mut depth = 1usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        while index < bytes.len() {
            let ch = bytes[index] as char;
            if escaped {
                escaped = false;
                index += 1;
                continue;
            }
            if ch == '\\' && !single {
                escaped = true;
                index += 1;
                continue;
            }
            match ch {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '$' if !single && !double && bytes.get(index + 1) == Some(&b'{') => {
                    depth += 1;
                    index += 1;
                }
                // Plain command-group braces inside the body (`${ f() { :; }
                // }`) must nest the depth as well, or the function body's `}`
                // would close the substitution span early.
                '{' if !single && !double => depth += 1,
                '}' if !single && !double => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return word.get(start..=index);
                    }
                }
                _ => {}
            }
            index += 1;
        }
        return None;
    }
    None
}

pub(in crate::executor) fn word_contains_current_shell_command_substitution(word: &str) -> bool {
    current_shell_command_substitution_span(word).is_some()
}

/// True when the word's first funsub/valsub span is not nested inside an
/// outer `${...}` parameter form. A nested span (`${word-${ echo x; }}`)
/// must expand through the outer parameter's operator machinery, not the
/// top-level funsub routing: the walker cannot run an outer parameter form,
/// and re-routing the whole word to itself recursed until the stack
/// overflowed (probe ${word-${ echo funsub; }}).
pub(in crate::executor) fn funsub_span_is_top_level(word: &str) -> bool {
    let Some(span) = current_shell_command_substitution_span(word) else {
        return false;
    };
    let base = word.as_ptr() as usize;
    let Some(start) = (span.as_ptr() as usize).checked_sub(base) else {
        return false;
    };
    let bytes = word.as_bytes();
    let mut opens = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut index = 0usize;
    while index < start.min(bytes.len()) {
        let ch = bytes[index] as char;
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => {
                single = !single;
                index += 1;
            }
            '"' if !single => {
                double = !double;
                index += 1;
            }
            '$' if !single && bytes.get(index + 1) == Some(&b'{') => {
                opens += 1;
                index += 2;
            }
            '}' if !single && !double => {
                opens = opens.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    opens == 0
}

#[cfg(test)]
mod current_shell_detector_tests {
    use super::{
        current_shell_command_substitution_span, word_contains_current_shell_command_substitution,
    };

    #[test]
    fn detects_current_shell_braced_command_body() {
        // Bash 5.3 param_expand (parser.h FUNSUB_CHAR): `${` followed by
        // whitespace or `|` opens a foreground current-shell command
        // substitution.
        let word = "${| value=new; echo alpha; echo; }";
        assert_eq!(current_shell_command_substitution_span(word), Some(word));
        assert!(word_contains_current_shell_command_substitution(word));
        assert!(word_contains_current_shell_command_substitution(
            "prefix${| echo reply }suffix"
        ));
    }

    #[test]
    fn whitespace_led_braced_body_is_current_shell() {
        // Bash 5.3: `${ printf '%s\n' aa bb cc dd; }` captures command
        // output in the current shell (comsub2.tests). Plain command-group
        // braces inside the body must not close the span early.
        assert!(word_contains_current_shell_command_substitution(
            "${ printf '%s\\n' aa bb cc dd; }"
        ));
        assert!(word_contains_current_shell_command_substitution(
            "AA${ printf 'x'; }BB"
        ));
        assert!(word_contains_current_shell_command_substitution(
            "${ func() { echo func-inside; }; }"
        ));
    }

    #[test]
    fn ignores_ordinary_braced_parameters() {
        assert!(!word_contains_current_shell_command_substitution(
            "${value}"
        ));
        assert!(!word_contains_current_shell_command_substitution(
            "${value:-fallback}"
        ));
    }
}
