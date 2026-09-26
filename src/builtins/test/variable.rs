use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};
use std::collections::HashMap;
use std::env;

const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";

pub(crate) fn variable_is_set(operand: &str, env_vars: &HashMap<String, String>) -> bool {
    if let Some(name) = operand
        .strip_suffix("[@]")
        .or_else(|| operand.strip_suffix("[*]"))
    {
        let arrays = marked_vars(env_vars, ARRAY_VARS);
        let assocs = marked_vars(env_vars, ASSOC_VARS);
        if assocs.iter().any(|marked| marked == name) {
            return false;
        }
        if arrays.iter().any(|marked| marked == name) {
            return env_vars
                .get(name)
                .map(|value| !array_entries(value).is_empty())
                .unwrap_or(false);
        }
        return env_vars.contains_key(name) || env::var_os(name).is_some();
    }

    // GNU test.c:650-668: a `[`-shaped operand that fails
    // valid_array_reference is NOT an array reference at all and falls
    // through to find_variable(arg), the literal-name lookup below. The
    // flag set reaching valid_array_reference is flag-0 for BOTH callers:
    // `test -v` passes AV_NOEXPAND (0x020), which sets none of the VA_*
    // bits the function consumes (arrayfunc.h:62 vs :69-70), and `[[ -v ]]`
    // forces array_expand_once off via set_expand_once(0,0)
    // (execute_cmd.c:4027). `a[80's]` (unterminated quote) and `F[]]`
    // (empty subscript) therefore report unset in every option state.
    let valid_arrayref = !operand.contains('[')
        || crate::executor::subscript_expansion::valid_array_reference_env(
            operand, false, false, env_vars,
        );
    if !valid_arrayref {
        return env_vars.contains_key(operand) || env::var_os(operand).is_some();
    }

    if let Some((name, subscript)) = parse_array_subscript(operand) {
        let arrays = marked_vars(env_vars, ARRAY_VARS);
        let assocs = marked_vars(env_vars, ASSOC_VARS);
        let Some(value) = env_vars.get(name) else {
            return subscript == "0" && dynamic_parameter_is_set(name);
        };

        if assocs.iter().any(|marked| marked == name) {
            // The executor's -v pre-pass (rewrite_operand_array_subscript)
            // hex-encodes the resolved key behind the \x1e carrier so `]`,
            // `=` or quoting inside a key cannot corrupt the re-parse.
            let key = crate::executor::arithmetic::decode_arithmetic_assoc_key(subscript)
                .unwrap_or_else(|| subscript.to_string());
            return assoc_key_is_set(value, &key);
        }

        if arrays.iter().any(|marked| marked == name) || is_array_storage(value) {
            return crate::executor::arithmetic::eval_conditional_arith_value(subscript, env_vars)
                .and_then(|index| resolve_array_index(value, index))
                .map(|index| array_index_is_set(value, index))
                .unwrap_or(false);
        }

        return subscript == "0" && (!value.is_empty() || env_vars.contains_key(name));
    }

    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let assocs = marked_vars(env_vars, ASSOC_VARS);
    if assocs.iter().any(|marked| marked == operand) {
        return env_vars
            .get(operand)
            .map(|value| assoc_key_is_set(value, "0"))
            .unwrap_or(false);
    }
    if arrays.iter().any(|marked| marked == operand) {
        return env_vars
            .get(operand)
            .map(|value| array_index_is_set(value, 0))
            .unwrap_or(false);
    }

    env_vars.contains_key(operand)
        || env::var_os(operand).is_some()
        || dynamic_parameter_is_set(operand)
}

fn dynamic_parameter_is_set(name: &str) -> bool {
    matches!(
        name,
        "EPOCHSECONDS"
            | "EPOCHREALTIME"
            | "SECONDS"
            | "RANDOM"
            | "SRANDOM"
            | "BASHPID"
            | "BASH_SUBSHELL"
            | "BASH_ARGV0"
            | "FUNCNAME"
            | "GROUPS"
            | "LINENO"
            | "BASH_COMMAND"
            | "SHELLOPTS"
            | "BASHOPTS"
            | "PIPESTATUS"
    )
}

fn parse_array_subscript(value: &str) -> Option<(&str, &str)> {
    let (name, subscript) = value.split_once('[')?;
    Some((name, subscript.strip_suffix(']')?))
}

fn is_array_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')') || value.starts_with(STORAGE_WORD_PREFIX)
}

fn array_index_is_set(value: &str, index: usize) -> bool {
    array_entries(value)
        .into_iter()
        .any(|(entry_index, _)| entry_index == index)
}

fn resolve_array_index(value: &str, index: i128) -> Option<usize> {
    if index >= 0 {
        return usize::try_from(index).ok();
    }

    let max_index = array_entries(value)
        .into_iter()
        .map(|(entry_index, _)| entry_index)
        .max()?;
    let resolved = i128::try_from(max_index)
        .ok()?
        .checked_add(1)?
        .checked_add(index)?;
    usize::try_from(resolved).ok()
}

fn array_entries(value: &str) -> Vec<(usize, String)> {
    let value = value.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };

    inner
        .split_whitespace()
        .enumerate()
        .filter_map(|(default_index, part)| {
            if let Some((left, right)) = part.split_once('=') {
                let index = left
                    .strip_prefix('[')
                    .and_then(|left| left.strip_suffix(']'))
                    .and_then(|index| index.parse::<usize>().ok())?;
                return Some((index, strip_array_value_quotes(right).to_string()));
            }
            Some((default_index, strip_array_value_quotes(part).to_string()))
        })
        .collect()
}

fn assoc_key_is_set(value: &str, key: &str) -> bool {
    let value = value.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };

    split_storage_words(inner).any(|part| {
        let Some((left, _)) = part.split_once('=') else {
            return false;
        };
        left.strip_prefix('[')
            .and_then(|left| left.strip_suffix(']'))
            .map(unquote_storage_value)
            .as_deref()
            == Some(key)
    })
}

fn split_storage_words(value: &str) -> impl Iterator<Item = String> + '_ {
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

fn unquote_storage_value(value: &str) -> String {
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
    unquoted
}

fn strip_array_value_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn marked_vars(env_vars: &HashMap<String, String>, key: &str) -> Vec<String> {
    env_vars
        .get(key)
        .map(|value| {
            value
                .split(DATA_DOLLAR)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
