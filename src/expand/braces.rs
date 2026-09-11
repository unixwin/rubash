//! Brace expansion for {a,b,c} comma-separated lists and {1..5}/{a..e} sequences.
//!
//! GNU Bash source ownership:
// - braces.c

/// Expand brace patterns in a word, returning multiple words.
/// Handles {a,b,c} comma-separated lists and {1..5}, {a..e} sequences.
/// Returns a single-element vec if no braces found (no expansion needed).
pub fn expand_braces(word: &str) -> Vec<String> {
    brace_expand(word)
}

fn brace_expand(text: &str) -> Vec<String> {
    // GNU braces.c brace_expand recursion
    let Some((open, close, etype)) = find_first_valid_brace(text) else {
        return vec![text.to_string()];
    };
    let preamble = &text[..open];
    let amble = &text[open + 1..close];
    let postamble = &text[close + 1..];

    let tack: Vec<String> = if etype == BRACE_SEQ {
        // valid_seqterm already passed, try expand_seqterm
        if let Some(seq) = expand_range(amble) {
            seq
        } else {
            // GNU: if sequence expansion fails (overflow etc) and postamble non-empty,
            // treat the whole brace as literal and recurse on postamble.
            // If postamble empty, return literal whole text.
            if !postamble.is_empty() {
                let mut literal = Vec::new();
                literal.push(format!("{{{}}}", amble));
                literal
            } else {
                return vec![text.to_string()];
            }
        }
    } else if etype == BRACE_COMMA {
        expand_amble(amble)
    } else {
        vec![format!("{{{}}}", amble)]
    };

    // array_concat(preamble, tack)
    let mut result: Vec<String> = Vec::new();
    if preamble.is_empty() && tack.is_empty() {
        result.push(String::new());
    } else if preamble.is_empty() {
        result.extend(tack.clone());
    } else if tack.is_empty() {
        result.push(preamble.to_string());
    } else {
        // preamble is single string, tack is Vec<String>
        for t in &tack {
            result.push(format!("{}{}", preamble, t));
        }
    }
    // handle empty tack case where result is just preamble?
    if result.is_empty() {
        result.push(preamble.to_string());
    }

    if postamble.is_empty() {
        return result;
    }
    let post_expanded = brace_expand(postamble);
    // array_concat(result, post_expanded)
    let mut final_result = Vec::new();
    for r in &result {
        for p in &post_expanded {
            final_result.push(format!("{}{}", r, p));
        }
    }
    // handle cases where result or post_expanded is empty?
    if final_result.is_empty() {
        return result;
    }
    final_result
}

