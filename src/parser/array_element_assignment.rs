use super::{
    arithmetic_expansions_in_word, brace_expansions_in_word, extglob_patterns_in_word,
    parameter_expansions_in_word, pathname_patterns_in_word, tilde_expansions_in_word,
    word_quotes_in_raw, ArrayElementAssignment, CommandNode,
};

pub(super) fn record_array_element_assignment_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
    raw: &str,
) {
    if let Some(mut assignment) = array_element_assignment_from_word(word, raw) {
        assignment.word_index = Some(word_index);
        command.array_element_assignments.push(assignment);
    }
}

fn array_element_assignment_from_word(word: &str, raw: &str) -> Option<ArrayElementAssignment> {
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
    let raw_value = array_element_raw_value(raw).unwrap_or(value);
    Some(ArrayElementAssignment {
        name: name.to_string(),
        subscript: subscript.to_string(),
        value: value.to_string(),
        operator: operator.to_string(),
        append,
        word_index: None,
        subscript_brace_expansions: brace_expansions_in_word(subscript),
        subscript_parameter_expansions: parameter_expansions_in_word(subscript),
        subscript_arithmetic_expansions: arithmetic_expansions_in_word(subscript),
        brace_expansions: brace_expansions_in_word(value),
        parameter_expansions: parameter_expansions_in_word(value),
        arithmetic_expansions: arithmetic_expansions_in_word(value),
        extglob_patterns: extglob_patterns_in_word(value),
        pathname_patterns: pathname_patterns_in_word(value, raw_value),
        tilde_expansions: tilde_expansions_in_word(value),
        word_quotes: word_quotes_in_raw(raw_value),
    })
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
