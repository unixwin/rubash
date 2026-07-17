use super::*;
use crate::lexer::Token;

pub(super) fn compound_assignment_from_word(
    word: &str,
    value: String,
    word_index: Option<usize>,
) -> Option<CompoundAssignment> {
    let (name, rhs) = word.split_once('=')?;
    if !rhs.is_empty() {
        return None;
    }

    let append = name.ends_with('+');
    let operator = if append { "+=" } else { "=" }.to_string();
    let name = name.strip_suffix('+').unwrap_or(name).to_string();
    let ast_value = value.replace('\x1d', "");
    Some(CompoundAssignment {
        name: name.clone(),
        name_metadata: Box::new(build_word_metadata(0, &name, &name)),
        elements: compound_assignment_elements(&ast_value),
        value: ast_value,
        operator: operator.clone(),
        operator_metadata: Box::new(build_word_metadata(0, &operator, &operator)),
        append,
        open_delimiter: "(".to_string(),
        open_delimiter_metadata: Box::new(build_word_metadata(0, "(", "(")),
        close_delimiter: ")".to_string(),
        close_delimiter_metadata: Box::new(build_word_metadata(0, ")", ")")),
        word_index,
    })
}

fn compound_assignment_elements(value: &str) -> Vec<CompoundAssignmentElement> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };

    split_compound_assignment_words(inner)
        .into_iter()
        .enumerate()
        .map(|(element_index, word)| compound_assignment_element(&word, element_index))
        .collect()
}

fn compound_assignment_element(word: &str, element_index: usize) -> CompoundAssignmentElement {
    if let Some((subscript, value)) = split_compound_element_operator(word, "]+=") {
        return CompoundAssignmentElement {
            subscript: Some(subscript.to_string()),
            value: value.to_string(),
            operator: Some("+=".to_string()),
            append: true,
            element_index,
            subscript_brace_expansions: brace_expansions_in_word_with_raw(subscript, subscript),
            subscript_parameter_expansions: parameter_expansions_in_word(subscript),
            subscript_arithmetic_expansions: arithmetic_expansions_in_word(subscript),
            brace_expansions: brace_expansions_in_word_with_raw(value, value),
            parameter_expansions: parameter_expansions_in_word(value),
            arithmetic_expansions: arithmetic_expansions_in_word(value),
            extglob_patterns: extglob_patterns_in_word_with_raw(value, value),
            pathname_patterns: pathname_patterns_in_word(value, value),
            tilde_expansions: tilde_expansions_in_word(value),
            word_quotes: word_quotes_in_raw(value),
        };
    }

    if let Some((subscript, value)) = split_compound_element_operator(word, "]=") {
        return CompoundAssignmentElement {
            subscript: Some(subscript.to_string()),
            value: value.to_string(),
            operator: Some("=".to_string()),
            append: false,
            element_index,
            subscript_brace_expansions: brace_expansions_in_word_with_raw(subscript, subscript),
            subscript_parameter_expansions: parameter_expansions_in_word(subscript),
            subscript_arithmetic_expansions: arithmetic_expansions_in_word(subscript),
            brace_expansions: brace_expansions_in_word_with_raw(value, value),
            parameter_expansions: parameter_expansions_in_word(value),
            arithmetic_expansions: arithmetic_expansions_in_word(value),
            extglob_patterns: extglob_patterns_in_word_with_raw(value, value),
            pathname_patterns: pathname_patterns_in_word(value, value),
            tilde_expansions: tilde_expansions_in_word(value),
            word_quotes: word_quotes_in_raw(value),
        };
    }

    CompoundAssignmentElement {
        subscript: None,
        value: word.to_string(),
        operator: None,
        append: false,
        element_index,
        subscript_brace_expansions: Vec::new(),
        subscript_parameter_expansions: Vec::new(),
        subscript_arithmetic_expansions: Vec::new(),
        brace_expansions: brace_expansions_in_word_with_raw(word, word),
        parameter_expansions: parameter_expansions_in_word(word),
        arithmetic_expansions: arithmetic_expansions_in_word(word),
        extglob_patterns: extglob_patterns_in_word_with_raw(word, word),
        pathname_patterns: pathname_patterns_in_word(word, word),
        tilde_expansions: tilde_expansions_in_word(word),
        word_quotes: word_quotes_in_raw(word),
    }
}

