use super::{CommandNode, ExtglobPattern};

pub(super) fn record_extglob_patterns_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
) {
    let patterns = extglob_patterns_in_word(word)
        .into_iter()
        .map(|mut pattern| {
            pattern.word_index = Some(word_index);
            pattern
        });
    command.extglob_patterns.extend(patterns);
}

pub(super) fn record_extglob_patterns_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    word_index: Option<usize>,
) {
    let patterns = extglob_patterns_in_word(value)
        .into_iter()
        .map(|mut pattern| {
            pattern.assignment_name = Some(assignment_name.to_string());
            pattern.word_index = word_index;
            pattern
        });
    command.extglob_patterns.extend(patterns);
}

fn extglob_patterns_in_word(word: &str) -> Vec<ExtglobPattern> {
    let chars = word.chars().collect::<Vec<_>>();
    let mut patterns = Vec::new();
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

        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(next_index) = skip_braced_parameter(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if matches!(chars[index], '@' | '!' | '+' | '?' | '*') && chars.get(index + 1) == Some(&'(')
        {
            if let Some((pattern, next_index)) = extglob_pattern(&chars, index) {
                patterns.push(pattern);
                index = next_index;
                continue;
            }
        }
        index += 1;
    }
    patterns
}

fn extglob_pattern(chars: &[char], start: usize) -> Option<(ExtglobPattern, usize)> {
    let operator = chars[start];
    let open = start + 1;
    let close = matching_group_end(chars, open)?;
    let pattern = chars[open + 1..close].iter().collect::<String>();
    let (alternatives, operators) = split_alternatives(&pattern);
    Some((
        ExtglobPattern {
            text: chars[start..=close].iter().collect(),
            open_delimiter: format!("{operator}("),
            operator,
            pattern,
            close_delimiter: ")".to_string(),
            operators,
            alternatives,
            word_index: None,
            assignment_name: None,
        },
        close + 1,
    ))
}

fn matching_group_end(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    while index < chars.len() {
        match chars[index] {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            '[' => {
                index += 1;
                while index < chars.len() && chars[index] != ']' {
                    if chars[index] == '\\' {
                        index += 1;
                    }
                    index += 1;
                }
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    None
}

fn split_alternatives(pattern: &str) -> (Vec<String>, Vec<String>) {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut alternatives = Vec::new();
    let mut operators = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                alternatives.push(chars[start..index].iter().collect());
                operators.push("|".to_string());
                start = index + 1;
            }
            '[' => {
                index += 1;
                while index < chars.len() && chars[index] != ']' {
                    if chars[index] == '\\' {
                        index += 1;
                    }
                    index += 1;
                }
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    alternatives.push(chars[start..].iter().collect());
    (alternatives, operators)
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
