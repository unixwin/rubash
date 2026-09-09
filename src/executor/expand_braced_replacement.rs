use super::*;

// Private markers used by the patsub replacement pipeline. They survive
// expand_embedded_parameters untouched and are resolved by
// finish_patsub_replacement into the final replacement text.
const PATSUB_QUOTED_VALUE_START: char = '\x0b';
const PATSUB_QUOTED_VALUE_END: char = '\x0c';
const PATSUB_QUOTED_AMP: char = '\x0e';
const PATSUB_QUOTED_BACKSLASH: char = '\x0f';

impl Executor {
    pub(in crate::executor) fn expand_braced_replacement_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        let (var_name, pattern, replacement, global) = parse_parameter_replacement(name)?;
        // GNU subst.c match_upattern applies FNMATCH_IGNCASE when nocasematch
        // is set, so pattern substitution honors the shopt (bash 4.3+).
        let pattern = self.expand_parameter_pattern_word(
            &pattern
                .replace(r"\/", "/")
                .replace('\x14', "/")
                .replace('\x18', "/"),
        );
        let replacement = self.expand_patsub_replacement_text(replacement);
        if let Some(value) =
            self.indirect_replacement_parameter(var_name, &pattern, &replacement, global)
        {
            return Some(value);
        }
        if matches!(var_name, "@" | "*") {
            return Some(
                self.positional_params
                    .iter()
                    .map(|value| self.replace_patsub_pattern(value, &pattern, &replacement, global))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let Ok(index) = var_name.parse::<usize>() {
            return Some(
                self.positional_params
                    .get(index.saturating_sub(1))
                    .map(|value| {
                        self.replace_patsub_pattern(
                            &value.replace('\x1b', ""),
                            &pattern,
                            &replacement,
                            global,
                        )
                    })
                    .unwrap_or_default(),
            );
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return Some(self.replace_patsub_pattern(&value, &pattern, &replacement, global));
        }
        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return Some(
                self.env_vars
                    .get(array_name)
                    .map(|value| {
                        let values = array_values(value)
                            .into_iter()
                            .map(|value| {
                                self.replace_patsub_pattern(&value, &pattern, &replacement, global)
                            })
                            .collect::<Vec<_>>();
                        self.join_expanded_array_values(values, var_name)
                    })
                    .unwrap_or_default(),
            );
        }
        if is_shell_name(var_name) {
            // GNU variables.c get_string_value: a bare array name expands to
            // element [0], so pattern substitution must operate on that
            // element (${av/??/xx} -> "xxcd"), not the raw typed storage
            // (previously leaked `xx[0]="abcd"` fragments). This also keeps
            // the nameref chain resolution the old inline code performed.
            return Some(
                self.parameter_pattern_scalar_value(var_name)
                    .map(|value| {
                        self.replace_patsub_pattern(&value, &pattern, &replacement, global)
                    })
                    .unwrap_or_default(),
            );
        }
        None
    }

    /// One entry point for pattern substitution: reads nocasematch and the
    /// patsub_replacement shopt (default on in bash 5.2) and runs the single
    /// replacement pass in parameter_replace.rs.
    pub(in crate::executor) fn replace_patsub_pattern(
        &self,
        value: &str,
        pattern: &str,
        replacement: &str,
        global: bool,
    ) -> String {
        replace_parameter_pattern(
            value,
            pattern,
            replacement,
            global,
            self.nocasematch_enabled(),
            crate::builtins::shopt::option_enabled(&self.env_vars, "patsub_replacement"),
        )
    }

