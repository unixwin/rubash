use crate::executor::substitution_metadata::encode_raw_byte_marker;

/// Expand one backslash escape from the FORMAT string. Returns the expanded
/// text plus an optional bare builtin_error body (no shell prefix): GNU
/// printf.def tescape() reports `missing hex digit for \x` (and the \u/\U
/// analogue) through builtin_error without setting conversion_error, so the
/// diagnostic is stderr-only and the exit status stays 0.
pub(super) fn expand_format_escape<I>(
    chars: &mut std::iter::Peekable<I>,
) -> (String, Option<String>)
where
    I: Iterator<Item = char>,
{
    match chars.next() {
        Some('a') => ("\x07".to_string(), None),
        Some('b') => ("\x08".to_string(), None),
        // \e and \f decode to carrier bytes (0x1b QUOTED_WORD_PREFIX, 0x0c):
        // as data they must be marker-tagged so no carrier consumer claims
        // them (unicode1.sub printf -v). Same contract as push_ansi_c_byte.
        Some('e') | Some('E') => (encode_raw_byte(0x1b), None),
        Some('f') => (encode_raw_byte(0x0c), None),
        Some('n') => ("\n".to_string(), None),
        Some('r') => ("\r".to_string(), None),
        Some('t') => ("\t".to_string(), None),
        Some('v') => ("\x0b".to_string(), None),
        Some('\\') => ("\\".to_string(), None),
        // GNU printf.def:1148-1158: \', \", \? in the format string are
        // recognized as escape sequences with backslash removal (sawc==0).
        Some('\'') => ("'".to_string(), None),
        Some('"') => ("\"".to_string(), None),
        Some('?') => ("?".to_string(), None),
        Some('x') => {
            let (expanded, missing) = format_escape_codepoint(
                read_escape_digits(chars, 16, 2),
                "\\x",
                "missing hex digit for \\x",
            );
            (expanded, missing)
        }
        Some('u') => format_unicode_escape(read_escape_digits_raw(chars, 16, 4), "\\u", 'u'),
        Some('U') => format_unicode_escape(read_escape_digits_raw(chars, 16, 8), "\\U", 'U'),
        // GNU printf.def tescape() octal: `temp = 2 + (!evalue && !!sawc)`.
        // In the FORMAT string sawc is NULL, so a leading-\0 escape takes at
        // most two more digits (three total); the fourth digit stays literal
        // (`printf '\0007'` emits NUL then `7`). The %b path passes sawc and
        // takes three more (see expand_percent_b).
        Some('0') => (
            format_escape_byte(read_escape_digits(chars, 8, 2).or(Some(0)), ""),
            None,
        ),
        Some(octal @ '1'..='7') => (
            format_escape_byte(read_prefixed_escape_digits(chars, octal, 8, 3), ""),
            None,
        ),
        Some(other) => (format!("\\{other}"), None),
        None => ("\\".to_string(), None),
    }
}

fn format_escape_codepoint(
    value: Option<u32>,
    fallback: &str,
    diagnostic: &str,
) -> (String, Option<String>) {
    // GNU printf.def:1121-1140: \uNNNN/\UNNNNNNNN convert through u32cconv,
    // which encodes every value <= 0x7fffffff (surrogates and the 5/6-byte
    // UTF-8 forms included) and emits nothing for larger values. A missing
    // digit run is the only fallback case.
    match value {
        Some(value) => (
            crate::executor::substitution_metadata::u32cconv_utf8_text(value),
            None,
        ),
        None => (fallback.to_string(), Some(format!("printf: {diagnostic}"))),
    }
}

fn format_unicode_escape(
    value: Option<(u32, String)>,
    prefix: &str,
    kind: char,
) -> (String, Option<String>) {
    match value {
        Some((codepoint, _raw)) => (unicode_escape_text(codepoint), None),
        None => (
            prefix.to_string(),
            Some(format!("printf: missing unicode digit for \\{kind}")),
        ),
    }
}

fn format_escape_byte(value: Option<u32>, fallback: &str) -> String {
    value
        .map(|byte| encode_raw_byte(byte as u8))
        .unwrap_or_else(|| fallback.to_string())
}

