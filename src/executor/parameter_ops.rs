use super::*;
use crate::lexer::dolbrace::{scan_braced_parameter_body, BraceContext, DolbraceState};

/// Quote removal for the rhs of a ${parameter<op>word} expansion, run on the
/// dequoted word text: an escaped glob metacharacter becomes the CTLESC
/// port (`\x11` + char), mirroring GNU subst.c:11671-11674 (the backslash
/// branch of expand_word_internal in unquoted context adds the escaped
/// character as CTLESC + c — a quoted literal). The CTLESC carrier keeps
/// the character out of pathname expansion (glob.rs unquoted_glob_pattern_p
/// skips CTLESC-protected characters, the pathexp.c port) and is dropped
/// before argv by the same dequote every other CTLESC producer uses. For
/// `\[`/`\@`/`\+` the backslash is now also consumed, which matches GNU's
/// value (`${x:+a\[b}` is `a[b`, not `a\[b`); every other escape keeps its
/// existing rubash treatment byte-for-byte.
pub(in crate::executor) fn mark_escaped_glob_metachars(word: &str) -> String {
    let mut output = String::with_capacity(word.len());
    let mut chars = word.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && matches!(chars.peek(), Some('*' | '?' | '[' | ']' | '@' | '+' | '!')) {
            // The generic unescape would drop this backslash (or, for the
            // bracket/extglob starters, leave it as a data pair); replace
            // the pair with the protected form so the char stays quoted.
            output.push(crate::executor::markers::CTLESC);
            output.push(chars.next().expect("peeked a metachar"));
            continue;
        }
        output.push(ch);
    }
    output
}

/// GNU subst.c:4462 expand_string_for_rhs -> call_expand_word_internal with
/// quoted == 0: in an UNQUOTED ${parameter<op>word} rhs, quote removal
/// consumes the backslash before ANY character. The backslash branch of
/// expand_word_internal (subst.c:11671-11674) adds the escaped character —
/// CTLESC + c for the glob-metacharacter class (quoted status survives for
/// pathname expansion, pathexp.c), the plain character for everything else
/// — so `${x:+a\qb}` is `aqb` and `${x:-e\@f}` is `e@f` in assignment and
/// fragment positions exactly like in the whole-word argument position.
/// mark_escaped_glob_metachars runs first so `\*` keeps its CTLESC port;
/// the remaining `\X` pairs (ordinary X) then lose the backslash.
pub(in crate::executor) fn unescape_unquoted_rhs_escapes(word: &str) -> String {
    let mut output = String::with_capacity(word.len());
    let mut chars = word.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek().copied() {
                Some(next) => {
                    chars.next();
                    if matches!(next, '*' | '?' | '[' | ']' | '@' | '+' | '!') {
                        output.push(crate::executor::markers::CTLESC);
                    } else if matches!(next, ' ' | '\t' | '\n') {
                        // Escaped whitespace keeps its quoted status for
                        // field splitting — the same \x1c sentinel the
                        // alternate-rhs walker emits (posixexp2 37:
                        // ${x:-a\ b} stays one field in argument position).
                        output.push(crate::executor::markers::IFS_GLUE);
                    }
                    output.push(next);
                }
                None => output.push(ch),
            }
            continue;
        }
        output.push(ch);
    }
    output
}