    /// Expands the replacement side of a pat-substitution in one pass,
    /// mirroring GNU subst.c parameter_brace_patsub:9424-9444. The
    /// replacement is expanded with its own quote state intact
    /// (expand_string_for_patsub); a leading tilde is expanded
    /// (expand_string_for_pat); and quote_string_for_repl (subst.c:3891)
    /// turns quoted `&`/backslash into the `\&`/`\\` data consumed by the
    /// strcreplace pass in pat_subst.
    pub(in crate::executor) fn expand_patsub_replacement_text(
        &self,
        replacement: &str,
    ) -> String {
        let patsub_replacement =
            crate::builtins::shopt::option_enabled(&self.env_vars, "patsub_replacement");
        let chars: Vec<char> = replacement.chars().collect();
        // GNU expands a leading tilde in the replacement string regardless
        // of outer quoting and of the shopt state (new-exp16.sub P1/P2).
        if chars.first() == Some(&'~') && (replacement == "~" || replacement.starts_with("~/")) {
            if let Some(expanded) = tilde_expand::expand_word_prefix(replacement, &self.env_vars) {
                let expanded = self.expand_embedded_parameters(&expanded);
                return self.finish_patsub_replacement(&expanded, patsub_replacement);
            }
        }
        let marked = Self::mark_patsub_replacement_quotes(&chars);
        let expanded = self.expand_embedded_parameters(&marked);
        self.finish_patsub_replacement(&expanded, patsub_replacement)
    }

    /// Pre-expansion pass: remove the replacement's own quotes while
    /// recording which `&`/backslash characters were quoted (markers), and
    /// protect literal `$`/quote/backtick characters from the expander.
    fn mark_patsub_replacement_quotes(chars: &[char]) -> String {
        let mut marked = String::with_capacity(chars.len() + 8);
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            match ch {
                '\'' => {
                    // Single-quoted span: no expansions inside; `&` and `\`
                    // become quoted replacement data (CTLESC in GNU).
                    index += 1;
                    while index < chars.len() && chars[index] != '\'' {
                        push_single_quoted_replacement_char(&mut marked, chars[index]);
                        index += 1;
                    }
                    index += 1;
                }
                '"' => {
                    index += 1;
                    while index < chars.len() && chars[index] != '"' {
                        index = push_double_quoted_replacement_char(&mut marked, chars, index);
                    }
                    index += 1;
                }
                '\\' => {
                    index = push_unquoted_escape(&mut marked, chars, index);
                }
                '\x14' => {
                    // Lexer marker for an escaped backslash: one literal,
                    // quoted backslash.
                    marked.push(PATSUB_QUOTED_BACKSLASH);
                    index += 1;
                }
                '\x17' => {
                    marked.push('\x17');
                    index += 1;
                }
                '\x18' => {
                    marked.push('\x18');
                    index += 1;
                }
                '$' => {
                    // $'...' inside the ${...} body is ANSI-C quoting: the
                    // body is its own quoting context (GNU parameter_brace_patsub
                    // expands the replacement with its internal quote state).
                    // Decode it here into quoted replacement data.
                    if chars.get(index + 1) == Some(&'\'') {
                        index = push_ansi_c_replacement_span(&mut marked, chars, index);
                    } else {
                        index = push_replacement_dollar(&mut marked, chars, index, false);
                    }
                }
                '`' => {
                    let end = backtick_replacement_span_end(chars, index);
                    marked.extend(chars[index..end].iter());
                    index = end;
                }
                other => {
                    marked.push(other);
                    index += 1;
                }
            }
        }
        marked
    }

    /// Resolve the pipeline markers into the final replacement text. With
    /// patsub_replacement on, quoted `&`/backslash become the `\&`/`\\`
    /// data consumed by strcreplace (quote_string_for_repl, subst.c:3921);
    /// with the shopt off, plain quote removal applies and `&` stays
    /// literal data.
    fn finish_patsub_replacement(&self, expanded: &str, patsub_replacement: bool) -> String {
        let mut output = String::with_capacity(expanded.len());
        let mut in_quoted_value = false;
        for ch in expanded.chars() {
            match ch {
                PATSUB_QUOTED_VALUE_START => in_quoted_value = true,
                PATSUB_QUOTED_VALUE_END => in_quoted_value = false,
                PATSUB_QUOTED_AMP => {
                    if patsub_replacement {
                        output.push_str("\\&");
                    } else {
                        output.push('&');
                    }
                }
                PATSUB_QUOTED_BACKSLASH => {
                    if patsub_replacement {
                        output.push_str("\\\\");
                    } else {
                        output.push('\\');
                    }
                }
                '&' | '\\' if in_quoted_value && patsub_replacement => {
                    // Expanded value characters inside a quoted region are
                    // quoted data for the strcreplace pass (CTLESC in GNU).
                    if ch == '&' {
                        output.push_str("\\&");
                    } else {
                        output.push_str("\\\\");
                    }
                }
                other => output.push(other),
            }
        }
        output
    }

    fn indirect_replacement_parameter(
        &self,
        var_name: &str,
        pattern: &str,
        replacement: &str,
        global: bool,
    ) -> Option<String> {
        let indirect_name = var_name.strip_prefix('!')?;
        if let Some(target_name) = self.nameref_target_name(indirect_name) {
            return Some(self.replace_patsub_pattern(&target_name, pattern, replacement, global));
        }

        let target_expr = self.env_vars.get(indirect_name)?;
        let values = self.indirect_target_values(target_expr);
        if values.is_empty() {
            return Some(String::new());
        }

        let values = values
            .into_iter()
            .map(|value| self.replace_patsub_pattern(&value, pattern, replacement, global))
            .collect::<Vec<_>>();
        Some(self.join_expanded_array_values(values, target_expr))
    }
}