fn encode_raw_byte(byte: u8) -> String {
    // ASCII bytes that collide with text-layer carriers must travel as
    // raw-byte marker pairs (same rule as push_ansi_c_byte in
    // lexer/ansi.rs): printf '\x15' emits byte 0x15, and a bare char 0x15
    // would be claimed by the PROTECTED_BACKSLASH restore.
    if byte.is_ascii() && !crate::lexer::ansi::is_assignment_carrier_byte(byte as u32) {
        char::from(byte).to_string()
    } else {
        encode_raw_byte_marker(byte)
    }
}

pub(super) fn raw_bytes(value: &str) -> Vec<u8> {
    crate::executor::substitution_metadata::shell_text_to_raw_bytes(value)
}

/// Expand a `%b` argument. Returns the expanded text, whether a `\c` escape
/// stopped the output, and an optional bare builtin_error body for the
/// missing-digit diagnostics (GNU tescape: builtin_error only, exit status
/// unaffected).
pub(super) fn expand_percent_b(value: &str) -> (String, bool, Option<String>) {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    let mut stop_output = false;
    let mut diagnostic = None;

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('c') => {
                stop_output = true;
                break;
            }
            Some('a') => output.push('\x07'),
            Some('b') => output.push('\x08'),
            Some('e') | Some('E') => output.push_str(&encode_raw_byte(0x1b)),
            Some('f') => output.push_str(&encode_raw_byte(0x0c)),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('v') => output.push('\x0b'),
            Some('\\') => output.push('\\'),
            Some('x') => {
                let (expanded, missing) = format_escape_codepoint(
                    read_escape_digits(&mut chars, 16, 2),
                    "\\x",
                    "missing hex digit for \\x",
                );
                if missing.is_some() && diagnostic.is_none() {
                    diagnostic = missing;
                }
                output.push_str(&expanded);
            }
            Some('u') => {
                let (expanded, missing) =
                    format_unicode_escape(read_escape_digits_raw(&mut chars, 16, 4), "\\u", 'u');
                if missing.is_some() && diagnostic.is_none() {
                    diagnostic = missing;
                }
                output.push_str(&expanded);
            }
            Some('U') => {
                let (expanded, missing) =
                    format_unicode_escape(read_escape_digits_raw(&mut chars, 16, 8), "\\U", 'U');
                if missing.is_some() && diagnostic.is_none() {
                    diagnostic = missing;
                }
                output.push_str(&expanded);
            }
            // GNU printf.def tescape with sawc != NULL: a leading-0 octal
            // escape consumes up to three MORE digits (four total), so
            // %b '\0007' is a single BEL.
            Some('0') => {
                let value = read_escape_digits(&mut chars, 8, 3).or(Some(0));
                push_escape_byte(&mut output, value, "");
            }
            Some(octal @ '1'..='7') => {
                let value = read_prefixed_escape_digits(&mut chars, octal, 8, 3);
                push_escape_byte(&mut output, value, "");
            }
            // GNU printf.def:1148-1158 + default: \', \", \? and all
            // unrecognized escapes in %b keep the backslash (sawc!=0).
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }

    (output, stop_output, diagnostic)
}

fn read_prefixed_escape_digits<I>(
    chars: &mut std::iter::Peekable<I>,
    first: char,
    radix: u32,
    max: usize,
) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = first.to_string();
    while value.len() < max {
        let Some(ch) = chars.peek().copied() else {
            break;
        };
        if ch.to_digit(radix).is_none() {
            break;
        }
        value.push(ch);
        chars.next();
    }
    u32::from_str_radix(&value, radix).ok()
}

fn read_escape_digits<I>(chars: &mut std::iter::Peekable<I>, radix: u32, max: usize) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = String::new();
    while value.len() < max {
        let Some(ch) = chars.peek().copied() else {
            break;
        };
        if ch.to_digit(radix).is_none() {
            break;
        }
        value.push(ch);
        chars.next();
    }
    if value.is_empty() {
        None
    } else {
        u32::from_str_radix(&value, radix).ok()
    }
}

/// Read up to `max` hex digits, returning both the parsed value and the
/// raw digit string. The value is what printf encodes, so a partial digit
/// run (`\uff`) still yields a code point and is canonicalized on output.
fn read_escape_digits_raw<I>(
    chars: &mut std::iter::Peekable<I>,
    radix: u32,
    max: usize,
) -> Option<(u32, String)>
where
    I: Iterator<Item = char>,
{
    let mut raw = String::new();
    while raw.len() < max {
        let Some(ch) = chars.peek().copied() else {
            break;
        };
        if ch.to_digit(radix).is_none() {
            break;
        }
        raw.push(ch);
        chars.next();
    }
    if raw.is_empty() {
        None
    } else {
        u32::from_str_radix(&raw, radix).ok().map(|v| (v, raw))
    }
}

