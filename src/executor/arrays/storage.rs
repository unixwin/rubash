use std::collections::{BTreeMap, HashMap};

use crate::executor::{mark_env_name, split_storage_words, unquote_storage_value, ARRAY_VARS};

pub(in crate::executor) fn normalize_array_expanded_value(value: String) -> String {
    if value.contains('"') && value.chars().all(|ch| matches!(ch, '\\' | '"')) {
        "\"\"".to_string()
    } else {
        value
    }
}

pub(in crate::executor) fn array_values(value: &str) -> Vec<String> {
    // TODO(array.c/assoc.c/subst.c): This is a lossy representation used while
    // arrays are still stored in the scalar variable table.
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered_array_values(rendered);
    }

    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![value.to_string()]
        };
    };

    if inner.is_empty() {
        return Vec::new();
    }

    split_storage_words(inner)
        .map(|part| {
            let value = part
                .split_once('=')
                .map(|(_, value)| value)
                .map(unquote_storage_value)
                .unwrap_or_else(|| unquote_storage_value(&part));
            normalize_array_expanded_value(value)
        })
        .collect()
}

pub(in crate::executor) fn indexed_array_entries(value: &str) -> BTreeMap<usize, String> {
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered_array_entries(rendered);
    }

    array_values(value).into_iter().enumerate().collect()
}

pub(in crate::executor) fn array_indices(value: &str) -> Vec<String> {
    indexed_array_entries(value)
        .keys()
        .map(usize::to_string)
        .collect()
}

pub(in crate::executor) fn array_value_at(value: &str, index: usize) -> Option<String> {
    let mut entries = indexed_array_entries(value);
    entries.remove(&index).map(normalize_array_expanded_value)
}

pub(in crate::executor) fn resolve_indexed_array_subscript(
    value: &str,
    index: i128,
) -> Option<usize> {
    if index >= 0 {
        return usize::try_from(index).ok();
    }

    let max_index = indexed_array_entries(value).keys().next_back().copied()?;
    let resolved = i128::try_from(max_index)
        .ok()?
        .checked_add(1)?
        .checked_add(index)?;
    usize::try_from(resolved).ok()
}

pub(in crate::executor) fn parse_array_integer_subscript(name: &str) -> Option<(&str, i128)> {
    let (array_name, subscript) = parse_array_subscript(name)?;
    let index = subscript.parse::<i128>().ok()?;
    Some((array_name, index))
}

pub(in crate::executor) fn parse_array_numeric_subscript(name: &str) -> Option<(&str, usize)> {
    let (array_name, subscript) = parse_array_subscript(name)?;
    let index = subscript.parse::<usize>().ok()?;
    Some((array_name, index))
}

pub(in crate::executor) fn parse_array_subscript(name: &str) -> Option<(&str, &str)> {
    let (array_name, subscript) = name.split_once('[')?;
    Some((array_name, subscript.strip_suffix(']')?))
}

pub(in crate::executor) fn format_indexed_array_storage(
    entries: BTreeMap<usize, String>,
) -> String {
    let rendered = entries
        .into_iter()
        .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("\x1d({rendered})")
}

pub(in crate::executor) fn store_indexed_array(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    values: Vec<String>,
) {
    let entries = values.into_iter().enumerate().collect();
    env_vars.insert(name.to_string(), format_indexed_array_storage(entries));
    mark_env_name(env_vars, ARRAY_VARS, name);
}

pub(in crate::executor) fn quote_array_value(value: &str) -> String {
    if value.contains(['\n', '\r', '\'']) {
        return format!(
            "$'{}'",
            value
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\'', "\\'")
        );
    }

    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    )
}

pub(in crate::executor) fn is_array_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')') || value.starts_with('\x1d')
}

pub(in crate::executor) fn is_marked_array_var(
    env_vars: &HashMap<String, String>,
    name: &str,
) -> bool {
    const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
    const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
    [ARRAY_VARS, ASSOC_VARS].iter().any(|key| {
        env_vars
            .get(*key)
            .map(|value| value.split('\x1f').any(|marked| marked == name))
            .unwrap_or(false)
    })
}

fn rendered_array_values(value: &str) -> Vec<String> {
    rendered_array_entries(value).into_values().collect()
}

fn rendered_array_entries(value: &str) -> BTreeMap<usize, String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return BTreeMap::new();
    };

    rendered_array_parts(inner)
        .into_iter()
        .filter_map(|part| {
            let (key, value) = part.as_str().split_once('=')?;
            let index = key
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<usize>()
                .ok()?;
            Some((index, decode_rendered_array_value(value)))
        })
        .collect()
}

fn rendered_array_parts(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = inner.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(quote_ch) => {
                current.push(ch);
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else if ch == quote_ch {
                    quote = None;
                }
            }
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

fn decode_rendered_array_value(value: &str) -> String {
    if let Some(inner) = value
        .strip_prefix("$'")
        .and_then(|value| value.strip_suffix('\''))
    {
        return inner
            .replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\'", "'")
            .replace("\\\\", "\\");
    }

    unquote_storage_value(value)
}