/// Copy a char from a single-quoted replacement span: `$` and backticks are
/// literal (no expansion inside single quotes), `&` and backslash become
/// quoted replacement data.
fn push_single_quoted_replacement_char(marked: &mut String, ch: char) {
    match ch {
        '&' => marked.push(PATSUB_QUOTED_AMP),
        '\\' | '\x14' => marked.push(PATSUB_QUOTED_BACKSLASH),
        '$' => marked.push('\x1f'),
        '`' => marked.push('\x1a'),
        // Decoded quote data must survive the expander, which drops a bare
        // quote as an unclosed span.
        '\'' | '\x17' => marked.push('\x17'),
        '"' | '\x18' => marked.push('\x18'),
        other => marked.push(other),
    }
}

/// Copy one char of a double-quoted replacement span; returns the next
/// index. Backslashes escape only `$`, backtick, `"`, and `\`; every
/// other backslash is a literal, quoted backslash.
fn push_double_quoted_replacement_char(
    marked: &mut String,
    chars: &[char],
    index: usize,
) -> usize {
    match chars[index] {
        '\\' => match chars.get(index + 1) {
            Some('$') => {
                let end = dollar_expression_end(chars, index + 1);
                marked.push(PATSUB_QUOTED_VALUE_START);
                marked.extend(chars[index + 1..end].iter());
                marked.push(PATSUB_QUOTED_VALUE_END);
                end
            }
            Some('`') => {
                marked.push('`');
                index + 2
            }
            Some('"') => {
                marked.push('\x18');
                index + 2
            }
            Some('\\') | Some('\x14') => {
                marked.push(PATSUB_QUOTED_BACKSLASH);
                index + 2
            }
            Some(other) => {
                marked.push(PATSUB_QUOTED_BACKSLASH);
                match *other {
                    // Quote data inside a double-quoted replacement span must
                    // reach the expander protected (GNU subst.c
                    // string_extract_double_quoted keeps \' literally; a bare
                    // quote here would be eaten as a span delimiter).
                    '\'' | '\x17' => marked.push('\x17'),
                    '&' => marked.push(PATSUB_QUOTED_AMP),
                    _ => marked.push(*other),
                }
                index + 2
            }
            None => {
                marked.push(PATSUB_QUOTED_BACKSLASH);
                index + 1
            }
        },
        '$' => push_replacement_dollar(marked, chars, index, true),
        '&' => {
            // Inside double quotes `&` is quoted replacement data.
            marked.push(PATSUB_QUOTED_AMP);
            index + 1
        }
        '`' => {
            let end = backtick_replacement_span_end(chars, index);
            marked.extend(chars[index..end].iter());
            end
        }
        '\x14' => {
            marked.push(PATSUB_QUOTED_BACKSLASH);
            index + 1
        }
        '\x17' => {
            marked.push('\x17');
            index + 1
        }
        '\x18' => {
            marked.push('\x18');
            index + 1
        }
        '\'' | '\x17' => {
            // Quote data inside a double-quoted replacement span: protect it
            // from the expander, which drops a bare quote as a span
            // delimiter (GNU keeps dquoted ' as literal data).
            marked.push('\x17');
            index + 1
        }
        other => {
            marked.push(other);
            index + 1
        }
    }
}

