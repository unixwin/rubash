//! indexed module.
//!
//! GNU Bash source ownership:
// - array.c
// - array.h

pub fn is_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')')
}

pub fn values(value: &str) -> Vec<String> {
    // TODO(array.c/subst.c): This is a lossy storage parser while arrays live
    // in the scalar variable table. Replace it with a real ARRAY model.
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

    inner
        .split_whitespace()
        .map(|part| {
            part.split_once('=')
                .map(|(_, value)| value)
                .unwrap_or(part)
                .trim_matches('"')
                .to_string()
        })
        .collect()
}

pub fn assignment_index(left: &str) -> Option<usize> {
    left.strip_prefix('[')?.strip_suffix(']')?.parse().ok()
}

pub fn assignment_tokens(value: &str) -> Vec<String> {
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

    inner.split_whitespace().map(str::to_string).collect()
}
