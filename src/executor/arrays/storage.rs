use std::collections::{BTreeMap, HashMap};

use crate::executor::markers::STORAGE_WORD_PREFIX_STR;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};
use crate::executor::{mark_env_name, split_storage_words, unquote_storage_value, ARRAY_VARS};

pub(in crate::executor) fn normalize_array_expanded_value(value: String) -> String {
    // GNU array.c: array element values are stored and retrieved verbatim.
    // A previous version special-cased values containing only backslashes
    // and double quotes (e.g. a single `"`), converting them to `""`.
    // This corrupted values like [0x0022]=\" which stores a literal `"`
    // (unicode1.sub C_UTF_8 array). The roundtrip must be identity.
    value
}

pub(in crate::executor) fn array_values(value: &str) -> Vec<String> {
    // TODO(array.c/assoc.c/subst.c): This is a lossy representation used while
    // arrays are still stored in the scalar variable table.
    if let Some(rendered) = value.strip_prefix(STORAGE_WORD_PREFIX) {
        return rendered_array_values(rendered);
    }

    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![value.to_string()]
        };
    };

    if inner.is_empty() {
        return Vec::new();
    }

    split_storage_words(inner)
        .map(|part| {
            let value = part
                .split_once('=')
                .map(|(_, value)| value)
                .map(unquote_storage_value)
                .unwrap_or_else(|| unquote_storage_value(&part));
            normalize_array_expanded_value(value)
        })
        .collect()
}

pub(in crate::executor) fn indexed_array_entries(value: &str) -> BTreeMap<usize, String> {
    if let Some(rendered) = value.strip_prefix(STORAGE_WORD_PREFIX) {
        return rendered_array_entries(rendered);
    }

    array_values(value).into_iter().enumerate().collect()
}

pub(in crate::executor) fn array_indices(value: &str) -> Vec<String> {
    indexed_array_entries(value)
        .keys()
        .map(usize::to_string)
        .collect()
}

pub(in crate::executor) fn array_value_at(value: &str, index: usize) -> Option<String> {
    let mut entries = indexed_array_entries(value);
    entries.remove(&index).map(normalize_array_expanded_value)
}

pub(in crate::executor) fn resolve_indexed_array_subscript(
    value: &str,
    index: i128,
) -> Option<usize> {
    if index >= 0 {
        return usize::try_from(index).ok();
    }

    let max_index = indexed_array_entries(value).keys().next_back().copied()?;
    let resolved = i128::try_from(max_index)
        .ok()?
        .checked_add(1)?
        .checked_add(index)?;
    usize::try_from(resolved).ok()
}

pub(in crate::executor) fn parse_array_integer_subscript(name: &str) -> Option<(&str, i128)> {
    let (array_name, subscript) = parse_array_subscript(name)?;
    let index = subscript.parse::<i128>().ok()?;
    Some((array_name, index))
}

pub(in crate::executor) fn parse_array_numeric_subscript(name: &str) -> Option<(&str, usize)> {
    let (array_name, subscript) = parse_array_subscript(name)?;
    let index = subscript.parse::<usize>().ok()?;
    Some((array_name, index))
}

pub(in crate::executor) fn parse_array_subscript(name: &str) -> Option<(&str, &str)> {
    let (array_name, subscript) = name.split_once('[')?;
    Some((array_name, subscript.strip_suffix(']')?))
}

pub(in crate::executor) fn format_indexed_array_storage(
    entries: BTreeMap<usize, String>,
) -> String {
    let rendered = entries
        .into_iter()
        .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{STORAGE_WORD_PREFIX_STR}({rendered})")
}