fn split_compound_element_operator<'a>(
    word: &'a str,
    operator: &str,
) -> Option<(&'a str, &'a str)> {
    if !word.starts_with('[') {
        return None;
    }

    let assignment = operator.strip_prefix(']')?;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in word.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && !single {
            escaped = true;
            continue;
        }

        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ']' if !single && !double => {
                let value_start = index + ch.len_utf8();
                let value = &word[value_start..];
                return value
                    .strip_prefix(assignment)
                    .map(|value| (&word[1..index], value));
            }
            _ => {}
        }
    }

    None
}

fn split_compound_assignment_words(inner: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let chars = inner.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut double = false;
    let mut single = false;
    let mut escaped = false;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            current.push(ch);
            escaped = false;
            index += 1;
            continue;
        }

        if ch == '\\' && !single {
            current.push(ch);
            escaped = true;
            index += 1;
            continue;
        }

        if ch == '$' && !single {
            if matches!(chars.get(index + 1), Some('(')) {
                current.push(ch);
                current.push('(');
                paren_depth += 1;
                index += 2;
                continue;
            }
            if matches!(chars.get(index + 1), Some('{')) {
                current.push(ch);
                current.push('{');
                brace_depth += 1;
                index += 2;
                continue;
            }
            if matches!(chars.get(index + 1), Some('[')) {
                current.push(ch);
                current.push('[');
                bracket_depth += 1;
                index += 2;
                continue;
            }
        }

        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' if !single && !double && brace_depth == 0 && paren_depth == 0 => bracket_depth += 1,
            ']' if !single && bracket_depth > 0 => bracket_depth -= 1,
            '{' if !single && brace_depth > 0 => brace_depth += 1,
            '}' if !single && brace_depth > 0 => brace_depth -= 1,
            '(' if !single && paren_depth > 0 => paren_depth += 1,
            ')' if !single && paren_depth > 0 => paren_depth -= 1,
            _ => {}
        }

        if ch.is_ascii_whitespace()
            && !single
            && !double
            && brace_depth == 0
            && bracket_depth == 0
            && paren_depth == 0
        {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            index += 1;
            continue;
        }

        current.push(ch);
        index += 1;
    }

    if !current.is_empty() {
        words.push(current);
    }

    words
}

pub(super) fn collect_compound_assignment(
    tokens: &[Token],
    start: usize,
) -> Option<(String, usize)> {
    // TODO(parse.y/arrayfunc.c): Bash parses `name=(...)` as a compound array
    // assignment WORD and later expands it with `assign_array_var_from_string`.
    // This preserves the simple parenthesized value shape used by alias.tests.
    if !is_keyword(tokens, start + 1, "(") {
        return None;
    }

    let mut i = start + 2;
    let mut values = Vec::new();
    while i < tokens.len() && !is_keyword(tokens, i, ")") {
        if let Some((left, rhs)) = compound_subscript_assignment_token(&tokens[i]) {
            if rhs.is_empty() {
                if compound_subscript_value_is_empty(tokens, i) {
                    values.push(left);
                    i += 1;
                    continue;
                }
                if let Some((word, next_i)) = collect_compound_or_keyword_word_value(tokens, i + 1)
                {
                    values.push(format!("{}{}", left, quote_compound_assignment_word(&word)));
                    i = next_i;
                    continue;
                }
            }
            values.push(format!("{}{}", left, quote_compound_assignment_word(&rhs)));
            i += 1;
            continue;
        }

        if let Some((subscript, raw_subscript, operator, rhs, value_index, next_i)) =
            collect_split_compound_subscript_assignment(tokens, i)
        {
            let left = format!(
                "[{}]{}",
                quote_compound_assignment_raw_subscript(&subscript, &raw_subscript),
                operator
            );
            if let Some(rhs) = rhs {
                values.push(format!("{}{}", left, quote_compound_assignment_word(&rhs)));
                i = next_i;
                continue;
            }
            if compound_split_subscript_value_is_empty(tokens, value_index) {
                values.push(left);
                i = next_i;
                continue;
            }
            if let Some((rhs, next_i)) =
                collect_compound_or_keyword_word_value(tokens, value_index + 1)
            {
                values.push(format!("{}{}", left, quote_compound_assignment_word(&rhs)));
                i = next_i;
                continue;
            }
        }

        if let Some((word, next_i)) = collect_compound_or_keyword_word_value(tokens, i) {
            values.push(quote_compound_assignment_token_word(
                tokens, i, next_i, &word,
            ));
            i = next_i;
            continue;
        }
        i += 1;
    }

    if !is_keyword(tokens, i, ")") {
        return None;
    }

    Some((format!("({})", values.join(" ")), i))
}

