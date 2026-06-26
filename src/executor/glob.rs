//! Pathname expansion (globbing) for command words.

use std::fs;

/// Expand glob patterns (* ? [...]) in a word against the filesystem.
pub(crate) fn pathname_expand_word(word: &str) -> Option<Vec<String>> {
    if word.starts_with('"') || word.starts_with('\'' ) {
        return None;
    }
    if !word.chars().any(|c| c == '*' || c == '?' || c == '[') {
        return None;
    }
    if word.contains('=') || word.contains('{') || word.contains('}') {
        return None;
    }
    let (dir_path, pattern) = match word.rsplit_once('/') {
        Some((d, p)) => (d.to_string(), p),
        None => (".".to_string(), word.as_ref()),
    };
    let include_dotfiles = pattern.starts_with('.');
    let entries = match fs::read_dir(&dir_path) {
        Ok(rd) => rd,
        Err(_) => return None,
    };
    let mut matches: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !include_dotfiles && name.starts_with('.') {
                return None;
            }
            if super::case_pattern_matches(pattern, &name) {
                if dir_path == "." {
                    Some(name)
                } else {
                    Some(format!("{dir_path}/{name}"))
                }
            } else {
                None
            }
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    Some(matches)
}
