use super::*;

pub(in crate::executor) fn replace_parameter_pattern(
    value: &str,
    pattern: &str,
    replacement: &str,
    global: bool,
    nocase: bool,
    patsub_replacement: bool,
) -> String {
    // GNU subst.c: parameter_brace_patsub:9440-9444 sets MATCH_EXPREP only
    // when the patsub_replacement shopt is on AND shouldexp_replacement
    // (subst.c:9150-9181) finds an unquoted `&`, `\\&`, or `\\\\` in the
    // expanded replacement. pat_subst then runs strcreplace per match.
    let expand_amp = patsub_replacement && shouldexp_replacement(replacement);
    let amp = |matched: &str| -> String {
        if expand_amp {
            expand_replacement_amp(matched, replacement)
        } else {
            replacement.to_string()
        }
    };

    // GNU parameter_brace_patsub (subst.c:9451-9465): with `//` the match
    // type is MATCH_ANY and a leading `#`/`%` is literal; only the
    // single-slash form anchors at the beginning (`#`) or end (`%`).
    if !global {
        if let Some(prefix_pattern) = pattern.strip_prefix('#') {
            return replace_parameter_prefix(value, prefix_pattern, &amp);
        }

        if let Some(suffix_pattern) = pattern.strip_prefix('%') {
            return replace_parameter_suffix(value, suffix_pattern, &amp);
        }
    }

    if pattern.is_empty() {
        // GNU pat_subst:9197-9229: a null pattern anchored at the beginning
        // prefixes REP, at the end appends REP, with `&` expanding to the
        // empty match. Unanchored, MATCH_ANY matches the empty string
        // everywhere; a global substitution inserts REP at every position
        // and copies one character after each zero-length match
        // (pat_subst:9286-9304).
        if !global {
            return format!("{}{value}", amp(""));
        }
        let mut output = String::new();
        for (_, ch) in value.char_indices() {
            output.push_str(&amp(""));
            output.push(ch);
        }
        output.push_str(&amp(""));
        return output;
    }

    // Bash's glob `*` also matches an empty parameter value. Handle this
    // before the replacement loop so an empty match cannot loop forever.
    // GNU pat_subst:9230-9232: an empty string with a matching pattern
    // yields the replacement with `&` expanding to the empty match.
    if value.is_empty() {
        if parameter_pattern_match(pattern, "", nocase) {
            return amp("");
        }
        return String::new();
    }

    if global && replacement.is_empty() {
        if let Some(class) = parse_negated_bracket_filter(pattern) {
            return value
                .chars()
                .filter(|ch| bracket_filter_matches(&class, *ch, nocase))
                .collect();
        }
    }

    if !pattern_contains_glob(pattern) {
        // `\x14` is the internal marker for a literal backslash decoded by
        // decode_parameter_pattern_quotes; plain string replacement must
        // match the real `\` character in the value.
        let literal = normalize_parameter_pattern_backslashes(pattern);
        return replace_with_amp(value, &literal, replacement, global, nocase, expand_amp);
    }

    // A quoted backslash in a replacement pattern is a literal character,
    // not a glob escape. The lexer may leave it as the internal quote marker;
    // normalize it before matching the value.
    let pattern = normalize_parameter_pattern_backslashes(pattern);
    let pattern = pattern.as_str();
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let mut output = String::new();
    let mut cursor = 0;

    while cursor <= value.len() {
        let Some((start, end)) =
            find_parameter_pattern_match(value, pattern, cursor, &indices, nocase)
        else {
            output.push_str(&value[cursor..]);
            return output;
        };

        output.push_str(&value[cursor..start]);
        output.push_str(&amp(&value[start..end]));
        if end == start {
            // GNU pat_subst:9286-9304: on a zero-length match copy one
            // character so the scan advances past it.
            match value[end..].chars().next() {
                Some(ch) => {
                    output.push(ch);
                    cursor = end + ch.len_utf8();
                }
                None => return output,
            }
        } else {
            cursor = end;
        }

        if !global {
            output.push_str(&value[cursor..]);
            return output;
        }
    }

    output
}