/// Emit an ANSI-C `$'...'` span starting at `index` (the `$`) as quoted
/// replacement data; returns the index after the closing quote. Escapes:
/// `\\xNN` hex, `\\'`, `\\\\`, and backslash+char passes through with the
/// backslash as quoted data.
fn push_ansi_c_replacement_span(marked: &mut String, chars: &[char], index: usize) -> usize {
    let mut cursor = index + 2; // skip $ and opening '
    while cursor < chars.len() {
        let ch = chars[cursor];
        if ch == '\'' {
            return cursor + 1;
        }
        if ch == '\\' {
            match chars.get(cursor + 1) {
                Some('x') | Some('X') => {
                    let mut value = 0u32;
                    let mut digits = 0;
                    let mut next = cursor + 2;
                    while next < chars.len() && digits < 2 {
                        match chars[next].to_digit(16) {
                            Some(d) => {
                                value = value * 16 + d;
                                digits += 1;
                                next += 1;
                            }
                            None => break,
                        }
                    }
                    if digits > 0 {
                        if let Some(decoded) = char::from_u32(value) {
                            push_single_quoted_replacement_char(marked, decoded);
                        }
                        cursor = next;
                    } else {
                        push_single_quoted_replacement_char(marked, '\\');
                        push_single_quoted_replacement_char(marked, *chars.get(cursor + 1).unwrap_or(&'x'));
                        cursor += 2;
                    }
                }
                Some(&'\'') => {
                    push_single_quoted_replacement_char(marked, '\'');
                    cursor += 2;
                }
                Some(&'\\') => {
                    push_single_quoted_replacement_char(marked, '\\');
                    cursor += 2;
                }
                Some(&other) => {
                    // Unrecognized escape keeps the backslash as data.
                    push_single_quoted_replacement_char(marked, '\\');
                    push_single_quoted_replacement_char(marked, other);
                    cursor += 2;
                }
                None => {
                    push_single_quoted_replacement_char(marked, '\\');
                    cursor += 1;
                }
            }
            continue;
        }
        push_single_quoted_replacement_char(marked, ch);
        cursor += 1;
    }
    cursor
}

/// Handle an unquoted backslash in a replacement: GNU quote removal
/// consumes the escape; `&` and backslash become quoted data markers,
/// `$`/quote/backtick become their literal protected forms.
fn push_unquoted_escape(marked: &mut String, chars: &[char], index: usize) -> usize {
    match chars.get(index + 1) {
        Some('\'') | Some('\x17') => {
            marked.push('\x17');
            index + 2
        }
        Some('"') | Some('\x18') => {
            marked.push('\x18');
            index + 2
        }
        Some('\\') | Some('\x14') => {
            marked.push(PATSUB_QUOTED_BACKSLASH);
            index + 2
        }
        Some('&') => {
            marked.push(PATSUB_QUOTED_AMP);
            index + 2
        }
        Some('$') => {
            marked.push('\x1f');
            index + 2
        }
        Some('`') => {
            marked.push('\x1a');
            index + 2
        }
        Some(_) => {
            marked.push(chars[index + 1]);
            index + 2
        }
        None => {
            marked.push(PATSUB_QUOTED_BACKSLASH);
            index + 1
        }
    }
}

