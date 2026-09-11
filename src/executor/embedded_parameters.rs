use super::*;

thread_local! {
    static EXPAND_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

const MAX_EXPAND_DEPTH: usize = 50;

impl Executor {
    pub(in crate::executor) fn expand_embedded_parameters(&self, word: &str) -> String {
        self.expand_embedded_parameters_with_context(word, false)
    }

    // Variant for `${var-word}` style alternate words: whitespace that was
    // inside quotes is marked \x1c so the field splitter keeps it glued
    // (posixexp2 37), while bare spaces stay splittable.
    pub(in crate::executor) fn expand_embedded_parameters_protect_ifs(&self, word: &str) -> String {
        self.expand_embedded_parameters_with_context_inner(word, false, true)
    }

    pub(in crate::executor) fn expand_embedded_parameters_for_heredoc(&self, word: &str) -> String {
        self.expand_embedded_parameters_with_context(word, true)
    }

    fn expand_embedded_parameters_with_context(&self, word: &str, heredoc: bool) -> String {
        self.expand_embedded_parameters_with_context_inner(word, heredoc, false)
    }

    fn expand_embedded_parameters_with_context_inner(
        &self,
        word: &str,
        heredoc: bool,
        protect_ifs: bool,
    ) -> String {
        let depth = EXPAND_DEPTH.with(|d| d.get());
        if depth >= MAX_EXPAND_DEPTH {
            return word.to_string();
        }
        EXPAND_DEPTH.with(|d| d.set(depth + 1));
        let result = self.expand_embedded_parameters_inner(word, heredoc, protect_ifs);
        EXPAND_DEPTH.with(|d| d.set(depth));
        result
    }

    fn expand_embedded_parameters_inner(
        &self,
        word: &str,
        heredoc: bool,
        protect_ifs: bool,
    ) -> String {
        // TODO(subst.c/subst.h): This is a narrow parameter-expansion subset.
        // GNU Bash handles quoting state, operators like ${name:-word},
        // positional/special parameters, arrays, command substitution, and IFS
        // word splitting here. Keep extending this toward subst.c semantics.
        let mut output = String::new();
        let mut chars = word.chars().peekable();
        let mut in_double = false;

        while let Some(ch) = chars.next() {
            if protect_ifs && in_double && matches!(ch, ' ' | '\t' | '\n') {
                output.push('\x1c');
                output.push(ch);
                continue;
            }
            if ch == crate::lexer::PARAM_NAME_END_MARKER {
                // Lexer quote removal emits this where a quote boundary
                // terminates an unbraced $name; the name already stopped
                // (the marker is a non-name character) and must not leak
                // into the expansion output.
                continue;
            }
            if ch == '\x1a' {
                output.push('`');
                continue;
            }

            if ch == '\x1f' {
                output.push('$');
                continue;
            }

            if ch == '\x17' {
                output.push('\'');
                continue;
            }

            if ch == '\x18' {
                output.push('"');
                continue;
            }
            if ch == crate::lexer::ANSI_C_QUOTE_MARKER {
                output.push('\'');
                continue;
            }
            if ch == crate::lexer::ANSI_C_DQUOTE_MARKER {
                output.push('"');
                continue;
            }

            // GNU parse.y word scanner: outside double quotes a backslash
            // escapes the following quote, producing a quoted literal that
            // survives as data (`echo a\'b` -> a'b, `echo a\"b` -> a"b).
            // Inside double quotes `\"` is an escape for `"`; a backslash
            // before a single quote stays literal (both characters), so that
            // case falls through to the default push below. Heredoc text
            // treats quotes as data.
            if ch == '\\' && !heredoc {
                match chars.peek() {
                    Some('\'') if !in_double => {
                        chars.next();
                        output.push('\'');
                        continue;
                    }
                    Some('"') => {
                        chars.next();
                        output.push('"');
                        continue;
                    }
                    _ => {}
                }
            }

            // Quotes that survive to expansion belong to parameter-expansion
            // bodies (the lexer keeps `${...}` verbatim): GNU removes them
            // here, with single-quoted content staying literal. Heredoc text
            // treats quotes as data.
            if !heredoc && ch == '"' {
                in_double = !in_double;
                continue;
            }

            if !heredoc && ch == '\'' && !in_double {
                for quoted_ch in chars.by_ref() {
                    if quoted_ch == '\'' {
                        break;
                    }
                    if protect_ifs && matches!(quoted_ch, ' ' | '\t' | '\n') {
                        output.push('\x1c');
                    }
                    output.push(quoted_ch);
                }
                continue;
            }

            if ch == '\\' && chars.peek() == Some(&'`') {
                chars.next();
                output.push('\x1a');
                continue;
            }

            if ch == '`' {
                let mut source = String::new();
                let mut escaped = false;
                let mut closed = false;
                while let Some(source_ch) = chars.next() {
                    if escaped {
                        push_backtick_escaped_source_char(&mut source, source_ch, &mut chars);
                        escaped = false;
                        continue;
                    }
                    if source_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if source_ch == '`' {
                        closed = true;
                        break;
                    }
                    source.push(source_ch);
                }
                if closed {
                    output.push_str(&protect_command_substitution_output(
                        &self.expand_command_substitution(&decode_backtick_substitution_source(
                            &source,
                        )),
                    ));
                } else {
                    output.push('`');
                    output
                        .push_str(&self.expand_embedded_parameters_with_context(&source, heredoc));
                }
                continue;
            }

            if ch != '$' {
                output.push(ch);
                continue;
            }

            match chars.peek().copied() {
                Some('?') => {
                    chars.next();
                    output.push_str(&self.exit_code.to_string());
                }
                Some('$') => {
                    chars.next();
                    output.push_str(&self.shell_pid_value().to_string());
                }
                Some('!') => {
                    chars.next();
                    output.push_str(&self.last_background_pid_value());
                }
                Some('@') => {
                    chars.next();
                    output.push_str(&self.positional_params.join(" "));
                }
                Some('*') => {
                    chars.next();
                    output.push_str(&self.positional_params_star_joined());
                }
                Some('#') => {
                    chars.next();
                    output.push_str(&self.positional_params.len().to_string());
                }
                Some('-') => {
                    chars.next();
                    output.push_str(&self.shell_option_flags());
                }
                Some('{') => {
                    chars.next();
                    let name = collect_braced_parameter_name(&mut chars);
                    output.push_str(&self.expand_word(&format!("${{{name}}}")));
                }
                Some('(') => {
                    chars.next();
                    if chars.peek().copied() == Some('(') {
                        chars.next();
                        let mut expression = String::new();
                        let mut paren_depth: usize = 0;
                        while let Some(expression_ch) = chars.next() {
                            match expression_ch {
                                '(' => {
                                    paren_depth += 1;
                                    expression.push(expression_ch);
                                }
                                ')' if paren_depth == 0 && chars.peek().copied() == Some(')') => {
                                    chars.next();
                                    break;
                                }
                                ')' => {
                                    paren_depth = paren_depth.saturating_sub(1);
                                    expression.push(expression_ch);
                                }
                                _ => expression.push(expression_ch),
                            }
                        }
                        let expression = self.expand_arithmetic_special_parameters(&expression);
                        let (value, actual_category) =
                            eval_conditional_arith_value_categorized(&expression, &self.env_vars);
                        if let Some(value) = value {
                            output.push_str(&value.to_string());
                        } else {
                            self.arithmetic_last_error_category.set(actual_category);
                            // Bash reports arithmetic expansion errors
                            // (floating point, negative exponent, division
                            // by zero, ...) on stderr and sets rc=1; Rubash
                            // was silently dropping them. Classify fatality
                            // unconditionally: a readonly diagnostic may have
                            // consumed the print gate, but the evaluation
                            // error still decides list abandonment.
                            let actual_fatal = self.arithmetic_last_error_category.take().is_some();
                            if !actual_fatal
                                && !crate::executor::arithmetic::arithmetic_expansion_is_fatal(
                                    &expression,
                                )
                            {
                                self.arithmetic_nonfatal_error.set(true);
                            } else {
                                self.arithmetic_fatal_error.set(true);
                            }
                            if !self.arithmetic_expansion_error.replace(true) {
                                let message = crate::executor::arithmetic::arithmetic_error_message(
                                    &expression,
                                    true,
                                )
                                .unwrap_or_else(|| {
                                    format!(
                                        "{expression}: syntax error in expression (error token is \"{expression}\")"
                                    )
                                });
                                eprintln!("{}: {message}", self.diagnostic_prefix());
                            }
                        }
                        continue;
                    }
                    let mut depth = 1;
                    let mut source = String::new();
                    let mut single = false;
                    let mut double = false;
                    let mut escaped = false;
                    let mut case_depth = 0usize;
                    let mut word = String::new();
                    while let Some(source_ch) = chars.next() {
                        if escaped {
                            source.push(source_ch);
                            escaped = false;
                            continue;
                        }
                        if source_ch == '\\' && !single {
                            source.push(source_ch);
                            escaped = true;
                            continue;
                        }
                        update_command_substitution_case_depth(
                            source_ch,
                            single,
                            double,
                            &mut word,
                            &mut case_depth,
                        );
                        match source_ch {
                            '\'' if !double => {
                                single = !single;
                                source.push(source_ch);
                            }
                            '"' if !single => {
                                double = !double;
                                source.push(source_ch);
                            }
                            '<' if !single && !double && chars.peek().copied() == Some('<') => {
                                copy_command_substitution_heredoc(&mut chars, &mut source);
                            }
                            '(' if !single && !double && case_depth == 0 => {
                                depth += 1;
                                source.push(source_ch);
                            }
                            ')' if !single && !double && case_depth == 0 => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                source.push(source_ch);
                            }
                            _ => source.push(source_ch),
                        }
                    }
                    output.push_str(&protect_command_substitution_output(
                        &self.expand_command_substitution(&source),
                    ));
                }
                Some('[') => {
                    chars.next();
                    let mut expression = String::new();
                    let mut bracket_depth: usize = 0;
                    let mut closed = false;
                    for expression_ch in chars.by_ref() {
                        match expression_ch {
                            '[' => {
                                bracket_depth += 1;
                                expression.push(expression_ch);
                            }
                            ']' if bracket_depth == 0 => {
                                closed = true;
                                break;
                            }
                            ']' => {
                                bracket_depth = bracket_depth.saturating_sub(1);
                                expression.push(expression_ch);
                            }
                            _ => expression.push(expression_ch),
                        }
                    }
                    if closed {
                        let expression = self.expand_arithmetic_special_parameters(&expression);
                        if let Some(value) =
                            eval_conditional_arith_value(&expression, &self.env_vars)
                        {
                            output.push_str(&value.to_string());
                        }
                    } else {
                        output.push_str("$[");
                        output.push_str(&expression);
                    }
                }
                Some(first) if first.is_ascii_digit() => {
                    chars.next();
                    let index = first.to_digit(10).unwrap_or(0) as usize;
                    if index == 0 {
                        output.push_str(&self.script_name_value());
                    } else {
                        output.push_str(
                            self.positional_params
                                .get(index - 1)
                                .map(String::as_str)
                                .unwrap_or(""),
                        );
                    }
                }
                Some(first) if is_shell_name_start(first) => {
                    let mut name = String::new();
                    while let Some(name_ch) = chars.peek().copied() {
                        if !is_shell_name_char(name_ch) {
                            break;
                        }
                        chars.next();
                        name.push(name_ch);
                    }
                    if let Some(value) = self.dynamic_parameter_value(&name).or_else(|| {
                        self.shell_variable_value(&name)
                            .or_else(|| std::env::var(&name).ok())
                    }) {
                        let value = shell_safe_value(&value);
                        if heredoc {
                            output.push_str(&protect_command_substitution_output(&value));
                        } else {
                            output.push_str(&value);
                        }
                    }
                }
                Some(other) => {
                    chars.next();
                    output.push('$');
                    output.push(other);
                }
                None => output.push('$'),
            }
        }

        output
            .replace('\x14', "\\")
            .replace(crate::lexer::PARAM_NAME_END_MARKER, "")
    }

    pub(in crate::executor) fn expand_embedded_parameters_preserving_escaped_single_quotes(
        &self,
        word: &str,
    ) -> String {
        const PROTECTED_ESCAPED_SINGLE_QUOTE: char = '\x16';
        const PROTECTED_LITERAL_BACKSLASH: char = '\x19';
        const PROTECTED_LITERAL_DOLLAR: char = '\x12';
        let mut escaped_dollar_protected = String::with_capacity(word.len());
        let mut chars = word.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                let mut slash_count = 1usize;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    slash_count += 1;
                }
                if chars.peek() == Some(&'$') {
                    if matches!(slash_count % 4, 1 | 2) {
                        chars.next();
                        escaped_dollar_protected.push(PROTECTED_LITERAL_DOLLAR);
                    } else {
                        // Three or four source slashes leave one quoting
                        // slash before an expanding dollar after shell quote
                        // removal; larger groups repeat this pattern.
                        for _ in 0..(slash_count / 4).max(1) {
                            escaped_dollar_protected.push('\\');
                        }
                    }
                } else {
                    for _ in 0..slash_count {
                        escaped_dollar_protected.push('\\');
                    }
                }
            } else {
                escaped_dollar_protected.push(ch);
            }
        }
        let protected = escaped_dollar_protected
            .replace('\x17', "\x16")
            .replace('\x14', &PROTECTED_LITERAL_BACKSLASH.to_string());
        self.expand_embedded_parameters(&protected)
            .replace(PROTECTED_ESCAPED_SINGLE_QUOTE, "\x17")
            .replace(PROTECTED_LITERAL_BACKSLASH, "\x14")
            .replace(PROTECTED_LITERAL_DOLLAR, "$")
    }
}

fn decode_backtick_substitution_source(source: &str) -> String {
    source
        .replace('\x1a', "`")
        .replace('\x11', "")
        .replace(crate::lexer::PARAM_NAME_END_MARKER, "")
        .replace('\x1f', "$")
        .replace('\x15', "\\")
}

fn push_backtick_escaped_source_char(
    source: &mut String,
    ch: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    match ch {
        '$' | '`' | '\\' => source.push(ch),
        '\n' => {}
        '\r' if chars.peek().copied() == Some('\n') => {
            chars.next();
        }
        _ => {
            source.push('\\');
            source.push(ch);
        }
    }
}

fn update_command_substitution_case_depth(
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
) {
    if single || double {
        word.clear();
        return;
    }

    if ch == '_' || ch.is_ascii_alphanumeric() {
        word.push(ch);
        return;
    }

    match word.as_str() {
        "case" => *case_depth += 1,
        "esac" => *case_depth = case_depth.saturating_sub(1),
        _ => {}
    }
    word.clear();
}
