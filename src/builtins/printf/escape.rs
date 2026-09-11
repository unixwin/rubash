use crate::executor::substitution_metadata::encode_raw_byte_marker;

pub(super) fn expand_format_escape<I>(chars: &mut std::iter::Peekable<I>) -> String
where
    I: Iterator<Item = char>,
{
    match chars.next() {
        Some('a') => "\x07".to_string(),
        Some('b') => "\x08".to_string(),
        Some('e') | Some('E') => "\x1b".to_string(),
        Some('f') => "\x0c".to_string(),
        Some('n') => "\n".to_string(),
        Some('r') => "\r".to_string(),
        Some('t') => "\t".to_string(),
        Some('v') => "\x0b".to_string(),
        Some('\\') => "\\".to_string(),
        Some('x') => format_escape_codepoint(read_escape_digits(chars, 16, 2), "\\x"),
        Some('u') => format_unicode_escape(read_escape_digits_raw(chars, 16, 4), "\\u"),
        Some('U') => format_unicode_escape(read_escape_digits_raw(chars, 16, 8), "\\U"),
        Some('0') => format_escape_byte(read_escape_digits(chars, 8, 3).or(Some(0)), ""),
        Some(octal @ '1'..='7') => {
            format_escape_byte(read_prefixed_escape_digits(chars, octal, 8, 3), "")
        }
        Some(other) => format!("\\{other}"),
        None => "\\".to_string(),
    }
}

fn format_escape_codepoint(value: Option<u32>, fallback: &str) -> String {
    // GNU printf.def:1121-1140: \uNNNN/\UNNNNNNNN convert through u32cconv,
    // which encodes every value <= 0x7fffffff (surrogates and the 5/6-byte
    // UTF-8 forms included) and emits nothing for larger values. A missing
    // digit run is the only fallback case.
    match value {
        Some(value) => crate::executor::substitution_metadata::u32cconv_utf8_text(value),
        None => fallback.to_string(),
    }
}

fn format_escape_byte(value: Option<u32>, fallback: &str) -> String {
    value
        .map(|byte| encode_raw_byte(byte as u8))
        .unwrap_or_else(|| fallback.to_string())
}

fn encode_raw_byte(byte: u8) -> String {
    if byte.is_ascii() {
        char::from(byte).to_string()
    } else {
        encode_raw_byte_marker(byte)
    }
}

pub(super) fn raw_bytes(value: &str) -> Vec<u8> {
    crate::executor::substitution_metadata::shell_text_to_raw_bytes(value)
}

pub(super) fn expand_percent_b(value: &str) -> (String, bool) {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    let mut stop_output = false;

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
            Some('e') | Some('E') => output.push('\x1b'),
            Some('f') => output.push('\x0c'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('v') => output.push('\x0b'),
            Some('\\') => output.push('\\'),
            Some('x') => {
                push_escape_codepoint(&mut output, read_escape_digits(&mut chars, 16, 2), "\\x")
            }
            Some('u') => push_unicode_escape(
                &mut output,
                read_escape_digits_raw(&mut chars, 16, 4),
                "\\u",
            ),
            Some('U') => push_unicode_escape(
                &mut output,
                read_escape_digits_raw(&mut chars, 16, 8),
                "\\U",
            ),
            Some('0') => {
                let value = read_escape_digits(&mut chars, 8, 3).or(Some(0));
                push_escape_byte(&mut output, value, "");
            }
            Some(octal @ '1'..='7') => {
                let value = read_prefixed_escape_digits(&mut chars, octal, 8, 3);
                push_escape_byte(&mut output, value, "");
            }
            Some(other) => {
                // GNU printf treats unrecognized %b escapes as the literal character.
                output.push(other);
            }
            None => output.push('\\'),
        }
    }

    (output, stop_output)
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
fn format_unicode_escape(value: Option<(u32, String)>, prefix: &str) -> String {
    match value {
        Some((codepoint, _raw)) => unicode_escape_text(codepoint),
        None => prefix.to_string(),
    }
}

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
/// Values above the Unicode range convert to nothing in either mode.
fn unicode_escape_text(codepoint: u32) -> String {
    if codepoint > 0x10_FFFF {
        return String::new();
    }
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

/// Push a `\u`/`\U` escape result into an output buffer (for `%b` expansion).
fn push_unicode_escape(output: &mut String, value: Option<(u32, String)>, prefix: &str) {
    match value {
        Some((codepoint, _raw)) => output.push_str(&unicode_escape_text(codepoint)),
        None => output.push_str(prefix),
    }
}

fn push_escape_codepoint(output: &mut String, value: Option<u32>, fallback: &str) {
    // Same GNU u32cconv table as the format-string path (printf.def uses one
    // decode for \u/\U in both the format and %b argument expansion).
    match value {
        Some(value) => output.push_str(
            &crate::executor::substitution_metadata::u32cconv_utf8_text(value),
        ),
        None => output.push_str(fallback),
    }
}

fn push_escape_byte(output: &mut String, value: Option<u32>, fallback: &str) {
    match value {
        Some(byte) => output.push_str(&encode_raw_byte(byte as u8)),
        None => output.push_str(fallback),
    }
}

pub(super) fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    if value == "~" {
        return "\\~".to_string();
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

    let mut quoted = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '/' | '.' | '-' | ':') {
            quoted.push(ch);
        } else {
            quoted.push('\\');
            quoted.push(ch);
        }
    }
    quoted
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
            '\x1b' => quoted.push_str("\\E"),
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
            '\x1b' => quoted.push_str("\\E"),
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
