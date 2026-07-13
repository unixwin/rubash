//! Pathname expansion (globbing) for command words.

use std::fs;
use std::path::Path;

/// Check if a shopt option is enabled.
fn shopt_enabled(env_vars: &std::collections::HashMap<String, String>, name: &str) -> bool {
    crate::builtins::shopt::option_enabled(env_vars, name)
}

/// Check if a word contains glob or extglob pattern characters.
fn contains_glob_or_extglob(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if matches!(ch, '*' | '?' | '[' | '\\') {
            return true;
        }
        if matches!(ch, '@' | '+' | '!') && chars.get(i + 1) == Some(&'(') {
            return true;
        }
    }
    false
}

fn contains_extglob(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    chars
        .iter()
        .enumerate()
        .any(|(i, ch)| matches!(ch, '@' | '*' | '+' | '?' | '!') && chars.get(i + 1) == Some(&'('))
}

/// Expand glob patterns (* ? [...]) in a word against the filesystem.
pub(crate) fn pathname_expand_word(
    word: &str,
    env_vars: &std::collections::HashMap<String, String>,
) -> Option<Vec<String>> {
    if word.is_empty() {
        return None;
    }
    if word.starts_with('"') || word.starts_with('\'') {
        return None;
    }
    if !contains_glob_or_extglob(word) {
        return None;
    }
    if word.contains('=') || word.contains('{') || word.contains('}') {
        return None;
    }

    let nullglob = shopt_enabled(env_vars, "nullglob");
    let dotglob = shopt_enabled(env_vars, "dotglob");
    let nocaseglob = shopt_enabled(env_vars, "nocaseglob");
    let globstar = shopt_enabled(env_vars, "globstar");
    let extglob = shopt_enabled(env_vars, "extglob");

    if word.contains("**") && globstar {
        return globstar_expand(word, nullglob, nocaseglob, dotglob);
    }

    if word.contains('/') {
        return pathname_expand_segments(word, nullglob, nocaseglob, dotglob, extglob);
    }

    let (dir_path, pattern) = match word.rsplit_once('/') {
        Some((d, p)) => (d.to_string(), p),
        None => (".".to_string(), word.as_ref()),
    };
    let include_dotfiles = dotglob || pattern.starts_with('.');
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
            let matched = pathname_pattern_matches(pattern, &name, nocaseglob, extglob);
            if matched {
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
        if nullglob {
            return Some(Vec::new());
        }
        return None;
    }
    matches.sort();
    Some(matches)
}

fn pathname_expand_segments(
    word: &str,
    nullglob: bool,
    nocaseglob: bool,
    dotglob: bool,
    extglob: bool,
) -> Option<Vec<String>> {
    let parts: Vec<&str> = word.split('/').collect();
    let mut prefixes = if word.starts_with('/') {
        vec!["/".to_string()]
    } else {
        vec![String::new()]
    };
    let mut saw_pattern = false;

    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let is_last = index == parts.len() - 1;
        let part_has_pattern = contains_glob_or_extglob(part);
        saw_pattern |= part_has_pattern;
        let mut next = Vec::new();

        for prefix in &prefixes {
            if part_has_pattern {
                let dir = if prefix.is_empty() {
                    "."
                } else {
                    prefix.as_str()
                };
                let entries = match fs::read_dir(dir) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                let include_dotfiles = dotglob || part.starts_with('.');
                for entry in entries.filter_map(Result::ok) {
                    let Some(name) = entry.file_name().into_string().ok() else {
                        continue;
                    };
                    if !include_dotfiles && name.starts_with('.') {
                        continue;
                    }
                    if pathname_pattern_matches(part, &name, nocaseglob, extglob) {
                        next.push(join_path_segment(prefix, &name));
                    }
                }
            } else {
                let candidate = join_path_segment(prefix, part);
                if !is_last || !saw_pattern || Path::new(&candidate).exists() {
                    next.push(candidate);
                }
            }
        }

        prefixes = next;
        if prefixes.is_empty() {
            break;
        }
    }

    if prefixes.is_empty() {
        if nullglob {
            return Some(Vec::new());
        }
        return None;
    }
    prefixes.sort();
    Some(prefixes)
}

fn join_path_segment(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_string()
    } else if prefix == "/" {
        format!("/{segment}")
    } else {
        format!("{prefix}/{segment}")
    }
}

fn pathname_pattern_matches(pattern: &str, word: &str, nocaseglob: bool, extglob: bool) -> bool {
    if extglob && contains_extglob(pattern) {
        extglob_pattern_matches(pattern, word, nocaseglob)
    } else if nocaseglob {
        case_pattern_matches_nocase(pattern, word)
    } else {
        super::case_pattern_matches(pattern, word)
    }
}

fn case_pattern_matches_nocase(pattern: &str, word: &str) -> bool {
    let pattern_lower = pattern.to_lowercase();
    let word_lower = word.to_lowercase();
    super::case_pattern_matches(&pattern_lower, &word_lower)
}

fn extglob_pattern_matches(pattern: &str, word: &str, nocaseglob: bool) -> bool {
    if nocaseglob {
        let pattern_lower = pattern.to_lowercase();
        let word_lower = word.to_lowercase();
        super::conditional::extglob_case_pattern_matches(&pattern_lower, &word_lower)
    } else {
        super::conditional::extglob_case_pattern_matches(pattern, word)
    }
}

fn globstar_expand(
    word: &str,
    nullglob: bool,
    nocaseglob: bool,
    dotglob: bool,
) -> Option<Vec<String>> {
    let parts: Vec<&str> = word.split("**").collect();
    if parts.len() != 2 {
        return None;
    }
    let prefix = parts[0];
    let suffix = parts[1].trim_start_matches('/');

    let base_dir = if prefix.is_empty() {
        ".".to_string()
    } else {
        prefix.to_string()
    };

    let mut matches = Vec::new();
    collect_globstar_matches(
        Path::new(&base_dir),
        suffix,
        &mut matches,
        nocaseglob,
        dotglob,
    );

    if matches.is_empty() {
        if nullglob {
            return Some(Vec::new());
        }
        return None;
    }
    matches.sort();
    Some(matches)
}

fn collect_globstar_matches(
    dir: &Path,
    suffix: &str,
    matches: &mut Vec<String>,
    nocaseglob: bool,
    dotglob: bool,
) {
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in entries.filter_map(Result::ok) {
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name.starts_with('.') && !dotglob {
            continue;
        }
        let path = dir.join(&name);
        if path.is_dir() {
            let matched = if nocaseglob {
                case_pattern_matches_nocase(suffix, &name)
            } else {
                super::case_pattern_matches(suffix, &name)
            };
            if matched {
                matches.push(path.to_string_lossy().to_string());
            }
            collect_globstar_matches(&path, suffix, matches, nocaseglob, dotglob);
        } else {
            let matched = if nocaseglob {
                case_pattern_matches_nocase(suffix, &name)
            } else {
                super::case_pattern_matches(suffix, &name)
            };
            if matched {
                matches.push(path.to_string_lossy().to_string());
            }
        }
    }
}
