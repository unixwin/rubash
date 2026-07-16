use super::*;

pub(in crate::executor) fn split_assignment_word(word: &str) -> Option<(&str, &str)> {
    let (name, value) = word.split_once('=')?;
    let (base_name, _) = assignment_name_and_append(name);
    if is_shell_name(base_name) {
        Some((name, value))
    } else {
        None
    }
}

pub(in crate::executor) fn assignment_name_and_append(name: &str) -> (&str, bool) {
    name.strip_suffix('+')
        .map(|base| (base, true))
        .unwrap_or((name, false))
}

pub(in crate::executor) fn arithmetic_expression_arg(expression: &str) -> String {
    expression.replace(COMPOUND_ASSIGNMENT_MARKER, "")
}

pub(in crate::executor) fn arithmetic_assignment_suffix(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(|ch| matches!(ch, b'+' | b'-' | b'*' | b'/' | b'%'))
}

pub(in crate::executor) fn single_unquoted_parameter_name(value: &str) -> Option<&str> {
    if let Some(name) = value
        .strip_prefix("${")
        .and_then(|name| name.strip_suffix('}'))
    {
        return is_shell_name(name).then_some(name);
    }
    let name = value.strip_prefix('$')?;
    is_shell_name(name).then_some(name)
}

pub(in crate::executor) fn append_assoc_value(current: &str, value: &str) -> String {
    let mut entries = assoc_entries(current);
    let tokens = array_assignment_tokens(value);
    let explicit_subscripts = tokens
        .iter()
        .any(|token| assoc_assignment_token(token).is_some());

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
        if let Some((key, rhs, append)) = assoc_assignment_token(&token) {
            let key = unquote_storage_value(key);
            let rhs = unquote_storage_value(rhs);
            if append {
                if let Some((_, entry_value)) = entries
                    .iter_mut()
                    .rev()
                    .find(|(entry_key, _)| entry_key == &key)
                {
                    *entry_value = append_scalar_value(entry_value, &rhs);
                } else {
                    entries.push((key, rhs));
                }
                continue;
            }
            entries.push((key, rhs));
            continue;
        }
        entries.push(("0".to_string(), unquote_storage_value(&token)));
    }

    format_assoc_storage(entries)
}

fn assoc_assignment_token(token: &str) -> Option<(&str, &str, bool)> {
    if let Some((left, rhs)) = token.split_once("+=") {
        if let Some(key) = left
            .strip_prefix('[')
            .and_then(|left| left.strip_suffix(']'))
        {
            return Some((key, rhs, true));
        }
    }

    let (left, rhs) = token.split_once('=')?;
    let key = left
        .strip_prefix('[')
        .and_then(|left| left.strip_suffix(']'))?;
    Some((key, rhs, false))
}

pub(in crate::executor) fn append_assoc_scalar_value(current: &str, value: &str) -> String {
    let mut entries = assoc_entries(current);
    let value = unquote_storage_value(value);
    if let Some((_, entry_value)) = entries.iter_mut().rev().find(|(key, _)| key == "0") {
        *entry_value = value;
    } else {
        entries.push(("0".to_string(), value));
    }
    format_assoc_storage(entries)
}

pub(in crate::executor) fn format_assoc_storage(entries: Vec<(String, String)>) -> String {
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

pub(in crate::executor) fn quote_assoc_key(key: &str) -> String {
    if !key.is_empty()
        && !key
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '"' | '\\' | ']'))
    {
        return key.to_string();
    }

    quote_assoc_storage_value(key)
}

pub(in crate::executor) fn quote_assoc_storage_value(value: &str) -> String {
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

pub(in crate::executor) fn assoc_entries(value: &str) -> Vec<(String, String)> {
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

pub(in crate::executor) fn assoc_value_at(value: &str, key: &str) -> Option<String> {
    assoc_entries(value)
        .into_iter()
        .rev()
        .find_map(|(entry_key, entry_value)| (entry_key == key).then_some(entry_value))
}

pub(in crate::executor) fn assoc_keys(value: &str) -> Vec<String> {
    assoc_entries(value)
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

pub(in crate::executor) fn split_storage_words(value: &str) -> impl Iterator<Item = String> + '_ {
    StorageWordIter {
        input: value,
        offset: 0,
    }
}

struct StorageWordIter<'a> {
    input: &'a str,
    offset: usize,
}

impl Iterator for StorageWordIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ch) = self.input.get(self.offset..)?.chars().next() {
            if !ch.is_ascii_whitespace() {
                break;
            }
            self.offset += ch.len_utf8();
        }

        let mut word = String::new();
        let mut in_double = false;
        let mut escaped = false;
        for (relative, ch) in self.input[self.offset..].char_indices() {
            if escaped {
                word.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' && in_double {
                word.push(ch);
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_double = !in_double;
                word.push(ch);
                continue;
            }
            if ch.is_ascii_whitespace() && !in_double {
                self.offset += relative + ch.len_utf8();
                return Some(word);
            }
            word.push(ch);
        }
        self.offset = self.input.len();
        (!word.is_empty()).then_some(word)
    }
}

pub(in crate::executor) fn unquote_storage_value(value: &str) -> String {
    if value == "\\\"\\" {
        return "\"\"".to_string();
    }

    if let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return inner.to_string();
    }

    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value.to_string();
    };

    let mut unquoted = String::new();
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            if !matches!(ch, '$' | '`' | '"' | '\\' | '\n') {
                unquoted.push('\\');
            }
            unquoted.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            unquoted.push(ch);
        }
    }
    if escaped {
        unquoted.push('\\');
    }
    if unquoted == "\\\"\\" {
        return "\"\"".to_string();
    }
    unquoted
}

pub(in crate::executor) fn quote_compound_field_value(value: &str) -> String {
    quote_array_value(value)
}
