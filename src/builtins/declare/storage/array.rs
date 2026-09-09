use std::collections::BTreeMap;

use super::glob::pathname_expand_array_token;
use super::{
    eval_arith_value, parse_array_tokens, parse_array_words, split_storage_words,
    unquote_storage_value,
};

pub(in crate::builtins::declare) fn append_array_value(
    current: &str,
    value: &str,
    integer: bool,
) -> String {
    let mut entries = indexed_array_entries(current);
    let mut next_index = entries
        .keys()
        .next_back()
        .map(|index| index + 1)
        .unwrap_or(0);
    let scalar_append = !value.starts_with('(');

    for token in parse_array_tokens(value) {
        if let Some(matches) = pathname_expand_array_token(&token) {
            for value in matches {
                entries.insert(next_index, value);
                next_index += 1;
            }
            continue;
        }

        if let Some((left, rhs)) = token.split_once("+=") {
            if let Some(index) = array_assignment_index(left, &entries) {
                let current = entries.get(&index).cloned().unwrap_or_default();
                let rhs = unquote_storage_value(rhs);
                let value = if integer {
                    (eval_arith_value(&current) + eval_arith_value(&rhs)).to_string()
                } else {
                    format!("{current}{rhs}")
                };
                entries.insert(index, value);
                next_index = index + 1;
                continue;
            }
            if array_assignment_has_subscript(left) {
                continue;
            }
        }

        if let Some((left, rhs)) = token.split_once('=') {
            if let Some(index) = array_assignment_index(left, &entries) {
                entries.insert(index, unquote_storage_value(rhs));
                next_index = index + 1;
                continue;
            }
            if array_assignment_has_subscript(left) {
                continue;
            }
        }

        // GNU keeps whitespace inside a quoted compound word as ONE element
        // for both quote families ('a b' and "a b" each store a single
        // element; only unquoted whitespace splits).
        let quoted_token = (token.starts_with('"') && token.ends_with('"'))
            || (token.starts_with('\'') && token.ends_with('\''));
        let token = unquote_storage_value(&token);
        let unquoted_command_substitution = token.starts_with('\x1d');
        let token = token.strip_prefix('\x1d').unwrap_or(&token);
        if token.contains(char::is_whitespace) && (!quoted_token || unquoted_command_substitution) {
            for value in token.split_whitespace() {
                entries.insert(next_index, value.to_string());
                next_index += 1;
            }
            continue;
        }

        if scalar_append && !entries.is_empty() {
            let current = entries.get(&0).cloned().unwrap_or_default();
            let appended = if integer {
                (eval_arith_value(&current) + eval_arith_value(&token)).to_string()
            } else {
                format!("{current}{token}")
            };
            entries.insert(0, appended);
        } else {
            entries.insert(next_index, token.to_string());
            next_index += 1;
        }
    }

    if integer {
        for element in entries.values_mut() {
            *element = eval_arith_value(element).to_string();
        }
    }

    format_indexed_array_storage(entries)
}

pub(in crate::builtins::declare) fn indexed_array_entries(value: &str) -> BTreeMap<usize, String> {
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered_array_entries(rendered);
    }

    parse_array_words(value).into_iter().enumerate().collect()
}

fn rendered_array_entries(rendered: &str) -> BTreeMap<usize, String> {
    let inner = rendered
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(rendered);
    let mut default_index = 0;
    split_storage_words(inner)
        .filter_map(|part| {
            if let Some((left, right)) = part.split_once('=') {
                if let Some(index) = left
                    .strip_prefix('[')
                    .and_then(|value| value.strip_suffix(']'))
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    default_index = index + 1;
                    return Some((index, unquote_storage_value(right)));
                }
            }
            let index = default_index;
            default_index += 1;
            Some((index, unquote_storage_value(&part)))
        })
        .collect()
}

pub(in crate::builtins::declare) fn format_indexed_array_storage(
    entries: BTreeMap<usize, String>,
) -> String {
    let rendered = entries
        .into_iter()
        .map(|(index, value)| format!("[{index}]={}", super::quote_declare_value(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("\x1d({rendered})")
}

fn array_assignment_index(left: &str, entries: &BTreeMap<usize, String>) -> Option<usize> {
    let expression = left.strip_prefix('[')?.strip_suffix(']')?;
    if expression.trim().is_empty() {
        return entries
            .keys()
            .next_back()
            .map(|index| index + 1)
            .or(Some(0));
    }
    let index = eval_arith_value(expression);
    if index >= 0 {
        return usize::try_from(index).ok();
    }
    let max_index = entries.keys().next_back().copied()?;
    let resolved = i128::try_from(max_index)
        .ok()?
        .checked_add(1)?
        .checked_add(index)?;
    usize::try_from(resolved).ok()
}

fn array_assignment_has_subscript(left: &str) -> bool {
    left.contains('[') || left.contains(']')
}
