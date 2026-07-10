use super::*;

pub(in crate::executor) fn normalize_single_element_array_assignment(
    value: &str,
) -> Option<String> {
    let inner = value.strip_prefix('(')?.strip_suffix(')')?;
    Some(format!("({})", strip_matching_quotes(inner.trim())))
}

pub(in crate::executor) fn strip_matching_quotes(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

pub(in crate::executor) fn strip_wrapping_subshell_group(source: &str) -> Option<&str> {
    let inner = source.strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in source.char_indices() {
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
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && index + ch.len_utf8() != source.len() {
                    return None;
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(inner.trim())
}

#[derive(Clone, Copy)]
pub(in crate::executor) enum MatchLength {
    Shortest,
    Longest,
}

#[derive(Clone, Copy)]
pub(in crate::executor) enum PatternRemoval {
    ShortestPrefix,
    LongestPrefix,
    ShortestSuffix,
    LongestSuffix,
}

pub(in crate::executor) fn parse_indirect_pattern_removal(
    name: &str,
) -> Option<(&str, &str, PatternRemoval)> {
    for (operator, operation) in [
        ("##", PatternRemoval::LongestPrefix),
        ("%%", PatternRemoval::LongestSuffix),
        ("#", PatternRemoval::ShortestPrefix),
        ("%", PatternRemoval::ShortestSuffix),
    ] {
        if let Some((left, pattern)) = name.split_once(operator) {
            if !left.is_empty() {
                return Some((left, pattern, operation));
            }
        }
    }
    None
}

pub(in crate::executor) fn remove_matching_prefix(
    value: &str,
    pattern: &str,
    length: MatchLength,
) -> String {
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let iter: Box<dyn Iterator<Item = usize>> = match length {
        MatchLength::Shortest => Box::new(indices.into_iter()),
        MatchLength::Longest => Box::new(indices.into_iter().rev()),
    };

    for end in iter {
        if case_pattern_matches(pattern, &value[..end]) {
            return value[end..].to_string();
        }
    }

    value.to_string()
}

pub(in crate::executor) fn remove_matching_suffix(
    value: &str,
    pattern: &str,
    length: MatchLength,
) -> String {
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let iter: Box<dyn Iterator<Item = usize>> = match length {
        MatchLength::Shortest => Box::new(indices.into_iter().rev()),
        MatchLength::Longest => Box::new(indices.into_iter()),
    };

    for start in iter {
        if case_pattern_matches(pattern, &value[start..]) {
            return value[..start].to_string();
        }
    }

    value.to_string()
}

pub(in crate::executor) fn remove_parameter_pattern(
    value: &str,
    pattern: &str,
    operation: PatternRemoval,
) -> String {
    match operation {
        PatternRemoval::ShortestPrefix => {
            remove_matching_prefix(value, pattern, MatchLength::Shortest)
        }
        PatternRemoval::LongestPrefix => {
            remove_matching_prefix(value, pattern, MatchLength::Longest)
        }
        PatternRemoval::ShortestSuffix => {
            remove_matching_suffix(value, pattern, MatchLength::Shortest)
        }
        PatternRemoval::LongestSuffix => {
            remove_matching_suffix(value, pattern, MatchLength::Longest)
        }
    }
}

pub(in crate::executor) fn strip_surrounding_quotes(s: &str) -> String {
    if s.len() >= 2 {
        if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

pub(in crate::executor) fn decode_parameter_pattern_quotes(pattern: &str) -> String {
    let mut output = String::new();
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'\'') {
            index += 2;
            let mut quoted = String::new();
            let mut escaped = false;
            while index < chars.len() {
                let ch = chars[index];
                index += 1;
                if escaped {
                    quoted.push('\\');
                    quoted.push(ch);
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '\'' {
                    break;
                }
                quoted.push(ch);
            }
            if escaped {
                quoted.push('\\');
            }
            output.push_str(&decode_ansi_c_escapes(&quoted));
            continue;
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'"') {
            index += 2;
            while index < chars.len() {
                let ch = chars[index];
                index += 1;
                if ch == '"' {
                    break;
                }
                if ch == '\\' {
                    if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                        chars.get(index).copied()
                    {
                        index += 1;
                        if escaped != '\n' {
                            output.push(escaped);
                        }
                        continue;
                    }
                }
                output.push(ch);
            }
            continue;
        }

        match chars[index] {
            '\x17' => {
                output.push('\'');
                index += 1;
            }
            '\'' => {
                if let Some(close_offset) = chars[index + 1..].iter().position(|ch| *ch == '\'') {
                    let close = index + 1 + close_offset;
                    for ch in &chars[index + 1..close] {
                        output.push(*ch);
                    }
                    index = close + 1;
                } else {
                    output.push('\'');
                    index += 1;
                }
            }
            '"' => {
                index += 1;
                while index < chars.len() {
                    let ch = chars[index];
                    index += 1;
                    if ch == '"' {
                        break;
                    }
                    if ch == '\\' {
                        if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                            chars.get(index).copied()
                        {
                            index += 1;
                            if escaped != '\n' {
                                output.push(escaped);
                            }
                            continue;
                        }
                    }
                    output.push(ch);
                }
            }
            '\\' => {
                if let Some(ch) = chars.get(index + 1) {
                    output.push(*ch);
                    index += 2;
                } else {
                    index += 1;
                }
            }
            ch => {
                output.push(ch);
                index += 1;
            }
        }
    }
    output
}