/// Case sensitivity wrapper for pattern-substitution matching. GNU applies
/// FNMATCH_IGNCASE in match_upattern (subst.c:5382) whenever nocasematch is
/// set, making ${var/pat/rep} case-insensitive (bash 4.3+; new-exp8.sub).
fn parameter_pattern_match(pattern: &str, word: &str, nocase: bool) -> bool {
    if nocase {
        case_pattern_matches_nocase(pattern, word)
    } else {
        case_pattern_matches(pattern, word)
    }
}

fn normalize_parameter_pattern_backslashes(pattern: &str) -> String {
    pattern
        .replace("\x18\x18", "\\")
        .replace('\x14', "\\")
        .replace('\x18', "\\")
}

pub(in crate::executor) fn replace_parameter_prefix(
    value: &str,
    pattern: &str,
    amp: &dyn Fn(&str) -> String,
) -> String {
    let Some(end) = find_parameter_prefix_match(value, pattern) else {
        return value.to_string();
    };
    format!("{}{}", amp(&value[..end]), &value[end..])
}

pub(in crate::executor) fn replace_parameter_suffix(
    value: &str,
    pattern: &str,
    amp: &dyn Fn(&str) -> String,
) -> String {
    let Some(start) = find_parameter_suffix_match(value, pattern) else {
        return value.to_string();
    };
    format!("{}{}", &value[..start], amp(&value[start..]))
}

pub(super) fn pattern_contains_glob(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | '\\'))
}

pub(in crate::executor) fn find_parameter_prefix_match(
    value: &str,
    pattern: &str,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(0);
    }

    value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .rev()
        .find(|end| case_pattern_matches(pattern, &value[..*end]))
}

pub(in crate::executor) fn find_parameter_suffix_match(
    value: &str,
    pattern: &str,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(value.len());
    }

    value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .find(|start| case_pattern_matches(pattern, &value[*start..]))
}

pub(in crate::executor) fn find_parameter_pattern_match(
    value: &str,
    pattern: &str,
    cursor: usize,
    indices: &[usize],
    nocase: bool,
) -> Option<(usize, usize)> {
    let start_index = indices.iter().position(|index| *index >= cursor)?;

    // GNU strmatch scans per position with the pattern itself, so a
    // bracket-class pattern over a long value stays linear. The descending
    // longest-first end scan here tries O(n) *failing* candidate ends per
    // start position, which is O(n^3) on new-exp8.sub's ~13k-char value
    // (deletion with pat3=[[:alnum:]_] costs ~1e9 char comparisons in the
    // debug build -> multi-minute hang where GNU finishes instantly).
    // Restrict the candidate ends with two pattern-shape facts, both of
    // which preserve exact leftmost-longest match semantics:
    //   1. A pattern without a star can match at most BOUND characters
    //      (one per ? / bracket / escaped / literal atom), so ends beyond
    //      start + BOUND characters are pruned (indices[k] is the byte
    //      offset of the k-th char, so char distance is an index delta).
    //   2. With a star, the match must still END with the literal text
    //      after the last star (when that tail is wildcard-free), so only
    //      byte offsets directly following an occurrence of the tail can
    //      be match ends. A tail that never occurs in the value means no
    //      match can exist anywhere.
    let bound = pattern_match_length_bound(pattern);
    let tail_ends = pattern_literal_tail_ends(pattern, value);

    for (start_pos, start) in indices[start_index..].iter().enumerate() {
        let ends = &indices[start_index + start_pos + 1..];
        let mut best: Option<usize> = None;
        for (end_rel, end) in ends.iter().enumerate() {
            if let Some(bound) = bound {
                if end_rel + 1 > bound {
                    break;
                }
            }
            if let Some(allowed) = &tail_ends {
                if !allowed.contains(end) {
                    continue;
                }
            }
            if parameter_pattern_match(pattern, &value[*start..*end], nocase) {
                best = Some(*end);
            }
        }
        if let Some(end) = best {
            return Some((*start, end));
        }
    }

    None
}

