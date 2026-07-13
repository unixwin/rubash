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
    let body = word.strip_prefix('[')?;
    let pos = body.find(operator)?;
    Some((&body[..pos], &body[pos + operator.len()..]))
}

fn split_compound_assignment_words(inner: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = inner.chars();
    let mut double = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }

        if ch == '"' {
            current.push(ch);
            double = !double;
            continue;
        }

        if ch.is_ascii_whitespace() && !double {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        current.push(ch);
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
        if let Some((left, rhs)) = compound_subscript_assignment(&tokens[i].value) {
            if rhs.is_empty() {
                if let Some((word, next_i)) = collect_compound_or_keyword_word_value(tokens, i + 1)
                {
                    values.push(format!("{}{}", left, quote_compound_assignment_word(&word)));
                    i = next_i;
                    continue;
                }
            }
            values.push(format!("{}{}", left, quote_compound_assignment_word(rhs)));
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

pub(super) fn compound_subscript_assignment(value: &str) -> Option<(String, &str)> {
    if !value.starts_with('[') {
        return None;
    }

    for operator in ["]+=", "]="] {
        let Some(pos) = value.find(operator) else {
            continue;
        };
        let split = pos + operator.len();
        let subscript = &value[1..pos];
        let assignment = if operator == "]=" { "=" } else { "+=" };
        return Some((
            format!(
                "[{}]{}",
                quote_compound_assignment_word(subscript),
                assignment
            ),
            &value[split..],
        ));
    }

    None
}

pub(super) fn quote_compound_assignment_word(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '"' | '\\'))
    {
        return value.to_string();
    }

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
