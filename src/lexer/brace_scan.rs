use super::token::{Token, TokenKind};

pub(super) fn has_unclosed_brace_group(input: &str) -> bool {
    let trimmed = input.trim_start();
    if !(trimmed.starts_with('{')
        || input.contains("&& {")
        || input.contains("|| {")
        || input.contains("; {"))
    {
        return false;
    }

    unquoted_brace_group_depth(input) > 0
}

pub(super) fn opens_function_body_after_previous_signature(input: &str, output: &[Token]) -> bool {
    if input.trim() != "{" {
        return false;
    }

    output
        .iter()
        .rev()
        .find(|token| token.kind != TokenKind::Semicolon)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == ")")
}

pub(super) fn unquoted_brace_group_depth(input: &str) -> usize {
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if single || double {
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'{') {
            index = skip_braced_parameter_in_chars(&chars, index + 2);
            continue;
        }
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }

    depth
}

pub(super) fn skip_braced_parameter_in_chars(chars: &[char], mut index: usize) -> usize {
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
        }
        index += 1;
    }
    index
}