/// Maximum number of characters a pattern without a star can match, or
/// `None` when the pattern contains a star (unbounded).
fn pattern_match_length_bound(pattern: &str) -> Option<usize> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut bound = 0usize;
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' => return None,
            '?' => {
                bound += 1;
                index += 1;
            }
            '[' => {
                let mut close = index + 1;
                if close < chars.len() && (chars[close] == '!' || chars[close] == '^') {
                    close += 1;
                }
                if close < chars.len() && chars[close] == ']' {
                    close += 1;
                }
                while close < chars.len() && chars[close] != ']' {
                    close += 1;
                }
                bound += 1;
                index = if close < chars.len() { close + 1 } else { index + 1 };
            }
            '\\' => {
                bound += 1;
                index += 2;
            }
            _ => {
                bound += 1;
                index += 1;
            }
        }
    }
    Some(bound)
}

/// Byte offsets `end` where the value up to `end` ends with the pattern's
/// literal tail (the wildcard-free text after the last star), or `None`
/// when the pattern has no usable literal tail. An empty set means the
/// tail never occurs in the value, so no match exists at all.
fn pattern_literal_tail_ends(
    pattern: &str,
    value: &str,
) -> Option<std::collections::HashSet<usize>> {
    let tail = pattern.rsplit_once('*')?.1;
    if tail.is_empty()
        || tail
            .chars()
            .any(|ch| matches!(ch, '?' | '*' | '[' | ']' | '\\'))
    {
        return None;
    }
    let tail = normalize_parameter_pattern_backslashes(tail);
    if tail.is_empty() {
        return None;
    }
    let mut ends = std::collections::HashSet::new();
    let mut search = 0usize;
    while let Some(found) = value[search..].find(&tail) {
        let end = search + found + tail.len();
        ends.insert(end);
        search += found + 1;
    }
    Some(ends)
}

#[derive(Clone, Copy)]
enum BracketFilterItem {
    Char(char),
    Range(char, char),
}

fn parse_negated_bracket_filter(pattern: &str) -> Option<Vec<BracketFilterItem>> {
    let inner = pattern
        .strip_prefix("[^")
        .or_else(|| pattern.strip_prefix("[!"))?
        .strip_suffix(']')?;
    if inner.is_empty() {
        return None;
    }

    let chars = inner.chars().collect::<Vec<_>>();
    let mut items = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if index + 2 < chars.len() && chars[index + 1] == '-' {
            items.push(BracketFilterItem::Range(chars[index], chars[index + 2]));
            index += 3;
        } else {
            items.push(BracketFilterItem::Char(chars[index]));
            index += 1;
        }
    }
    Some(items)
}

