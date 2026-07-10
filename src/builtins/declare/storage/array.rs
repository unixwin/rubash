use std::collections::BTreeMap;

use super::glob::pathname_expand_array_token;
use super::{
    eval_arith_value, parse_array_tokens, parse_array_words, quote_double, split_storage_words,
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
    let scalar_append = integer && !value.starts_with('(');

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
                entries.insert(
                    index,
                    (eval_arith_value(&current) + eval_arith_value(&rhs)).to_string(),
                );
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
        let token = unquote_storage_value(&token);
        if let Some(expanded_array) = token.strip_prefix('\x1d') {
            for value in expanded_array.split_whitespace() {
                entries.insert(next_index, value.to_string());
                next_index += 1;
            }
            continue;
        }
        if scalar_append && !entries.is_empty() {
            let current = entries.get(&0).cloned().unwrap_or_default();
            entries.insert(
                0,
                (eval_arith_value(&current) + eval_arith_value(&token)).to_string(),
            );
        } else {
            entries.insert(next_index, token);
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

fn array_assignment_index(left: &str, entries: &BTreeMap<usize, String>) -> Option<usize> {
    let index = eval_arith_value(left.strip_prefix('[')?.strip_suffix(']')?);
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
    left.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .is_some()
}
pub(in crate::builtins::declare) fn indexed_array_entries(value: &str) -> BTreeMap<usize, String> {
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered_array_entries(rendered);
    }

    parse_array_words(value).into_iter().enumerate().collect()
}

pub(in crate::builtins::declare) fn rendered_array_entries(value: &str) -> BTreeMap<usize, String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return BTreeMap::new();
    };

    split_storage_words(inner)
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            let index = key
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<usize>()
                .ok()?;
            Some((index, unquote_storage_value(value)))
        })
        .collect()
}

pub(in crate::builtins::declare) fn format_indexed_array_storage(
    entries: BTreeMap<usize, String>,
) -> String {
    let rendered = entries
        .into_iter()
        .map(|(index, value)| format!("[{index}]=\"{}\"", quote_double(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("\x1d({rendered})")
}