fn compound_subscript_value_is_empty(tokens: &[Token], index: usize) -> bool {
    if is_keyword(tokens, index + 1, ")") {
        return true;
    }

    tokens
        .get(index + 1)
        .is_some_and(|token| compound_subscript_assignment_token(token).is_some())
}

fn compound_split_subscript_value_is_empty(tokens: &[Token], operator_index: usize) -> bool {
    if is_keyword(tokens, operator_index + 1, ")") {
        return true;
    }

    tokens.get(operator_index + 1).is_some_and(|token| {
        token.value == "[" || compound_subscript_assignment_token(token).is_some()
    })
}

fn collect_split_compound_subscript_assignment(
    tokens: &[Token],
    start: usize,
) -> Option<(String, String, String, Option<String>, usize, usize)> {
    let token = tokens.get(start)?;
    if token.value == "[" {
        return collect_bracket_token_compound_subscript_assignment(tokens, start);
    }

    let first = token.value.strip_prefix('[')?;
    let first_raw = token.raw.strip_prefix('[').unwrap_or(first);
    let Some((head, operator, rhs)) = split_subscript_close_operator(first) else {
        return collect_spanning_compound_subscript_assignment(tokens, start, first, first_raw);
    };

    Some((
        head.to_string(),
        first_raw
            .strip_suffix(&format!("]{}{}", operator, rhs))
            .unwrap_or(head)
            .to_string(),
        operator.to_string(),
        Some(rhs.to_string()),
        start,
        start + 1,
    ))
}

fn collect_bracket_token_compound_subscript_assignment(
    tokens: &[Token],
    start: usize,
) -> Option<(String, String, String, Option<String>, usize, usize)> {
    let mut subscript = String::new();
    let mut raw_subscript = String::new();
    let mut j = start + 1;
    while j < tokens.len() && tokens[j].value != "]" {
        push_compound_subscript_piece(&mut subscript, &mut raw_subscript, &tokens[j]);
        j += 1;
    }
    if j >= tokens.len() || tokens[j].value != "]" {
        return None;
    }
    let operator_index = j + 1;
    let operator = tokens.get(operator_index)?.value.as_str();
    if !matches!(operator, "=" | "+=") {
        return None;
    }

    Some((
        subscript,
        raw_subscript,
        operator.to_string(),
        None,
        operator_index,
        operator_index + 1,
    ))
}

fn collect_spanning_compound_subscript_assignment(
    tokens: &[Token],
    start: usize,
    first: &str,
    first_raw: &str,
) -> Option<(String, String, String, Option<String>, usize, usize)> {
    let mut subscript = first.to_string();
    let mut raw_subscript = first_raw.to_string();
    let mut j = start + 1;
    while let Some(token) = tokens.get(j) {
        if let Some((head, operator, rhs)) = split_subscript_close_operator(&token.value) {
            push_compound_subscript_fragment(&mut subscript, head);
            let raw_head = token
                .raw
                .strip_suffix(&format!("]{}{}", operator, rhs))
                .unwrap_or(head);
            push_compound_subscript_fragment(&mut raw_subscript, raw_head);
            return Some((
                subscript,
                raw_subscript,
                operator.to_string(),
                Some(rhs.to_string()),
                j,
                j + 1,
            ));
        }

        if let Some(head) = token.value.strip_suffix(']') {
            push_compound_subscript_fragment(&mut subscript, head);
            let raw_head = token.raw.strip_suffix(']').unwrap_or(head);
            push_compound_subscript_fragment(&mut raw_subscript, raw_head);
            let operator_index = j + 1;
            let operator = tokens.get(operator_index)?.value.as_str();
            if !matches!(operator, "=" | "+=") {
                return None;
            }
            return Some((
                subscript,
                raw_subscript,
                operator.to_string(),
                None,
                operator_index,
                operator_index + 1,
            ));
        }

        push_compound_subscript_piece(&mut subscript, &mut raw_subscript, token);
        j += 1;
    }

    None
}

fn split_subscript_close_operator(value: &str) -> Option<(&str, &str, &str)> {
    for operator in ["]+=", "]="] {
        if let Some((head, rhs)) = value.split_once(operator) {
            return Some((head, if operator == "]=" { "=" } else { "+=" }, rhs));
        }
    }

    None
}

fn push_compound_subscript_piece(
    subscript: &mut String,
    raw_subscript: &mut String,
    token: &Token,
) {
    push_compound_subscript_fragment(subscript, &token.value);
    push_compound_subscript_fragment(raw_subscript, &token.raw);
}