pub(in crate::executor) fn format_indexed_array_values(values: Vec<String>) -> String {
    let rendered = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| format!("[{index}]={}", quote_array_value(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{STORAGE_WORD_PREFIX_STR}({rendered})")
}

pub(in crate::executor) fn store_indexed_array(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    values: Vec<String>,
) {
    env_vars.insert(name.to_string(), format_indexed_array_values(values));
    mark_env_name(env_vars, ARRAY_VARS, name);
}

pub(in crate::executor) fn quote_array_value(value: &str) -> String {
    // The element value reaching storage may still carry the lexer's
    // data-quote markers for $'...'-decoded quotes (U+E010/U+E011, the
    // CTLESC analog for the protection GNU gives decoded bytes via
    // sh_single_quote at parse.y:5566-5575). The markers exist so
    // intermediate quote removal can tell data quotes from syntax quotes;
    // the array cell holds the real character, exactly like GNU's
    // assign_compound_array_list stores the dequoted word bytes
    // (arrayfunc.c:700+). Restoring here — at the storage serialization
    // boundary — keeps the rendered \x1d(...) form printable verbatim by
    // declare -p (a bare marker inside "..." printed raw) while
    // decode_rendered_array_value still hands readers the real quote
    // (issue #109: `x=($'a"b')` then `declare -p x` printed the E011
    // carrier bytes instead of "a\"b").
    let value = value
        .replace(crate::lexer::ANSI_C_QUOTE_MARKER_STR, "'")
        .replace(crate::lexer::ANSI_C_DQUOTE_MARKER_STR, "\"");
    let value = value.as_str();
    // GNU array.c array_to_assign (947-989) / array_to_kvpair (895-945):
    // every element value goes through ansic_quote when it holds a
    // non-printing character (strtrans.c ansic_shouldquote, 341-361) and
    // through sh_double_quote otherwise. Printable non-ASCII stays in the
    // double-quoted form (ansic_wshouldquote passes it).
    if ansic_shouldquote(value) {
        return ansic_quote(value);
    }

    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('\u{60}', "\\`")
    )
}

/// shquote.c sh_contains_shell_metas (376-407): returns 1 if the string
/// contains shell metacharacters that force quoting of a bare word.
pub(in crate::executor) fn sh_contains_shell_metas(value: &str) -> bool {
    let chars: Vec<char> = value.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        match ch {
            ' ' | '\t' | '\n' | '\'' | '"' | '\\' | '|' | '&' | ';' | '(' | ')' | '<' | '>'
            | '!' | '{' | '}' | '*' | '[' | '?' | ']' | '^' | '$' | '`' => return true,
            '~' => {
                if index == 0 || chars[index - 1] == '=' || chars[index - 1] == ':' {
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

/// assoc.c assoc_to_assign key rule (429-437): `$'...'` for keys holding
/// non-printing characters (ansic_shouldquote), `sh_double_quote` for keys
/// with shell metas, `sh_double_quote` for a bare `*` or `@` key
/// (ALL_ELEMENT_SUB), otherwise the bare key.
pub(in crate::executor) fn quote_assoc_display_key(key: &str) -> String {
    if ansic_shouldquote(key) {
        return ansic_quote(key);
    }
    if sh_contains_shell_metas(key) {
        return format!(
            "\"{}\"",
            key.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('\u{60}', "\\`")
        );
    }
    if key.len() == 1 && matches!(key, "*" | "@") {
        return format!("\"{key}\"");
    }
    key.to_string()
}

/// strtrans.c ansic_shouldquote (341-361): dollar-single-quote quoting is
/// value holds a non-printing byte. High-bit bytes follow the UTF-8-locale
/// multibyte path (351-354): a printable decoded character is fine, an
/// undecodable or non-printing one forces quoting.
pub(crate) fn ansic_shouldquote(value: &str) -> bool {
    // strtrans.c ansic_shouldquote (341-361) tests the raw byte stream;
    // raw-byte markers (U+E000 series) must decode first or a stored
    // non-printing byte reads as a printable PUA wide char.
    let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(value);
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte < 0x80 {
            if !is_print_byte(byte) {
                return true;
            }
            index += 1;
            continue;
        }
        match decode_utf8_char(&bytes[index..]) {
            Some(ch) if is_printable_wide(ch) => index += ch.len_utf8(),
            _ => return true,
        }
    }
    false
}

/// strtrans.c ansic_quote (230-308): the $'...' form with the named C
/// escapes, backslash and single-quote escaped, printable bytes (and whole
/// printable UTF-8 characters under a UTF-8 locale, 266-282) literal, and
/// every other byte as a three-digit octal escape (291-294).
pub(crate) fn ansic_quote(value: &str) -> String {
    // strtrans.c ansic_quote (230-308) walks the raw byte stream; decode
    // raw-byte markers first (same boundary as ansic_quote_with_markers in
    // execution_misc.rs) so stored non-printing bytes render as octal.
    let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(value);
    let mut out = String::with_capacity(4 * bytes.len() + 4);
    out.push_str("$'");
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            0x1b => {
                out.push_str("\\E");
                index += 1;
            }
            0x07 => {
                out.push_str("\\a");
                index += 1;
            }
            0x08 => {
                out.push_str("\\b");
                index += 1;
            }
            0x09 => {
                out.push_str("\\t");
                index += 1;
            }
            0x0a => {
                out.push_str("\\n");
                index += 1;
            }
            0x0b => {
                out.push_str("\\v");
                index += 1;
            }
            0x0c => {
                out.push_str("\\f");
                index += 1;
            }
            0x0d => {
                out.push_str("\\r");
                index += 1;
            }
            b'\\' => {
                out.push_str("\\\\");
                index += 1;
            }
            b'\'' => {
                out.push_str("\\'");
                index += 1;
            }
            0x20..=0x7e => {
                out.push(byte as char);
                index += 1;
            }
            _ if byte >= 0x80 => match decode_utf8_char(&bytes[index..]) {
                Some(ch) if is_printable_wide(ch) => {
                    out.push(ch);
                    index += ch.len_utf8();
                }
                _ => {
                    push_octal_escape(&mut out, byte);
                    index += 1;
                }
            },
            _ => {
                push_octal_escape(&mut out, byte);
                index += 1;
            }
        }
    }
    out.push('\'');
    out
}

fn push_octal_escape(out: &mut String, byte: u8) {
    out.push('\\');
    out.push((b'0' + ((byte >> 6) & 0o7)) as char);
    out.push((b'0' + ((byte >> 3) & 0o7)) as char);
    out.push((b'0' + (byte & 0o7)) as char);
}

/// strtrans.c ISPRINT for single-byte characters.
fn is_print_byte(byte: u8) -> bool {
    (0x20..=0x7e).contains(&byte)
}

/// strtrans.c iswprint under a UTF-8 locale: Unicode control characters
/// (C0, DEL, the C1 range) are non-printing, everything else prints.
fn is_printable_wide(ch: char) -> bool {
    !ch.is_control()
}

/// Decode one UTF-8 character at the slice start; None for an invalid or
/// truncated sequence (the mbrtowc MB_INVALIDCH/MB_NULLWCH cases).
fn decode_utf8_char(bytes: &[u8]) -> Option<char> {
    let len = match bytes.first()? {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    if bytes.len() < len {
        return None;
    }
    std::str::from_utf8(&bytes[..len])
        .ok()
        .and_then(|s| s.chars().next())
}

pub(in crate::executor) fn is_array_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')') || value.starts_with(STORAGE_WORD_PREFIX)
}

pub(in crate::executor) fn is_marked_array_var(
    env_vars: &HashMap<String, String>,
    name: &str,
) -> bool {
    const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
    const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
    [ARRAY_VARS, ASSOC_VARS].iter().any(|key| {
        env_vars
            .get(*key)
            .map(|value| value.split(DATA_DOLLAR).any(|marked| marked == name))
            .unwrap_or(false)
    })
}

fn rendered_array_values(value: &str) -> Vec<String> {
    rendered_array_entries(value).into_values().collect()
}

fn rendered_array_entries(value: &str) -> BTreeMap<usize, String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return BTreeMap::new();
    };

    rendered_array_parts(inner)
        .into_iter()
        .filter_map(|part| {
            let (key, value) = part.as_str().split_once('=')?;
            let index = key
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<usize>()
                .ok()?;
            Some((index, decode_rendered_array_value(value)))
        })
        .collect()
}

