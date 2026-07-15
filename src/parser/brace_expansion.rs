use super::{BraceExpansion, CommandNode};

pub(super) fn record_brace_expansions_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
    raw: &str,
) {
    let expansions = brace_expansions_in_word_with_raw(word, raw)
        .into_iter()
        .map(|mut expansion| {
            expansion.word_index = Some(word_index);
            expansion
        });
    command.brace_expansions.extend(expansions);
}

pub(super) fn record_brace_expansions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    raw_value: &str,
    word_index: Option<usize>,
) {
    let expansions = brace_expansions_in_word_with_raw(value, raw_value)
        .into_iter()
        .map(|mut expansion| {
            expansion.assignment_name = Some(assignment_name.to_string());
            expansion.word_index = word_index;
            expansion
        });
    command.brace_expansions.extend(expansions);
}

pub(super) fn brace_expansions_in_word_with_raw(word: &str, raw: &str) -> Vec<BraceExpansion> {
    if raw == word {
        return brace_expansions_in_word(word);
    }

    brace_expansions_in_raw_word(raw)
}

pub(super) fn brace_expansions_in_word(word: &str) -> Vec<BraceExpansion> {
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
            if let Some(next_index) = skip_dollar_paren(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'[') {
            if let Some(next_index) = skip_dollar_bracket(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(next_index) = skip_braced_parameter(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '{' {
            if let Some((expansion, next_index)) = brace_expansion(&chars, index) {
                let nested = brace_expansions_in_word(&expansion.body);
                expansions.push(expansion);
                expansions.extend(nested);
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
    expansions
}

fn brace_expansions_in_raw_word(word: &str) -> Vec<BraceExpansion> {
    let chars = word.chars().collect::<Vec<_>>();
    let mut expansions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'\'') {
            if let Some(next_index) = skip_quoted(&chars, index + 2, '\'') {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'"') {
            if let Some(next_index) = skip_quoted(&chars, index + 2, '"') {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '\'' {
            if let Some(next_index) = skip_quoted(&chars, index + 1, '\'') {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '"' {
            if let Some(next_index) = skip_quoted(&chars, index + 1, '"') {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '`' {
            if let Some(next_index) = skip_backtick(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
            if let Some(next_index) = skip_dollar_paren(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'[') {
            if let Some(next_index) = skip_dollar_bracket(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(next_index) = skip_braced_parameter(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '{' {
            if let Some((expansion, next_index)) = brace_expansion(&chars, index) {
                let nested = brace_expansions_in_word(&expansion.body);
                expansions.push(expansion);
                expansions.extend(nested);
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
    expansions
}

fn brace_expansion(chars: &[char], start: usize) -> Option<(BraceExpansion, usize)> {
    let mut index = start + 1;
    let mut depth = 1usize;
    let mut has_comma = false;
    let mut has_double_dot = false;
    let mut operators: Vec<String> = Vec::new();
    while index < chars.len() {
        match chars[index] {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if !has_comma && !has_double_dot {
                        return None;
                    }
                    return Some((
                        BraceExpansion {
                            text: chars[start..=index].iter().collect(),
                            open_delimiter: "{".to_string(),
                            open_delimiter_metadata: delimiter_metadata("{"),
                            body: chars[start + 1..index].iter().collect(),
                            close_delimiter: "}".to_string(),
                            close_delimiter_metadata: delimiter_metadata("}"),
                            operator_metadata: operators
                                .iter()
                                .map(|operator| operator_metadata(operator))
                                .collect(),
                            operators,
                            range: has_double_dot && !has_comma,
                            word_index: None,
                            assignment_name: None,
                        },
                        index + 1,
                    ));
                }
            }
            ',' if depth == 1 => {
                has_comma = true;
                operators.push(",".to_string());
            }
            '.' if depth == 1 && chars.get(index + 1) == Some(&'.') => {
                has_double_dot = true;
                operators.push("..".to_string());
                index += 1;
            }
            '\\' => index += 1,
            ch if ch.is_ascii_whitespace() => return None,
            _ => {}
        }
        index += 1;
    }
    None
}

fn operator_metadata(operator: &str) -> crate::parser::WordMetadata {
    crate::parser::WordMetadata::literal(0, operator.to_string(), operator.to_string())
}

fn delimiter_metadata(delimiter: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::new(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}

fn skip_quoted(chars: &[char], mut index: usize, delimiter: char) -> Option<usize> {
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
        if ch == delimiter {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn skip_dollar_paren(chars: &[char], start: usize) -> Option<usize> {
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

fn skip_dollar_bracket(chars: &[char], start: usize) -> Option<usize> {
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

fn skip_braced_parameter(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 2;
    let mut depth = 1usize;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            depth += 1;
            index += 2;
            continue;
        }
        if chars[index] == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index + 1);
            }
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