fn expand_amble(amble: &str) -> Vec<String> {
    let parts = split_brace_commas(amble);
    // GNU braces.c expand_amble: every top-level alternative is brace-expanded
    // and the results are appended (array_concat) — a list, not a cross
    // product. Empty alternatives are KEPT as empty elements here; the
    // resulting bare empty word is dropped later by ordinary unquoted
    // empty-word removal. That is what makes
    //   a{,}b  -> [ab][ab]   {a,}x -> [ax][x]   x{a,} -> [xa][x]
    // while
    //   {a,}   -> [a]        (the trailing empty word is discarded)
    // and
    //   {,}    -> one empty word (printf '<%s>' prints <>)
    let mut result: Vec<String> = Vec::new();
    for part in parts {
        result.extend(brace_expand(part));
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

const BRACE_COMMA: i32 = 0x01;
const BRACE_SEQ: i32 = 0x02;
// const BRACE_NONE: i32 = 0x04;

fn valid_seqterm(amble: &str) -> bool {
    // Port of braces.c valid_seqterm minimal check
    let Some(pos) = amble.find("..") else {
        return false;
    };
    let lhs = &amble[..pos];
    let rhs = &amble[pos + 2..];
    if lhs.is_empty() || rhs.is_empty() {
        return false;
    }
    if lhs.starts_with('.') || rhs.starts_with('}') {
        return false;
    }
    // lhs type
    let lhs_is_int = lhs
        .as_bytes()
        .first()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
        || ((lhs.starts_with('+') || lhs.starts_with('-'))
            && lhs
                .as_bytes()
                .get(1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false));
    let lhs_is_char = lhs.len() == 1 && lhs.as_bytes()[0].is_ascii_alphabetic();
    if !lhs_is_int && !lhs_is_char {
        return false;
    }
    // rhs type: need to handle possible increment part ".."
    // Extract first segment of rhs before next ".." if any
    let rhs_first = if let Some(p) = rhs.find("..") {
        &rhs[..p]
    } else {
        rhs
    };
    if rhs_first.is_empty() {
        return false;
    }
    let rhs_is_int = rhs_first
        .as_bytes()
        .first()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
        || ((rhs_first.starts_with('+') || rhs_first.starts_with('-'))
            && rhs_first
                .as_bytes()
                .get(1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false));
    let rhs_is_char = rhs_first.len() == 1 && rhs_first.as_bytes()[0].is_ascii_alphabetic();
    if !rhs_is_int && !rhs_is_char {
        return false;
    }
    // Types must match (both int or both char)
    let lhs_is_int_type = lhs_is_int;
    let rhs_is_int_type = rhs_is_int;
    if lhs_is_int_type != rhs_is_int_type {
        return false;
    }
    // Also ensure rhs segment is not empty and not starting with '}'
    true
}

fn find_first_valid_brace(text: &str) -> Option<(usize, usize, i32)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut escaped = false;

    while i < bytes.len() {
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if bytes[i] == b'\\' {
            escaped = true;
            i += 1;
            continue;
        }
        // Skip single-quoted strings
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        // Skip double-quoted strings
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        // GNU bash runs brace expansion on the word itself; the text of a
        // command substitution is a separate parse unit and the outer shell
        // must not split braces inside it (parse.y read_token_word hands the
        // $()/backtick body to parse_comsub, never to brace_expand). Skip
        // the whole substitution body here.
        if bytes[i] == b'`' {
            i = skip_backtick_body(bytes, i);
            continue;
        }
        if bytes[i] == b'$' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                i = skip_dollar_paren_body(bytes, i);
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                // $'...' ANSI-C quoting: the single quote opens a quoted
                // unit, not a quoting state toggle for later bytes.
                i = skip_single_quoted(bytes, i + 1);
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                // Skip the whole ${...} parameter body: its closing brace is
                // part of the expansion (subst.c extract_dollar_brace_string),
                // never the closing brace of an enclosing brace group, and a
                // comma inside it (foo{bar,${var}.}) does not split.
                i = skip_dollar_brace_body(bytes, i);
                continue;
            }
        }
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        // Bash ignores a brace opener at a word boundary when it is followed
        // by whitespace or a closing brace; this keeps `{ a,b}` literal.
        let preceded_by_whitespace = i == 0 || bytes[i - 1].is_ascii_whitespace();
        let followed_by_whitespace_or_close =
            i + 1 >= bytes.len() || bytes[i + 1].is_ascii_whitespace() || bytes[i + 1] == b'}';
        if preceded_by_whitespace && followed_by_whitespace_or_close {
            i += 1;
            continue;
        }
        // Skip ${...} parameter expansions
        if i > 0 && bytes[i - 1] == b'$' {
            i += 1;
            continue;
        }

        let inner_start = i + 1;
        let mut depth = 1u32;
        let mut j = inner_start;
        let mut has_comma = false;
        let mut has_double_dot = false;
        let mut j_escaped = false;
        let mut j_single = false;
        let mut j_double = false;

        while j < bytes.len() && depth > 0 {
            if j_escaped {
                j_escaped = false;
                j += 1;
                continue;
            }
            if bytes[j] == b'\'' && !j_double {
                j_single = !j_single;
                j += 1;
                continue;
            }
            if bytes[j] == b'"' && !j_single {
                j_double = !j_double;
                j += 1;
                continue;
            }
            if j_single || j_double {
                j += 1;
                continue;
            }
            match bytes[j] {
                b'\\' => {
                    j_escaped = true;
                    j += 1;
                    continue;
                }
                b'$' if j + 1 < bytes.len() && bytes[j + 1] == b'{' => {
                    j = skip_dollar_brace_body(bytes, j);
                    continue;
                }
                b'$' if j + 1 < bytes.len() && bytes[j + 1] == b'(' => {
                    j = skip_dollar_paren_body(bytes, j);
                    continue;
                }
                b'`' => {
                    j = skip_backtick_body(bytes, j);
                    continue;
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                b',' if depth == 1 => has_comma = true,
                b'.' if depth == 1 && j + 1 < bytes.len() && bytes[j + 1] == b'.' => {
                    has_double_dot = true;
                }
                _ => {}
            }
            j += 1;
        }

        if depth != 0 {
            i += 1;
            continue;
        }

        let inner = &text[inner_start..j];
        let etype = if has_comma {
            BRACE_COMMA
        } else if has_double_dot {
            BRACE_SEQ
        } else {
            0 // BRACE_NONE
        };
        if etype == 0 {
            i += 1;
            continue;
        }
        if etype == BRACE_SEQ && !valid_seqterm(inner) {
            // GNU valid_seqterm check: skip invalid sequence brace
            i += 1;
            continue;
        }
        return Some((i, j, etype));
    }
    None
}

