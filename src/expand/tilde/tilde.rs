//! tilde module.
//!
//! GNU Bash source ownership:
// - lib/tilde/tilde.c
// - lib/tilde/tilde.h

use std::collections::HashMap;

pub const QUOTED_ASSIGNMENT_VALUE: char = '\x1c';

pub fn home_value(env_vars: &HashMap<String, String>) -> String {
    env_vars
        .get("HOME")
        .filter(|v| !v.is_empty())
        .cloned()
        .or_else(|| std::env::var("HOME").ok().filter(|v| !v.is_empty()))
        .or_else(|| env_vars.get("USERPROFILE").cloned().filter(|v| !v.is_empty()))
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

pub fn expand_word_prefix(word: &str, env_vars: &HashMap<String, String>) -> Option<String> {
    if let Some(rest) = word.strip_prefix("~/") {
        return Some(format!("{}/{}", home_value(env_vars), rest));
    }

    match word {
        "~" => Some(home_value(env_vars)),
        "~+" => env_vars.get("PWD").cloned(),
        "~-" => env_vars.get("OLDPWD").cloned(),
        _ => None,
    }
}

pub fn expand_assignment_value(value: &str, env_vars: &HashMap<String, String>) -> String {
    let Some(value) = value.strip_prefix(QUOTED_ASSIGNMENT_VALUE) else {
        return expand_assignment_tilde_value(value, &home_value(env_vars), true);
    };

    value.to_string()
}

pub fn strip_assignment_quote_marker(value: &str) -> &str {
    value.strip_prefix(QUOTED_ASSIGNMENT_VALUE).unwrap_or(value)
}

pub fn expand_assignment_tilde_value(value: &str, home: &str, expand_after_colon: bool) -> String {
    if home.is_empty() {
        return value.to_string();
    }

    if !expand_after_colon {
        return expand_tilde_segment(value, home);
    }

    let mut output = String::new();
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if index == 0 || ch != ':' {
            continue;
        }
        output.push_str(&expand_tilde_segment(&value[start..index], home));
        output.push(':');
        start = index + ch.len_utf8();
    }
    output.push_str(&expand_tilde_segment(&value[start..], home));
    output
}

fn expand_tilde_segment(segment: &str, home: &str) -> String {
    let Some(rest) = segment.strip_prefix('~') else {
        return segment.to_string();
    };

    if rest.is_empty() {
        return home.to_string();
    }

    if rest.starts_with('/') {
        return format!("{home}{rest}");
    }

    segment.to_string()
}
