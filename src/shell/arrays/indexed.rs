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
            indexed_assignment_value(part)
                .unwrap_or(part)
                .trim_matches('"')
                .to_string()
        })
        .collect()
}

fn indexed_assignment_value(part: &str) -> Option<&str> {
    // TODO(array.c): Real Bash ARRAY_ELEMENT storage is structured. While this
    // scalar encoding remains, only peel `[index]=value` renderings; arithmetic
    // expression elements like `(a[n]=++n)<7&&a[0]` must stay intact.
    let (left, value) = part.split_once('=')?;
    left.strip_prefix('[')?.strip_suffix(']')?;
    Some(value)
}

pub fn value_at(value: &str, index: usize) -> String {
    values(value).get(index).cloned().unwrap_or_default()
}

pub fn set_value_at(current: &str, index: usize, value: String) -> String {
    let mut elements = values(current);
    while elements.len() <= index {
        elements.push(String::new());
    }
    elements[index] = value;
    format!("({})", elements.join(" "))
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
