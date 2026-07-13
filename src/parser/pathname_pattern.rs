use super::{CommandNode, PathnamePattern};

pub(super) fn record_pathname_patterns_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
    raw: &str,
) {
    let patterns = pathname_patterns_in_word(word, raw)
        .into_iter()
        .map(|mut pattern| {
            pattern.word_index = Some(word_index);
            pattern
        });
    command.pathname_patterns.extend(patterns);
}

pub(super) fn pathname_patterns_in_word(word: &str, raw: &str) -> Vec<PathnamePattern> {
    if word.contains('=') {
        return Vec::new();
    }

    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut has_star = false;
    let mut has_question = false;
    let mut has_bracket = false;
    let mut globstar = false;
    let mut operators = Vec::new();

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

        match chars[index] {
            '*' => {
                has_star = true;
                if chars.get(index + 1) == Some(&'*') {
                    globstar = true;
                    operators.push("**".to_string());
                    index += 1;
                } else {
                    operators.push("*".to_string());
                }
            }
            '?' => {
                has_question = true;
                operators.push("?".to_string());
            }
            '[' => {
                if let Some(next_index) = skip_bracket_class(&chars, index) {
                    has_bracket = true;
                    operators.push(chars[index..next_index].iter().collect());
                    index = next_index;
                    continue;
                }
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }

    if has_star || has_question || has_bracket {
        vec![PathnamePattern {
            text: word.to_string(),
            operators,
            has_star,
            has_question,
            has_bracket,
            globstar,
            word_index: None,
            assignment_name: None,
        }]
    } else {
        Vec::new()
    }
}

fn skip_bracket_class(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    if matches!(chars.get(index), Some('!' | '^')) {
        index += 1;
    }
    if chars.get(index) == Some(&']') {
        index += 1;
    }
    while index < chars.len() {
        match chars[index] {
            ']' => return Some(index + 1),
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_quoted(chars: &[char], start: usize, delimiter: char) -> Option<usize> {
    let mut index = start;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if delimiter == '"' && ch == '\\' {
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