fn bracket_filter_matches(items: &[BracketFilterItem], ch: char, nocase: bool) -> bool {
    items.iter().any(|item| match *item {
        BracketFilterItem::Char(value) => {
            if nocase {
                value.to_lowercase().eq(ch.to_lowercase())
            } else {
                value == ch
            }
        }
        BracketFilterItem::Range(start, end) => start <= ch && ch <= end,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::executor) enum ParameterTransform {
    Quote,
    Escape,
    Assignment,
    Attributes,
    KeyValueQuoted,
    KeyValueSplit,
    Prompt,
    Upper,
    UpperFirst,
    Lower,
}

pub(in crate::executor) fn parse_parameter_transform(
    name: &str,
) -> Option<(&str, ParameterTransform)> {
    let (var_name, operation) = name.rsplit_once('@')?;
    let transform = match operation {
        "Q" => ParameterTransform::Quote,
        "E" => ParameterTransform::Escape,
        "A" => ParameterTransform::Assignment,
        "a" => ParameterTransform::Attributes,
        "K" => ParameterTransform::KeyValueQuoted,
        "k" => ParameterTransform::KeyValueSplit,
        "P" => ParameterTransform::Prompt,
        "U" => ParameterTransform::Upper,
        "u" => ParameterTransform::UpperFirst,
        "L" => ParameterTransform::Lower,
        _ => return None,
    };
    Some((var_name, transform))
}

pub(in crate::executor) fn apply_parameter_transform(
    value: &str,
    transform: ParameterTransform,
) -> String {
    match transform {
        // GNU string_transform (subst.c:8740) routes Q/K/k through
        // sh_quote_reusable and 'A' through string_var_assignment, whose
        // value part is also sh_quote_reusable (subst.c:8660).
        ParameterTransform::Quote => shell_reusable_quote(value),
        ParameterTransform::Escape => decode_ansi_c_escapes(value),
        ParameterTransform::Assignment => shell_reusable_quote(value),
        ParameterTransform::Attributes => String::new(),
        ParameterTransform::KeyValueQuoted => shell_reusable_quote(value),
        ParameterTransform::KeyValueSplit => shell_reusable_quote(value),
        ParameterTransform::Prompt => value.to_string(),
        ParameterTransform::Upper => value.chars().flat_map(char::to_uppercase).collect(),
        ParameterTransform::UpperFirst => uppercase_first_char(value),
        ParameterTransform::Lower => value.chars().flat_map(char::to_lowercase).collect(),
    }
}

fn uppercase_first_char(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_uppercase().chain(chars).collect::<String>()
}

pub(in crate::executor) fn format_key_value_transform_part(
    key: &str,
    value: &str,
    quoted: bool,
) -> String {
    if quoted {
        format!("{} {}", quote_key_value_transform_key(key), quote_array_value(value))
    } else {
        format!("{key} {value}")
    }
}

/// GNU assoc_to_kvpair (assoc.c:346) key quoting: $'...' for characters
/// that need ANSI-C quoting, double quotes when the key contains shell
/// metacharacters (sh_contains_shell_metas, lib/sh/stringlib.c) or is a
/// lone @ / * (ALL_ELEMENT_SUB), bare otherwise. Values are always
/// double-quoted by the caller.
fn quote_key_value_transform_key(key: &str) -> String {
    if key.is_empty() {
        return key.to_string();
    }
    if key.chars().any(|ch| (ch as u32) < 0x20 || (ch as u32) == 0x7f) {
        let body = key
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        return format!("$'{body}'");
    }
    if key == "@" || key == "*" || key_value_word_contains_shell_metas(key) {
        return quote_array_value(key);
    }
    key.to_string()
}

/// sh_contains_shell_metas (lib/sh/stringlib.c): IFS whitespace, quoting
/// characters, shell metacharacters, reserved words, globbing characters,
/// expansion characters, ^, and tilde/hash in leading positions.
fn key_value_word_contains_shell_metas(word: &str) -> bool {
    for (index, ch) in word.char_indices() {
        match ch {
            ' ' | '\t' | '\n' | '\'' | '"' | '\\' | '|' | '&' | ';' | '(' | ')' | '<' | '>'
            | '!' | '{' | '}' | '*' | '[' | '?' | ']' | '^' | '$' | '`' => return true,
            '~' => {
                if index == 0 || word[..index].ends_with('=') || word[..index].ends_with(':') {
                    return true;
                }
            }
            '#' => {
                if index == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

pub(in crate::executor) fn shell_single_quote_assignment_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// GNU subst.c shouldexp_replacement (subst.c:9150-9181): the replacement
/// needs the strcreplace pass when it contains an unquoted `&`, a `\&`,
/// or a `\\` (an escaped backslash).
fn shouldexp_replacement(replacement: &str) -> bool {
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.peek() {
                Some('&') | Some('\\') => return true,
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '&' => return true,
            _ => {}
        }
    }
    false
}

/// Expands `&` in a patsub replacement string to the matched text, like GNU
/// strcreplace (stringlib.c:180) called from pat_subst (subst.c:9252) with
/// flags 2: each `&` copies the match, `\&` is a literal `&`, and with
/// flag 2 `\\` is a literal backslash. Any other backslash stays literal
/// together with the character it precedes.
/// GNU sh_quote_reusable (lib/sh/shquote.c:353): render a value so it can be
/// reused as shell input — `''` for the empty string, `$'...'` (ansic_quote)
/// when any non-printing character is present, else `'...'` single quotes.
/// Used by the string transforms @Q/@K/@k and by string_var_assignment.
/// TODO(declare -p): subst.c's declare -p value rendering should share this
/// helper; it is intentionally left untouched in this pass.
pub(in crate::executor) fn shell_reusable_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if ansic_should_quote_value(value) {
        ansic_quote_value(value)
    } else {
        shell_single_quote_assignment_value(value)
    }
}

/// GNU ansic_shouldquote (lib/sh/strtrans.c:341): $'...' quoting is used when
/// the string contains any character that is not printable in the locale
/// (ISPRINT for ASCII bytes, iswprint for the rest).
fn ansic_should_quote_value(value: &str) -> bool {
    value
        .chars()
        .any(|ch| !is_ansic_printable(ch))
}

fn is_ansic_printable(ch: char) -> bool {
    if ch.is_ascii() {
        (0x20..=0x7e).contains(&(ch as u8))
    } else {
        // Non-ASCII bytes go through mbrtowc + iswprint in GNU; valid UTF-8
        // non-ASCII code points that are not control characters print.
        !ch.is_control()
    }
}

/// GNU ansic_quote (lib/sh/strtrans.c:230): `$'...'` with named escapes for
/// \a \b \e(\E) \f \n \r \t \v, `\\` and `\'` doubled, printable characters
/// copied verbatim, and every other byte rendered as a 3-digit octal escape.
fn ansic_quote_value(value: &str) -> String {
    let mut output = String::from("$'");
    for ch in value.chars() {
        match ch {
            '\x1b' => output.push_str("\\E"),
            '\x07' => output.push_str("\\a"),
            '\x08' => output.push_str("\\b"),
            '\x0b' => output.push_str("\\v"),
            '\x0c' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\\' => output.push_str("\\\\"),
            '\'' => output.push_str("\\'"),
            c if is_ansic_printable(c) => output.push(c),
            c => {
                // GNU walks unsigned bytes; each non-printable byte becomes
                // \NNN, so encode the UTF-8 byte sequence byte-for-byte.
                let mut buffer = [0u8; 4];
                for byte in c.encode_utf8(&mut buffer).as_bytes() {
                    output.push_str(&format!("\\{byte:03o}"));
                }
            }
        }
    }
    output.push('\'');
    output
}

/// Expands `&` in a patsub replacement string to the matched text, like Bash
/// (subst.c replace_pattern): `&` copies the match, `\&` is a literal `&`.
fn expand_replacement_amp(matched: &str, replacement: &str) -> String {
    let mut output = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '&' => output.push_str(matched),
            '\\' => match chars.peek() {
                Some('&') => {
                    chars.next();
                    output.push('&');
                }
                Some('\\') => {
                    chars.next();
                    output.push('\\');
                }
                _ => output.push('\\'),
            },
            _ => output.push(ch),
        }
    }
    output
}

/// Plain-string pattern replacement honoring `&` expansion.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_replacement_matches_empty_value() {
        assert_eq!(replace_parameter_pattern("", "*", "w", false, false, true), "w");
    }

    #[test]
    fn ampersand_expansion_follows_shopt() {
        // shopt patsub_replacement on: & copies the match (subst.c:9252).
        assert_eq!(replace_parameter_pattern("abcd", "b", "x&y", false, false, true), "axbycd");
        // shopt off: & stays literal (no MATCH_EXPREP, subst.c:9430-9431).
        assert_eq!(replace_parameter_pattern("abcd", "b", "x&y", false, false, false), "ax&ycd");
    }

    #[test]
    fn escaped_ampersand_and_backslash_are_literal() {
        // strcreplace flags=2 (stringlib.c:223-226): \& -> &, \\ -> \.
        assert_eq!(replace_parameter_pattern("abcd", "b", "\\&", false, false, true), "a&cd");
        assert_eq!(replace_parameter_pattern("abcd", "b", "\\\\", false, false, true), "a\\cd");
    }

    #[test]
    fn global_substitution_treats_hash_anchor_as_literal() {
        // subst.c:9452-9453: MATCH_GLOBREP forces MATCH_ANY, so a leading
        // `#` in a `//` substitution is a literal pattern character.
        assert_eq!(replace_parameter_pattern("abc", "#abc", "foo", true, false, true), "abc");
        assert_eq!(replace_parameter_pattern("abc", "#a", "foo", false, false, true), "foobc");
    }

    #[test]
    fn anchored_empty_pattern_inserts_replacement() {
        // pat_subst:9197-9229: null pattern + MATCH_BEG prefixes REP with
        // `&` expanding to the empty match.
        assert_eq!(replace_parameter_pattern("one", "#", "&two", false, false, true), "twoone");
        assert_eq!(replace_parameter_pattern("one", "%", "&two", false, false, true), "onetwo");
    }

    #[test]
    fn shortest_suffix_glob_preserves_quoted_value_apostrophe() {
        assert_eq!(remove_matching_suffix("x'a'y", "*a*", MatchLength::Shortest), "x'");
    }
}

