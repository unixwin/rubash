use super::*;

pub(in crate::executor) fn replace_parameter_pattern(
    value: &str,
    pattern: &str,
    replacement: &str,
    global: bool,
    nocase: bool,
    patsub_replacement: bool,
    extglob: bool,
) -> String {
    // rubash#1533: in a byte locale patsub scans BYTE positions (GNU
    // pat_subst walks the raw string): a match may start or end in the
    // middle of a multibyte character (`${v//?/x}` on 'héllo' under
    // LC_ALL=C replaces all six bytes, not five characters). Run the
    // whole substitution in unit space — both sides staged as match
    // units, where one unit is one byte — and convert the assembled
    // result back to canonical shell text at the end. The replacement
    // is output text, not a matched side: it is inserted verbatim and
    // survives the round trip (only byte-char units are folded back
    // to raw bytes; every other char re-encodes as itself).
    if super::conditional::pattern::sides_need_byte_locale_units(pattern, value) {
        let unit_value: String = super::conditional::pattern::stage_word_match_units(value, true)
            .into_iter()
            .collect();
        let unit_pattern: String =
            super::conditional::pattern::stage_pattern_match_units(pattern, true)
                .into_iter()
                .collect();
        let unit_output = replace_parameter_pattern_inner(
            &unit_value,
            &unit_pattern,
            replacement,
            global,
            nocase,
            patsub_replacement,
            extglob,
        );
        let raw = super::conditional::pattern::match_units_to_raw_bytes(&unit_output);
        return crate::executor::substitution_metadata::bytes_to_shell_text(&raw);
    }
    replace_parameter_pattern_inner(
        value,
        pattern,
        replacement,
        global,
        nocase,
        patsub_replacement,
        extglob,
    )
}