/// Emit the `$`-expression starting at `index`. Variable expansions inside
/// a double-quoted span are wrapped in quoted-value markers so their
/// results count as quoted replacement data (GNU CTLESC-quoting); unquoted
/// expansions pass through raw so their results stay active.
fn push_replacement_dollar(
    marked: &mut String,
    chars: &[char],
    index: usize,
    quoted_value: bool,
) -> usize {
    let next = chars.get(index + 1).copied();
    let expands = match next {
        Some('{') | Some('(') | Some('[') => true,
        Some(c) => {
            is_shell_name_start(c)
                || c.is_ascii_digit()
                || matches!(c, '#' | '@' | '*' | '$' | '!' | '?' | '-')
        }
        None => false,
    };
    if !expands {
        // An unexpandable `$` stays literal next to the following char;
        // protect that char from being reinterpreted as quoting by the
        // expander.
        marked.push('$');
        return match chars.get(index + 1) {
            Some('\'') => {
                marked.push('\x17');
                index + 2
            }
            Some('"') => {
                marked.push('\x18');
                index + 2
            }
            Some('\\') | Some('\x14') => push_unquoted_escape(marked, chars, index + 1),
            Some('`') => {
                marked.push('\x1a');
                index + 2
            }
            Some(other) => {
                marked.push(*other);
                index + 2
            }
            None => index + 1,
        };
    }
    let end = dollar_expression_end(chars, index);
    let value_expansion = !matches!(next, Some('(') | Some('['));
    if quoted_value && value_expansion {
        marked.push(PATSUB_QUOTED_VALUE_START);
        marked.extend(chars[index..end].iter());
        marked.push(PATSUB_QUOTED_VALUE_END);
    } else {
        marked.extend(chars[index..end].iter());
    }
    end
}

/// End index (exclusive) of the `$`-expression starting at `start`.
fn dollar_expression_end(chars: &[char], start: usize) -> usize {
    match chars.get(start + 1) {
        None => start + 1,
        Some('{') => braced_replacement_body_end(chars, start + 1),
        Some('(') => dollar_paren_end(chars, start + 1),
        Some('[') => dollar_bracket_end(chars, start + 1),
        Some(c) if is_shell_name_start(*c) => {
            let mut end = start + 1;
            while chars.get(end).is_some_and(|ch| is_shell_name_char(*ch)) {
                end += 1;
            }
            end
        }
        Some(c) if c.is_ascii_digit() => start + 2,
        Some(c) if matches!(c, '#' | '@' | '*' | '$' | '!' | '?' | '-') => start + 2,
        Some(_) => start + 1,
    }
}

/// End index of a `${...}` body opened at `open` (the `{`), honoring
/// quotes, backslash escapes, and nested `${` depth.
fn braced_replacement_body_end(chars: &[char], open: usize) -> usize {
    let mut depth = 1usize;
    let mut index = open + 1;
    let mut in_single = false;
    let mut in_double = false;
    while index < chars.len() {
        let ch = chars[index];
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\\' => index += 2,
            '\'' => {
                in_single = true;
                index += 1;
            }
            '"' => {
                in_double = !in_double;
                index += 1;
            }
            '$' if chars.get(index + 1) == Some(&'{') => {
                depth += 1;
                index += 2;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    chars.len()
}

/// End index of a `$(...)` body opened at `open` (the `(`), honoring
/// quotes and nested parens.
fn dollar_paren_end(chars: &[char], open: usize) -> usize {
    let mut depth = 1usize;
    let mut index = open + 1;
    let mut in_single = false;
    let mut in_double = false;
    while index < chars.len() {
        let ch = chars[index];
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\\' => index += 2,
            '\'' => {
                in_single = true;
                index += 1;
            }
            '"' => {
                in_double = !in_double;
                index += 1;
            }
            '`' => index = backtick_replacement_span_end(chars, index),
            '(' => {
                depth += 1;
                index += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    chars.len()
}

/// End index of a `$[...]` body opened at `open` (the `[`).
fn dollar_bracket_end(chars: &[char], open: usize) -> usize {
    let mut depth = 1usize;
    let mut index = open + 1;
    while index < chars.len() {
        match chars[index] {
            '[' => {
                depth += 1;
                index += 1;
            }
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    chars.len()
}

/// End index of a backtick command substitution starting at `start`.
fn backtick_replacement_span_end(chars: &[char], start: usize) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '`' => return index + 1,
            _ => index += 1,
        }
    }
    chars.len()
}
