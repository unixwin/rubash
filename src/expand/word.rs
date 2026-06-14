//! word module.
//!
//! GNU Bash source ownership:
// - subst.c

pub fn substring_parameter(name: &str) -> Option<(&str, &str, Option<&str>)> {
    // TODO(subst.c/expr.c): Bash evaluates substring offset and length as
    // arithmetic expressions after nested parameter expansion. This still
    // handles only scalar shell-name parameters.
    let (var_name, rest) = name.split_once(':')?;
    if !is_shell_name(var_name) {
        return None;
    }
    if rest
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '-' | '=' | '+' | '?'))
    {
        return None;
    }
    let (offset, length) = if let Some((offset, length)) = split_substring_offset_length(rest) {
        (offset, Some(length))
    } else {
        (rest, None)
    };
    Some((var_name, offset, length))
}

pub fn substring_value(value: &str, offset: usize, length: Option<usize>) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if offset >= chars.len() {
        return String::new();
    }
    let end = length
        .map(|length| offset.saturating_add(length).min(chars.len()))
        .unwrap_or(chars.len());
    chars[offset..end].iter().collect()
}

pub fn read_braced_parameter_name<I>(chars: &mut std::iter::Peekable<I>) -> String
where
    I: Iterator<Item = char>,
{
    let mut name = String::new();
    let mut nested = 0_i32;
    while let Some(ch) = chars.next() {
        match ch {
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                nested += 1;
                name.push('$');
                name.push('{');
            }
            '}' if nested > 0 => {
                nested -= 1;
                name.push('}');
            }
            '}' => break,
            _ => name.push(ch),
        }
    }
    name
}

pub fn array_slice_parameter(name: &str) -> Option<(&str, usize)> {
    // TODO(subst.c/arrayfunc.c): Bash supports full substring expansion for
    // arrays, negative offsets, lengths, quoting, and sparse indices. This
    // maps the positive `${array[@]:N}` shape used by arith6.sub.
    let (array_expr, offset) = name.split_once(':')?;
    let array_name = array_expr
        .strip_suffix("[@]")
        .or_else(|| array_expr.strip_suffix("[*]"))?;
    let offset = offset.parse().ok()?;
    Some((array_name, offset))
}

pub fn array_values_for_slice(value: &str) -> Vec<String> {
    if !crate::shell::arrays::indexed::is_storage(value) {
        return crate::shell::arrays::indexed::values(value);
    }
    crate::shell::arrays::indexed::values(value)
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect()
}

fn split_substring_offset_length(value: &str) -> Option<(&str, &str)> {
    // TODO(subst.c/expr.c): The offset and length are arithmetic expressions,
    // so `:` can belong to `?:`. For now, keep ternary offsets intact and use
    // the final top-level colon as length only when another top-level colon is
    // already present.
    let mut depth = 0_i32;
    let mut brace_depth = 0_i32;
    let mut colons = Vec::new();
    let mut previous = '\0';
    for (index, ch) in value.char_indices() {
        match ch {
            '(' | '[' if brace_depth == 0 => depth += 1,
            ')' | ']' if brace_depth == 0 && depth > 0 => depth -= 1,
            '{' if previous == '$' => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            ':' if depth == 0 && brace_depth == 0 => colons.push(index),
            _ => {}
        }
        previous = ch;
    }
    let split = if colons.len() > 1 || !value.contains('?') {
        colons.last().copied()?
    } else {
        return None;
    };
    Some((&value[..split], &value[split + 1..]))
}

fn is_shell_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