fn replace_parameter_pattern_inner(
    value: &str,
    pattern: &str,
    replacement: &str,
    global: bool,
    nocase: bool,
    patsub_replacement: bool,
    extglob: bool,
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
            return replace_parameter_prefix(value, prefix_pattern, &amp, nocase, extglob);
        }

        if let Some(suffix_pattern) = pattern.strip_prefix('%') {
            return replace_parameter_suffix(value, suffix_pattern, &amp, nocase, extglob);
        }
    }

    if pattern.is_empty() {
        // GNU pat_subst:9208-9229 handles a null pattern only when mtype is
        // MATCH_BEG (prefix REP) or MATCH_END (append REP); those anchored
        // cases are already handled above by replace_parameter_prefix/suffix.
        // For the unanchored MATCH_ANY case the main loop (pat_subst:9238)
        // calls match_pattern, whose match_upattern wraps the empty pattern
        // as `**` for the strmatch pre-check, but the per-position scan
        // (match_upattern:5397-5431) calls match_pattern_char with the
        // original empty pattern. match_pattern_char (gm_loop.c:44-68)
        // reads `*pat` as NUL, falls through to the `default` case, and
        // compares FOLD(*string) == FOLD(0) which is always false for a
        // non-empty string — so no position matches and the loop breaks
        // immediately, returning the value unchanged.
        return value.to_string();
    }

    // Bash's glob `*` also matches an empty parameter value. Handle this
    // before the replacement loop so an empty match cannot loop forever.
    // GNU pat_subst:9230-9232: an empty string with a matching pattern
    // yields the replacement with `&` expanding to the empty match.
    if value.is_empty() {
        if parameter_pattern_match(pattern, "", nocase, extglob) {
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
    // not a glob escape. The pattern matcher (case_pattern_atom_matches)
    // already handles \x18 as a literal backslash, so preserve it instead
    // of converting to a real `\` which would be treated as a glob escape.
    // Convert \x14 (another internal backslash marker) to \x18 so the
    // matcher handles it uniformly.
    let pattern = pattern.replace(
        crate::executor::markers::DATA_BACKSLASH,
        crate::executor::markers::PATTERN_LITERAL_BACKSLASH_STR,
    );
    let pattern = pattern.as_str();
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let mut output = String::new();
    let mut cursor = 0;

    // GNU pat_subst (subst.c:9238+): the scan runs over positions
    // [0, len) — a zero-length match at the very end does NOT earn a
    // replacement (`${w//*(a)/Q}` on `abac` is `QQbQQc`, no trailing Q);
    // the empty-VALUE case was handled above.
    while cursor < value.len() {
        let Some((start, end)) =
            find_parameter_pattern_match(value, pattern, cursor, &indices, nocase, extglob)
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
fn parameter_pattern_match(pattern: &str, word: &str, nocase: bool, extglob: bool) -> bool {
    if extglob && pattern_uses_extglob_syntax(pattern) {
        if nocase {
            return super::conditional::extglob_case_pattern_matches_nocase(pattern, word);
        }
        return super::conditional::extglob_case_pattern_matches(pattern, word);
    }
    if nocase {
        case_pattern_matches_nocase(pattern, word)
    } else {
        case_pattern_matches(pattern, word)
    }
}

/// Prechecked twin of [`parameter_pattern_match`] for the finders below:
/// the caller has evaluated both byte gates once for the whole
/// (pattern, value) pair and they do not fire, and every candidate is a
/// char-boundary slice of that same value, so the per-candidate gate
/// re-evaluation inside the case matchers could only repeat the same
/// negative verdict at O(candidate length) per boundary — on a
/// multi-thousand-char value with an anchored pattern (`${v/#seg/…}`,
/// `${v/%gz/…}`) that re-scan was the dominant cost of the expansion.
fn parameter_pattern_match_prechecked(
    pattern: &str,
    word: &str,
    nocase: bool,
    extglob: bool,
) -> bool {
    if extglob && pattern_uses_extglob_syntax(pattern) {
        if nocase {
            return super::conditional::extglob_case_pattern_matches_nocase(pattern, word);
        }
        return super::conditional::extglob_case_pattern_matches(pattern, word);
    }
    super::conditional::pattern::case_pattern_matches_prechecked(pattern, word, nocase)
}

/// One-time gate verdict for a (pattern, value) pair whose char-boundary
/// slices are about to be matched one by one: both byte gates are
/// containment predicates, so a gate-clean pair stays clean for every
/// slice (see case_pattern_matches_prechecked).
fn pattern_pair_gates_clean(pattern: &str, value: &str) -> bool {
    !super::conditional::pattern::sides_need_byte_char_matching(pattern, value)
        && !super::conditional::pattern::sides_need_byte_locale_units(pattern, value)
}

fn normalize_parameter_pattern_backslashes(pattern: &str) -> String {
    // The three passes rewrite the internal backslash markers only; a
    // pattern carrying none of them is already normalized, so skip the
    // three allocating replace passes (rubash#608).
    if !pattern.contains(crate::executor::markers::DATA_BACKSLASH)
        && !pattern.contains(crate::executor::markers::PATTERN_LITERAL_BACKSLASH)
    {
        return pattern.to_string();
    }
    pattern
        .replace(
            &crate::executor::markers::PATTERN_LITERAL_BACKSLASH_STR.repeat(2),
            "\\",
        )
        .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
        .replace(crate::executor::markers::PATTERN_LITERAL_BACKSLASH, "\\")
}

pub(in crate::executor) fn replace_parameter_prefix(
    value: &str,
    pattern: &str,
    amp: &dyn Fn(&str) -> String,
    nocase: bool,
    extglob: bool,
) -> String {
    let Some(end) = find_parameter_prefix_match(value, pattern, nocase, extglob) else {
        return value.to_string();
    };
    format!("{}{}", amp(&value[..end]), &value[end..])
}

pub(in crate::executor) fn replace_parameter_suffix(
    value: &str,
    pattern: &str,
    amp: &dyn Fn(&str) -> String,
    nocase: bool,
    extglob: bool,
) -> String {
    let Some(start) = find_parameter_suffix_match(value, pattern, nocase, extglob) else {
        return value.to_string();
    };
    format!("{}{}", &value[..start], amp(&value[start..]))
}

pub(super) fn pattern_contains_glob(pattern: &str) -> bool {
    // GNU pat_subst (subst.c:9238) routes EVERY pattern through
    // match_pattern/match_upattern — there is no literal fast path. The
    // literal shortcut here is admitted only for patterns that cannot
    // carry operator meaning: the basic glob set plus the ksh extglob
    // operator groups (`@(`, `!(`, `+(`, `*(`, `?(` — FNM_EXTMATCH when
    // the extglob option is on, literal characters otherwise, which the
    // compiled matcher also answers correctly with extglob off). Without
    // the extglob shapes `${v/@(abc)/X}` took the literal path and never
    // substituted (rubash#349).
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | '\\'))
        || pattern_uses_extglob_syntax(pattern)
}

pub(in crate::executor) fn find_parameter_prefix_match(
    value: &str,
    pattern: &str,
    nocase: bool,
    extglob: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(0);
    }

    let prechecked = pattern_pair_gates_clean(pattern, value);
    let candidate_matches = |word: &str| {
        if prechecked {
            parameter_pattern_match_prechecked(pattern, word, nocase, extglob)
        } else {
            parameter_pattern_match(pattern, word, nocase, extglob)
        }
    };
    // A star-free pattern matches at most pattern_match_length_bound
    // chars (the invariant find_parameter_pattern_match documents), so
    // candidate ends past that char distance cannot match and are not
    // tested; extglob groups change the match width, so the prune is
    // disabled for extglob patterns, exactly as in the scanning finder.
    let bound = if extglob && pattern_uses_extglob_syntax(pattern) {
        None
    } else {
        pattern_match_length_bound(pattern)
    };
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    indices
        .iter()
        .enumerate()
        .rev()
        .filter(|(pos, _)| bound.map_or(true, |bound| *pos <= bound))
        .find(|(_, end)| candidate_matches(&value[..**end]))
        .map(|(_, end)| *end)
}

