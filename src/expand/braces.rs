//! Brace expansion for {a,b,c} comma-separated lists and {1..5}/{a..e} sequences.
//!
//! GNU Bash source ownership:
// - braces.c

/// Expand brace patterns in a word, returning multiple words.
/// Handles {a,b,c} comma-separated lists and {1..5}, {a..e} sequences.
/// Returns a single-element vec if no braces found (no expansion needed).
pub fn expand_braces(word: &str) -> Vec<String> {
    let mut result = vec![word.to_string()];
    let mut changed = true;
    while changed {
        changed = false;
        let mut new_result = Vec::new();
        for w in &result {
            if let Some(expanded) = expand_single_brace(w) {
                new_result.extend(expanded);
                changed = true;
            } else {
                new_result.push(w.clone());
            }
        }
        result = new_result;
    }
    result
}

fn expand_single_brace(s: &str) -> Option<Vec<String>> {
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip escaped characters
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        // Skip single-quoted strings
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        // Skip double-quoted strings
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        // Skip ${...} parameter expansions
        if i > 0 && bytes[i - 1] == b'$' {
            i += 1;
            continue;
        }

        let prefix = &s[..i];
        let inner_start = i + 1;
        let mut depth = 1u32;
        let mut j = inner_start;
        let mut has_comma = false;
        let mut has_double_dot = false;

        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                b',' if depth == 1 => has_comma = true,
                b'.' if depth == 1 && j + 1 < bytes.len() && bytes[j + 1] == b'.' => {
                    has_double_dot = true;
                }
                b'\\' => {
                    j += 1;
                }
                _ => {}
            }
            j += 1;
        }

        if depth != 0 {
            i += 1;
            continue;
        }

        let inner = &s[inner_start..j];
        let suffix = &s[j + 1..];

        if has_comma {
            let items: Vec<&str> = split_brace_commas(inner);
            if items.len() >= 2 {
                let mut out = Vec::new();
                for item in items {
                    out.push(format!("{prefix}{item}{suffix}"));
                }
                return Some(out);
            }
        } else if has_double_dot {
            if let Some(items) = expand_range(inner) {
                let mut out = Vec::new();
                for item in items {
                    out.push(format!("{prefix}{item}{suffix}"));
                }
                return Some(out);
            }
        }

        i += 1;
    }
    None
}

fn split_brace_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0u32;
    let mut start = 0;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            b'\\' => {} // skip escaped
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

fn expand_range(s: &str) -> Option<Vec<String>> {
    let parts = s.split("..").collect::<Vec<_>>();
    let ([left, right] | [left, right, _]) = parts.as_slice() else {
        return None;
    };
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let step = match parts.as_slice() {
        [_, _] => 1,
        [_, _, step] => step.parse::<i64>().ok()?.abs().max(1),
        _ => return None,
    };

    // Numeric range
    if let (Ok(start), Ok(end)) = (left.parse::<i64>(), right.parse::<i64>()) {
        let width = numeric_range_width(left, right);
        let step = if start <= end { step } else { -step };
        let mut result = Vec::new();
        let mut current = start;
        while (step > 0 && current <= end) || (step < 0 && current >= end) {
            result.push(format_numeric_range_value(current, width));
            current += step;
        }
        return Some(result);
    }
    // Alpha range
    let start = left.as_bytes()[0];
    let end = right.as_bytes()[0];
    if left.len() == 1
        && right.len() == 1
        && start.is_ascii_alphabetic()
        && end.is_ascii_alphabetic()
    {
        let step = i16::try_from(step).ok()?;
        let step: i16 = if start <= end { step } else { -step };
        let mut result = Vec::new();
        let mut current = start as i16;
        while (step > 0 && current <= end as i16) || (step < 0 && current >= end as i16) {
            result.push((current as u8 as char).to_string());
            current += step;
        }
        return Some(result);
    }
    None
}

fn numeric_range_width(left: &str, right: &str) -> Option<usize> {
    let left_digits = left.trim_start_matches('-');
    let right_digits = right.trim_start_matches('-');
    let padded = [left_digits, right_digits]
        .iter()
        .any(|value| value.len() > 1 && value.starts_with('0'));
    padded.then(|| left_digits.len().max(right_digits.len()))
}

fn format_numeric_range_value(value: i64, width: Option<usize>) -> String {
    let Some(width) = width else {
        return value.to_string();
    };
    if value < 0 {
        format!("-{:0width$}", value.unsigned_abs(), width = width)
    } else {
        format!("{value:0width$}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comma_brace() {
        assert_eq!(expand_braces("{a,b,c}"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_nested_comma() {
        assert_eq!(expand_braces("x{a,b}y"), vec!["xay", "xby"]);
    }

    #[test]
    fn test_range_numeric() {
        assert_eq!(expand_braces("{1..3}"), vec!["1", "2", "3"]);
    }

    #[test]
    fn test_range_numeric_step_and_padding() {
        assert_eq!(expand_braces("{1..5..2}"), vec!["1", "3", "5"]);
        assert_eq!(expand_braces("{1..6..4}"), vec!["1", "5"]);
        assert_eq!(expand_braces("{5..1..2}"), vec!["5", "3", "1"]);
        assert_eq!(expand_braces("{01..03}"), vec!["01", "02", "03"]);
    }

    #[test]
    fn test_range_alpha() {
        assert_eq!(expand_braces("{a..c}"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_range_alpha_step() {
        assert_eq!(expand_braces("{a..e..2}"), vec!["a", "c", "e"]);
        assert_eq!(expand_braces("{e..a..2}"), vec!["e", "c", "a"]);
    }

    #[test]
    fn test_no_brace() {
        assert_eq!(expand_braces("hello"), vec!["hello"]);
    }
}