pub(in crate::executor) fn decode_parameter_word_quotes(word: &str) -> String {
    // GNU posixexp2 37 (subst.c:4462 expand_string_for_rhs): whitespace that
    // was quoted inside the rhs keeps its quoted status through field
    // splitting, so the decoder emits the IFS_GLUE sentinel before every
    // whitespace character inside a quoted region. Without it
    // `a=(${v:-"a b"})` split into two elements where GNU stores ONE
    // (rubash#212; arrayfunc.c:610 expand_words_no_vars splits only
    // unquoted expansion whitespace).
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
                    if matches!(ch, ' ' | '\t' | '\n') {
                        output.push(crate::executor::markers::IFS_GLUE);
                    }
                    output.push(ch);
                }
            }
            '\'' => {
                if let Some(close_offset) = chars[index + 1..].iter().position(|ch| *ch == '\'') {
                    let close = index + 1 + close_offset;
                    for ch in &chars[index + 1..close] {
                        if matches!(ch, ' ' | '\t' | '\n') {
                            output.push(crate::executor::markers::IFS_GLUE);
                        }
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
    restore_protected_replacement_quotes_cow(value).into_owned()
}

/// Borrowing form: a value without the protected-escape sentinel is the
/// identity (the walker tail's marker-restore chain borrows through — GNU
/// dequote walks in place).
pub(in crate::executor) fn restore_protected_replacement_quotes_cow(
    value: &str,
) -> std::borrow::Cow<'_, str> {
    if !value.contains(crate::executor::markers::PROTECTED_ESCAPED_SQUOTE) {
        return std::borrow::Cow::Borrowed(value);
    }
    std::borrow::Cow::Owned(
        value.replace(crate::executor::markers::PROTECTED_ESCAPED_SQUOTE, "\\'"),
    )
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

/// When `word` is exactly one `$((...))` arithmetic expansion, return its
/// body — the text between the opening `$((` and ITS closing `))`. GNU
/// parse.y:5516 read_token_word hands every `$((` to parse.y:4451
/// parse_comsub, which (parse.y:4462-4470) delegates to parse.y:3877
/// parse_matched_pair(P_ARITH): the span closes at the FIRST `))` that
/// returns the paren depth — seeded by the `$(` plus the second `(` — to
/// zero, with quotes and backslash escapes opaque to the count
/// (parse.y:3999-4000 LEX_PASSNEXT, parse.y:4041-4046 push_delimiter).
/// After the span the word walk continues (parse.y:5521 `goto
/// next_character`), so a later `$((`/`$param` in the same word is an
/// independent expansion event. The naive `strip_prefix("$((") +
/// strip_suffix("))")` admission — which pairs the FIRST `$((` with the
/// LAST `))` of the word — glues the in-between material into one merged
/// expression (rubash#376: `"$((1+1)):$((2+2))"` evaluated `1+1)):4`
/// instead of printing `2:4`). This is the whole-word admission for the
/// `$((...))` fast paths: `Some(body)` only when the first balanced
/// closer is the word's final characters, `None` when it closes early
/// (the caller falls through to the embedded walker, which already
/// anchors each `$((` independently).
pub(in crate::executor) fn whole_word_arithmetic_substitution_body(word: &str) -> Option<&str> {
    let rest = word.strip_prefix("$((")?;
    let bytes = rest.as_bytes();
    // Same scan contract as the embedded walker's collector
    // (embedded_mutations.rs collect_dollar_paren_arithmetic_expansion):
    // depth starts at 2 — the `$(` plus the second `(`.
    let mut depth: usize = 2;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if single {
            single = ch != b'\'';
            index += 1;
            continue;
        }
        if double {
            if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            b'\\' => escaped = true,
            b'\'' => single = true,
            b'"' => double = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    // `bytes[index - 1]` is the first `)` of the `))` pair;
                    // the closer must be the word's final two characters.
                    return (index + 1 == bytes.len()).then_some(&rest[..index - 1]);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
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

/// GNU subst.c:9805-9880 (extract_variable_name -> op dispatch):
/// string_extract(SX_VARNAME) stops the parameter name at the first
/// operator character, and `/` is the substitution separator ONLY when it
/// is that terminating char. A `/` after another operator or inside a
/// quoted span is word text — `${v#"$x/"}` is prefix removal, not
/// `${v/pat/repl}`. The old split_once("/") claimed the quoted `/`, the
/// pattern `x` then carried a lone `"`, and the whole expansion died with
/// a phantom unclosed-quote diagnostic (rubash#282 residual).
pub(in crate::executor) fn parse_parameter_replacement(
    name: &str,
) -> Option<(&str, &str, &str, bool)> {
    let bytes = name.as_bytes();
    let mut head = super::expand_word::brace_name_head(bytes).len();
    if head == 0 {
        // The name begins with an operator char, so the extract stopped at
        // once. GNU then accepts the single-char special parameters:
        // `@` on its own (subst.c:9815-9821, it is also a transform op)
        // and `-`, `?`, `#` with any further name text up to the next
        // terminator (subst.c:9837-9862 VALID_SPECIAL_LENGTH_PARAM).
        head = match bytes.first() {
            Some(b'@') => 1,
            Some(b'-' | b'?' | b'#') => 1 + parameter_name_tail_len(&bytes[1..]),
            _ => 0,
        };
    }
    if bytes.get(head) != Some(&b'/') {
        return None;
    }
    let var_name = &name[..head];
    // subst.c:9387-9390 MATCH_GLOBREP: a second `/` selects global
    // replacement and is consumed before the pattern.
    let global = bytes.get(head + 1) == Some(&b'/');
    let rest = &name[head + if global { 2 } else { 1 }..];
    // subst.c:9408-9409: when the pattern itself starts with `/`, that
    // char is skipped before the separator scan (`${v///r/-}`,
    // `${v////-}`).
    if let Some(pattern_rest) = rest.strip_prefix('/') {
        return match split_parameter_separator(pattern_rest) {
            Some((inner, replacement)) => {
                Some((var_name, &rest[..inner.len() + 1], replacement, global))
            }
            None => Some((var_name, rest, "", global)),
        };
    }
    let (pattern, replacement) = split_parameter_separator(rest).unwrap_or((rest, ""));
    Some((var_name, pattern, replacement, global))
}

/// subst.c:9844-9860 string_extract(string, &t_index, "#%:-=?+/@}", 0):
/// after a single-char special parameter, the name continues up to the
/// next operator terminator with no escape or subscript handling.
fn parameter_name_tail_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|b| {
            !matches!(
                b,
                b'#' | b'%' | b':' | b'-' | b'=' | b'?' | b'+' | b'/' | b'@' | b'}'
            )
        })
        .count()
}

/// Port of GNU subst.c:2199 skip_to_delim (flags=0, as called from
/// parameter_brace_patsub at subst.c:9408): the pattern/replacement `/`
/// is only a delimiter at top level — backslash pairs, backquotes,
/// single/double quotes, `$(...)`, `${...}` and process substitutions are
/// skipped as units, so a `/` inside any of them stays word text.
fn split_parameter_separator(value: &str) -> Option<(&str, &str)> {
    let chars: Vec<(usize, char)> = value.char_indices().collect();
    let mut index = 0;
    while index < chars.len() {
        let (byte, ch) = chars[index];
        match ch {
            '/' => return Some((&value[..byte], &value[byte + '/'.len_utf8()..])),
            '\\' | crate::executor::markers::DATA_BACKSLASH => index += 2,
            '`' => index = skip_backquote_span(&chars, index),
            '\'' | crate::executor::markers::DATA_SQUOTE => {
                index = skip_literal_span(&chars, index, ch)
            }
            '"' | crate::executor::markers::DATA_DQUOTE => {
                index = skip_double_quoted_span(&chars, index, ch)
            }
            '$' if matches!(chars.get(index + 1).map(|c| c.1), Some('(' | '{')) => {
                let close = if chars[index + 1].1 == '(' { ')' } else { '}' };
                index = skip_matched_span(&chars, index + 1, close);
            }
            // `$'...'` / `$"..."`: GNU's reader (parse.y:5305+
            // read_token_word) has already turned the ANSI-C/locale-quoted
            // span into CTLESC-carried word text before subst.c:2199
            // skip_to_delim / subst.c:9408 ever scan for the `/`
            // separator, so a `/` inside the span is never a separator and
            // the span's quotes never open skip_single_quoted. Rubash
            // hands the RAW span through, so skip it as a unit here —
            // ANSI-C quoting processes backslash escapes (`$'\''` is one
            // quoted `'`), which is the escape-aware double-quote-style
            // scan, not the plain literal one (nquote2.sub
            // `t "${v/$'\''/x}"`: without this the span's second `'`
            // opened a phantom literal span that swallowed the separator
            // and the replacement never ran).
            '$' if matches!(chars.get(index + 1).map(|c| c.1), Some('\'')) => {
                index = skip_double_quoted_span(&chars, index + 1, '\'');
            }
            '$' if matches!(chars.get(index + 1).map(|c| c.1), Some('"')) => {
                index = skip_double_quoted_span(&chars, index + 1, '"');
            }
            '<' | '>' if chars.get(index + 1).map(|c| c.1) == Some('(') => {
                index = skip_matched_span(&chars, index + 1, ')')
            }
            _ => index += 1,
        }
    }
    None
}

/// Index just past a `` `...` `` span opened at `index` (chars index),
/// or chars.len() when unclosed.
fn skip_backquote_span(chars: &[(usize, char)], index: usize) -> usize {
    let mut i = index + 1;
    while i < chars.len() {
        match chars[i].1 {
            '`' => return i + 1,
            '\\' | crate::executor::markers::DATA_BACKSLASH => i += 2,
            _ => i += 1,
        }
    }
    i
}

/// Index just past a `q...q` span (single quote or its data marker)
/// opened at `index`.
fn skip_literal_span(chars: &[(usize, char)], index: usize, quote: char) -> usize {
    let mut i = index + 1;
    while i < chars.len() {
        if chars[i].1 == quote {
            return i + 1;
        }
        i += 1;
    }
    i
}

/// Index just past a double-quoted span opened at `index`; `\\X` pairs
/// inside are skipped (subst.c skip_double_quoted).
fn skip_double_quoted_span(chars: &[(usize, char)], index: usize, quote: char) -> usize {
    let mut i = index + 1;
    while i < chars.len() {
        match chars[i].1 {
            c if c == quote => return i + 1,
            '\\' | crate::executor::markers::DATA_BACKSLASH => i += 2,
            _ => i += 1,
        }
    }
    i
}

/// Index just past the `close` matching `chars[index]` (an open char),
/// nesting counted and quotes/escapes/backquotes/`$(`${}` skipped as
/// units — subst.c:2086 skip_matched_pair flags=0.
fn skip_matched_span(chars: &[(usize, char)], index: usize, close: char) -> usize {
    let open = chars[index].1;
    let mut i = index + 1;
    let mut depth = 1usize;
    while i < chars.len() {
        let ch = chars[i].1;
        if ch == close {
            depth -= 1;
            i += 1;
            if depth == 0 {
                return i;
            }
        } else if ch == open {
            depth += 1;
            i += 1;
        } else {
            match ch {
                '\\' | crate::executor::markers::DATA_BACKSLASH => i += 2,
                '`' => i = skip_backquote_span(chars, i),
                '\'' | crate::executor::markers::DATA_SQUOTE => i = skip_literal_span(chars, i, ch),
                '"' | crate::executor::markers::DATA_DQUOTE => {
                    i = skip_double_quoted_span(chars, i, ch)
                }
                '$' if matches!(chars.get(i + 1).map(|c| c.1), Some('(' | '{')) => {
                    let inner = if chars[i + 1].1 == '(' { ')' } else { '}' };
                    i = skip_matched_span(chars, i + 1, inner);
                }
                // `$'...'` / `$"..."` — same parse.y:5305+ pre-decoding
                // argument as split_parameter_separator: the span's quotes
                // must not open skip_literal_span here (the closing `}`
                // of the enclosing `${...}` inside such a span would be
                // mis-located otherwise).
                '$' if matches!(chars.get(i + 1).map(|c| c.1), Some('\'')) => {
                    i = skip_double_quoted_span(chars, i + 1, '\'');
                }
                '$' if matches!(chars.get(i + 1).map(|c| c.1), Some('"')) => {
                    i = skip_double_quoted_span(chars, i + 1, '"');
                }
                _ => i += 1,
            }
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_word_arithmetic_body_admits_single_substitution() {
        // parse.y:3877 parse_matched_pair: closer is the first `))` that
        // zeroes the depth seeded by `$(` + the second `(`.
        assert_eq!(
            whole_word_arithmetic_substitution_body("$((1+1))"),
            Some("1+1")
        );
        assert_eq!(
            whole_word_arithmetic_substitution_body("$(( (1+2) ))"),
            Some(" (1+2) ")
        );
        assert_eq!(whole_word_arithmetic_substitution_body("1+1))"), None);
        assert_eq!(whole_word_arithmetic_substitution_body("$((1+1"), None);
    }

    #[test]
    fn whole_word_arithmetic_body_rejects_early_closing_rubash376() {
        // rubash#376: the first `$((` closes at its own `))`; the trailing
        // material (separator + later `$((`/literal) is ordinary word text,
        // never part of the expression.
        assert_eq!(
            whole_word_arithmetic_substitution_body("$((1+1)):$((2+2))"),
            None
        );
        assert_eq!(
            whole_word_arithmetic_substitution_body("$((1+1)) $((2+2))"),
            None
        );
        assert_eq!(whole_word_arithmetic_substitution_body("$((1+1))x"), None);
        assert_eq!(
            whole_word_arithmetic_substitution_body("$((v)):$PATH"),
            None
        );
        // Balanced `))` INSIDE the body keeps the span open (depth counting).
        assert_eq!(
            whole_word_arithmetic_substitution_body("$(( (1+2) )):$((3+4))"),
            None
        );
    }

    #[test]
    fn whole_word_arithmetic_body_quotes_and_escapes_are_opaque() {
        // parse.y:3999-4000 LEX_PASSNEXT, parse.y:4041-4046 push_delimiter:
        // quotes and backslash escapes keep their `)`s out of the count.
        assert_eq!(
            whole_word_arithmetic_substitution_body(r#"$((1")"+1))"#),
            Some(r#"1")"+1"#)
        );
        assert_eq!(
            whole_word_arithmetic_substitution_body(r"$((1\)+1))"),
            Some(r"1\)+1")
        );
        // A quoted `))` does not close; the real closer at the end does.
        assert_eq!(
            whole_word_arithmetic_substitution_body(r#"$(( '))' + 1 ))"#),
            Some(r#" '))' + 1 "#)
        );
    }

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
    fn slash_after_another_operator_is_not_substitution() {
        // subst.c:9805-9880: `/` is the substitution separator only when it
        // terminates the name extract; `${v#"$x/"}` is prefix removal.
        assert_eq!(parse_parameter_replacement("y#\"$Z/\""), None);
        assert_eq!(parse_parameter_replacement("v%a/b"), None);
        assert_eq!(parse_parameter_replacement("v:1:2"), None);
        assert_eq!(parse_parameter_replacement("v=x/y"), None);
    }

    #[test]
    fn separator_skips_quoted_and_nested_slashes() {
        // subst.c:2199 skip_to_delim + subst.c:9408: quotes, backquotes,
        // $(...)/${...} and process substitutions are skipped as units.
        assert_eq!(
            parse_parameter_replacement("v/a'/'c/X"),
            Some(("v", "a'/'c", "X", false))
        );
        assert_eq!(
            parse_parameter_replacement("v/a\"/\"b/Z"),
            Some(("v", "a\"/\"b", "Z", false))
        );
        assert_eq!(
            parse_parameter_replacement("w/$(x/y)/r"),
            Some(("w", "$(x/y)", "r", false))
        );
        assert_eq!(
            parse_parameter_replacement("w/`x/y`/r"),
            Some(("w", "`x/y`", "r", false))
        );
        assert_eq!(
            parse_parameter_replacement("w/${n/y}/r"),
            Some(("w", "${n/y}", "r", false))
        );
    }

    #[test]
    fn single_char_special_parameter_names() {
        // subst.c:9815-9821 / 9837-9862: `@` alone, and `-`/`?`/`#` plus
        // name text up to the next terminator, are valid patsub names.
        assert_eq!(
            parse_parameter_replacement("@/a/b"),
            Some(("@", "a", "b", false))
        );
        assert_eq!(
            parse_parameter_replacement("#/x/y"),
            Some(("#", "x", "y", false))
        );
        assert_eq!(
            parse_parameter_replacement("?/x/y"),
            Some(("?", "x", "y", false))
        );
        assert_eq!(
            parse_parameter_replacement("-/x/y"),
            Some(("-", "x", "y", false))
        );
        assert_eq!(
            parse_parameter_replacement("1/a/b"),
            Some(("1", "a", "b", false))
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
