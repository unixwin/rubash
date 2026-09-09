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

#[derive(Clone, Copy, PartialEq, Eq)]
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
    extglob: bool,
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
        if removal_pattern_matches(pattern, &value[..end], extglob) {
            return value[end..].to_string();
        }
    }

    value.to_string()
}

pub(in crate::executor) fn remove_matching_suffix(
    value: &str,
    pattern: &str,
    length: MatchLength,
    extglob: bool,
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
        if removal_pattern_matches(pattern, &value[start..], extglob) {
            return value[..start].to_string();
        }
    }

    value.to_string()
}

pub(in crate::executor) fn remove_parameter_pattern(
    value: &str,
    pattern: &str,
    operation: PatternRemoval,
    extglob: bool,
) -> String {
    match operation {
        PatternRemoval::ShortestPrefix => {
            remove_matching_prefix(value, pattern, MatchLength::Shortest, extglob)
        }
        PatternRemoval::LongestPrefix => {
            remove_matching_prefix(value, pattern, MatchLength::Longest, extglob)
        }
        PatternRemoval::ShortestSuffix => {
            remove_matching_suffix(value, pattern, MatchLength::Shortest, extglob)
        }
        PatternRemoval::LongestSuffix => {
            remove_matching_suffix(value, pattern, MatchLength::Longest, extglob)
        }
    }
}

/// True when the pattern contains an extglob operator `+(`/`*(`/`?(`/`@(`/
/// `!(` outside a backslash escape.
pub(in crate::executor) fn pattern_uses_extglob_syntax(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => {
                index += 2;
                continue;
            }
            '+' | '*' | '?' | '@' | '!' => {
                if chars.get(index + 1) == Some(&'(') {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

/// GNU subst.c matches removal patterns with FNM_EXTMATCH when extglob is
/// enabled (match_upattern); otherwise the pattern is an ordinary glob.
fn removal_pattern_matches(pattern: &str, word: &str, extglob: bool) -> bool {
    if extglob && pattern_uses_extglob_syntax(pattern) {
        super::conditional::extglob_case_pattern_matches(pattern, word)
    } else {
        case_pattern_matches(pattern, word)
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
            push_quoted_pattern_str(&mut output, &decode_ansi_c_escapes(&quoted));
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
                            push_quoted_pattern_char(&mut output, escaped);
                        }
                        continue;
                    }
                }
                push_quoted_pattern_char(&mut output, ch);
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
                        push_quoted_pattern_char(&mut output, *ch);
                    }
                    index = close + 1;
                } else {
                    output.push('\'');
                    index += 1;
                }
            }
            '"' => {
                if !chars[index + 1..].iter().any(|ch| *ch == '"') {
                    output.push('"');
                    index += 1;
                    continue;
                }
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
                                push_quoted_pattern_char(&mut output, escaped);
                            }
                            continue;
                        }
                    }
                    push_quoted_pattern_char(&mut output, ch);
                }
            }
            '\\' => {
                if let Some(ch) = chars.get(index + 1) {
                    if *ch == '\\' {
                        output.push('\x18');
                    } else {
                        push_quoted_pattern_char(&mut output, *ch);
                    }
                    index += 2;
                } else {
                    // Trailing backslash is a literal backslash in a pattern
                    // (`${P#\}` matches a single `\`), not a dropped char.
                    output.push('\x18');
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

fn push_quoted_pattern_str(output: &mut String, value: &str) {
    for ch in value.chars() {
        push_quoted_pattern_char(output, ch);
    }
}

fn push_quoted_pattern_char(output: &mut String, ch: char) {
    if ch == '\'' {
        // A decoded literal quote must survive the embedded-parameter
        // expander, which drops a bare quote as an unclosed span. Emit it
        // escaped; the expander turns \\' into data.
        output.push('\\');
        output.push('\'');
        return;
    }
    if matches!(ch, '*' | '?' | '[' | '\\') {
        output.push('\x11');
    }
    output.push(ch);
}
