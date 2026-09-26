use super::*;
use crate::lexer::dolbrace::{scan_braced_parameter_body, BraceContext, DolbraceState};

pub(in crate::executor) fn decode_parameter_word_quotes(word: &str) -> String {
    let mut output = String::new();
    let chars = word.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            crate::executor::markers::DATA_SQUOTE => {
                output.push('\'');
                index += 1;
            }
            crate::executor::markers::DATA_DQUOTE => {
                output.push('"');
                index += 1;
            }
            '"' => {
                index += 1;
                while index < chars.len() {
                    let ch = chars[index];
                    index += 1;
                    if ch == '"' {
                        break;
                    }
                    output.push(ch);
                }
            }
            '\'' => {
                if let Some(close_offset) = chars[index + 1..].iter().position(|ch| *ch == '\'') {
                    let close = index + 1 + close_offset;
                    for ch in &chars[index + 1..close] {
                        output.push(*ch);
                    }
                    index = close + 1;
                } else {
                    output.push('\'');
                    index += 1;
                }
            }
            ch => {
                output.push(ch);
                index += 1;
            }
        }
    }
    output
}

pub(in crate::executor) fn restore_protected_replacement_quotes(value: &str) -> String {
    value.replace(crate::executor::markers::PROTECTED_ESCAPED_SQUOTE, "\\'")
}

pub(in crate::executor) fn parse_parameter_error_operator(
    inner: &str,
    posix: bool,
) -> Option<(&str, &str, bool)> {
    // GNU subst.c:122 VALID_INDIR_PARAM excludes `?`/`#` under
    // posixly_correct, so `${!?}`/`${!:?'}` in posix mode are the `!`
    // parameter under `?`/`:?`, not an indirect expansion through `$?`.
    let error_name = |name: &str| is_parameter_error_name(name) || (posix && name == "!");
    if let Some((name, message)) = inner.split_once(":?") {
        if error_name(name) {
            return Some((name, message, true));
        }
    }

    if let Some((name, message)) = inner.split_once('?') {
        if error_name(name) {
            return Some((name, message, false));
        }
    }

    None
}

pub(in crate::executor) fn parse_parameter_assignment_operator(
    inner: &str,
) -> Option<(&str, bool)> {
    if let Some((name, _)) = inner.split_once(":=") {
        if is_shell_name(name)
            || name.parse::<usize>().is_ok_and(|index| index > 0)
            || parse_array_subscript(name).is_some()
        {
            return Some((name, true));
        }
    }

    if let Some((name, _)) = inner.split_once('=') {
        if is_shell_name(name)
            || name.parse::<usize>().is_ok_and(|index| index > 0)
            || parse_array_subscript(name).is_some()
        {
            return Some((name, false));
        }
    }

    None
}

pub(in crate::executor) fn matching_parameter_brace(input: &str) -> Option<usize> {
    matching_parameter_brace_in_context(input, false, false)
}