/// Format a `\u`/`\U` escape (format string).

/// Encode a parsed `\u`/`\U` code point under the active locale.
///
/// GNU printf.def walks the value through u32cconv() (lib/sh/unicode.c) with
/// the locale active. In a multibyte locale the result is the UTF-8 encoding
/// (`\u00FF` -> C3 BF under C.utf8). In a single-byte locale a value that does
/// not fit in one ASCII byte is re-emitted as a literal escape in canonical
/// form: `\u%04X` up to 0xFFFF, `\U%08X` above, so `\U000000FF` folds onto
/// `\u00FF` and `\U0001F600` stays `\U0001F600`. The canonical form is chosen
/// from the parsed value, not the digit run as typed, which is why a partial
/// `\uff` also becomes `\u00FF` (unicode2.sub, LC_CTYPE=C).
/// GNU printf.def:1132-1140 passes every value > 0x7F through u32cconv()
/// (lib/sh/unicode.c:239), which on a 4-byte-wchar_t platform (Linux/glibc
/// with __STDC_ISO_10646__) calls wctomb for all values <= 0x7FFFFFFF,
/// including the 5/6-byte UTF-8 forms above 0x10FFFF. There is no 0x10FFFF
/// cap in the C source. u32cconv_utf8_text handles the full range: valid
/// Unicode scalars become chars, 5/6-byte forms become marker pairs, and
/// values > 0x7FFFFFFF produce nothing (matching wctomb's -1 return).
fn unicode_escape_text(codepoint: u32) -> String {
    if codepoint <= 0x7F || crate::locale::is_multi_byte() {
        return crate::executor::substitution_metadata::u32cconv_utf8_text(codepoint);
    }
    // Emit the escape literally: backslash + u/U + canonical hex. The
    // backslash is built from its code point so the format strings below stay
    // free of backslash escapes.
    let mut out = String::new();
    out.push(char::from_u32(0x5c).expect("ASCII backslash is a valid char"));
    if codepoint <= 0xFFFF {
        out.push('u');
        out.push_str(&format!("{:04X}", codepoint));
    } else {
        out.push('U');
        out.push_str(&format!("{:08X}", codepoint));
    }
    out
}

fn push_escape_byte(output: &mut String, value: Option<u32>, fallback: &str) {
    match value {
        Some(byte) => output.push_str(&encode_raw_byte(byte as u8)),
        None => output.push_str(fallback),
    }
}

/// GNU shquote.c:95 sh_single_quote(): wrap in `'...'`, embedding a
/// literal `'` as `'\''`. The single-character string `'` collapses to
/// `\'`. Selected by printf's altform flag (`%#q`, printf.def:699).
pub(super) fn single_shell_quote(value: &str) -> String {
    if value == "'" {
        return "\\'".to_string();
    }
    let mut quoted = String::with_capacity(value.len() + 3);
    quoted.push('\'');
    for ch in value.chars() {
        quoted.push(ch);
        if ch == '\'' {
            quoted.push_str("\\''");
        }
    }
    quoted.push('\'');
    quoted
}

pub(super) fn shell_quote(value: &str) -> String {
    shell_quote_form(value, false)
}

/// GNU printf.def:695-702 quoting decision for %q/%Q: empty -> `''`;
/// ansic_shouldquote (control characters / non-printable bytes) ->
/// ansic_quote ($'...'); altform (`%#q') -> sh_single_quote; otherwise
/// sh_backslash_quote with flags 3.
pub(super) fn shell_quote_form(value: &str, altform: bool) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    // Raw-byte marker pairs (U+E000 + U+E0xx payload) carry bytes that do
    // not form printable characters. GNU printf %q renders such a value as
    // $'...' with one octal escape per non-printable byte (strtrans.c
    // ansic_quote over the byte string; unicode3.sub payload
    // $'5\247@3\231+...'). The marker chars themselves are private-use and
    // never is_control(), so they must be decoded before the quote decision.
    let sentinel = char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
        .expect("raw-byte sentinel is a valid char");
    if value.contains(sentinel) {
        let bytes =
            crate::executor::substitution_metadata::decode_raw_byte_markers(value.as_bytes());
        return ansi_c_shell_quote_bytes(&bytes);
    }

    if value.chars().any(|ch| ch.is_control()) {
        return ansi_c_shell_quote(value);
    }

    if altform {
        return single_shell_quote(value);
    }

    // GNU shquote.c sh_backslash_quote with flags=3 (printf.def:702): the
    // default bstab table determines which characters get a backslash.
    // Additionally, `#` at the start of the string is backslash-quoted
    // (comment char), and `~` at the start or after `:` / `=` is
    // backslash-quoted (tilde expansion, flags & 1). Non-ASCII printable
    // characters are copied verbatim (shquote.c COPY_CHAR_P in the
    // HANDLE_MULTIBYTE branch).
    backslash_quote(value)
}

