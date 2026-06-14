//! indexed module.
//!
//! GNU Bash source ownership:
// - array.c
// - array.h

pub fn is_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')')
}

pub fn values(value: &str) -> Vec<String> {
    entries(value)
        .into_iter()
        .map(|(_index, value)| value)
        .collect()
}

pub fn entries(value: &str) -> Vec<(usize, String)> {
    // TODO(array.c/subst.c): This is a lossy storage parser while arrays live
    // in the scalar variable table. Replace it with a real ARRAY model.
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![(0, value.to_string())]
        };
    };

    if inner.is_empty() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    let mut next_index = 0;
    for part in inner.split_whitespace() {
        if part.starts_with('[') && indexed_assignment(part).is_none() {
            if let Some((_left, value)) = part.split_once('=') {
                entries.push((next_index, value.trim_matches('"').to_string()));
                next_index += 1;
            }
            continue;
        }
        if let Some((index, value)) = indexed_assignment(part) {
            entries.push((index, value.trim_matches('"').to_string()));
            next_index = next_index.max(index.saturating_add(1));
        } else {
            entries.push((next_index, part.trim_matches('"').to_string()));
            next_index += 1;
        }
    }
    entries.sort_by_key(|(index, _value)| *index);
    entries
}

fn indexed_assignment(part: &str) -> Option<(usize, &str)> {
    // TODO(array.c): Real Bash ARRAY_ELEMENT storage is structured. While this
    // scalar encoding remains, only peel `[index]=value` renderings; arithmetic
    // expression elements like `(a[n]=++n)<7&&a[0]` must stay intact.
    let (left, value) = part.split_once('=')?;
    let index = left.strip_prefix('[')?.strip_suffix(']')?.parse().ok()?;
    Some((index, value))
}

pub fn value_at(value: &str, index: usize) -> String {
    entries(value)
        .into_iter()
        .find_map(|(entry_index, value)| (entry_index == index).then_some(value))
        .unwrap_or_default()
}

pub fn set_value_at(current: &str, index: usize, value: String) -> String {
    let mut entries = entries(current);
    if let Some((_entry_index, entry_value)) = entries
        .iter_mut()
        .find(|(entry_index, _value)| *entry_index == index)
    {
        *entry_value = value;
    } else {
        entries.push((index, value));
    }
    entries.sort_by_key(|(index, _value)| *index);
    format!(
        "({})",
        entries
            .into_iter()
            .map(|(index, value)| format!("[{index}]={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
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
