use super::*;
use crate::lexer::{Token, TokenKind};

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
    let name = name.strip_suffix('+').unwrap_or(name).to_string();
    Some(CompoundAssignment {
        name,
        value,
        append,
        word_index,
    })
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

            if j + 1 < tokens.len() && matches!(tokens[j].value.as_str(), "=" | "+=") {
                values.push(format!(
                    "[{}]{}{}",
                    quote_compound_assignment_word(&subscript),
                    tokens[j].value,
                    quote_compound_assignment_word(&tokens[j + 1].value)
                ));
                i = j + 2;
                continue;
            }
        }

        if matches!(
            tokens[i].kind,
            TokenKind::Word | TokenKind::Variable | TokenKind::Assignment | TokenKind::CommandSubst
        ) {
            values.push(quote_compound_assignment_word(&tokens[i].value));
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
