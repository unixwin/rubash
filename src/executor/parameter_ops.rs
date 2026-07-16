use super::*;

pub(in crate::executor) fn decode_parameter_word_quotes(word: &str) -> String {
    let mut output = String::new();
    let chars = word.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\x17' => {
                output.push('\'');
                index += 1;
            }
            '"' => {
                index += 1;
                while index < chars.len() {
                    let ch = chars[index];
                    index += 1;
                    if ch == '"' {
                        break;
                    }
                    output.push(ch);
                }
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
            ch => {
                output.push(ch);
                index += 1;
            }
        }
    }
    output
}

pub(in crate::executor) fn decode_parameter_replacement_quotes(replacement: &str) -> String {
    const PROTECTED_BACKSLASH_QUOTE: char = '\x16';
    let mut protected = String::new();
    let chars = replacement.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' && chars.get(index + 1) == Some(&'\x17') {
            protected.push(PROTECTED_BACKSLASH_QUOTE);
            index += 2;
            continue;
        }
        protected.push(chars[index]);
        index += 1;
    }
    decode_parameter_pattern_quotes(&protected)
}

pub(in crate::executor) fn restore_protected_replacement_quotes(value: &str) -> String {
    value.replace('\x16', "\\'")
}

pub(in crate::executor) fn parse_parameter_error_operator(
    inner: &str,
) -> Option<(&str, &str, bool)> {
    if let Some((name, message)) = inner.split_once(":?") {
        if is_parameter_error_name(name) {
            return Some((name, message, true));
        }
    }

    if let Some((name, message)) = inner.split_once('?') {
        if is_parameter_error_name(name) {
            return Some((name, message, false));
        }
    }

    None
}

pub(in crate::executor) fn parse_parameter_assignment_operator(
    inner: &str,
) -> Option<(&str, bool)> {
    if let Some((name, _)) = inner.split_once(":=") {
        if is_shell_name(name)
            || name.parse::<usize>().is_ok_and(|index| index > 0)
            || parse_array_subscript(name).is_some()
        {
            return Some((name, true));
        }
    }

    if let Some((name, _)) = inner.split_once('=') {
        if is_shell_name(name)
            || name.parse::<usize>().is_ok_and(|index| index > 0)
            || parse_array_subscript(name).is_some()
        {
            return Some((name, false));
        }
    }

    None
}

pub(in crate::executor) fn matching_parameter_brace(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{') {
            depth += 1;
            index += 2;
            continue;
        }
        if bytes[index] == b'}' {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
        index += 1;
    }
    None
}

pub(in crate::executor) fn braced_parameter_spans_whole_word(word: &str) -> bool {
    let Some(rest) = word.strip_prefix("${") else {
        return false;
    };
    matching_parameter_brace(rest).is_some_and(|index| index + 1 == rest.len())
}

pub(in crate::executor) fn command_substitution_spans_whole_word(word: &str) -> bool {
    let Some(rest) = word.strip_prefix("$(") else {
        return false;
    };

    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in rest.char_indices() {
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
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index + ch.len_utf8() == rest.len();
                }
            }
            _ => {}
        }
    }
    false
}

pub(in crate::executor) fn backtick_substitution_spans_whole_word(word: &str) -> bool {
    let Some(rest) = word.strip_prefix('`') else {
        return false;
    };

    let mut escaped = false;
    for (index, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '`' {
            return index + ch.len_utf8() == rest.len();
        }
    }
    false
}

pub(in crate::executor) fn is_parameter_error_name(name: &str) -> bool {
    is_shell_name(name)
        || name
            .strip_prefix('!')
            .is_some_and(|name| !name.is_empty() && is_shell_name(name))
        || matches!(name, "#" | "@" | "*" | "?" | "$" | "-" | "0")
        || name.parse::<usize>().is_ok()
        || parse_array_subscript(name).is_some()
}

pub(in crate::executor) fn has_indirect_parameter_word_operator(name: &str) -> bool {
    let Some(indirect) = name.strip_prefix('!') else {
        return false;
    };
    [":-", ":=", ":?", ":+", "-", "=", "?", "+"]
        .iter()
        .any(|operator| {
            indirect
                .split_once(operator)
                .is_some_and(|(left, _)| !left.is_empty())
        })
}

pub(in crate::executor) fn parameter_substring(
    value: &str,
    offset: isize,
    length: Option<isize>,
) -> String {
    let char_count = value.chars().count();
    let start = if offset < 0 {
        char_count.saturating_sub(offset.unsigned_abs())
    } else {
        offset as usize
    };
    let take = match length {
        Some(length) if length < 0 => char_count
            .saturating_sub(start)
            .saturating_sub(length.unsigned_abs()),
        Some(length) => usize::try_from(length).unwrap_or(usize::MAX),
        None => usize::MAX,
    };

    value.chars().skip(start).take(take).collect()
}

pub(in crate::executor) fn positional_parameter_substring(
    params: &[String],
    offset: isize,
    length: Option<isize>,
) -> Vec<String> {
    let start = if offset < 0 {
        params.len().saturating_sub(offset.unsigned_abs())
    } else {
        (offset as usize).saturating_sub(1)
    };
    let take = match length {
        Some(length) if length < 0 => params
            .len()
            .saturating_sub(start)
            .saturating_sub(length.unsigned_abs()),
        Some(length) => usize::try_from(length).unwrap_or(usize::MAX),
        None => usize::MAX,
    };

    params.iter().skip(start).take(take).cloned().collect()
}

pub(in crate::executor) fn parse_parameter_replacement(
    name: &str,
) -> Option<(&str, &str, &str, bool)> {
    if let Some((var_name, rest)) = name.split_once("//") {
        let (pattern, replacement) = rest.split_once('/').unwrap_or((rest, ""));
        return Some((var_name, pattern, replacement, true));
    }

    let (var_name, rest) = name.split_once('/')?;
    let (pattern, replacement) = rest.split_once('/').unwrap_or((rest, ""));
    Some((var_name, pattern, replacement, false))
}