pub(in crate::executor) fn find_parameter_suffix_match(
    value: &str,
    pattern: &str,
    nocase: bool,
    extglob: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(value.len());
    }

    let prechecked = pattern_pair_gates_clean(pattern, value);
    let candidate_matches = |word: &str| {
        if prechecked {
            parameter_pattern_match_prechecked(pattern, word, nocase, extglob)
        } else {
            parameter_pattern_match(pattern, word, nocase, extglob)
        }
    };
    // Bound prune, twin of find_parameter_prefix_match: a star-free
    // pattern matches at most pattern_match_length_bound chars, so
    // candidate starts whose suffix is longer than the bound cannot
    // match; disabled for extglob patterns.
    let bound = if extglob && pattern_uses_extglob_syntax(pattern) {
        None
    } else {
        pattern_match_length_bound(pattern)
    };
    let indices: Vec<usize> = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect();
    let total = indices.len() - 1;
    indices
        .iter()
        .enumerate()
        .filter(|(pos, _)| bound.map_or(true, |bound| total - *pos <= bound))
        .find(|(_, start)| candidate_matches(&value[**start..]))
        .map(|(_, start)| *start)
}

pub(in crate::executor) fn find_parameter_pattern_match(
    value: &str,
    pattern: &str,
    cursor: usize,
    indices: &[usize],
    nocase: bool,
    extglob: bool,
) -> Option<(usize, usize)> {
    let start_index = indices.iter().position(|index| *index >= cursor)?;

    // The byte gates are evaluated once for the whole (pattern, value)
    // pair; every candidate below is a char-boundary slice of the value,
    // so the verdict cannot change per candidate (see
    // case_pattern_matches_prechecked).
    let prechecked = pattern_pair_gates_clean(pattern, value);
    let candidate_matches = |word: &str| {
        if prechecked {
            parameter_pattern_match_prechecked(pattern, word, nocase, extglob)
        } else {
            parameter_pattern_match(pattern, word, nocase, extglob)
        }
    };

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
    // The bound/tail prunes assume glob-shape atoms; extglob groups change
    // both the match width and what follows the last `*` (which may be a
    // group operator), so disable them for extglob patterns.
    let extglob_pattern = extglob && pattern_uses_extglob_syntax(pattern);
    let bound = if extglob_pattern {
        None
    } else {
        pattern_match_length_bound(pattern)
    };
    let tail_ends = if extglob_pattern {
        None
    } else {
        pattern_literal_tail_ends(pattern, value)
    };
    // GNU sm_loop.c:121-135: a pattern whose last char is an unquoted `\`
    // can only match a `\` that is the LAST character of the whole string
    // (`p == pe && sc == '\\' && n+1 == se`). The per-substring matcher sees
    // each candidate end as its own `se`, so a lone trailing `\` must also
    // restrict candidates to those ending at value's end.
    let ends_at_value_end = !extglob_pattern && pattern_ends_with_lone_backslash(pattern);

    for (start_pos, start) in indices[start_index..].iter().enumerate() {
        // Candidate ends include `start` itself: a pattern that can match
        // the empty string (`*(...)`, `?(...)`, `*`) matches zero-length
        // at this position, and GNU pat_subst replaces that empty match
        // (subst.c:9238 match loop; rubash#1091). It is tried first so
        // the ascending scan still ends with the LONGEST match.
        let ends = &indices[start_index + start_pos..];
        let mut best: Option<usize> = None;
        for (end_rel, end) in ends.iter().enumerate() {
            if let Some(bound) = bound {
                if end_rel > bound {
                    break;
                }
            }
            if let Some(allowed) = &tail_ends {
                if !allowed.contains(end) {
                    continue;
                }
            }
            if ends_at_value_end && *end != value.len() {
                continue;
            }
            if candidate_matches(&value[*start..*end]) {
                best = Some(*end);
            }
        }
        if let Some(end) = best {
            return Some((*start, end));
        }
    }

    None
}