/// Index just past the matching close brace of the ${ at `start` (which
/// points at the $). Tracks brace nesting while staying inside quoted
/// spans, mirroring subst.c extract_dollar_brace_string's search.
fn skip_dollar_brace_body(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && !single {
            i += 2;
            continue;
        }
        if ch == b'\'' && !double {
            single = !single;
        } else if ch == b'"' && !single {
            double = !double;
        } else if !single && !double {
            if ch == b'{' {
                depth += 1;
            } else if ch == b'}' {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Index just past the closing backtick starting at `start`. Backslash
/// escapes are honored (parse.y lexes \` inside a word as escaped data).
fn skip_backtick_body(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// Index just past the matching close paren of the $( at `start` (which
/// points at the $). Tracks parenthesis nesting while staying inside
/// single- and double-quoted spans, mirroring parse.y parse_comsub's
/// matching-paren search.
fn skip_dollar_paren_body(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && !single {
            i += 2;
            continue;
        }
        if ch == b'\'' && !double {
            single = !single;
        } else if ch == b'"' && !single {
            double = !double;
        } else if !single && !double {
            if ch == b'(' {
                depth += 1;
            } else if ch == b')' {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Index just past the closing single quote starting at `start`.
fn skip_single_quoted(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'\'' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn split_brace_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0u32;
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut escaped = false;
    // GNU braces.c searches for splitting commas outside quoted spans, so a
    // comma inside "..." or '...' (echo {"x,x"}) does not split the group.
    let mut single = false;
    let mut double = false;
    while i < bytes.len() {
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        match bytes[i] {
            b'\\' => {
                escaped = true;
                i += 1;
                continue;
            }
            b'\'' if !double => single = !single,
            b'"' if !single => double = !double,
            b'{' if !single && !double => depth += 1,
            b'}' if !single && !double => depth = depth.saturating_sub(1),
            b',' if depth == 0 && !single && !double => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

fn expand_range(s: &str) -> Option<Vec<String>> {
    let parts = s.split("..").collect::<Vec<_>>();
    let ([left, right] | [left, right, _]) = parts.as_slice() else {
        return None;
    };
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let step = match parts.as_slice() {
        [_, _] => 1,
        [_, _, step] => step.parse::<i64>().ok()?.abs().max(1),
        _ => return None,
    };

    // Numeric range
    if let (Ok(start), Ok(end)) = (left.parse::<i64>(), right.parse::<i64>()) {
        let width = numeric_range_width(left, right);
        let step = if start <= end { step } else { -step };
        let mut result = Vec::new();
        let mut current = start;
        while (step > 0 && current <= end) || (step < 0 && current >= end) {
            result.push(format_numeric_range_value(current, width));
            current += step;
        }
        return Some(result);
    }
    // Alpha range
    let start = left.as_bytes()[0];
    let end = right.as_bytes()[0];
    if left.len() == 1
        && right.len() == 1
        && start.is_ascii_alphabetic()
        && end.is_ascii_alphabetic()
    {
        let step = i16::try_from(step).ok()?;
        let step: i16 = if start <= end { step } else { -step };
        let mut result = Vec::new();
        let mut current = start as i16;
        // GNU braces.c renders the backslash position of a character
        // sequence as an EMPTY element (it is the quoting character):
        // {a..Z} yields ... ] "" [ ... so the baseline shows "]  [".
        while (step > 0 && current <= end as i16) || (step < 0 && current >= end as i16) {
            let byte = current as u8;
            if byte == b'\\' {
                result.push(String::new());
            } else {
                result.push((byte as char).to_string());
            }
            current += step;
        }
        return Some(result);
    }
    None
}

fn numeric_range_width(left: &str, right: &str) -> Option<usize> {
    let left_digits = left.trim_start_matches('-');
    let right_digits = right.trim_start_matches('-');
    let padded = [left_digits, right_digits]
        .iter()
        .any(|value| value.len() > 1 && value.starts_with('0'));
    padded.then(|| left.len().max(right.len()))
}

fn format_numeric_range_value(value: i64, width: Option<usize>) -> String {
    let Some(width) = width else {
        return value.to_string();
    };
    if value < 0 {
        format!(
            "-{:0width$}",
            value.unsigned_abs(),
            width = width.saturating_sub(1)
        )
    } else {
        format!("{value:0width$}")
    }
}

#[cfg(test)]
mod dollar_probe_tests {
    use super::expand_braces;

    #[test]
    fn dollar_body_splits_like_gnu() {
        assert_eq!(expand_braces("foo{bar,${a}.}"), vec!["foobar", "foo${a}."]);
        assert_eq!(expand_braces("${a}{x,y}"), vec!["${a}x", "${a}y"]);
        assert_eq!(expand_braces("{a,${a},b}"), vec!["a", "${a}", "b"]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comma_brace() {
        assert_eq!(expand_braces("{a,b,c}"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_nested_comma() {
        assert_eq!(expand_braces("x{a,b}y"), vec!["xay", "xby"]);
    }

    #[test]
    fn test_range_numeric() {
        assert_eq!(expand_braces("{1..3}"), vec!["1", "2", "3"]);
    }

    #[test]
    fn test_range_numeric_step_and_padding() {
        assert_eq!(expand_braces("{1..5..2}"), vec!["1", "3", "5"]);
        assert_eq!(expand_braces("{1..6..4}"), vec!["1", "5"]);
        assert_eq!(expand_braces("{5..1..2}"), vec!["5", "3", "1"]);
        assert_eq!(expand_braces("{01..03}"), vec!["01", "02", "03"]);
        assert_eq!(expand_braces("{-03..01..2}"), vec!["-03", "-01", "001"]);
        assert_eq!(
            expand_braces("{-003..001..2}"),
            vec!["-003", "-001", "0001"]
        );
    }

    #[test]
    fn test_range_alpha() {
        assert_eq!(expand_braces("{a..c}"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_range_alpha_step() {
        assert_eq!(expand_braces("{a..e..2}"), vec!["a", "c", "e"]);
        assert_eq!(expand_braces("{e..a..2}"), vec!["e", "c", "a"]);
    }

    #[test]
    fn test_nested_escaped_brace_preserves_literal_suffix() {
        assert_eq!(
            expand_braces(r"{x,y,\{a,b,c}}"),
            vec![r"x}", r"y}", r"\{a}", r"b}", r"c}"],
        );
    }

    #[test]
    fn test_no_brace() {
        assert_eq!(expand_braces("hello"), vec!["hello"]);
    }

    #[test]
    fn test_escaped_commas_do_not_split_brace_items() {
        assert_eq!(expand_braces(r"a{b\,c,d}"), vec![r"ab\,c", "ad"]);
        assert_eq!(expand_braces(r"{x\,y,z}"), vec![r"x\,y", "z"]);
    }

    #[test]
    fn test_escaped_braces_do_not_start_or_end_nested_groups() {
        assert_eq!(expand_braces(r"{a\{b,c}"), vec![r"a\{b", "c"]);
        assert_eq!(expand_braces(r"{a\}b,c}"), vec![r"a\}b", "c"]);
    }

    #[test]
    fn test_adjacent_brace_groups() {
        assert_eq!(expand_braces("{a,b}{1..2}"), vec!["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn test_escaped_brace_group_adjacent_to_real_group() {
        // Escaped braces stay literal (backslashes stripped later by quote
        // removal in command_prepare) while unescaped groups still expand.
        assert_eq!(expand_braces(r"\{a,b}{1,2}"), vec![r"\{a,b}1", r"\{a,b}2"]);
        assert_eq!(
            expand_braces(r"a\{b,c}d{e,f}g"),
            vec![r"a\{b,c}deg", r"a\{b,c}dfg"]
        );
    }

    #[test]
    fn test_invalid_nested_sequences_expand_only_nested_commas() {
        // Expectations below are taken from GNU bash 5.3.0 directly
        // (braces.tests lines 136-142), not from the pre-rewrite sketch:
        //   echo {{1,2,3}..4}   -> {1..4} {2..4} {3..4}
        //   echo {6..{7,8,9}}   -> {6..7} {6..8} {6..9}
        //   echo {{a..c}..{1..3}} -> 9 words {a..1} ... {c..3}
        // Note the nested sequence braces are RETAINED in the output: the
        // invalid outer {..} never expands, it just carries the endpoint text.
        assert_eq!(
            expand_braces("{{1,2,3}..4}"),
            vec!["{1..4}", "{2..4}", "{3..4}"],
        );
        assert_eq!(
            expand_braces("{6..{7,8,9}}"),
            vec!["{6..7}", "{6..8}", "{6..9}"],
        );
        // GNU cross-products the two nested sequences even though neither is a
        // comma group in the valid_seqterm sense.
        assert_eq!(
            expand_braces("{{a..c}..{1..3}}"),
            vec![
                "{a..1}", "{a..2}", "{a..3}", "{b..1}", "{b..2}", "{b..3}", "{c..1}", "{c..2}",
                "{c..3}",
            ],
        );
        assert_eq!(
            expand_braces("{{a..c}..{1,10}}"),
            vec!["{a..1}", "{a..10}", "{b..1}", "{b..10}", "{c..1}", "{c..10}"],
        );
    }

    #[test]
    fn test_debug_adjacent_braces() {
        let result = expand_braces("{a,b}{1,2}");
        println!("Debug: expand_braces = {:?}", result);
        assert_eq!(result, vec!["a1", "a2", "b1", "b2"]);
    }
}
