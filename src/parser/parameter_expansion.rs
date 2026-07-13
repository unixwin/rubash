use super::{CommandNode, ParameterExpansion};

pub(super) fn record_parameter_expansions_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
) {
    let expansions = parameter_expansions_in_word(word)
        .into_iter()
        .map(|mut expansion| {
            expansion.word_index = Some(word_index);
            expansion
        });
    command.parameter_expansions.extend(expansions);
}

pub(super) fn record_parameter_expansions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    word_index: Option<usize>,
) {
    let expansions = parameter_expansions_in_word(value)
        .into_iter()
        .map(|mut expansion| {
            expansion.assignment_name = Some(assignment_name.to_string());
            expansion.word_index = word_index;
            expansion
        });
    command.parameter_expansions.extend(expansions);
}

fn parameter_expansions_in_word(word: &str) -> Vec<ParameterExpansion> {
    let chars = word.chars().collect::<Vec<_>>();
    let mut expansions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            if let Some(next_index) = skip_backtick(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
            if chars.get(index + 2) == Some(&'(') {
                if let Some(next_index) = skip_arithmetic_expansion(&chars, index) {
                    index = next_index;
                    continue;
                }
            } else if let Some(next_index) = skip_command_substitution(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'[') {
            if let Some(next_index) = skip_bracket_arithmetic_expansion(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(next_index) = skip_braced_command_substitution(&chars, index) {
                index = next_index;
                continue;
            }
            if let Some((expansion, next_index)) = braced_parameter_expansion(&chars, index) {
                expansions.push(expansion);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' {
            if let Some((expansion, next_index)) = simple_parameter_expansion(&chars, index) {
                expansions.push(expansion);
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
    expansions
}

fn braced_parameter_expansion(chars: &[char], start: usize) -> Option<(ParameterExpansion, usize)> {
    let mut index = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'{') && !single {
            depth += 1;
            index += 2;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '}' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let parameter = chars[start + 2..index].iter().collect::<String>();
                    let (name, operator, operator_prefix, word) = parameter_parts(&parameter);
                    return Some((
                        ParameterExpansion {
                            text: chars[start..=index].iter().collect(),
                            open_delimiter: "${".to_string(),
                            parameter,
                            close_delimiter: "}".to_string(),
                            name,
                            operator,
                            operator_prefix,
                            word,
                            braced: true,
                            word_index: None,
                            assignment_name: None,
                        },
                        index + 1,
                    ));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_braced_command_substitution(chars: &[char], start: usize) -> Option<usize> {
    let body_start = start + 2;
    let pipe_output = chars.get(body_start) == Some(&'|');
    if !pipe_output && !chars.get(body_start).is_some_and(|ch| ch.is_whitespace()) {
        return None;
    }

    let mut index = if pipe_output {
        body_start + 1
    } else {
        body_start
    };
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '{' if !single && !double => depth += 1,
            '}' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn simple_parameter_expansion(chars: &[char], start: usize) -> Option<(ParameterExpansion, usize)> {
    let next = *chars.get(start + 1)?;
    if next.is_ascii_alphabetic() || next == '_' {
        let mut end = start + 2;
        while chars
            .get(end)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        {
            end += 1;
        }
        return Some((simple_expansion(chars, start, end), end));
    }

    if next.is_ascii_digit() || matches!(next, '*' | '@' | '#' | '?' | '-' | '$' | '!' | '_') {
        let end = start + 2;
        return Some((simple_expansion(chars, start, end), end));
    }

    None
}

fn simple_expansion(chars: &[char], start: usize, end: usize) -> ParameterExpansion {
    let parameter = chars[start + 1..end].iter().collect::<String>();
    ParameterExpansion {
        text: chars[start..end].iter().collect(),
        open_delimiter: "$".to_string(),
        name: parameter.clone(),
        parameter,
        close_delimiter: String::new(),
        operator: None,
        operator_prefix: false,
        word: None,
        braced: false,
        word_index: None,
        assignment_name: None,
    }
}

fn parameter_parts(parameter: &str) -> (String, Option<String>, bool, Option<String>) {
    if let Some(name) = parameter.strip_prefix('#') {
        return (name.to_string(), Some("#".to_string()), true, None);
    }

    if let Some(name) = parameter.strip_prefix('!') {
        return (name.to_string(), Some("!".to_string()), true, None);
    }

    for operator in [
        ":-", ":=", ":?", ":+", "##", "%%", "//", "^^", ",,", "~~", ":", "-", "=", "?", "+", "#",
        "%", "/", "^", ",", "~", "@",
    ] {
        if let Some(index) = top_level_operator(parameter, operator) {
            return (
                parameter[..index].to_string(),
                Some(operator.to_string()),
                false,
                Some(parameter[index + operator.len()..].to_string()),
            );
        }
    }

    (parameter.to_string(), None, false, None)
}

fn top_level_operator(parameter: &str, operator: &str) -> Option<usize> {
    let chars = parameter.chars().collect::<Vec<_>>();
    let operator_chars = operator.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        if chars[index] == '\\' && !single {
            index += 2;
            continue;
        }

        match chars[index] {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '$' if !single && chars.get(index + 1) == Some(&'{') => {
                brace_depth += 1;
                index += 2;
                continue;
            }
            '}' if !single && !double && brace_depth > 0 => brace_depth -= 1,
            '(' if !single && !double => paren_depth += 1,
            ')' if !single && !double && paren_depth > 0 => paren_depth -= 1,
            '[' if !single && !double => bracket_depth += 1,
            ']' if !single && !double && bracket_depth > 0 => bracket_depth -= 1,
            _ => {}
        }

        if !single
            && !double
            && brace_depth == 0
            && paren_depth == 0
            && bracket_depth == 0
            && chars[index..].starts_with(&operator_chars)
            && operator_can_start(parameter, index, operator)
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn operator_can_start(_parameter: &str, index: usize, operator: &str) -> bool {
    if index == 0 {
        return false;
    }

    if operator == "/" || operator == "//" {
        return index > 0;
    }

    true
}

fn skip_command_substitution(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_arithmetic_expansion(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 3;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index + 1 < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double && depth > 0 => depth -= 1,
            ')' if !single && !double && chars.get(index + 1) == Some(&')') => {
                return Some(index + 2);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_bracket_arithmetic_expansion(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 2;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' if !single && !double => depth += 1,
            ']' if !single && !double && depth > 0 => depth -= 1,
            ']' if !single && !double => return Some(index + 1),
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_backtick(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '`' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}