/// True when `pattern` ends with an unescaped `\` — an odd-length trailing
/// run of raw backslashes. CTLESC-quoted or PATTERN_LITERAL_BACKSLASH chars
/// are already-marked literals and never count.
fn pattern_ends_with_lone_backslash(pattern: &str) -> bool {
    let mut count = 0usize;
    for ch in pattern.chars().rev() {
        if ch == '\\' {
            count += 1;
        } else {
            break;
        }
    }
    count % 2 == 1
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
                index = if close < chars.len() {
                    close + 1
                } else {
                    index + 1
                };
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

/// GNU subst.c:8944-8951 parameter_brace_transform +
/// valid_parameter_transform (subst.c:8898): when the text after the
/// parameter name is `@xform`, xform must be exactly one valid transform
/// character. Returns the base variable name when `inner` is such a
/// `name@junk` form with an invalid suffix; the caller then applies the
/// unset-NULL (subst.c:8927) vs fatal-error split. `#[...]` length forms
/// are excluded because GNU parses `#` first, so a later `@` is operand
/// text, not a transform operator.
pub(in crate::executor) fn invalid_at_transform_base(inner: &str) -> Option<&str> {
    // GNU subst.c:9978-10007 parameter_brace_expand: `${!PREFIX@}` /
    // `${!PREFIX*}` is the variable-name LIST form, decided before the
    // parameter_brace_transform validation — a bang-led head whose only
    // tail is the `@`/`*` list marker is never a transform introducer
    // (rubash#338: `${!v@}` was misread as `${!v` + empty `@` transform
    // and rejected as bad substitution through the set-variable gate).
    if let Some(head) = inner.strip_suffix(['@', '*']) {
        if let Some(name) = head.strip_prefix('!') {
            if !name.is_empty() && is_shell_name(name) {
                return None;
            }
        }
    }
    // GNU parameter_brace_transform reads the transform introducer as
    // the FIRST `@` outside any [...] subscript — for the special
    // parameters `@`/`*` themselves, the second character — and requires
    // everything after it to be exactly one valid transform character.
    // Splitting at the LAST `@` (rsplit_once) misread a chained form
    // like `v@Q@L` as variable `v@Q` with transform L and expanded it
    // silently empty; GNU reports a bad substitution (rubash#1271).
    // Another operator character at bracket depth 0 (`/`, `#`, `%`,
    // the `:-` family, `^`/`,`) ends the parameter name first, so the
    // `@` behind it is operand text, not an introducer.
    let scan_from = if inner.starts_with(['@', '*']) { 1 } else { 0 };
    let mut depth = 0usize;
    let mut at = None;
    for (index, ch) in inner.char_indices() {
        if index < scan_from {
            continue;
        }
        match ch {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '@' if depth == 0 => {
                at = Some(index);
                break;
            }
            '/' | '#' | '%' | '-' | '+' | '=' | '?' | '^' | ',' | ':' if depth == 0 => {
                return None;
            }
            _ => {}
        }
    }
    let (head, tail) = inner.split_at(at?);
    let tail = &tail[1..];
    let bracketed = head.ends_with(']') && head.contains('[');
    let valid_head = is_shell_name(head)
        || matches!(head, "@" | "*" | "!" | "?" | "-" | "$")
        || (!head.is_empty() && head.chars().all(|c: char| c.is_ascii_digit()))
        || head
            .strip_prefix('!')
            .is_some_and(|h: &str| is_shell_name(h) || (h.ends_with(']') && h.contains('[')));
    let valid_tail = tail.len() == 1
        && matches!(
            tail,
            "a" | "A" | "K" | "k" | "E" | "P" | "Q" | "U" | "u" | "L"
        );
    if !(valid_head || bracketed) || valid_tail {
        return None;
    }
    let base = head.split('[').next().unwrap_or(head);
    Some(base.strip_prefix('!').unwrap_or(base))
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
        // ${var@U}/${var@L} run the same casemod.c simple 1:1 mapping
        // as ${var^^}/${var,,} (rubash#1063).
        ParameterTransform::Upper => value
            .chars()
            .map(crate::locale::simple_case_upper)
            .collect(),
        ParameterTransform::UpperFirst => uppercase_first_char(value),
        ParameterTransform::Lower => value
            .chars()
            .map(crate::locale::simple_case_lower)
            .collect(),
    }
}

