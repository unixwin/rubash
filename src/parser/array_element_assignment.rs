use super::{ArrayElementAssignment, CommandNode};

pub(super) fn record_array_element_assignment_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
) {
    if let Some(mut assignment) = array_element_assignment_from_word(word) {
        assignment.word_index = Some(word_index);
        command.array_element_assignments.push(assignment);
    }
}

fn array_element_assignment_from_word(word: &str) -> Option<ArrayElementAssignment> {
    let open = word.find('[')?;
    let name = &word[..open];
    if !is_shell_name(name) {
        return None;
    }

    let close = matching_subscript_end(word, open)?;
    let operator_start = close + 1;
    let (append, value_start) = if word[operator_start..].starts_with("+=") {
        (true, operator_start + 2)
    } else if word[operator_start..].starts_with('=') {
        (false, operator_start + 1)
    } else {
        return None;
    };

    Some(ArrayElementAssignment {
        name: name.to_string(),
        subscript: word[open + 1..close].to_string(),
        value: word[value_start..].to_string(),
        append,
        word_index: None,
    })
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
