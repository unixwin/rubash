use super::{parse_array_tokens, split_storage_words, unquote_storage_value};

pub(in crate::builtins::declare) fn parse_assoc_words(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };
    split_storage_words(inner)
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((
                unquote_storage_value(key.trim_start_matches('[').trim_end_matches(']')),
                unquote_storage_value(value),
            ))
        })
        .collect()
}
pub(in crate::builtins::declare) fn append_assoc_value(current: &str, value: &str) -> String {
    let mut entries = parse_assoc_words(current);
    let tokens = parse_array_tokens(value);
    let explicit_subscripts = tokens.iter().any(|token| {
        token
            .split_once('=')
            .and_then(|(left, _)| left.strip_prefix('[')?.strip_suffix(']'))
            .is_some()
    });

    if !explicit_subscripts {
        for pair in tokens.chunks(2) {
            let Some(key) = pair.first() else {
                continue;
            };
            let key = unquote_storage_value(key);
            let value = pair
                .get(1)
                .map(|value| unquote_storage_value(value))
                .unwrap_or_default();
            entries.push((key, value));
        }
        return format_assoc_storage(entries);
    }

    for token in tokens {
        if let Some((left, rhs)) = token.split_once('=') {
            if let Some(key) = left
                .strip_prefix('[')
                .and_then(|left| left.strip_suffix(']'))
            {
                entries.push((unquote_storage_value(key), unquote_storage_value(rhs)));
                continue;
            }
        }
        entries.push(("0".to_string(), unquote_storage_value(&token)));
    }

    format_assoc_storage(entries)
}

fn format_assoc_storage(entries: Vec<(String, String)>) -> String {
    format!(
        "({})",
        entries
            .into_iter()
            .map(|(key, value)| {
                format!(
                    "[{}]={}",
                    quote_assoc_key(&key),
                    quote_assoc_storage_value(&value)
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    )
}

pub(in crate::builtins::declare) fn quote_assoc_key(key: &str) -> String {
    if !key.is_empty()
        && !key
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '"' | '\\' | ']'))
    {
        return key.to_string();
    }

    quote_assoc_storage_value(key)
}

fn quote_assoc_storage_value(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '"' | '\\'))
    {
        return value.to_string();
    }

    let mut quoted = String::from("\"");
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}