fn uppercase_first_char(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(value.len());
    out.push(crate::locale::simple_case_upper(first));
    out.push_str(chars.as_str());
    out
}

pub(in crate::executor) fn format_key_value_transform_part(
    key: &str,
    value: &str,
    quoted: bool,
) -> String {
    if quoted {
        let quoted_key = quote_key_value_transform_key(key);
        let quoted_value = quote_array_value(value);
        format!("{quoted_key} {quoted_value}")
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
    if key
        .chars()
        .any(|ch| (ch as u32) < 0x20 || (ch as u32) == 0x7f)
    {
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
    // Raw-byte marker pairs carry bytes that never form printable
    // characters; decode them to the byte view first so the ansic form
    // quotes per GNU strtrans.c ansic_quote (one octal escape per
    // non-printable byte, printable multibyte runs verbatim).
    // Only a genuine E000+payload pair marks stored raw bytes (rubash#1060):
    // a literal user U+E000 decodes through to a printable PUA char and
    // takes the normal char paths below.
    if crate::executor::substitution_metadata::has_raw_byte_marker_pair(value) {
        let bytes =
            crate::executor::substitution_metadata::decode_raw_byte_markers(value.as_bytes());
        return ansic_quote_bytes(&bytes);
    }
    let visible = crate::locale::decode_to_visible_text(value);
    if ansic_should_quote_value(&visible) {
        ansic_quote_value(&visible)
    } else {
        shell_single_quote_assignment_value(&visible)
    }
}

/// GNU strtrans.c ansic_quote over the raw byte string: named C escapes,
/// printable multibyte runs verbatim, one 3-digit octal escape per
/// non-printable or invalid byte (utf8_mbstrlen walks one byte at a time).
fn ansic_quote_bytes(bytes: &[u8]) -> String {
    let mut output = String::from("$'");
    let mut rest: &[u8] = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                push_ansic_escaped_chars(&mut output, text.chars());
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if let Ok(text) = std::str::from_utf8(&rest[..valid]) {
                    push_ansic_escaped_chars(&mut output, text.chars());
                }
                output.push_str(&format!("\\{:03o}", rest[valid]));
                rest = &rest[valid + 1..];
            }
        }
    }
    output.push('\'');
    output
}

