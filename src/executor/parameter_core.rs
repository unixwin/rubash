use super::*;

impl Executor {
    pub(in crate::executor) fn is_brace_expand_enabled(&self) -> bool {
        crate::builtins::set::shell_option_enabled(&self.env_vars, "braceexpand")
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
        self.apply_parameter_assignment_expansions_in_word(word);

        if let Some(word) = word.strip_prefix('\x1b') {
            return self.expand_embedded_parameters_mut(word);
        }

        if let Some(word) = word.strip_prefix('\x1d') {
            //  marks a quoted word; quoted defaults never tilde-expand.
            return self
                .expand_quoted_parameter_word_mut(word, SubstitutionQuoteContext::DoubleQuoted);
        }

        if let Some((raw_name, value)) = word.split_once('=') {
            let name = self.expand_embedded_parameters_mut(raw_name);
            let (base_name, _) = assignment_name_and_append(&name);
            if raw_name.contains('$')
                && !raw_name.contains(['{', '(', ')', '}'])
                && is_shell_name(base_name)
            {
                let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
                let value = tilde_expand::strip_assignment_quote_marker(value);
                if let Some(prepared) = self.expand_escaped_indirect_parameter_literal(value) {
                    return format!("{name}={}", unescape_remaining_shell_escapes(&prepared));
                }
                let expanded = self.expand_embedded_parameters_mut(value);
                if !quoted
                    && !expanded.contains('=')
                    && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
                        || expanded.starts_with("~/"))
                {
                    return format!("{name}={}", self.expand_assignment_tilde(&expanded));
                }

                return format!("{name}={expanded}");
            }
        }

        if let Some((name, value)) = split_assignment_word(word) {
            let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
            let value = tilde_expand::strip_assignment_quote_marker(value);
            if quoted {
                if let Some(expanded) = self.expand_quoted_array_assignment_value(value) {
                    return format!("{name}={expanded}");
                }
            }
            let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
            let raw_value = value
                .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(value);
            if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(raw_value) {
                let marker = if compound_assignment {
                    COMPOUND_ASSIGNMENT_MARKER.to_string()
                } else {
                    String::new()
                };
                return format!("{name}={marker}{expanded}");
            }
            if let Some(expanded) = self.expand_compound_positional_at_assignment(raw_value, quoted) {
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
            if compound_assignment
                && !value.contains('$')
                && !value.contains('`')
            {
                return format!("{name}={value}");
            }
            let expanded = self.expand_embedded_parameters_mut(value);
            if !quoted
                && !expanded.contains('=')
                && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
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
            let actual_fatal = self.arithmetic_last_error_category.take().is_some();
            if actual_fatal || crate::executor::arithmetic::arithmetic_expansion_is_fatal(expression) {
                self.arithmetic_fatal_error.set(true);
                if !self.arithmetic_expansion_error.replace(true) {
                    let message = crate::executor::arithmetic::arithmetic_error_message(expression, true)
                        .unwrap_or_else(|| format!("{expression}: syntax error in expression (error token is \"{expression}\")"));
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
            if self.arithmetic_nounset_error.get() {
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
        if word_contains_current_shell_command_substitution(word)
            && funsub_span_is_top_level(word)
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
            "#" => return self.positional_params.len().to_string(),
            "@" | "*" => return self.positional_params.join(" "),
            "?" => return self.exit_code.to_string(),
            "$" => return self.shell_pid_value().to_string(),
            "!" => return self.last_background_pid_value(),
            "-" => return self.shell_option_flags(),
            "0" => return self.script_name_value(),
            _ => {}
        }

        if let Ok(index) = name.parse::<usize>() {
            return self
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
        let (var_name, rest) = name.split_once(':')?;
        if var_name.is_empty() || matches!(rest.chars().next(), Some('=' | '+' | '?')) {
            return None;
        }
        if rest.starts_with('-') {
            return None;
        }

        // Split offset/length on a *top-level* `:` only: `${v:${w:-4}}` has
        // offset `${w:-4}` whose inner `:` is default-value syntax, not the
        // slice separator (Bash extracts nested `${...}` as one unit).
        let (offset, length, has_length) = split_top_level_colon(rest);
        let offset = offset.trim_start();
        if offset.is_empty() && length.is_empty() && !has_length {
            return None;
        }

        let offset = if offset.is_empty() {
            0
        } else {
            self.eval_parameter_substring_offset(offset)?
        };
        let length = if !has_length {
            None
        } else if length.is_empty() {
            Some(0)
        } else {
            Some(self.eval_parameter_substring_offset(length)?)
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
        let expression = self.expand_arithmetic_special_parameters(expression);
        // Expand nested parameter expansions in the offset/length expression
        // first: `${v:${w:-4}}` has offset `${w:-4}` which must become `4`
        // before arithmetic evaluation (Bash evaluates the slice offset as
        // an arithmetic expression after parameter expansion).
        let expression = self.expand_embedded_parameters(&expression);
        let evaluated = eval_conditional_arith_value(&expression, &self.env_vars)?;
        isize::try_from(evaluated).ok()
    }

    pub(in crate::executor) fn parse_parameter_substring_mut<'a>(
        &mut self,
        name: &'a str,
    ) -> Option<(&'a str, isize, Option<isize>)> {
        let (var_name, rest) = name.split_once(':')?;
        if var_name.is_empty() || matches!(rest.chars().next(), Some('=' | '+' | '?')) {
            return None;
        }
        if rest.starts_with('-') {
            return None;
        }

        let (offset, length, has_length) = split_top_level_colon(rest);
        let offset = offset.trim_start();
        if offset.is_empty() && length.is_empty() && !has_length {
            return None;
        }

        let offset = if offset.is_empty() {
            0
        } else {
            self.eval_parameter_substring_offset_mut(offset)?
        };
        let length = if !has_length {
            None
        } else if length.is_empty() {
            Some(0)
        } else {
            Some(self.eval_parameter_substring_offset_mut(length)?)
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
        let expression = self.expand_arithmetic_special_parameters(expression);
        let expression = self.expand_embedded_parameters_mut(&expression);
        let evaluated = self.eval_arithmetic_expansion_value(&expression)?;
        isize::try_from(evaluated).ok()
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