fn replace_with_amp(
    value: &str,
    pattern: &str,
    replacement: &str,
    global: bool,
    nocase: bool,
    expand_amp: bool,
) -> String {
    let mut output = String::new();
    let mut last = 0;
    let mut replaced = false;
    for (index, matched) in LiteralMatches::new(value, pattern, nocase) {
        if replaced && !global {
            break;
        }
        output.push_str(&value[last..index]);
        if expand_amp {
            output.push_str(&expand_replacement_amp(matched, replacement));
        } else {
            output.push_str(replacement);
        }
        last = index + matched.len();
        replaced = true;
    }
    output.push_str(&value[last..]);
    output
}

/// Literal occurrences of `needle` in `value` as (byte offset, matched
/// text), case-folded when nocase is set (GNU match_upattern uses
/// FNMATCH_IGNCASE for the no-wildcard fast path too, subst.c:5382). The
/// matched text is sliced from the original value so `&` replacement
/// expands to the actual case-preserved match.
struct LiteralMatches<'a> {
    value: &'a str,
    chars: Vec<(usize, char)>,
    needle_chars: Vec<char>,
    nocase: bool,
    pos: usize,
}

impl<'a> LiteralMatches<'a> {
    fn new(value: &'a str, needle: &'a str, nocase: bool) -> Self {
        Self {
            value,
            chars: value.char_indices().collect(),
            needle_chars: needle.chars().collect(),
            nocase,
            pos: 0,
        }
    }
}

impl<'a> Iterator for LiteralMatches<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        let n = self.needle_chars.len();
        if n == 0 {
            return None;
        }
        while self.pos + n <= self.chars.len() {
            let hit = (0..n).all(|k| {
                let (_, vc) = self.chars[self.pos + k];
                let nc = self.needle_chars[k];
                if self.nocase {
                    vc.to_lowercase().eq(nc.to_lowercase())
                } else {
                    vc == nc
                }
            });
            if hit {
                let (byte_index, _) = self.chars[self.pos];
                let matched_len: usize = self.chars[self.pos..self.pos + n]
                    .iter()
                    .map(|(_, ch)| ch.len_utf8())
                    .sum();
                let matched = &self.value[byte_index..byte_index + matched_len];
                self.pos += n;
                return Some((byte_index, matched));
            }
            self.pos += 1;
        }
        None
    }
}
