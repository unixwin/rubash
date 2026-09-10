use super::{
    arithmetic_expansions_in_word, brace_expansions_in_word_with_raw,
    extglob_patterns_in_word_with_raw, parameter_expansions_in_word, pathname_patterns_in_word,
    tilde_expansions_in_word_with_raw, word_quotes_in_raw, ArrayElementAssignment, CommandNode,
};

pub(super) fn record_array_element_assignment_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
    raw: &str,
) -> bool {
    // The de-quoted word can lose the subscript delimiters' shape (quoting
    // a bracket merges it with the closing `]`: A["]"] de-quotes to
    // A[]]=rbracket and the `=` no longer follows the subscript). Fall back
    // to the raw spelling, whose quote-aware subscript scan still finds the
    // real delimiters and keeps raw_subscript/raw_value verbatim.
    if let Some(mut assignment) = array_element_assignment_from_word(word, raw)
        .or_else(|| array_element_assignment_from_word(raw, raw))
    {
        assignment.word_index = Some(word_index);
        command.array_element_assignments.push(assignment);
        return true;
    }
    false
}

pub(super) fn array_element_subscript_has_escaped_quote(raw: &str) -> bool {
    let Some(subscript) = array_element_raw_subscript(raw) else {
        return false;
    };
    // An escaped quote inside a quoted region is ordinary quoted data, not an
    // arithmetic operand: an associative array subscript is a string key, so
    // `a["x\"y"]=v` is a legal assignment in GNU 5.3.0 (assoc6.sub:44) and
    // must not be flagged. Only an unquoted escaped quote -- `a[\" \"]=15` --
    // reaches the arithmetic parser as a bad operand. Walk the raw subscript
    // keeping the same quote state the lexer uses and report only escaped
    // quotes seen outside any quote region.
    let chars: Vec<char> = subscript.chars().collect();
    let mut index = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    while index < chars.len() {
        if in_single {
            if chars[index] == '\'' {
                in_single = false;
            }
            index += 1;
            continue;
        }
        if in_double {
            if chars[index] == '\\' {
                index += 2;
                continue;
            }
            if chars[index] == '"' {
                in_double = false;
            }
            index += 1;
            continue;
        }
        match chars[index] {
            '\'' => {
                in_single = true;
                index += 1;
            }
            '"' => {
                in_double = true;
                index += 1;
            }
            '\\' if index + 1 < chars.len() && matches!(chars[index + 1], '\'' | '"') => {
                return true;
            }
            '\\' => {
                index += 2;
            }
            _ => {
                index += 1;
            }
        }
    }
    false
}

pub(super) fn array_element_assignment_from_word(
    word: &str,
    raw: &str,
) -> Option<ArrayElementAssignment> {
    let open = word.find('[')?;
    let name = &word[..open];
    if !is_shell_name(name) {
        return None;
    }

    let close = matching_subscript_end(word, open)?;
    let operator_start = close + 1;
    let (operator, append, value_start) = if word[operator_start..].starts_with("+=") {
        ("+=", true, operator_start + 2)
    } else if word[operator_start..].starts_with('=') {
        ("=", false, operator_start + 1)
    } else {
        return None;
    };

    let subscript = &word[open + 1..close];
    let value = &word[value_start..];
    let raw_subscript = array_element_raw_subscript(raw).unwrap_or(subscript);
    let raw_value = array_element_raw_value(raw).unwrap_or(value);
    Some(ArrayElementAssignment {
        name: name.to_string(),
        name_metadata: Box::new(super::build_word_metadata(0, name, name)),
        open_delimiter: "[".to_string(),
        open_delimiter_metadata: Box::new(super::build_word_metadata(0, "[", "[")),
        subscript: subscript.to_string(),
        subscript_metadata: Box::new(super::build_word_metadata(0, subscript, raw_subscript)),
        close_delimiter: "]".to_string(),
        close_delimiter_metadata: Box::new(super::build_word_metadata(0, "]", "]")),
        value: value.to_string(),
        operator: operator.to_string(),
        operator_metadata: Box::new(super::build_word_metadata(0, operator, operator)),
        append,
        word_index: None,
        subscript_brace_expansions: brace_expansions_in_word_with_raw(subscript, raw_subscript),
        subscript_parameter_expansions: parameter_expansions_in_word(subscript),
        subscript_arithmetic_expansions: arithmetic_expansions_in_word(subscript),
        brace_expansions: brace_expansions_in_word_with_raw(value, raw_value),
        parameter_expansions: parameter_expansions_in_word(value),
        arithmetic_expansions: arithmetic_expansions_in_word(value),
        extglob_patterns: extglob_patterns_in_word_with_raw(value, raw_value),
        pathname_patterns: pathname_patterns_in_word(value, raw_value),
        tilde_expansions: tilde_expansions_in_word_with_raw(value, raw_value),
        word_quotes: word_quotes_in_raw(raw_value),
    })
}

fn array_element_raw_subscript(raw: &str) -> Option<&str> {
    let open = raw.find('[')?;
    let close = matching_subscript_end(raw, open)?;
    Some(&raw[open + 1..close])
}

fn array_element_raw_value(raw: &str) -> Option<&str> {
    let open = raw.find('[')?;
    let close = matching_subscript_end(raw, open)?;
    let operator_start = close + 1;
    if raw[operator_start..].starts_with("+=") {
        Some(&raw[operator_start + 2..])
    } else {
        raw[operator_start..].strip_prefix('=')
    }
}

fn matching_subscript_end(word: &str, open: usize) -> Option<usize> {
    let chars = word.char_indices().collect::<Vec<_>>();
    let start = chars.iter().position(|(index, _)| *index == open)?;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in chars.into_iter().skip(start) {
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
            '[' if !single && !double => depth += 1,
            ']' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_shell_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
