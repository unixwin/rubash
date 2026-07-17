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
    let mut ansi_single = false;
    let mut escaped = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;

    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            index += 2;
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
        update_brace_group_case_depth(
            &chars,
            index,
            ch,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
        );
        match ch {
            '{' if case_depth == 0 => depth += 1,
            '}' if case_depth == 0 => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }

    depth
}

fn update_brace_group_case_depth(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
) {
    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if brace_group_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = update_brace_group_reserved_word_depth(
        chars,
        index,
        word,
        *current_word_boundary,
        case_depth,
    );
    word.clear();
    *word_boundary = reserved_word_allows_next || brace_group_separator_allows_reserved_word(ch);
}

fn update_brace_group_reserved_word_depth(
    chars: &[char],
    index: usize,
    word: &str,
    word_boundary: bool,
    case_depth: &mut usize,
) -> bool {
    if !word_boundary {
        return false;
    }

    match word {
        "case" => {
            *case_depth += 1;
            false
        }
        "esac" if !case_pattern_starts_with_esac_chars(chars, index) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "esac" => false,
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done" => true,
        _ => false,
    }
}

fn case_pattern_starts_with_esac_chars(chars: &[char], delimiter_index: usize) -> bool {
    if !matches!(chars.get(delimiter_index), Some(')' | '|')) {
        return false;
    }

    let mut close = delimiter_index;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return false,
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        return false;
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            return true;
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return true;
        }
        if word.is_empty() {
            if brace_group_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && brace_group_reserved_word_allows_next(&word);
        word.clear();
        word_boundary = reserved_word_allows_next || brace_group_separator_allows_reserved_word(ch);
        scan += 1;
    }

    word == "esac" && word_boundary
}

fn brace_group_reserved_word_allows_next(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "select"
            | "while"
            | "until"
            | "then"
            | "do"
            | "else"
            | "elif"
            | "in"
            | "fi"
            | "done"
            | "esac"
    )
}

fn brace_group_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '{' | '\n')
}

pub(super) fn skip_braced_parameter_in_chars(chars: &[char], mut index: usize) -> usize {
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            index += 2;
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
