use super::*;

pub(in crate::executor) fn replace_parameter_pattern(
    value: &str,
    pattern: &str,
    replacement: &str,
    global: bool,
) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }

    if global && replacement.is_empty() {
        if let Some(class) = parse_negated_bracket_filter(pattern) {
            return value
                .chars()
                .filter(|ch| bracket_filter_matches(&class, *ch))
                .collect();
        }
    }

    if let Some(prefix_pattern) = pattern.strip_prefix('#') {
        return replace_parameter_prefix(value, prefix_pattern, replacement);
    }

    if let Some(suffix_pattern) = pattern.strip_prefix('%') {
        return replace_parameter_suffix(value, suffix_pattern, replacement);
    }

    if !pattern_contains_glob(pattern) {
        return if global {
            value.replace(pattern, replacement)
        } else {
            value.replacen(pattern, replacement, 1)
        };
    }

    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let mut output = String::new();
    let mut cursor = 0;

    while cursor <= value.len() {
        let Some((start, end)) = find_parameter_pattern_match(value, pattern, cursor, &indices)
        else {
            output.push_str(&value[cursor..]);
            return output;
        };

        output.push_str(&value[cursor..start]);
        output.push_str(replacement);
        cursor = end;

        if !global {
            output.push_str(&value[cursor..]);
            return output;
        }
    }

    output
}

pub(in crate::executor) fn replace_parameter_prefix(
    value: &str,
    pattern: &str,
    replacement: &str,
) -> String {
    let Some(end) = find_parameter_prefix_match(value, pattern) else {
        return value.to_string();
    };
    format!("{replacement}{}", &value[end..])
}

pub(in crate::executor) fn replace_parameter_suffix(
    value: &str,
    pattern: &str,
    replacement: &str,
) -> String {
    let Some(start) = find_parameter_suffix_match(value, pattern) else {
        return value.to_string();
    };
    format!("{}{replacement}", &value[..start])
}

pub(super) fn pattern_contains_glob(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | '\\'))
}

pub(in crate::executor) fn find_parameter_prefix_match(
    value: &str,
    pattern: &str,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(0);
    }

    value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .rev()
        .find(|end| case_pattern_matches(pattern, &value[..*end]))
}

pub(in crate::executor) fn find_parameter_suffix_match(
    value: &str,
    pattern: &str,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(value.len());
    }

    value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .find(|start| case_pattern_matches(pattern, &value[*start..]))
}

pub(in crate::executor) fn find_parameter_pattern_match(
    value: &str,
    pattern: &str,
    cursor: usize,
    indices: &[usize],
) -> Option<(usize, usize)> {
    let start_index = indices.iter().position(|index| *index >= cursor)?;

    for start in &indices[start_index..] {
        for end in indices[start_index..].iter().rev() {
            if end <= start {
                continue;
            }
            if case_pattern_matches(pattern, &value[*start..*end]) {
                return Some((*start, *end));
            }
        }
    }

    None
}

#[derive(Clone, Copy)]
enum BracketFilterItem {
    Char(char),
    Range(char, char),
}

fn parse_negated_bracket_filter(pattern: &str) -> Option<Vec<BracketFilterItem>> {
    let inner = pattern
        .strip_prefix("[^")
        .or_else(|| pattern.strip_prefix("[!"))?
        .strip_suffix(']')?;
    if inner.is_empty() {
        return None;
    }

    let chars = inner.chars().collect::<Vec<_>>();
    let mut items = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if index + 2 < chars.len() && chars[index + 1] == '-' {
            items.push(BracketFilterItem::Range(chars[index], chars[index + 2]));
            index += 3;
        } else {
            items.push(BracketFilterItem::Char(chars[index]));
            index += 1;
        }
    }
    Some(items)
}

fn bracket_filter_matches(items: &[BracketFilterItem], ch: char) -> bool {
    items.iter().any(|item| match *item {
        BracketFilterItem::Char(value) => value == ch,
        BracketFilterItem::Range(start, end) => start <= ch && ch <= end,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::executor) enum ParameterTransform {
    Quote,
    Escape,
    Assignment,
    Attributes,
    KeyValueQuoted,
    KeyValueSplit,
    Prompt,
    Upper,
    Lower,
}

pub(in crate::executor) fn parse_parameter_transform(
    name: &str,
) -> Option<(&str, ParameterTransform)> {
    let (var_name, operation) = name.rsplit_once('@')?;
    let transform = match operation {
        "Q" => ParameterTransform::Quote,
        "E" => ParameterTransform::Escape,
        "A" => ParameterTransform::Assignment,
        "a" => ParameterTransform::Attributes,
        "K" => ParameterTransform::KeyValueQuoted,
        "k" => ParameterTransform::KeyValueSplit,
        "P" => ParameterTransform::Prompt,
        "U" => ParameterTransform::Upper,
        "L" => ParameterTransform::Lower,
        _ => return None,
    };
    Some((var_name, transform))
}

pub(in crate::executor) fn apply_parameter_transform(
    value: &str,
    transform: ParameterTransform,
) -> String {
    match transform {
        ParameterTransform::Quote => shell_single_quote_assignment_value(value),
        ParameterTransform::Escape => decode_ansi_c_escapes(value),
        ParameterTransform::Assignment => shell_single_quote_assignment_value(value),
        ParameterTransform::Attributes => String::new(),
        ParameterTransform::KeyValueQuoted => shell_single_quote_assignment_value(value),
        ParameterTransform::KeyValueSplit => shell_single_quote_assignment_value(value),
        ParameterTransform::Prompt => value.to_string(),
        ParameterTransform::Upper => value.chars().flat_map(char::to_uppercase).collect(),
        ParameterTransform::Lower => value.chars().flat_map(char::to_lowercase).collect(),
    }
}

pub(in crate::executor) fn format_key_value_transform_part(
    key: &str,
    value: &str,
    quoted: bool,
) -> String {
    if quoted {
        format!("{key} {}", quote_array_value(value))
    } else {
        format!("{key} {value}")
    }
}

pub(in crate::executor) fn shell_single_quote_assignment_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
