use super::pattern::{extglob_match_literal, extglob_match_literal_nocase};

pub(in crate::executor) fn extglob_case_pattern_matches(pattern: &str, word: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let word: Vec<char> = word.chars().collect();
    extglob_matches_at(&pattern, 0, &word, 0)
}

pub(in crate::executor) fn extglob_case_pattern_matches_nocase(pattern: &str, word: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let word: Vec<char> = word.chars().collect();
    extglob_matches_at_with_case(&pattern, 0, &word, 0, true)
}

pub(in crate::executor) fn extglob_group_end(pattern: &[char], open_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open_idx;
    while i < pattern.len() {
        match pattern[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            '[' => {
                i += 1;
                while i < pattern.len() && pattern[i] != ']' {
                    i += 1;
                }
            }
            '\\' if i + 1 < pattern.len() => {
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Extglob-aware recursive pattern matcher.
pub(in crate::executor) fn extglob_matches_at(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
) -> bool {
    extglob_matches_at_with_case(pattern, p, word, w, false)
}

pub(in crate::executor) fn extglob_matches_at_with_case(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
    nocase: bool,
) -> bool {
    if p == pattern.len() {
        return w == word.len();
    }

    // Detect extglob: op followed by '('
    if p + 1 < pattern.len()
        && pattern[p + 1] == '('
        && matches!(pattern[p], '*' | '?' | '+' | '@' | '!')
    {
        let op = pattern[p];
        if let Some(end) = extglob_group_end(pattern, p + 1) {
            let inner = &pattern[p + 2..end];
            let rest = end + 1;
            return match op {
                '@' => {
                    // Exactly one occurrence
                    extglob_try_inner_lengths_with_case(inner, word, w, 1, nocase, |len| {
                        extglob_matches_at_with_case(pattern, rest, word, w + len, nocase)
                    })
                }
                '?' => {
                    // Zero or one
                    extglob_matches_at_with_case(pattern, rest, word, w, nocase)
                        || extglob_try_inner_lengths_with_case(inner, word, w, 1, nocase, |len| {
                            extglob_matches_at_with_case(pattern, rest, word, w + len, nocase)
                        })
                }
                '*' => {
                    // Zero or more
                    extglob_match_star_at_with_case(inner, pattern, rest, word, w, nocase)
                }
                '+' => {
                    // One or more
                    extglob_try_inner_lengths_with_case(inner, word, w, 1, nocase, |len| {
                        extglob_match_star_at_with_case(inner, pattern, rest, word, w + len, nocase)
                    })
                }
                '!' => {
                    // Not matching: try every possible split where inner does NOT match
                    extglob_match_negation_at_with_case(inner, pattern, rest, word, w, nocase)
                }
                _ => false,
            };
        }
    }

    // Standard glob matching
    if nocase {
        extglob_match_literal_nocase(pattern, p, word, w)
    } else {
        extglob_match_literal(pattern, p, word, w)
    }
}

/// Try matching inner pattern against word[w..w+len] for all valid lengths >= min_len,
/// calling `found` with the matched length. Returns true if any succeeds.
/// The inner pattern may contain `|` for alternation (e.g., `a|b`).
pub(in crate::executor) fn extglob_try_inner_lengths_with_case<F: Fn(usize) -> bool>(
    inner: &[char],
    word: &[char],
    w: usize,
    min_len: usize,
    nocase: bool,
    found: F,
) -> bool {
    // Split inner on '|' for alternation
    let alternatives = extglob_split_alternatives(inner);
    let remaining = word.len() - w;
    for alt in &alternatives {
        for len in min_len..=remaining {
            let slice = &word[w..w + len];
            if extglob_matches_at_with_case(alt, 0, slice, 0, nocase) && found(len) {
                return true;
            }
        }
    }
    false
}

/// Split an extglob inner pattern on '|' to get alternatives.
pub(in crate::executor) fn extglob_split_alternatives(inner: &[char]) -> Vec<Vec<char>> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0;
    for &ch in inner {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            '|' if depth == 0 => {
                result.push(std::mem::take(&mut current));
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    if result.is_empty() {
        result.push(inner.to_vec());
    }
    result
}

/// Match zero or more occurrences of inner, then rest of pattern.
pub(in crate::executor) fn extglob_match_star_at_with_case(
    inner: &[char],
    pattern: &[char],
    rest: usize,
    word: &[char],
    w: usize,
    nocase: bool,
) -> bool {
    // Try zero occurrences first
    if extglob_matches_at_with_case(pattern, rest, word, w, nocase) {
        return true;
    }
    // Split inner on '|' for alternation
    let alternatives = extglob_split_alternatives(inner);
    // Try consuming one or more
    let remaining = word.len() - w;
    for alt in &alternatives {
        for len in 1..=remaining {
            if extglob_matches_at_with_case(alt, 0, &word[w..w + len], 0, nocase) {
                if extglob_match_star_at_with_case(inner, pattern, rest, word, w + len, nocase) {
                    return true;
                }
            }
        }
    }
    false
}

/// Match negation: word at w must NOT match inner, then rest must match.
pub(in crate::executor) fn extglob_match_negation_at_with_case(
    inner: &[char],
    pattern: &[char],
    rest: usize,
    word: &[char],
    w: usize,
    nocase: bool,
) -> bool {
    // For negation, try every possible remainder; if inner doesn't match that prefix, check rest
    let alternatives = extglob_split_alternatives(inner);
    for split in w..=word.len() {
        let slice = &word[w..split];
        let any_alt_matches = alternatives
            .iter()
            .any(|alt| extglob_matches_at_with_case(alt, 0, slice, 0, nocase));
        if !any_alt_matches {
            // This part doesn't match any alternative - good for negation
            // Check if rest of pattern matches remaining word
            if extglob_matches_at_with_case(pattern, rest, word, split, nocase) {
                return true;
            }
        }
    }
    false
}