fn push_compound_subscript_fragment(out: &mut String, fragment: &str) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(fragment);
}

pub(super) fn compound_subscript_assignment(value: &str) -> Option<(String, &str)> {
    if !value.starts_with('[') {
        return None;
    }

    for operator in ["]+=", "]="] {
        let Some(pos) = find_compound_subscript_operator(value, operator) else {
            continue;
        };
        let split = pos + operator.len();
        let subscript = &value[1..pos];
        let assignment = if operator == "]=" { "=" } else { "+=" };
        return Some((
            format!(
                "[{}]{}",
                quote_compound_assignment_subscript(subscript),
                assignment
            ),
            &value[split..],
        ));
    }

    None
}

fn compound_subscript_assignment_token(token: &Token) -> Option<(String, String)> {
    if token.raw == token.value {
        let (left, rhs) = compound_subscript_assignment(&token.value)?;
        return Some((left, rhs.to_string()));
    }

    compound_subscript_assignment_from_raw(&token.raw)
}

fn compound_subscript_assignment_from_raw(raw: &str) -> Option<(String, String)> {
    if !raw.starts_with('[') {
        return None;
    }

    for operator in ["]+=", "]="] {
        let Some(pos) = find_compound_subscript_operator(raw, operator) else {
            continue;
        };
        let split = pos + operator.len();
        let raw_subscript = &raw[1..pos];
        let subscript = remove_compound_assignment_quotes(raw_subscript);
        let rhs = remove_compound_assignment_quotes(&raw[split..]);
        let assignment = if operator == "]=" { "=" } else { "+=" };
        let subscript = quote_compound_assignment_raw_subscript(&subscript, raw_subscript);
        return Some((format!("[{}]{}", subscript, assignment), rhs));
    }

    None
}

fn quote_compound_assignment_raw_subscript(subscript: &str, raw_subscript: &str) -> String {
    if raw_contains_shell_quotes(raw_subscript) {
        return quote_compound_assignment_word_forced(subscript);
    }

    quote_compound_assignment_subscript(subscript)
}

fn find_compound_subscript_operator(value: &str, operator: &str) -> Option<usize> {
    let assignment = operator.strip_prefix(']')?;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && !single {
            escaped = true;
            continue;
        }

        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ']' if !single && !double => {
                let value_start = index + ch.len_utf8();
                if value[value_start..].starts_with(assignment) {
                    return Some(index);
                }
            }
            _ => {}
        }
    }

    None
}

fn remove_compound_assignment_quotes(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    out.push(quoted);
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    match quoted {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                                chars.peek().copied()
                            {
                                chars.next();
                                if escaped != '\n' {
                                    out.push(escaped);
                                }
                            } else {
                                out.push('\\');
                            }
                        }
                        _ => out.push(quoted),
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

fn quote_compound_assignment_token_word(
    tokens: &[Token],
    index: usize,
    next_index: usize,
    word: &str,
) -> String {
    if compound_assignment_word_has_unquoted_command_substitution(tokens, index, next_index) {
        return quote_compound_assignment_word_forced(&format!("\x1d{word}"));
    }

    if next_index == index + 1
        && tokens
            .get(index)
            .is_some_and(|token| token.raw != token.value && raw_contains_shell_quotes(&token.raw))
    {
        return quote_compound_assignment_word_forced(word);
    }

    quote_compound_assignment_word(word)
}

fn raw_contains_shell_quotes(raw: &str) -> bool {
    raw.contains('\'') || raw.contains('"')
}

fn compound_assignment_word_has_unquoted_command_substitution(
    tokens: &[Token],
    index: usize,
    next_index: usize,
) -> bool {
    tokens[index..next_index].iter().any(|token| {
        !raw_is_outer_quoted(&token.raw)
            && (token.kind == crate::lexer::TokenKind::CommandSubst
                || token.raw.contains("$(")
                || token.raw.contains('`'))
    })
}

fn raw_is_outer_quoted(raw: &str) -> bool {
    (raw.starts_with('"') && raw.ends_with('"')) || (raw.starts_with('\'') && raw.ends_with('\''))
}

fn quote_compound_assignment_subscript(value: &str) -> String {
    if value.contains(']') {
        return quote_compound_assignment_word_forced(value);
    }

    quote_compound_assignment_word(value)
}

fn quote_compound_assignment_word_forced(value: &str) -> String {
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

pub(super) fn quote_compound_assignment_word(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '"' | '\\'))
    {
        return value.to_string();
    }

    quote_compound_assignment_word_forced(value)
}
