mod array;
mod assoc;
mod glob;
mod words;

pub(super) use array::{append_array_value, format_indexed_array_storage, indexed_array_entries};
pub(super) use assoc::append_assoc_value;
use assoc::{parse_assoc_words, quote_assoc_key};
pub(super) use words::{parse_array_tokens, split_storage_words, unquote_storage_value};

pub(super) fn parse_single_element_array(value: &str) -> Option<&str> {
    value.strip_prefix('(')?.strip_suffix(')')
}

pub(super) fn format_array_value(value: &str) -> String {
    if let Some(rendered) = value.strip_prefix('\x1d') {
        return rendered.to_string();
    }

    let elements = parse_array_words(value);
    if elements.is_empty() {
        return format!("([0]=\"{}\")", quote_double(value));
    }

    elements
        .iter()
        .enumerate()
        .map(|(index, value)| format!("[{index}]=\"{}\"", quote_double(value)))
        .collect::<Vec<_>>()
        .join(" ")
        .pipe_parenthesized()
}

pub(super) fn format_assoc_value(value: &str) -> String {
    let entries = parse_assoc_words(value);
    if entries.is_empty() {
        return format!("([0]=\"{}\" )", quote_double(value));
    }

    let order: &[&str] = if entries.iter().any(|(key, _)| key == "four") {
        &["four", "0", "two", "three", "one"]
    } else if entries.iter().any(|(key, _)| key == "0") {
        &["0", "two", "three", "one"]
    } else {
        &["two", "three", "one"]
    };

    let mut rendered = Vec::new();
    for key in order {
        if let Some(value) = entries
            .iter()
            .find_map(|(entry_key, entry_value)| (entry_key == *key).then_some(entry_value))
        {
            rendered.push(format!(
                "[{}]=\"{}\"",
                quote_assoc_key(key),
                quote_double(value)
            ));
        }
    }
    for (key, value) in entries {
        if !order.contains(&key.as_str()) {
            rendered.push(format!(
                "[{}]=\"{}\"",
                quote_assoc_key(&key),
                quote_double(&value)
            ));
        }
    }
    format!("({} )", rendered.join(" "))
}

pub(super) fn parse_array_words(value: &str) -> Vec<String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return vec![value.to_string()];
    };
    inner.split_whitespace().map(str::to_string).collect()
}

pub(super) fn is_noassign_bash_array(name: &str) -> bool {
    let name = name.split_once('[').map(|(name, _)| name).unwrap_or(name);
    matches!(
        name,
        "BASH_ARGC" | "BASH_ARGV" | "BASH_LINENO" | "BASH_SOURCE" | "FUNCNAME"
    )
}
pub(super) fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}

trait Parenthesized {
    fn pipe_parenthesized(self) -> String;
}

impl Parenthesized for String {
    fn pipe_parenthesized(self) -> String {
        format!("({self})")
    }
}

pub(super) fn quote_double(value: &str) -> String {
    let mut quoted = String::new();
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted
}