/// GNU shquote.c `sh_backslash_quote` with `flags = 3` (the `printf %q`
/// call site at printf.def:702). The default `bstab` table marks which
/// characters receive a backslash; `flags & 1` additionally quotes `~` at
/// the start of the word or after `:` / `=`, and `#` at the start is always
/// quoted as a comment character.
fn backslash_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() * 2);
    let chars: Vec<char> = value.chars().collect();
    for (position, &ch) in chars.iter().enumerate() {
        if needs_backslash(ch, position, &chars) {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted
}

fn needs_backslash(ch: char, position: usize, chars: &[char]) -> bool {
    if !ch.is_ascii() {
        // GNU sh_backslash_quote: multibyte characters are copied verbatim
        // (shquote.c COPY_CHAR_P in the HANDLE_MULTIBYTE branch).
        return false;
    }
    if bstab_needs_quote(ch) {
        return true;
    }
    // shquote.c: `#` at the start of the string is a comment char.
    if ch == '#' {
        return position == 0;
    }
    // shquote.c flags & 1: `~` at the start or after `:` / `=` is special.
    if ch == '~' {
        return position == 0 || matches!(chars.get(position - 1), Some(':' | '='));
    }
    false
}

/// GNU shquote.c `bstab` table: characters that always receive a backslash.
/// Control characters (TAB, NL) are listed for fidelity to the C source but
/// are caught earlier by the `is_control()` ansic_shouldquote gate.
fn bstab_needs_quote(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n'
            | ' '
            | '!'
            | '"'
            | '$'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | ','
            | ';'
            | '<'
            | '>'
            | '?'
            | '['
            | '\\'
            | ']'
            | '^'
            | '`'
            | '{'
            | '|'
            | '}'
    )
}

/// GNU strtrans.c ansic_quote over the raw byte string (printf %q of a
/// value whose bytes do not form printable characters): named escapes for
/// the C0 specials, verbatim printable runs (valid multibyte sequences
/// included), and one 3-digit octal escape per non-printable or invalid
/// byte, walking one byte at a time exactly like utf8_mbstrlen.
fn ansi_c_shell_quote_bytes(bytes: &[u8]) -> String {
    let mut quoted = String::from("$'");
    let mut rest: &[u8] = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                push_ansic_escaped_chars(&mut quoted, text.chars());
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if let Ok(text) = std::str::from_utf8(&rest[..valid]) {
                    push_ansic_escaped_chars(&mut quoted, text.chars());
                }
                quoted.push_str(&format!("\\{:03o}", rest[valid]));
                rest = &rest[valid + 1..];
            }
        }
    }
    quoted.push('\'');
    quoted
}

fn push_ansic_escaped_chars(quoted: &mut String, chars: impl Iterator<Item = char>) {
    for ch in chars {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            '\x07' => quoted.push_str("\\a"),
            '\x08' => quoted.push_str("\\b"),
            crate::executor::markers::QUOTED_WORD_PREFIX => quoted.push_str("\\E"),
            '\x0c' => quoted.push_str("\\f"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\x0b' => quoted.push_str("\\v"),
            ch if ch.is_ascii_control() => quoted.push_str(&format!("\\{:03o}", ch as u32)),
            ch => quoted.push(ch),
        }
    }
}

fn ansi_c_shell_quote(value: &str) -> String {
    let mut quoted = String::from("$'");
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            '\x07' => quoted.push_str("\\a"),
            '\x08' => quoted.push_str("\\b"),
            crate::executor::markers::QUOTED_WORD_PREFIX => quoted.push_str("\\E"),
            '\x0c' => quoted.push_str("\\f"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\x0b' => quoted.push_str("\\v"),
            ch if ch.is_ascii_control() => quoted.push_str(&format!("\\{:03o}", ch as u32)),
            ch => quoted.push(ch),
        }
    }
    quoted.push('\'');
    quoted
}
