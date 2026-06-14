//! assoc module.
//!
//! GNU Bash source ownership:
// - assoc.c
// - assoc.h

pub fn entries(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };

    inner
        .split_whitespace()
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((
                key.trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string(),
                value.to_string(),
            ))
        })
        .collect()
}

pub fn append_value(current: &str, value: &str) -> String {
    let mut entries = entries(current);
    for token in crate::shell::arrays::indexed::assignment_tokens(value) {
        if let Some((left, rhs)) = token.split_once('=') {
            if let Some(key) = left
                .strip_prefix('[')
                .and_then(|left| left.strip_suffix(']'))
            {
                entries.push((key.to_string(), rhs.to_string()));
                continue;
            }
        }
        entries.push(("0".to_string(), token));
    }

    format!(
        "({})",
        entries
            .into_iter()
            .map(|(key, value)| format!("[{key}]={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
