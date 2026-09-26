use super::*;
use crate::executor::embedded_mutations::mark_expansion_whitespace;
use crate::executor::markers::DATA_DOLLAR;

thread_local! {
    static EXPAND_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

const MAX_EXPAND_DEPTH: usize = 50;

impl Executor {
    pub(in crate::executor) fn expand_embedded_parameters(&self, word: &str) -> String {
        self.expand_embedded_parameters_with_context(word, false)
    }

    // Compound array assignment RHS (`a=( ... )`): the same preserve-quotes
    // contract as expand_compound_assignment_parameters_mut — GNU
    // arrayfunc.c:581 parse_string_to_word_list tokenizes the raw body
    // first, so element quote syntax must survive expansion for the
    // storage tokenizer, and expansion-produced whitespace in an unquoted
    // region is \x1c-tagged (assoc keeps it glued, indexed re-splits).
    pub(in crate::executor) fn expand_embedded_parameters_compound(&self, word: &str) -> String {
        self.expand_embedded_parameters_compound_inner(word)
    }

    fn expand_embedded_parameters_compound_inner(&self, word: &str) -> String {
        let depth = EXPAND_DEPTH.with(|d| d.get());
        if depth >= MAX_EXPAND_DEPTH {
            return word.to_string();
        }
        EXPAND_DEPTH.with(|d| d.set(depth + 1));
        let result = self.expand_embedded_parameters_inner(word, false, false, true);
        EXPAND_DEPTH.with(|d| d.set(depth));
        result
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
        let result = self.expand_embedded_parameters_inner(word, heredoc, protect_ifs, false);
        EXPAND_DEPTH.with(|d| d.set(depth));
        result
    }

    fn expand_embedded_parameters_inner(
        &self,
        word: &str,
        heredoc: bool,
        protect_ifs: bool,
        preserve_quotes: bool,
    ) -> String {
        // TODO(subst.c/subst.h): This is a narrow parameter-expansion subset.
        // GNU Bash handles quoting state, operators like ${name:-word},
        // positional/special parameters, arrays, command substitution, and IFS
        // word splitting here. Keep extending this toward subst.c semantics.
        let mut output = String::new();
        let mut chars = word.chars().peekable();
        let mut in_double = false;
        // Top-level `${` ordinal for the cross-pass subscript-eval memo —
        // matches the pre-scan counter (SUB_RES_XPASS).
        let mut frag_index = 0usize;

        while let Some(ch) = chars.next() {
            if protect_ifs && in_double && matches!(ch, ' ' | '\t' | '\n') {
                output.push(crate::executor::markers::IFS_GLUE);
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
            if ch == crate::executor::markers::DATA_BACKTICK {
                output.push('`');
                continue;
            }

            if ch == DATA_DOLLAR {
                output.push('$');
                continue;
            }

            if ch == crate::executor::markers::DATA_SQUOTE {
                // In preserve_quotes (compound RHS) the \x17 carrier is the
                // CTLESC port for a data quote: it must survive into the
                // storage word so split_storage_words does not re-read it as
                // quote syntax (array6.sub: ("${a[@]/#/-iname \'}") stores
                // `-iname 'abc`). unquote_storage_value decodes it.
                output.push(if preserve_quotes {
                    crate::executor::markers::DATA_SQUOTE
                } else {
                    '\''
                });
                continue;
            }

            if ch == crate::executor::markers::DATA_DQUOTE {
                output.push(if preserve_quotes {
                    crate::executor::markers::DATA_DQUOTE
                } else {
                    '"'
                });
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
                if preserve_quotes {
                    // Compound RHS: keep the escape pair verbatim so the
                    // storage tokenizer + unquote pass apply GNU's
                    // tokenize-then-dequote order (parse.y:5368-5397,
                    // subst.c:4807). Newline joins are dropped at read time.
                    match chars.peek().copied() {
                        Some('\n') | Some('\r') => {
                            chars.next();
                        }
                        Some(next) => {
                            chars.next();
                            output.push('\\');
                            output.push(next);
                        }
                        None => output.push('\\'),
                    }
                    continue;
                }
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
                if preserve_quotes {
                    output.push('"');
                }
                continue;
            }

            if !heredoc && ch == '\'' && !in_double {
                if preserve_quotes {
                    // A single-quoted element is literal text in GNU's raw
                    // token stream; keep the quote characters so the
                    // storage tokenizer sees the same word boundaries.
                    output.push('\'');
                    for quoted_ch in chars.by_ref() {
                        output.push(quoted_ch);
                        if quoted_ch == '\'' {
                            break;
                        }
                    }
                    continue;
                }
                for quoted_ch in chars.by_ref() {
                    if quoted_ch == '\'' {
                        break;
                    }
                    if protect_ifs && matches!(quoted_ch, ' ' | '\t' | '\n') {
                        output.push(crate::executor::markers::IFS_GLUE);
                    }
                    output.push(quoted_ch);
                }
                continue;
            }

            if ch == '\\' && chars.peek() == Some(&'`') {
                chars.next();
                if preserve_quotes {
                    output.push('\\');
                    output.push('`');
                } else {
                    output.push(crate::executor::markers::DATA_BACKTICK);
                }
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
                    // \x11 is a glob-marker introduced by
                    // decode_parameter_pattern_quotes (via
                    // push_quoted_pattern_char) to mark literal glob
                    // metacharacters that were inside quotes.  When the
                    // pattern contains a backtick command substitution,
                    // decode_parameter_pattern_quotes does not recognise
                    // the backtick structure and may collapse `\\` inside
                    // double quotes to `\x11\` (a single literal backslash
                    // with a glob marker).  Without this guard the `\`
                    // is treated as an escape for the closing backtick,
                    // consuming it and leaving the substitution unclosed.
                    // Push both the marker and the next character as
                    // literal data so the closing backtick is recognised.
                    if source_ch == crate::executor::markers::CTLESC {
                        source.push(source_ch);
                        if let Some(next) = chars.next() {
                            source.push(next);
                        }
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
                    let value = protect_command_substitution_output(
                        &substitution_result_visible_text(&self.expand_command_substitution(
                            &decode_backtick_substitution_source(&source),
                        )),
                    );
                    if preserve_quotes && !in_double {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
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
                    let value = self.shell_state.positional_params.join(" ");
                    if preserve_quotes && !in_double {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
                }
                Some('*') => {
                    chars.next();
                    let value = self.positional_params_star_joined();
                    if preserve_quotes && !in_double {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
                }
                Some('#') => {
                    chars.next();
                    output.push_str(&self.shell_state.positional_params.len().to_string());
                }
                Some('-') => {
                    chars.next();
                    output.push_str(&self.shell_option_flags());
                }
                Some('{') => {
                    chars.next();
                    // GNU param_expand resolves one `${}` expansion once:
                    // memoize array-element fetches for this fragment so a
                    // subscript's side effects run once (AEPV_MEMO).
                    let _memo_frame = crate::executor::expand_braced_indices::AepvMemoFrame::new();
                    // Same fragment-site record as the mutable walker —
                    // subscript side effects dedup across layered passes
                    // (SUB_RES_XPASS). A walked word that IS the `${}`
                    // fragment being evaluated inherits the enclosing site
                    // instead of re-keying on the synthetic string.
                    let whole_braced =
                        crate::executor::parameter_ops::braced_parameter_spans_whole_word(word)
                            && crate::executor::expand_braced_indices::sub_site_active();
                    let this_frag = frag_index;
                    frag_index += 1;
                    let _site_guard = (!whole_braced).then(|| {
                        crate::executor::expand_braced_indices::SubSiteGuard::new(this_frag)
                    });
                    let name = collect_braced_parameter_name(&mut chars);
                    let value = self.expand_word(&format!("${{{name}}}"));
                    if preserve_quotes && !in_double {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
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
                        // GNU evalexp applies the writes of an arithmetic
                        // expansion against the live environment; this
                        // `&self` walk captures them into the deferred
                        // queue (overlaid so earlier queued writes are
                        // visible) for the mutable caller to apply.
                        let overlaid = crate::executor::expand_braced_indices::env_vars_with_pending_subscript_writes(&self.shell_state.env_vars);
                        let (value, writes, actual_category) =
                            eval_conditional_arith_value_categorized_with_writes(
                                &expression,
                                &overlaid,
                            );
                        if !writes.is_empty() {
                            crate::executor::expand_braced_indices::PENDING_SUBSCRIPT_WRITES
                                .with(|pending| pending.borrow_mut().extend(writes));
                        }
                        if let Some(value) = value {
                            let value = value.to_string();
                            if preserve_quotes && !in_double {
                                output
                                    .push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                            } else {
                                output.push_str(&value);
                            }
                        } else {
                            self.shell_state
                                .arithmetic_last_error_category
                                .set(actual_category);
                            // Bash reports arithmetic expansion errors
                            // (floating point, negative exponent, division
                            // by zero, ...) on stderr and sets rc=1; Rubash
                            // was silently dropping them. Classify fatality
                            // unconditionally: a readonly diagnostic may have
                            // consumed the print gate, but the evaluation
                            // error still decides list abandonment.
                            let actual_fatal = self
                                .shell_state
                                .arithmetic_last_error_category
                                .take()
                                .is_some();
                            if !actual_fatal
                                && !crate::executor::arithmetic::arithmetic_expansion_is_fatal(
                                    &expression,
                                )
                            {
                                self.shell_state.arithmetic_nonfatal_error.set(true);
                            } else {
                                self.shell_state.arithmetic_fatal_error.set(true);
                            }
                            if !self.shell_state.arithmetic_expansion_error.replace(true) {
                                let message = crate::executor::arithmetic::arithmetic_error_message(
                                    &expression,
                                    true,
                                    &self.shell_state.env_vars,
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
                    let value =
                        protect_command_substitution_output(&substitution_result_visible_text(
                            &self.expand_command_substitution(&source),
                        ));
                    if preserve_quotes && !in_double {
                        output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                    } else {
                        output.push_str(&value);
                    }
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
                            eval_conditional_arith_value(&expression, &self.shell_state.env_vars)
                        {
                            let value = value.to_string();
                            if preserve_quotes && !in_double {
                                output
                                    .push_str(&mark_expansion_whitespace(&value, preserve_quotes));
                            } else {
                                output.push_str(&value);
                            }
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
                        let value = self
                            .shell_state
                            .positional_params
                            .get(index - 1)
                            .map(String::as_str)
                            .unwrap_or("");
                        if preserve_quotes && !in_double {
                            output.push_str(&mark_expansion_whitespace(value, preserve_quotes));
                        } else {
                            output.push_str(value);
                        }
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
                        } else if preserve_quotes && !in_double {
                            output.push_str(&mark_expansion_whitespace(&value, preserve_quotes));
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
            .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
            .replace(crate::lexer::PARAM_NAME_END_MARKER, "")
    }

    pub(in crate::executor) fn expand_embedded_parameters_preserving_escaped_single_quotes(
        &self,
        word: &str,
    ) -> String {
        const PROTECTED_ESCAPED_SINGLE_QUOTE: char =
            crate::executor::markers::PROTECTED_ESCAPED_SQUOTE;
        const PROTECTED_LITERAL_BACKSLASH: char =
            crate::executor::markers::PROTECTED_LITERAL_BACKSLASH;
        const PROTECTED_LITERAL_DOLLAR: char = crate::executor::markers::PROTECTED_LITERAL_DOLLAR;
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
            .replace(
                crate::executor::markers::DATA_SQUOTE,
                crate::executor::markers::PROTECTED_ESCAPED_SQUOTE_STR,
            )
            .replace(
                crate::executor::markers::DATA_BACKSLASH,
                &PROTECTED_LITERAL_BACKSLASH.to_string(),
            );
        self.expand_embedded_parameters(&protected)
            .replace(
                PROTECTED_ESCAPED_SINGLE_QUOTE,
                crate::executor::markers::DATA_SQUOTE_STR,
            )
            .replace(
                PROTECTED_LITERAL_BACKSLASH,
                crate::executor::markers::DATA_BACKSLASH_STR,
            )
            .replace(PROTECTED_LITERAL_DOLLAR, "$")
            // Decode protected backslash from command substitution output.
            // protect_command_substitution_output converts `\` to `\x15`;
            // expand_embedded_parameters_inner does not decode it, so it
            // survives expansion.  In a pattern context the `\x15` must be
            // restored to `\` so the pattern matcher sees a literal
            // backslash (comsub2.sub: `${qpath//"`printf '%s' \\`"/}`).
            .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
    }
}

fn decode_backtick_substitution_source(source: &str) -> String {
    source
        .replace(crate::executor::markers::DATA_BACKTICK, "`")
        .replace(crate::executor::markers::CTLESC, "")
        .replace(crate::lexer::PARAM_NAME_END_MARKER, "")
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
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
