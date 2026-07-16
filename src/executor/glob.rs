//! Pathname expansion (globbing) for command words.

use std::fs;
use std::path::Path;

pub(crate) enum PathnameExpansion {
    Matches(Vec<String>),
    NoMatch,
    Fail(String),
}

/// Check if a shopt option is enabled.
fn shopt_enabled(env_vars: &std::collections::HashMap<String, String>, name: &str) -> bool {
    crate::builtins::shopt::option_enabled(env_vars, name)
}

/// Check if a word contains glob or extglob pattern characters.
fn contains_glob_or_extglob(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if matches!(ch, '*' | '?' | '[') {
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
) -> PathnameExpansion {
    if word.is_empty() {
        return PathnameExpansion::NoMatch;
    }
    if word.starts_with('"') || word.starts_with('\'') {
        return PathnameExpansion::NoMatch;
    }
    if !contains_glob_or_extglob(word) {
        return PathnameExpansion::NoMatch;
    }
    if word.contains('=') || word.contains('{') || word.contains('}') {
        return PathnameExpansion::NoMatch;
    }
    if crate::builtins::set::shell_option_enabled(env_vars, "noglob") {
        return PathnameExpansion::NoMatch;
    }

    let nullglob = shopt_enabled(env_vars, "nullglob");
    let failglob = shopt_enabled(env_vars, "failglob");
    let dotglob = shopt_enabled(env_vars, "dotglob");
    let globskipdots = shopt_enabled(env_vars, "globskipdots");
    let nocaseglob = shopt_enabled(env_vars, "nocaseglob");
    let globstar = shopt_enabled(env_vars, "globstar");
    let extglob = shopt_enabled(env_vars, "extglob");

    if word.contains("**") && globstar {
        return globstar_expand(word, nullglob, failglob, nocaseglob, dotglob, globskipdots);
    }

    if word.contains('/') {
        return pathname_expand_segments(
            word,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            extglob,
        );
    }

    let (dir_path, pattern) = match word.rsplit_once('/') {
        Some((d, p)) => (d.to_string(), p),
        None => (".".to_string(), word.as_ref()),
    };
    let include_dotfiles = dotglob || pattern.starts_with('.');
    let entries = match fs::read_dir(&dir_path) {
        Ok(rd) => rd,
        Err(_) => return unmatched_expansion(word, nullglob, failglob),
    };
    let mut names = synthetic_dot_names(pattern, globskipdots);
    names.extend(
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok()),
    );
    let mut matches: Vec<String> = names
        .into_iter()
        .filter_map(|name| {
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
        return unmatched_expansion(word, nullglob, failglob);
    }
    matches.sort();
    PathnameExpansion::Matches(matches)
}

fn pathname_expand_segments(
    word: &str,
    nullglob: bool,
    failglob: bool,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
    extglob: bool,
) -> PathnameExpansion {
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
                let mut names = synthetic_dot_names(part, globskipdots);
                names.extend(
                    entries
                        .filter_map(Result::ok)
                        .filter_map(|entry| entry.file_name().into_string().ok()),
                );
                for name in names {
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
        return unmatched_expansion(word, nullglob, failglob);
    }
    prefixes.sort();
    PathnameExpansion::Matches(prefixes)
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
    failglob: bool,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
) -> PathnameExpansion {
    let parts: Vec<&str> = word.split("**").collect();
    if parts.len() != 2 {
        return PathnameExpansion::NoMatch;
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
        globskipdots,
    );

    if matches.is_empty() {
        return unmatched_expansion(word, nullglob, failglob);
    }
    matches.sort();
    PathnameExpansion::Matches(matches)
}

fn unmatched_expansion(word: &str, nullglob: bool, failglob: bool) -> PathnameExpansion {
    if failglob {
        PathnameExpansion::Fail(word.to_string())
    } else if nullglob {
        PathnameExpansion::Matches(Vec::new())
    } else {
        PathnameExpansion::NoMatch
    }
}

fn collect_globstar_matches(
    dir: &Path,
    suffix: &str,
    matches: &mut Vec<String>,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
) {
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    let mut names = synthetic_dot_names(suffix, globskipdots);
    names.extend(
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok()),
    );
    let include_dotfiles = dotglob || suffix.starts_with('.');
    for name in names {
        if name.starts_with('.') && !include_dotfiles {
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
            if name != "." && name != ".." {
                collect_globstar_matches(&path, suffix, matches, nocaseglob, dotglob, globskipdots);
            }
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

fn synthetic_dot_names(pattern: &str, globskipdots: bool) -> Vec<String> {
    if globskipdots || !pattern.starts_with('.') {
        Vec::new()
    } else {
        vec![".".to_string(), "..".to_string()]
    }
}
