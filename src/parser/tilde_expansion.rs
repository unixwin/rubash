use super::{CommandNode, TildeExpansion};

pub(super) fn record_tilde_expansions_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
    raw: &str,
) {
    let expansions = tilde_expansions_in_word_with_raw(word, raw)
        .into_iter()
        .map(|mut expansion| {
            expansion.word_index = Some(word_index);
            expansion
        });
    command.tilde_expansions.extend(expansions);
}

pub(super) fn record_tilde_expansions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    raw_value: &str,
    word_index: Option<usize>,
) {
    let expansions = tilde_expansions_in_assignment_value_with_raw(value, raw_value)
        .into_iter()
        .map(|mut expansion| {
            expansion.assignment_name = Some(assignment_name.to_string());
            expansion.word_index = word_index;
            expansion
        });
    command.tilde_expansions.extend(expansions);
}

pub(super) fn tilde_expansions_in_word_with_raw(word: &str, raw: &str) -> Vec<TildeExpansion> {
    if raw == word {
        return tilde_expansions_in_word(word);
    }

    if raw.starts_with('~') {
        let word = strip_tilde_quote_marker(word);
        return tilde_expansion_at(word, false).into_iter().collect();
    }

    Vec::new()
}

pub(super) fn tilde_expansions_in_word(word: &str) -> Vec<TildeExpansion> {
    if word.starts_with('\x1b') {
        return Vec::new();
    }

    tilde_expansion_at(word, false).into_iter().collect()
}

fn tilde_expansions_in_assignment_value_with_raw(
    value: &str,
    raw_value: &str,
) -> Vec<TildeExpansion> {
    let value = strip_tilde_quote_marker(value);
    if raw_value == value {
        return tilde_expansions_in_assignment_value(value);
    }

    let raw_segments = split_raw_assignment_tilde_segments(raw_value);
    let value_segments = value.split(':').collect::<Vec<_>>();
    if raw_segments.len() != value_segments.len() {
        return Vec::new();
    }

    raw_segments
        .into_iter()
        .zip(value_segments)
        .enumerate()
        .filter_map(|(index, (raw_segment, value_segment))| {
            if raw_segment.starts_with('~') {
                tilde_expansion_at(value_segment, index > 0)
            } else {
                None
            }
        })
        .collect()
}

fn tilde_expansions_in_assignment_value(value: &str) -> Vec<TildeExpansion> {
    if value.starts_with(crate::expand::tilde::tilde::QUOTED_ASSIGNMENT_VALUE) {
        return Vec::new();
    }

    let mut expansions = Vec::new();
    let mut start = 0usize;
    let mut after_colon = false;
    for (index, ch) in value.char_indices() {
        if index == 0 || ch != ':' {
            continue;
        }
        if let Some(expansion) = tilde_expansion_at(&value[start..index], after_colon) {
            expansions.push(expansion);
        }
        start = index + ch.len_utf8();
        after_colon = true;
    }
    if let Some(expansion) = tilde_expansion_at(&value[start..], after_colon) {
        expansions.push(expansion);
    }
    expansions
}

fn strip_tilde_quote_marker(value: &str) -> &str {
    value
        .strip_prefix('\x1b')
        .or_else(|| value.strip_prefix(crate::expand::tilde::tilde::QUOTED_ASSIGNMENT_VALUE))
        .unwrap_or(value)
}

fn split_raw_assignment_tilde_segments(raw: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    let chars = raw.char_indices().collect::<Vec<_>>();
    while index < chars.len() {
        let (byte_index, ch) = chars[index];
        match ch {
            ':' => {
                segments.push(&raw[start..byte_index]);
                start = byte_index + ch.len_utf8();
            }
            '\'' => {
                if let Some(next_index) = skip_raw_quote(&chars, index + 1, '\'') {
                    index = next_index;
                    continue;
                }
            }
            '"' => {
                if let Some(next_index) = skip_raw_quote(&chars, index + 1, '"') {
                    index = next_index;
                    continue;
                }
            }
            '$' if chars.get(index + 1).is_some_and(|(_, next)| *next == '\'') => {
                if let Some(next_index) = skip_raw_quote(&chars, index + 2, '\'') {
                    index = next_index;
                    continue;
                }
            }
            '$' if chars.get(index + 1).is_some_and(|(_, next)| *next == '"') => {
                if let Some(next_index) = skip_raw_quote(&chars, index + 2, '"') {
                    index = next_index;
                    continue;
                }
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    segments.push(&raw[start..]);
    segments
}

fn skip_raw_quote(chars: &[(usize, char)], mut index: usize, delimiter: char) -> Option<usize> {
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index].1;
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

fn tilde_expansion_at(segment: &str, after_colon: bool) -> Option<TildeExpansion> {
    let rest = segment.strip_prefix('~')?;
    let prefix_len = rest.find('/').map_or(segment.len(), |slash| slash + 1);
    let prefix = &segment[..prefix_len];
    if prefix == "~+"
        || prefix == "~-"
        || prefix == "~"
        || valid_tilde_dirstack(prefix)
        || valid_tilde_login(prefix)
    {
        return Some(TildeExpansion {
            text: segment.to_string(),
            open_delimiter: "~".to_string(),
            open_delimiter_metadata: delimiter_metadata("~"),
            prefix: prefix.to_string(),
            close_delimiter: String::new(),
            close_delimiter_metadata: delimiter_metadata(""),
            suffix: segment[prefix_len..].to_string(),
            after_colon,
            word_index: None,
            assignment_name: None,
        });
    }
    None
}

fn delimiter_metadata(delimiter: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::literal(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}

fn valid_tilde_dirstack(prefix: &str) -> bool {
    prefix
        .strip_prefix("~+")
        .or_else(|| prefix.strip_prefix("~-"))
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
}

fn valid_tilde_login(prefix: &str) -> bool {
    prefix.len() > 1
        && prefix[1..]
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}