fn push_ansic_escaped_chars(output: &mut String, chars: impl Iterator<Item = char>) {
    for ch in chars {
        match ch {
            crate::executor::markers::QUOTED_WORD_PREFIX => output.push_str("\\E"),
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
                let mut buffer = [0u8; 4];
                for byte in c.encode_utf8(&mut buffer).as_bytes() {
                    output.push_str(&format!("\\{byte:03o}"));
                }
            }
        }
    }
}

/// GNU ansic_shouldquote (lib/sh/strtrans.c:341): $'...' quoting is used when
/// the string contains any character that is not printable in the locale
/// (ISPRINT for ASCII bytes, iswprint for the rest).
fn ansic_should_quote_value(value: &str) -> bool {
    value.chars().any(|ch| !is_ansic_printable(ch))
}

fn is_ansic_printable(ch: char) -> bool {
    if ch.is_ascii() {
        (0x20..=0x7e).contains(&(ch as u8))
    } else {
        // Non-ASCII bytes go through mbrtowc + iswprint in GNU; valid UTF-8
        // non-ASCII code points that are not control characters print —
        // but only in a UTF-8 locale: ansic_quote's multibyte pass-through
        // (strtrans.c:266-282) is the UTF-8-locale branch, and under
        // LC_ALL=C every byte >= 0x80 is escaped octal instead
        // (rubash#1464).
        crate::locale::is_utf8() && !ch.is_control()
    }
}