/// Locate the `}` closing a `${` body, honoring the outer quote context and
/// POSIX mode. GNU parse.y's dolbrace state machine (Austin Group Interp 221)
/// treats single quotes inside a double-quoted `${...}` as literal in POSIX
/// mode, so the first `}` closes there.
pub(in crate::executor) fn matching_parameter_brace_in_context(
    input: &str,
    outer_double_quote: bool,
    posix: bool,
) -> Option<usize> {
    let replacement_context = input.find('/').is_some_and(|slash| {
        !input[..slash].contains(':')
            && input[..slash]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '#')
    });
    let context = BraceContext {
        outer_double_quote,
        posix,
        replacement_context,
        initial_state: DolbraceState::Param,
    };
    if let Some(scan) = scan_braced_parameter_body(input, context) {
        return scan.end.checked_sub(1);
    }
    let mut chars = input.char_indices().peekable();
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut saw_quote = false;
    // In `${var/pattern/replacement}`, quotes in the pattern and replacement
    // are part of the parameter operation. They must not hide the operation's
    // closing brace from the outer `${...}` scanner. A colon before the slash
    // identifies other forms such as `${var:-word/with/slashes}`.
    let replacement_context = input.find('/').is_some_and(|slash| {
        !input[..slash].contains(':')
            && input[..slash]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '#')
    });
    while let Some((index, ch)) = chars.next() {
        // GNU Bash treats `\` plus the next character as one unit while
        // scanning `${...}` (extract_dollar_brace_string advances by two).
        // `\\` is a literal backslash, so the following `}` still closes.
        if ch == '\\' {
            // A backslash quotes the following character while Bash scans a
            // braced parameter. This includes quote characters in a
            // replacement word; they must not leave the scanner in a false
            // single/double-quote state and hide the closing brace.
            chars.next();
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            saw_quote = true;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            saw_quote = true;
            continue;
        }
        if ch == '[' && !single && !double {
            continue;
        }
        if ch == ']' && !single && !double {
            continue;
        }
        if ch == '$' && chars.peek().is_some_and(|(_, ch)| *ch == '{') {
            chars.next();
            depth += 1;
            continue;
        }
        if ch == '}' && (replacement_context || (!single && (!double || depth > 0 || !saw_quote))) {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    if depth == 0 && (single || double) {
        return input.rfind('}');
    }
    None
}

pub(in crate::executor) fn braced_parameter_spans_whole_word(word: &str) -> bool {
    braced_parameter_spans_whole_word_in_context(word, false, false)
}

pub(in crate::executor) fn braced_parameter_spans_whole_word_in_context(
    word: &str,
    outer_double_quote: bool,
    posix: bool,
) -> bool {
    let Some(rest) = word.strip_prefix("${") else {
        return false;
    };
    matching_parameter_brace_in_context(rest, outer_double_quote, posix)
        .is_some_and(|index| index + 1 == rest.len())
}

/// When `word` is exactly one `${...}` expansion, return its body — the text
/// between the opening `${` and its matching `}`. GNU subst.c param_expand
/// expands one `${}` per word position; `"${a}x${b}"` is two expansions plus
/// literal text, so the naive `strip_prefix("${") + strip_suffix('}')`
/// extraction — which pairs the first `${` with the LAST `}` — glues the
/// middle into the body and feeds garbage to the operator/substring parsers
/// (iquote.sub: `"${del:0:1}${a#d}"` evaluated `1}${a#d` as arithmetic).
pub(in crate::executor) fn whole_word_braced_parameter_body(word: &str) -> Option<&str> {
    let rest = word.strip_prefix("${")?;
    let close = matching_parameter_brace(rest)?;
    (close + 1 == rest.len()).then_some(&rest[..close])
}

/// Whether a parameter default/alternate word contains a backslash-escaped
/// IFS whitespace character. In an unquoted word such an escape keeps the
/// whitespace literal and suppresses field splitting (parse.y parameter
/// scanner), which the String-based operator path cannot express because it
/// unescapes the whitespace to a real separator before field splitting runs.
pub(in crate::executor) fn parameter_word_has_escaped_whitespace(word: &str) -> bool {
    word.as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && matches!(pair[1], b' ' | b'\t' | b'\n'))
}