fn rendered_array_parts(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = inner.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(quote_ch) => {
                current.push(ch);
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else if ch == quote_ch {
                    quote = None;
                }
            }
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Decode the value side of a rendered storage entry. $'...' entries carry
/// the full strtrans.c escape set ansic_quote emits (named C escapes,
/// escaped backslash/quote and three-digit octal); the roundtrip must
/// restore the original bytes (array29 control-character renders).
/// Decode the value side of a rendered storage entry. $'...' entries carry
/// the full strtrans.c escape set ansic_quote emits (named C escapes,
/// escaped backslash/quote and three-digit octal); the roundtrip must
/// restore the original bytes (array29 control-character renders).
/// Decode the value side of a rendered storage entry. $'...' entries carry
/// the full strtrans.c escape set ansic_quote emits (named C escapes,
/// escaped backslash/quote and three-digit octal); the roundtrip must
/// restore the original bytes (array29 control-character renders).
/// Decode the value side of a rendered storage entry. $'...' entries carry
/// the full strtrans.c escape set ansic_quote emits (named C escapes,
/// escaped backslash/quote and three-digit octal); the roundtrip must
/// restore the original bytes (array29 control-character renders).
fn decode_rendered_array_value(value: &str) -> String {
    if let Some(inner) = value
        .strip_prefix("$'")
        .and_then(|value| value.strip_suffix('\''))
    {
        return decode_ansic_escapes(inner);
    }

    unquote_storage_value(value)
}

fn decode_ansic_escapes(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 1 < bytes.len() {
            let escape = bytes[index + 1];
            index += 2;
            match escape {
                b'E' => out.push(0x1b),
                b'a' => out.push(0x07),
                b'b' => out.push(0x08),
                b't' => out.push(0x09),
                b'n' => out.push(0x0a),
                b'v' => out.push(0x0b),
                b'f' => out.push(0x0c),
                b'r' => out.push(0x0d),
                b'\\' => out.push(b'\\'),
                b'\'' => out.push(b'\''),
                b'0'..=b'7' => {
                    let mut decoded = (escape - b'0') as u32;
                    let mut digits = 1;
                    while digits < 3 && index < bytes.len() && matches!(bytes[index], b'0'..=b'7') {
                        decoded = decoded * 8 + (bytes[index] - b'0') as u32;
                        digits += 1;
                        index += 1;
                    }
                    out.push(decoded as u8);
                }
                other => {
                    out.push(b'\\');
                    out.push(other);
                }
            }
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    // The decoded byte stream may not be UTF-8 (`$'\200'`): non-UTF-8 bytes
    // re-encode as raw-byte markers so they round-trip verbatim instead of
    // degrading to U+FFFD (array9.sub `a[2]=$'\x80'`). Carrier-range bytes
    // (0x14..=0x1f: DATA_DQUOTE/QUOTED_WORD_PREFIX/IFS_GLUE/...) must enter
    // pair-encoded exactly like scalar assignment storage
    // (bytes_to_assignment_shell_text), otherwise `[ "${a[i]}" = "$x" ]`
    // compares a raw carrier byte against the pair-encoded form (intl.tests
    // unicode1.sub U+0018/U+001B/U+001C/U+001D).
    crate::executor::substitution_metadata::bytes_to_assignment_shell_text(&out)
}
