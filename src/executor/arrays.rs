//! Array-related functions for the executor module.
//!
//! Contains free functions and `Executor` methods for working with
//! indexed arrays, array storage, and array subscripts.

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;

use super::{
    assoc_entries, assoc_value_at, case_pattern_matches, eval_arith_value,
    eval_conditional_arith_value, is_marked_var, is_shell_name, mark_env_name,
    pattern_contains_glob, quote_assoc_key, split_storage_words, strip_matching_quotes,
    unquote_storage_value, ARRAY_VARS, ASSOC_VARS, Executor,
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

pub(super) fn normalize_array_expanded_value(value: String) -> String {
    if value.contains('"') && value.chars().all(|ch| matches!(ch, '\\' | '"')) {
        "\"\"".to_string()
    } else {
        value
    }
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
) -> Option<usize> {
    let index = left
        .strip_prefix('[')?
        .strip_suffix(']')?
        .parse::<i128>()
        .ok()?;
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

pub(super) fn array_parameter_slice(value: &str, offset: isize, length: Option<usize>) -> Vec<String> {
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

pub(super) fn array_values(value: &str) -> Vec<String> {
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

pub(super) fn indexed_array_entries(value: &str) -> BTreeMap<usize, String> {
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered_array_entries(rendered);
    }

    array_values(value).into_iter().enumerate().collect()
}

pub(super) fn array_indices(value: &str) -> Vec<String> {
    indexed_array_entries(value)
        .keys()
        .map(usize::to_string)
        .collect()
}

pub(super) fn array_value_at(value: &str, index: usize) -> Option<String> {
    let mut entries = indexed_array_entries(value);
    entries.remove(&index).map(normalize_array_expanded_value)
}

pub(super) fn resolve_indexed_array_subscript(value: &str, index: i128) -> Option<usize> {
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

pub(super) fn parse_array_integer_subscript(name: &str) -> Option<(&str, i128)> {
    let (array_name, subscript) = parse_array_subscript(name)?;
    let index = subscript.parse::<i128>().ok()?;
    Some((array_name, index))
}

pub(super) fn parse_array_numeric_subscript(name: &str) -> Option<(&str, usize)> {
    let (array_name, subscript) = parse_array_subscript(name)?;
    let index = subscript.parse::<usize>().ok()?;
    Some((array_name, index))
}

pub(super) fn parse_array_subscript(name: &str) -> Option<(&str, &str)> {
    let (array_name, subscript) = name.split_once('[')?;
    Some((array_name, subscript.strip_suffix(']')?))
}

pub(super) fn rendered_array_entries(value: &str) -> BTreeMap<usize, String> {
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

pub(super) fn format_indexed_array_storage(entries: BTreeMap<usize, String>) -> String {
    let rendered = entries
        .into_iter()
        .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("\x1d({rendered})")
}

pub(super) fn store_indexed_array(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    values: Vec<String>,
) {
    let entries = values.into_iter().enumerate().collect();
    env_vars.insert(name.to_string(), format_indexed_array_storage(entries));
    mark_env_name(env_vars, ARRAY_VARS, name);
}

pub(super) fn is_noassign_bash_array(name: &str) -> bool {
    matches!(
        name,
        "BASH_ARGC" | "BASH_ARGV" | "BASH_LINENO" | "BASH_SOURCE" | "FUNCNAME"
    )
}

pub(super) fn split_mapfile_input(
    input: &str,
    delimiter: Option<char>,
    trim_delimiter: bool,
) -> Vec<String> {
    let Some(delimiter) = delimiter else {
        return input
            .split_inclusive('\n')
            .map(|line| {
                if trim_delimiter {
                    line.trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .to_string()
                } else {
                    line.to_string()
                }
            })
            .collect();
    };

    let mut values = Vec::new();
    let mut current = String::new();
    for ch in input.chars() {
        current.push(ch);
        if ch == delimiter {
            if trim_delimiter {
                current.pop();
            }
            values.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        values.push(current);
    }
    values
}

pub(super) fn rendered_array_values(value: &str) -> Vec<String> {
    rendered_array_entries(value).into_values().collect()
}

pub(super) fn rendered_array_parts(inner: &str) -> Vec<String> {
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

pub(super) fn decode_rendered_array_value(value: &str) -> String {
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

pub(super) fn quote_array_value(value: &str) -> String {
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

pub(super) fn is_array_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')') || value.starts_with('\x1d')
}

pub(super) fn is_marked_array_var(env_vars: &HashMap<String, String>, name: &str) -> bool {
    const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
    const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
    [ARRAY_VARS, ASSOC_VARS].iter().any(|key| {
        env_vars
            .get(*key)
            .map(|value| value.split('\x1f').any(|marked| marked == name))
            .unwrap_or(false)
    })
}

impl Executor {
    pub(super) fn indexed_array_stack(&self, name: &str) -> Vec<String> {
        self.env_vars
            .get(name)
            .map(|value| array_values(value))
            .unwrap_or_default()
    }

    pub(super) fn array_assignment_transform(&self, name: &str) -> String {
        let Some(value) = self.env_vars.get(name) else {
            return String::new();
        };

        if is_marked_var(&self.env_vars, ASSOC_VARS, name) {
            let entries = assoc_entries(value);
            if entries.is_empty() {
                return format!("declare -A {name}");
            }
            let rendered = entries
                .into_iter()
                .map(|(key, value)| {
                    format!("[{}]={}", quote_assoc_key(&key), quote_array_value(&value))
                })
                .collect::<Vec<_>>()
                .join(" ");
            return format!("declare -A {name}=({rendered} )");
        }

        if is_marked_array_var(&self.env_vars, name) || is_array_storage(value) {
            let rendered = indexed_array_entries(value)
                .into_iter()
                .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
                .collect::<Vec<_>>()
                .join(" ");
            return format!("declare -a {name}=({rendered})");
        }

        String::new()
    }

    pub(super) fn array_element_parameter_value(&self, expression: &str) -> Option<String> {
        let (array_name, key) = parse_array_subscript(expression)?;
        let storage_name = self.resolved_variable_name(array_name)?;
        let storage = self.parameter_array_storage(array_name)?;
        if is_marked_var(&self.env_vars, ASSOC_VARS, &storage_name) {
            let key = self.assoc_subscript_key(key);
            return assoc_value_at(&storage, &key);
        }
        let key = strip_matching_quotes(&self.expand_embedded_parameters(key)).to_string();
        eval_conditional_arith_value(&key, &self.env_vars)
            .and_then(|index| resolve_indexed_array_subscript(&storage, index))
            .and_then(|index| array_value_at(&storage, index))
    }

    pub(super) fn array_length(&self, name: &str) -> usize {
        if name == "GROUPS" {
            return self.groups_words().len();
        }
        self.parameter_array_storage(name)
            .map(|value| array_values(&value).len())
            .unwrap_or(0)
    }

    pub(super) fn array_at_word_values(&self, word: &str) -> Option<Vec<String>> {
        let word = word
            .strip_prefix('"')
            .and_then(|word| word.strip_suffix('"'))
            .unwrap_or(word);
        let word = word.strip_prefix('\x1d').unwrap_or(word);
        let name = word.strip_prefix("${")?.strip_suffix("[@]}")?;
        if is_noassign_bash_array(name)
            || matches!(name, "BASH_ALIASES" | "BASH_CMDS" | "BASH_VERSINFO")
        {
            return None;
        }
        self.parameter_array_storage(name)
            .map(|value| array_values(&value))
    }
}