pub(in crate::executor) fn command_substitution_spans_whole_word(word: &str) -> bool {
    let Some(rest) = word.strip_prefix("$(") else {
        return false;
    };

    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut index = 0usize;
    while index < chars.len() {
        let (offset, ch) = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        // Inside double quotes, `$(...)` is a nested command substitution
        // (GNU parse.y `xparse_dolparen` / subst.c
        // `extract_command_substitution`): a `"` inside the nested `$(...)`
        // does NOT close the outer double quote.  Skip the nested `$(...)`
        // as a unit *before* the `"` toggle below so the outer `double`
        // state is preserved.
        if double
            && ch == '$'
            && chars.get(index + 1).is_some_and(|(_, next)| *next == '(')
            && chars.get(index + 2).is_none_or(|(_, next)| *next != '(')
        {
            if let Some(next_index) = skip_nested_dollar_paren_whole_word(&chars, index) {
                index = next_index;
                continue;
            }
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return offset + ch.len_utf8() == rest.len();
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

/// Skip a nested `$(...)` command substitution that appears inside double
/// quotes, returning the character index just past the matching `)`.
/// The nested `$(...)` has its own independent quote state so a `"` inside
/// it does not affect the caller's `double` flag.  Mirrors GNU
/// `xparse_dolparen` (parse.y) and `extract_command_substitution` (subst.c).
fn skip_nested_dollar_paren_whole_word(chars: &[(usize, char)], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut cursor = start + 2; // skip `$(``
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while cursor < chars.len() {
        let (_, ch) = chars[cursor];
        if escaped {
            escaped = false;
            cursor += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            cursor += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            cursor += 1;
            continue;
        }
        // Recursively skip nested `$(...)` inside double quotes.
        if double
            && ch == '$'
            && chars.get(cursor + 1).is_some_and(|(_, next)| *next == '(')
            && chars.get(cursor + 2).is_none_or(|(_, next)| *next != '(')
        {
            if let Some(next_cursor) = skip_nested_dollar_paren_whole_word(chars, cursor) {
                cursor = next_cursor;
                continue;
            }
        }
        if ch == '"' && !single {
            double = !double;
            cursor += 1;
            continue;
        }
        if !single
            && !double
            && ch == '$'
            && chars.get(cursor + 1).is_some_and(|(_, next)| *next == '(')
            && chars.get(cursor + 2).is_none_or(|(_, next)| *next != '(')
        {
            depth += 1;
            cursor += 2;
            continue;
        }
        if !single && !double && ch == '(' {
            depth += 1;
        }
        if !single && !double && ch == ')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(cursor + 1);
            }
        }
        cursor += 1;
    }
    None
}

pub(in crate::executor) fn backtick_substitution_spans_whole_word(word: &str) -> bool {
    let Some(rest) = word.strip_prefix('`') else {
        return false;
    };

    let mut escaped = false;
    for (index, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '`' {
            return index + ch.len_utf8() == rest.len();
        }
    }
    false
}

pub(in crate::executor) fn is_parameter_error_name(name: &str) -> bool {
    is_shell_name(name)
        || name
            .strip_prefix('!')
            .is_some_and(|name| !name.is_empty() && is_shell_name(name))
        || matches!(name, "#" | "@" | "*" | "?" | "$" | "-" | "0")
        || name.parse::<usize>().is_ok()
        || parse_array_subscript(name).is_some()
}

pub(in crate::executor) fn has_indirect_parameter_word_operator(name: &str) -> bool {
    let Some(indirect) = name.strip_prefix('!') else {
        return false;
    };
    [":-", ":=", ":?", ":+", "-", "=", "?", "+"]
        .iter()
        .any(|operator| {
            indirect
                .split_once(operator)
                .is_some_and(|(left, _)| !left.is_empty())
        })
}

pub(in crate::executor) fn parameter_substring(
    value: &str,
    offset: isize,
    length: Option<isize>,
) -> String {
    if !crate::locale::is_multi_byte() {
        return parameter_substring_bytes(value, offset, length);
    }
    let char_count = value.chars().count();
    let Some(start) = parameter_substring_start(char_count, offset) else {
        return String::new();
    };
    let take = match length {
        Some(length) if length < 0 => {
            let remaining = char_count.saturating_sub(start);
            remaining.saturating_sub(length.unsigned_abs())
        }
        Some(length) => usize::try_from(length).unwrap_or(usize::MAX),
        None => usize::MAX,
    };

    value.chars().skip(start).take(take).collect()
}

/// Byte-indexed form of GNU subst.c substring expansion: with MB_CUR_MAX of 1
/// the offset and length are raw byte counts, so `${V:0:2}` takes two bytes
/// and can cut a multibyte sequence in half (intl4.sub under LC_CTYPE=C).
fn parameter_substring_bytes(value: &str, offset: isize, length: Option<isize>) -> String {
    let byte_count = locale_byte_span(value).len();
    let Some(start) = parameter_substring_start(byte_count, offset) else {
        return String::new();
    };
    let take = match length {
        Some(length) if length < 0 => {
            let remaining = byte_count.saturating_sub(start);
            remaining.saturating_sub(length.unsigned_abs())
        }
        Some(length) => usize::try_from(length).unwrap_or(usize::MAX),
        None => usize::MAX,
    };
    let raw = locale_byte_span(value);
    let end = byte_count.min(start.saturating_add(take));
    crate::executor::substitution_metadata::bytes_to_shell_text(&raw[start..end])
}

/// The byte view of a word: raw-byte marker pairs (substitution_metadata)
/// decode back to the bytes they carry, everything else keeps its UTF-8 bytes.
fn locale_byte_span(value: &str) -> Vec<u8> {
    let sentinel = char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
        .expect("raw-byte sentinel is a valid char");
    if value.contains(sentinel) {
        crate::executor::substitution_metadata::decode_raw_byte_markers(value.as_bytes())
    } else {
        value.as_bytes().to_vec()
    }
}

pub(in crate::executor) fn parameter_substring_start(
    char_count: usize,
    offset: isize,
) -> Option<usize> {
    if offset < 0 {
        char_count.checked_sub(offset.unsigned_abs())
    } else {
        usize::try_from(offset)
            .ok()
            .filter(|start| *start <= char_count)
    }
}

pub(in crate::executor) fn parameter_substring_has_negative_result(
    char_count: usize,
    offset: isize,
    length: isize,
) -> bool {
    if length >= 0 {
        return false;
    }
    let Some(start) = parameter_substring_start(char_count, offset) else {
        return false;
    };
    (char_count as i128) - (start as i128) + (length as i128) < 0
}

pub(in crate::executor) fn positional_parameter_substring(
    params: &[String],
    offset: isize,
    length: Option<isize>,
) -> Vec<String> {
    let start = if offset < 0 {
        params
            .len()
            .checked_sub(offset.unsigned_abs())
            .unwrap_or(params.len())
    } else {
        (offset as usize).saturating_sub(1)
    };
    let take = match length {
        Some(length) if length < 0 => params
            .len()
            .saturating_sub(start)
            .saturating_sub(length.unsigned_abs()),
        Some(length) => usize::try_from(length).unwrap_or(usize::MAX),
        None => usize::MAX,
    };

    params.iter().skip(start).take(take).cloned().collect()
}

/// GNU subst.c:3759-3763 pos_params: `${@:0}`/`${*:0}` prepend $0
/// (dollar_vars[0]) to the positional list before slicing.
pub(in crate::executor) fn positional_parameter_substring_with_zero(
    params: &[String],
    zero: &str,
    offset: isize,
    length: Option<isize>,
) -> Vec<String> {
    if offset != 0 {
        return positional_parameter_substring(params, offset, length);
    }
    let mut with_zero = Vec::with_capacity(params.len() + 1);
    with_zero.push(zero.to_string());
    with_zero.extend(params.iter().cloned());
    positional_parameter_substring(&with_zero, 1, length)
}

pub(in crate::executor) fn parse_parameter_replacement(
    name: &str,
) -> Option<(&str, &str, &str, bool)> {
    if let Some((var_name, rest)) = name.split_once("//").filter(|(var_name, _)| {
        !var_name.ends_with('\\') && !var_name.ends_with(crate::executor::markers::DATA_BACKSLASH)
    }) {
        // A slash immediately after `//` is part of the pattern. This is
        // ambiguous with the pattern/replacement separator, so skip it and
        // find the next unescaped slash (`${v////-}`, `${v///r/-}`).
        let separator = if rest.starts_with('/') {
            split_unescaped_parameter_separator(&rest[1..])
                .map(|(pattern, replacement)| (&rest[..pattern.len() + 1], replacement))
        } else {
            split_unescaped_parameter_separator(rest)
        };
        let (pattern, replacement) = separator.unwrap_or((rest, ""));
        return Some((var_name, pattern, replacement, true));
    }

    let (var_name, rest) = name.split_once('/')?;
    let (pattern, replacement) = split_unescaped_parameter_separator(rest).unwrap_or((rest, ""));
    Some((var_name, pattern, replacement, false))
}

fn split_unescaped_parameter_separator(value: &str) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' || ch == crate::executor::markers::DATA_BACKSLASH {
            escaped = true;
            continue;
        }
        if ch == '/' {
            return Some((&value[..index], &value[index + 1..]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_parameter_brace_skips_escaped_closing_brace() {
        let input = "foo:-string \\\\\\}}";

        assert_eq!(matching_parameter_brace(input), Some(input.len() - 1));
        assert!(braced_parameter_spans_whole_word("${foo:-string \\\\\\}}"));
    }

    #[test]
    fn matching_parameter_brace_closes_after_even_backslashes() {
        let input = "foo:-string \\\\}}";

        assert_eq!(matching_parameter_brace(input), Some(input.len() - 2));
    }

    #[test]
    fn matching_parameter_brace_closes_at_first_brace_in_bracket_pattern() {
        // GNU parse.y P_FIRSTCLOSE: the first unquoted '}' closes, even
        // inside a bracket pattern.  The remaining `]}` is literal text.
        assert_eq!(matching_parameter_brace("o%[}]}"), Some(3));
    }

    #[test]
    fn matching_parameter_brace_accepts_nested_array_subscript() {
        assert_eq!(matching_parameter_brace("A[${i}]}"), Some(7));
    }

    #[test]
    fn encoded_backslash_does_not_split_escaped_slash_pattern() {
        assert_eq!(
            parse_parameter_replacement(&format!(
                "{}{}{}",
                "v/b",
                crate::executor::markers::DATA_BACKSLASH_STR,
                "//x"
            )),
            Some((
                "v",
                format!("{}{}/", "b", crate::executor::markers::DATA_BACKSLASH_STR).as_str(),
                "x",
                false
            ))
        );
    }

    #[test]
    fn global_replacement_accepts_slash_at_pattern_start() {
        assert_eq!(
            parse_parameter_replacement("v////-"),
            Some(("v", "/", "-", true))
        );
        assert_eq!(
            parse_parameter_replacement("v///r/-"),
            Some(("v", "/r", "-", true))
        );
    }

    #[test]
    fn substring_byte_mode_indexes_in_bytes() {
        assert_eq!(parameter_substring_bytes("abcdef", 1, Some(3)), "bcd");
        assert_eq!(parameter_substring_bytes("abcdef", -2, None), "ef");
        assert_eq!(parameter_substring_bytes("abcdef", 0, Some(-2)), "abcd");
        assert_eq!(parameter_substring_bytes("abcdef", 6, None), "");
        assert_eq!(parameter_substring_bytes("abcdef", 9, None), "");
    }

    #[test]
    fn substring_byte_mode_cuts_mid_sequence() {
        // A 3-byte character sequence: cutting two bytes must land mid-way
        // through the first character rather than back off to a boundary.
        let value = "ಇಳಿಕೆ";
        let byte_total = value.as_bytes().len();
        let cut = parameter_substring_bytes(value, 0, Some(2));
        assert_eq!(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&cut),
            value.as_bytes()[..2].to_vec(),
        );
        let whole = parameter_substring_bytes(value, 0, Some(byte_total as isize));
        assert_eq!(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&whole),
            value.as_bytes().to_vec(),
        );
        let past_end = parameter_substring_bytes(value, 1, Some(byte_total as isize));
        assert_eq!(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&past_end),
            value.as_bytes()[1..].to_vec(),
        );
    }
}
