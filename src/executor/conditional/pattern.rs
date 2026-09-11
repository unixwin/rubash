pub(in crate::executor) fn case_pattern_matches(pattern: &str, word: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let word: Vec<char> = word.chars().collect();
    case_pattern_matches_at_with_case(&pattern, 0, &word, 0, false)
}

pub(in crate::executor) fn case_pattern_matches_nocase(pattern: &str, word: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let word: Vec<char> = word.chars().collect();
    case_pattern_matches_at_with_case(&pattern, 0, &word, 0, true)
}

pub(in crate::executor) fn case_pattern_matches_at_with_case(
    pattern: &[char],
    p_index: usize,
    word: &[char],
    w_index: usize,
    nocase: bool,
) -> bool {
    // Keep the wildcard backtracking iterative. A recursive `*` matcher uses
    // one stack frame per input character; ordinary Windows PATH values are
    // long enough to exhaust the process stack during a valid `case` test.
    let mut pattern_index = p_index;
    let mut word_index = w_index;
    let mut star_pattern_index = None;
    let mut star_word_index = word_index;

    while word_index < word.len() {
        if pattern_index < pattern.len() {
            if pattern[pattern_index] == '*' {
                star_pattern_index = Some(pattern_index);
                star_word_index = word_index;
                pattern_index += 1;
                continue;
            }

            let (matched, next_pattern_index) =
                case_pattern_atom_matches(pattern, pattern_index, word[word_index], nocase);
            if matched {
                pattern_index = next_pattern_index;
                word_index += 1;
                continue;
            }
        }

        let Some(star_index) = star_pattern_index else {
            return false;
        };
        star_word_index += 1;
        word_index = star_word_index;
        pattern_index = star_index + 1;
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

fn case_pattern_atom_matches(
    pattern: &[char],
    pattern_index: usize,
    candidate: char,
    nocase: bool,
) -> (bool, usize) {
    match pattern[pattern_index] {
        '\x18' => (candidate == '\\', pattern_index + 1),
        '\x11' if pattern_index + 1 < pattern.len() => (
            chars_match(pattern[pattern_index + 1], candidate, nocase),
            pattern_index + 2,
        ),
        '?' => (true, pattern_index + 1),
        '[' => {
            if let Some((matched, next_index)) = case_bracket_expression_matches_with_case(
                pattern,
                pattern_index,
                Some(candidate),
                nocase,
            ) {
                (matched, next_index)
            } else {
                (chars_match('[', candidate, nocase), pattern_index + 1)
            }
        }
        '\\' if pattern_index + 1 < pattern.len() => (
            chars_match(pattern[pattern_index + 1], candidate, nocase),
            pattern_index + 2,
        ),
        literal => (chars_match(literal, candidate, nocase), pattern_index + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::{case_pattern_matches, case_pattern_matches_nocase};

    #[test]
    fn wildcard_matching_handles_long_words_without_recursion() {
        let word = "x".repeat(32 * 1024);

        assert!(case_pattern_matches("*x*", &word));
        assert!(!case_pattern_matches("*missing*", &word));
    }

    #[test]
    fn wildcard_matching_backtracks_after_a_partial_literal_match() {
        assert!(case_pattern_matches("*ab", "aab"));
        assert!(case_pattern_matches_nocase("*foo*", "PATH_FOO_VALUE"));
    }
}

pub(in crate::executor) fn extglob_match_literal(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
) -> bool {
    extglob_match_literal_with_case(pattern, p, word, w, false)
}

pub(in crate::executor) fn extglob_match_literal_nocase(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
) -> bool {
    extglob_match_literal_with_case(pattern, p, word, w, true)
}

fn extglob_match_literal_with_case(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
    nocase: bool,
) -> bool {
    if p == pattern.len() {
        return w == word.len();
    }
    match pattern[p] {
        '*' => {
            extglob_matches_at_with_case(pattern, p + 1, word, w, nocase)
                || (w < word.len() && extglob_matches_at_with_case(pattern, p, word, w + 1, nocase))
        }
        '?' => w < word.len() && extglob_matches_at_with_case(pattern, p + 1, word, w + 1, nocase),
        '[' => {
            if let Some((matched, next)) =
                case_bracket_expression_matches_with_case(pattern, p, word.get(w).copied(), nocase)
            {
                matched && extglob_matches_at_with_case(pattern, next, word, w + 1, nocase)
            } else {
                w < word.len()
                    && chars_match(pattern[p], word[w], nocase)
                    && extglob_matches_at_with_case(pattern, p + 1, word, w + 1, nocase)
            }
        }
        '\\' if p + 1 < pattern.len() => {
            w < word.len()
                && chars_match(pattern[p + 1], word[w], nocase)
                && extglob_matches_at_with_case(pattern, p + 2, word, w + 1, nocase)
        }
        c => {
            w < word.len()
                && chars_match(c, word[w], nocase)
                && extglob_matches_at_with_case(pattern, p + 1, word, w + 1, nocase)
        }
    }
}

pub(in crate::executor) fn case_bracket_expression_matches_with_case(
    pattern: &[char],
    start: usize,
    candidate: Option<char>,
    nocase: bool,
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
    let candidate_cmp = comparable_char(candidate, nocase);
    while index < pattern.len() {
        if pattern[index] == ']' && saw_member {
            return Some((if negated { !matched } else { matched }, index + 1));
        }

        // lib/glob/glob.c parse_bracket: a backslash-escaped `]` is a literal
        // bracket member, not the closing bracket. `[[:alpha:]\]` therefore
        // has an unterminated bracket (the `]` after `\` is a member), so the
        // pattern never closes and cannot match (posixpat.tests ok 21).
        // `\x18` is a legacy protected-literal-backslash marker that may still
        // reach this matcher; treat it exactly like a real backslash here.
        if matches!(pattern[index], '\\' | '\x18' | '\x11') && index + 1 < pattern.len() {
            let lit = pattern[index + 1];
            if chars_match(lit, candidate, nocase) {
                matched = true;
            }
            saw_member = true;
            // sm_loop.c BRACKET:527-534+568: the escaped member may anchor a
            // range (`[\a-z]` is the range a-z); the range end may itself be
            // escaped (`[\a-\z]`).
            if index + 3 < pattern.len() && pattern[index + 2] == '-' && pattern[index + 3] != ']' {
                let mut end_index = index + 3;
                if matches!(pattern[end_index], '\\' | '\x18' | '\x11')
                    && end_index + 1 < pattern.len()
                {
                    end_index += 1;
                }
                let end = pattern[end_index];
                let start_cmp = comparable_char(lit, nocase);
                let end_cmp = comparable_char(end, nocase);
                if start_cmp <= candidate_cmp && candidate_cmp <= end_cmp {
                    matched = true;
                }
                index = end_index + 1;
                continue;
            }
            index += 2;
            continue;
        }

        // POSIX bracket prefixes: [.sym.] collating symbols and [=c=]
        // equivalence classes (lib/glob/sm_loop.c). A valid symbol collapses
        // to one member character and may anchor a range; an unknown symbol
        // degenerates to its literal characters (posixpat.tests collating
        // section).
        if pattern[index] == '[' {
            if let Some((members, next_index)) = parse_collating_or_equivalence(pattern, index) {
                for member in &members {
                    if chars_match(*member, candidate, nocase) {
                        matched = true;
                    }
                }
                saw_member = true;
                if members.len() == 1 {
                    if let Some((end_char, after)) = collating_range_end(pattern, next_index) {
                        let start_cmp = comparable_char(members[0], nocase);
                        let end_cmp = comparable_char(end_char, nocase);
                        if start_cmp <= candidate_cmp && candidate_cmp <= end_cmp {
                            matched = true;
                        }
                        saw_member = true;
                        index = after;
                        continue;
                    }
                }
                index = next_index;
                continue;
            }
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
            // sm_loop.c BRACKET:568-572: the range end may be an escaped
            // character (`[a-\z]` is the range a-z).
            let mut end_index = index + 2;
            if matches!(pattern[end_index], '\\' | '\x18' | '\x11') && end_index + 1 < pattern.len()
            {
                end_index += 1;
            }
            let end = pattern[end_index];
            let current_cmp = comparable_char(current, nocase);
            let end_cmp = comparable_char(end, nocase);
            if current_cmp <= candidate_cmp && candidate_cmp <= end_cmp {
                matched = true;
            }
            saw_member = true;
            index = end_index + 1;
        } else {
            if chars_match(current, candidate, nocase) {
                matched = true;
            }
            saw_member = true;
            index += 1;
        }
    }

    None
}

/// Parses a `[.sym.]` collating symbol or `[=c=]` equivalence class at
/// index (which must point at the opening `[`). Returns the member
/// characters and the index just past the closing `.]` / `=]`. A valid
/// single-character symbol yields that character; a recognized multi-
/// character name resolves through the POSIX portable character set table;
/// an unrecognized symbol degenerates to its literal characters, matching
/// bash's sm_loop.c fallback for undefined collating symbols.
fn parse_collating_or_equivalence(pattern: &[char], index: usize) -> Option<(Vec<char>, usize)> {
    let open = *pattern.get(index + 1)?;
    if open != '.' && open != '=' {
        return None;
    }
    let mut scan = index + 2;
    while scan + 1 < pattern.len() {
        if pattern[scan] == open && pattern[scan + 1] == ']' {
            let body: Vec<char> = pattern[index + 2..scan].to_vec();
            if body.is_empty() {
                return None;
            }
            let members = if body.len() == 1 {
                body
            } else {
                let name: String = body.iter().collect();
                match named_collating_symbol(&name) {
                    Some(c) => vec![c],
                    None => body,
                }
            };
            return Some((members, scan + 2));
        }
        scan += 1;
    }
    None
}

/// Resolves a range whose start is a single-character collating symbol:
/// dash_index points at the `-`; the endpoint may be another `[.sym.]` /
/// `[=c=]` span, an escaped character, or a plain character. Returns the
/// endpoint character and the index just past it.
fn collating_range_end(pattern: &[char], dash_index: usize) -> Option<(char, usize)> {
    if pattern.get(dash_index) != Some(&'-') {
        return None;
    }
    let end_index = dash_index + 1;
    let end = *pattern.get(end_index)?;
    if end == ']' {
        return None;
    }
    if end == '[' {
        let (members, next) = parse_collating_or_equivalence(pattern, end_index)?;
        if members.len() == 1 {
            return Some((members[0], next));
        }
        return None;
    }
    if matches!(end, '\\' | '\x18') {
        let after = *pattern.get(end_index + 1)?;
        return Some((after, end_index + 2));
    }
    Some((end, end_index + 1))
}

/// POSIX portable character set collating-symbol names (the set glibc's C
/// locale recognizes), mapped to their characters.
fn named_collating_symbol(name: &str) -> Option<char> {
    Some(match name {
        "NUL" => '\0',
        "space" => ' ',
        "tab" => '\t',
        "newline" => '\n',
        "vertical-tab" => '\u{000b}',
        "form-feed" => '\u{000c}',
        "carriage-return" => '\r',
        "exclamation-mark" => '!',
        "quotation-mark" => '"',
        "number-sign" => '#',
        "dollar-sign" => '$',
        "percent-sign" => '%',
        "ampersand" => '&',
        "apostrophe" => '\'',
        "left-parenthesis" => '(',
        "right-parenthesis" => ')',
        "asterisk" => '*',
        "plus-sign" => '+',
        "comma" => ',',
        "hyphen" => '-',
        "full-stop" => '.',
        "slash" => '/',
        "colon" => ':',
        "semicolon" => ';',
        "less-than-sign" => '<',
        "equals-sign" => '=',
        "greater-than-sign" => '>',
        "question-mark" => '?',
        "commercial-at" => '@',
        "left-square-bracket" => '[',
        "backslash" => '\\',
        "right-square-bracket" => ']',
        "circumflex-accent" => '^',
        "low-line" => '_',
        "grave-accent" => '`',
        "left-curly-bracket" => '{',
        "vertical-line" => '|',
        "right-curly-bracket" => '}',
        "tilde" => '~',
        _ => return None,
    })
}

fn chars_match(pattern: char, candidate: char, nocase: bool) -> bool {
    if nocase {
        pattern.eq_ignore_ascii_case(&candidate)
    } else {
        pattern == candidate
    }
}

fn comparable_char(ch: char, nocase: bool) -> char {
    if nocase {
        ch.to_ascii_lowercase()
    } else {
        ch
    }
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
use super::extglob::extglob_matches_at_with_case;
