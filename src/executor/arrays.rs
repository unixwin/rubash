//! Array-related functions for the executor module.
//!
//! Contains free functions and `Executor` methods for working with
//! indexed arrays, array storage, and array subscripts.

mod executor;
mod mapfile;
mod storage;

pub(super) use mapfile::split_mapfile_input;
pub(super) use storage::{
    array_indices, array_value_at, array_values, format_indexed_array_storage,
    indexed_array_entries, is_array_storage, is_marked_array_var, normalize_array_expanded_value,
    parse_array_integer_subscript, parse_array_numeric_subscript, parse_array_subscript,
    quote_array_value, resolve_indexed_array_subscript, store_indexed_array,
};

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;

use super::{
    assoc_entries, assoc_value_at, case_pattern_matches, eval_arith_value,
    eval_conditional_arith_value, is_marked_var, is_shell_name, pattern_contains_glob,
    quote_assoc_key, split_storage_words, strip_matching_quotes, unquote_storage_value, Executor,
    ASSOC_VARS,
};

pub(super) fn is_array_element_assignment_word(word: &str) -> bool {
    let Some((left, _)) = word.split_once('=') else {
        return false;
    };
    let left = left.strip_suffix('+').unwrap_or(left);
    let Some((name, index)) = left.split_once('[') else {
        return false;
    };
    is_shell_name(name) && index.ends_with(']')
}

pub(super) fn append_scalar_value(current: &str, value: &str) -> String {
    let mut output = current.to_string();
    output.push_str(value);
    output
}

pub(super) fn field_split_values_with_ifs(value: &str, ifs: Option<&str>) -> Vec<String> {
    let Some(ifs) = ifs else {
        return value.split_whitespace().map(str::to_string).collect();
    };
    if ifs.is_empty() {
        return vec![value.to_string()];
    }

    value
        .split(|ch| ifs.contains(ch))
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn pathname_expand_array_token(token: &str) -> Option<Vec<String>> {
    if token.starts_with('"') || token.starts_with('\'') || !pattern_contains_glob(token) {
        return None;
    }
    if token.contains('/') || token.contains('\\') {
        return None;
    }
    let include_dotfiles = token.starts_with('.');
    let mut matches = fs::read_dir(env::current_dir().ok()?)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| include_dotfiles || !name.starts_with('.'))
        .filter(|name| case_pattern_matches(token, name))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    Some(matches)
}

pub(super) fn append_array_value(
    current: &str,
    value: &str,
    integer: bool,
    ifs: Option<&str>,
    env_vars: &HashMap<String, String>,
) -> String {
    let mut entries = indexed_array_entries(current);
    let mut next_index = entries
        .keys()
        .next_back()
        .map(|index| index + 1)
        .unwrap_or(0);
    let scalar_append = integer && !value.starts_with('(');
    for token in array_assignment_tokens(value) {
        if let Some(matches) = pathname_expand_array_token(&token) {
            for value in matches {
                entries.insert(next_index, value);
                next_index += 1;
            }
            continue;
        }

        if let Some((left, rhs)) = token.split_once("+=") {
            if let Some(index) = array_assignment_index(left, &entries, env_vars) {
                let current = entries.get(&index).cloned().unwrap_or_default();
                let rhs = unquote_storage_value(rhs);
                let value = if integer {
                    (eval_arith_value(&current) + eval_arith_value(&rhs)).to_string()
                } else {
                    append_scalar_value(&current, &rhs)
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
            if let Some(index) = array_assignment_index(left, &entries, env_vars) {
                entries.insert(index, unquote_storage_value(rhs));
                next_index = index + 1;
                continue;
            }
            if array_assignment_has_subscript(left) {
                continue;
            }
        }

        let command_subst_token = token.starts_with("\"$(") && token.ends_with('"');
        let quoted_token = token.starts_with('"') && token.ends_with('"') && !command_subst_token;
        let token = unquote_storage_value(&token);
        if let Some(expanded_array) = token.strip_prefix('\x1d') {
            for value in field_split_values_with_ifs(expanded_array, ifs) {
                entries.insert(next_index, value.to_string());
                next_index += 1;
            }
            continue;
        }
        if token.contains('\n') || (token.contains(char::is_whitespace) && !quoted_token) {
            for value in field_split_values_with_ifs(&token, ifs) {
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

pub(super) fn array_assignment_index(
    left: &str,
    entries: &BTreeMap<usize, String>,
    env_vars: &HashMap<String, String>,
) -> Option<usize> {
    let expression = left.strip_prefix('[')?.strip_suffix(']')?;
    let index = eval_conditional_arith_value(expression, env_vars)?;
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

pub(super) fn array_assignment_has_subscript(left: &str) -> bool {
    left.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .is_some()
}

pub(super) fn array_assignment_tokens(value: &str) -> Vec<String> {
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

    split_storage_words(inner).collect()
}

pub(super) fn array_parameter_slice(
    value: &str,
    offset: isize,
    length: Option<usize>,
) -> Vec<String> {
    let values = array_values(value);
    let start = if offset < 0 {
        values.len().saturating_sub(offset.unsigned_abs())
    } else {
        offset as usize
    };

    values
        .into_iter()
        .skip(start)
        .take(length.unwrap_or(usize::MAX))
        .collect()
}

pub(super) fn is_noassign_bash_array(name: &str) -> bool {
    matches!(
        name,
        "BASH_ARGC" | "BASH_ARGV" | "BASH_LINENO" | "BASH_SOURCE" | "FUNCNAME"
    )
}
