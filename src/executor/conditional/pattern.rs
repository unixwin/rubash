pub(in crate::executor) fn case_pattern_matches(pattern: &str, word: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let word: Vec<char> = word.chars().collect();
    case_pattern_matches_at(&pattern, 0, &word, 0)
}

pub(in crate::executor) fn case_pattern_matches_at(
    pattern: &[char],
    p_index: usize,
    word: &[char],
    w_index: usize,
) -> bool {
    if p_index == pattern.len() {
        return w_index == word.len();
    }

    match pattern[p_index] {
        '\x18' => {
            w_index < word.len()
                && word[w_index] == '\\'
                && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
        '*' => {
            case_pattern_matches_at(pattern, p_index + 1, word, w_index)
                || (w_index < word.len()
                    && case_pattern_matches_at(pattern, p_index, word, w_index + 1))
        }
        '?' => {
            w_index < word.len() && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
        '[' => {
            let Some((matches_class, next_index)) =
                case_bracket_expression_matches(pattern, p_index, word.get(w_index).copied())
            else {
                return w_index < word.len()
                    && pattern[p_index] == word[w_index]
                    && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1);
            };

            matches_class && case_pattern_matches_at(pattern, next_index, word, w_index + 1)
        }
        '\\' if p_index + 1 < pattern.len() => {
            w_index < word.len()
                && pattern[p_index + 1] == word[w_index]
                && case_pattern_matches_at(pattern, p_index + 2, word, w_index + 1)
        }
        literal => {
            w_index < word.len()
                && literal == word[w_index]
                && case_pattern_matches_at(pattern, p_index + 1, word, w_index + 1)
        }
    }
}

pub(in crate::executor) fn extglob_match_literal(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
) -> bool {
    if p == pattern.len() {
        return w == word.len();
    }
    match pattern[p] {
        '*' => {
            extglob_matches_at(pattern, p + 1, word, w)
                || (w < word.len() && extglob_matches_at(pattern, p, word, w + 1))
        }
        '?' => w < word.len() && extglob_matches_at(pattern, p + 1, word, w + 1),
        '[' => {
            if let Some((matched, next)) =
                case_bracket_expression_matches(pattern, p, word.get(w).copied())
            {
                matched && extglob_matches_at(pattern, next, word, w + 1)
            } else {
                w < word.len()
                    && pattern[p] == word[w]
                    && extglob_matches_at(pattern, p + 1, word, w + 1)
            }
        }
        '\\' if p + 1 < pattern.len() => {
            w < word.len()
                && pattern[p + 1] == word[w]
                && extglob_matches_at(pattern, p + 2, word, w + 1)
        }
        c => w < word.len() && c == word[w] && extglob_matches_at(pattern, p + 1, word, w + 1),
    }
}

pub(in crate::executor) fn case_bracket_expression_matches(
    pattern: &[char],
    start: usize,
    candidate: Option<char>,
) -> Option<(bool, usize)> {
    let mut index = start + 1;
    if index >= pattern.len() {
        return None;
    }

    let negated = matches!(pattern[index], '!' | '^');
    if negated {
        index += 1;
    }

    let mut matched = false;
    let mut saw_member = false;
    let candidate = candidate?;
    while index < pattern.len() {
        if pattern[index] == ']' && saw_member {
            return Some((if negated { !matched } else { matched }, index + 1));
        }

        let current = pattern[index];
        if let Some((class_matched, next_index)) =
            bracket_posix_class_matches(pattern, index, candidate)
        {
            if class_matched {
                matched = true;
            }
            saw_member = true;
            index = next_index;
        } else if index + 2 < pattern.len()
            && pattern[index + 1] == '-'
            && pattern[index + 2] != ']'
        {
            let end = pattern[index + 2];
            if current <= candidate && candidate <= end {
                matched = true;
            }
            saw_member = true;
            index += 3;
        } else {
            if current == candidate {
                matched = true;
            }
            saw_member = true;
            index += 1;
        }
    }

    None
}

fn bracket_posix_class_matches(
    pattern: &[char],
    start: usize,
    candidate: char,
) -> Option<(bool, usize)> {
    if pattern.get(start) != Some(&'[') || pattern.get(start + 1) != Some(&':') {
        return None;
    }

    let mut end = start + 2;
    while end + 1 < pattern.len() {
        if pattern[end] == ':' && pattern[end + 1] == ']' {
            let class: String = pattern[start + 2..end].iter().collect();
            return Some((posix_class_matches(&class, candidate), end + 2));
        }
        end += 1;
    }
    None
}

fn posix_class_matches(class: &str, candidate: char) -> bool {
    match class {
        "alnum" => candidate.is_ascii_alphanumeric(),
        "alpha" => candidate.is_ascii_alphabetic(),
        "ascii" => candidate.is_ascii(),
        "blank" => matches!(candidate, ' ' | '\t'),
        "cntrl" => candidate.is_ascii_control(),
        "digit" => candidate.is_ascii_digit(),
        "graph" => candidate.is_ascii_graphic(),
        "lower" => candidate.is_ascii_lowercase(),
        "print" => candidate.is_ascii_graphic() || candidate == ' ',
        "punct" => candidate.is_ascii_punctuation(),
        "space" => candidate.is_ascii_whitespace(),
        "upper" => candidate.is_ascii_uppercase(),
        "word" => candidate.is_ascii_alphanumeric() || candidate == '_',
        "xdigit" => candidate.is_ascii_hexdigit(),
        _ => false,
    }
}
use super::extglob::extglob_matches_at;
