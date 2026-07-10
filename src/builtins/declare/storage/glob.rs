use std::{env, fs};

pub(in crate::builtins::declare) fn pathname_expand_array_token(
    token: &str,
) -> Option<Vec<String>> {
    if token.starts_with('"') || token.starts_with('\'') || !pattern_contains_glob(token) {
        return None;
    }
    if token.contains('/') || token.contains('\\') {
        return None;
    }
    let include_dotfiles = token.starts_with('.');
    let mut matches = fs::read_dir(env::current_dir().ok()?)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| include_dotfiles || !name.starts_with('.'))
        .filter(|name| glob_pattern_matches(token, name))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    Some(matches)
}

fn pattern_contains_glob(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']'))
}

fn glob_pattern_matches(pattern: &str, word: &str) -> bool {
    glob_pattern_matches_at(pattern.as_bytes(), 0, word.as_bytes(), 0)
}

fn glob_pattern_matches_at(pattern: &[u8], p_index: usize, word: &[u8], w_index: usize) -> bool {
    if p_index == pattern.len() {
        return w_index == word.len();
    }
    match pattern[p_index] {
        b'*' => {
            glob_pattern_matches_at(pattern, p_index + 1, word, w_index)
                || (w_index < word.len()
                    && glob_pattern_matches_at(pattern, p_index, word, w_index + 1))
        }
        b'?' => {
            w_index < word.len() && glob_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
        b'[' => {
            let Some(end) = pattern[p_index + 1..].iter().position(|ch| *ch == b']') else {
                return w_index < word.len()
                    && pattern[p_index] == word[w_index]
                    && glob_pattern_matches_at(pattern, p_index + 1, word, w_index + 1);
            };
            if w_index >= word.len() {
                return false;
            }
            let end = p_index + 1 + end;
            let class = &pattern[p_index + 1..end];
            let matched = class.iter().any(|ch| *ch == word[w_index]);
            matched && glob_pattern_matches_at(pattern, end + 1, word, w_index + 1)
        }
        current => {
            w_index < word.len()
                && current == word[w_index]
                && glob_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
    }
}
