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
    Some(CompoundAssignment {
        name,
        elements: compound_assignment_elements(&value),
        value,
        operator,
        append,
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
        .map(|word| compound_assignment_element(&word))
        .collect()
}

fn compound_assignment_element(word: &str) -> CompoundAssignmentElement {
    if let Some((subscript, value)) = split_compound_element_operator(word, "]+=") {
        return CompoundAssignmentElement {
            subscript: Some(subscript.to_string()),
            value: value.to_string(),
            operator: Some("+=".to_string()),
            append: true,
        };
    }

    if let Some((subscript, value)) = split_compound_element_operator(word, "]=") {
        return CompoundAssignmentElement {
            subscript: Some(subscript.to_string()),
            value: value.to_string(),
            operator: Some("=".to_string()),
            append: false,
        };
    }

    CompoundAssignmentElement {
        subscript: None,
        value: word.to_string(),
        operator: None,
        append: false,
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

        if tokens[i].value == "[" || tokens[i].value.starts_with('[') {
            let mut subscript = String::new();
            let mut j = i;
            if tokens[j].value == "[" {
                j += 1;
                while j < tokens.len() && tokens[j].value != "]" {
                    subscript.push_str(&tokens[j].value);
                    j += 1;
                }
                if j >= tokens.len() || tokens[j].value != "]" {
                    return None;
                }
                j += 1;
            } else if let Some(inner) = tokens[j]
                .value
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
            {
                subscript.push_str(inner);
                j += 1;
            }

            if matches!(
                tokens.get(j).map(|token| token.value.as_str()),
                Some("=" | "+=")
            ) {
                if let Some((rhs, next_i)) = collect_compound_or_keyword_word_value(tokens, j + 1) {
                    values.push(format!(
                        "[{}]{}{}",
                        quote_compound_assignment_word(&subscript),
                        tokens[j].value,
                        quote_compound_assignment_word(&rhs)
                    ));
                    i = next_i;
                    continue;
                }
            }
        }

        if let Some((word, next_i)) = collect_compound_or_keyword_word_value(tokens, i) {
            values.push(quote_compound_assignment_word(&word));
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
        let subscript = remove_compound_assignment_quotes(&raw[1..pos]);
        let rhs = remove_compound_assignment_quotes(&raw[split..]);
        let assignment = if operator == "]=" { "=" } else { "+=" };
        return Some((
            format!(
                "[{}]{}",
                quote_compound_assignment_subscript(&subscript),
                assignment
            ),
            rhs,
        ));
    }

    None
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