/// GNU ansic_quote (lib/sh/strtrans.c:230): `$'...'` with named escapes for
/// \a \b \e(\E) \f \n \r \t \v, `\\` and `\'` doubled, printable characters
/// copied verbatim, and every other byte rendered as a 3-digit octal escape.
fn ansic_quote_value(value: &str) -> String {
    let mut output = String::from("$'");
    for ch in value.chars() {
        match ch {
            crate::executor::markers::QUOTED_WORD_PREFIX => output.push_str("\\E"),
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
        assert_eq!(
            replace_parameter_pattern("", "*", "w", false, false, true, false),
            "w"
        );
    }

    #[test]
    fn ampersand_expansion_follows_shopt() {
        // shopt patsub_replacement on: & copies the match (subst.c:9252).
        assert_eq!(
            replace_parameter_pattern("abcd", "b", "x&y", false, false, true, false),
            "axbycd"
        );
        // shopt off: & stays literal (no MATCH_EXPREP, subst.c:9430-9431).
        assert_eq!(
            replace_parameter_pattern("abcd", "b", "x&y", false, false, false, false),
            "ax&ycd"
        );
    }

    #[test]
    fn escaped_ampersand_and_backslash_are_literal() {
        // strcreplace flags=2 (stringlib.c:223-226): \& -> &, \\ -> \.
        assert_eq!(
            replace_parameter_pattern("abcd", "b", "\\&", false, false, true, false),
            "a&cd"
        );
        assert_eq!(
            replace_parameter_pattern("abcd", "b", "\\\\", false, false, true, false),
            "a\\cd"
        );
    }

    #[test]
    fn global_substitution_treats_hash_anchor_as_literal() {
        // subst.c:9452-9453: MATCH_GLOBREP forces MATCH_ANY, so a leading
        // `#` in a `//` substitution is a literal pattern character.
        assert_eq!(
            replace_parameter_pattern("abc", "#abc", "foo", true, false, true, false),
            "abc"
        );
        assert_eq!(
            replace_parameter_pattern("abc", "#a", "foo", false, false, true, false),
            "foobc"
        );
    }

    #[test]
    fn anchored_empty_pattern_inserts_replacement() {
        // pat_subst:9197-9229: null pattern + MATCH_BEG prefixes REP with
        // `&` expanding to the empty match.
        assert_eq!(
            replace_parameter_pattern("one", "#", "&two", false, false, true, false),
            "twoone"
        );
        assert_eq!(
            replace_parameter_pattern("one", "%", "&two", false, false, true, false),
            "onetwo"
        );
    }

    #[test]
    fn shortest_suffix_glob_preserves_quoted_value_apostrophe() {
        assert_eq!(
            remove_matching_suffix("x'a'y", "*a*", MatchLength::Shortest, false),
            "x'"
        );
    }

    #[test]
    fn issue_608_literal_removal_single_candidate() {
        // GNU bash 5.3.20 verdicts (subst.c match_upattern): a
        // plain-literal pattern can only match the single fragment of
        // exactly its own length, for both match lengths — the fast
        // path must reproduce the boundary walk exactly.
        let v = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(
            remove_matching_prefix(v, "Q", MatchLength::Shortest, false),
            v
        );
        assert_eq!(
            remove_matching_prefix(v, "Q", MatchLength::Longest, false),
            v
        );
        assert_eq!(
            remove_matching_suffix(v, "Q", MatchLength::Shortest, false),
            v
        );
        assert_eq!(
            remove_matching_suffix(v, "Q", MatchLength::Longest, false),
            v
        );
        assert_eq!(
            remove_matching_prefix(v, "a", MatchLength::Shortest, false),
            "bcdefghijklmnopqrstuvwxyz"
        );
        assert_eq!(
            remove_matching_prefix(v, "abc", MatchLength::Longest, false),
            "defghijklmnopqrstuvwxyz"
        );
        assert_eq!(
            remove_matching_suffix(v, "xyz", MatchLength::Longest, false),
            "abcdefghijklmnopqrstuvw"
        );
        assert_eq!(
            remove_matching_suffix(v, "z", MatchLength::Shortest, false),
            "abcdefghijklmnopqrstuvwxy"
        );
        // Empty pattern matches only the empty fragment: no-op.
        assert_eq!(
            remove_matching_prefix(v, "", MatchLength::Shortest, false),
            v
        );
        assert_eq!(
            remove_matching_suffix(v, "", MatchLength::Longest, false),
            v
        );
        // A pattern longer than the value can never match.
        assert_eq!(
            remove_matching_prefix("ab", "abc", MatchLength::Shortest, false),
            "ab"
        );
        assert_eq!(
            remove_matching_suffix("ab", "abc", MatchLength::Shortest, false),
            "ab"
        );
        // A literal containing a space is still the literal class.
        assert_eq!(
            remove_matching_prefix("a b c", "a ", MatchLength::Shortest, false),
            "b c"
        );
        // Multibyte value: the single candidate is a byte length, so a
        // match must land on a char boundary.
        assert_eq!(
            remove_matching_prefix("héllo", "h", MatchLength::Shortest, false),
            "éllo"
        );
        assert_eq!(
            remove_matching_suffix("héllo", "o", MatchLength::Shortest, false),
            "héll"
        );
        // Case stays sensitive even though pat_subst honors nocasematch
        // elsewhere: GNU removal matching does not fold (verified
        // against the 5.3.20 oracle with nocasematch set).
        assert_eq!(
            remove_matching_prefix("ABC", "a", MatchLength::Shortest, false),
            "ABC"
        );
        assert_eq!(
            remove_matching_suffix("ABC", "c", MatchLength::Shortest, false),
            "ABC"
        );
        // Glob patterns keep the general walk (GNU verdicts).
        assert_eq!(
            remove_matching_prefix("aXbXc", "*X", MatchLength::Shortest, false),
            "bXc"
        );
        assert_eq!(
            remove_matching_prefix("aXbXc", "*X", MatchLength::Longest, false),
            "c"
        );
        assert_eq!(
            remove_matching_suffix("aXbXc", "X*", MatchLength::Shortest, false),
            "aXb"
        );
        assert_eq!(
            remove_matching_suffix("aXbXc", "X*", MatchLength::Longest, false),
            "a"
        );
    }

    #[test]
    fn issue_608_normalize_backslashes_marker_free_is_identity() {
        // Patterns without either internal backslash marker are already
        // normalized; the rewrite passes must not fire.
        assert_eq!(normalize_parameter_pattern_backslashes("abc"), "abc");
        assert_eq!(normalize_parameter_pattern_backslashes("a/b?c"), "a/b?c");
        // Marker-carrying patterns still normalize to a real backslash.
        let data_marked = format!("a{}b", crate::executor::markers::DATA_BACKSLASH);
        assert_eq!(
            normalize_parameter_pattern_backslashes(&data_marked),
            "a\\b"
        );
        let literal_marked = format!("a{}b", crate::executor::markers::PATTERN_LITERAL_BACKSLASH);
        assert_eq!(
            normalize_parameter_pattern_backslashes(&literal_marked),
            "a\\b"
        );
    }

    #[test]
    fn global_replace_matches_empty_extglob_spans() {
        // rubash#1091: GNU subst.c pat_subst global loop + match_upattern
        // MATCH_ANY (p1 ranges down to p itself) replaces the empty match
        // at every gap, copying one character past it (pat_subst:9286-9304)
        // — `*(a)`/`?(x)` are not skips. Byte-compared against GNU 5.3.20
        // (script-file mode).
        assert_eq!(
            replace_parameter_pattern("abac", "*(a)", "Q", true, false, false, true),
            "QQbQQc"
        );
        assert_eq!(
            replace_parameter_pattern("x", "*(y)", "Q", true, false, false, true),
            "Qx"
        );
        assert_eq!(
            replace_parameter_pattern("aaab", "*(a)", "Q", true, false, false, true),
            "QQb"
        );
        assert_eq!(
            replace_parameter_pattern("abc", "?(x)", "Q", true, false, false, true),
            "QaQbQc"
        );
        assert_eq!(
            replace_parameter_pattern("ab", "*(b)", "Q", true, false, false, true),
            "QaQ"
        );
        assert_eq!(
            replace_parameter_pattern("abac", "*(z)", "Q", true, false, false, true),
            "QaQbQaQc"
        );
        // Single (non-global) replacement keeps longest-at-leftmost: an
        // empty seed never beats a longer match at the same start, and a
        // zero-length single match inserts without consuming.
        assert_eq!(
            replace_parameter_pattern("abac", "*(a)", "Q", false, false, false, true),
            "Qbac"
        );
        assert_eq!(
            replace_parameter_pattern("abc", "?(x)", "Q", false, false, false, true),
            "Qabc"
        );
        // `+(...)` never matches empty: no seed, behavior unchanged.
        assert_eq!(
            replace_parameter_pattern("aaab", "+(a)", "Q", true, false, false, true),
            "Qb"
        );
        assert_eq!(
            replace_parameter_pattern("abac", "*", "Q", true, false, false, true),
            "Q"
        );
        assert_eq!(
            replace_parameter_pattern("abac", "?", "Q", true, false, false, true),
            "QQQQ"
        );
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
