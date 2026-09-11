pub(super) fn decode_ansi_c_quoted(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('a') => output.push('\x07'),
            Some('b') => output.push('\x08'),
            Some('e') | Some('E') => output.push('\x1b'),
            Some('f') => output.push('\x0c'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('v') => output.push('\x0b'),
            Some('\\') => output.push('\\'),
            Some('\'') => output.push('\''),
            Some('"') => output.push('"'),
            Some('?') => output.push('?'),
            Some('x') => {
                if chars.peek().copied() == Some('{') {
                    // ksh93/bash backslash x open-brace form (lib/sh/strtrans.c): consume
                    // hex digits until a non-xdigit or close-brace, cap at 0xFF.
                    // backslash x open-brace close-brace yields NUL.
                    chars.next();
                    let mut value = String::new();
                    while let Some(next) = chars.peek().copied() {
                        if next == '}' || next.to_digit(16).is_none() {
                            break;
                        }
                        value.push(next);
                        chars.next();
                    }
                    if chars.peek().copied() == Some('}') {
                        chars.next();
                    }
                    if value.is_empty() {
                        output.push('\0');
                    } else {
                        let parsed = u32::from_str_radix(&value, 16).unwrap_or(0) & 0xFF;
                        push_ansi_c_byte(&mut output, parsed);
                    }
                } else if let Some(value) = read_ansi_c_digits(&mut chars, 16, 2) {
                    push_ansi_c_byte(&mut output, value & 0xFF);
                } else {
                    output.push('\\');
                    output.push('x');
                }
            }
            Some(octal @ '0'..='7') => {
                let mut value = octal.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    let Some(next) = chars.peek().copied() else {
                        break;
                    };
                    let Some(digit) = next.to_digit(8) else {
                        break;
                    };
                    value = value * 8 + digit;
                    chars.next();
                }
                push_ansi_c_byte(&mut output, value & 0xFF);
            }
            Some(c) if c.is_ascii_digit() => {
                output.push('\\');
                output.push(c);
            }
            Some('u') => {
                // GNU strtrans.c ansicstr requires exactly 4 hex digits
                // for \uNNNN; a short run is literal \u followed by digits.
                push_unicode_ansi_c(
                    &mut output,
                    read_ansi_c_digits_raw(&mut chars, 16, 4),
                    "\\u",
                );
            }
            Some('U') => {
                push_unicode_ansi_c(
                    &mut output,
                    read_ansi_c_digits_raw(&mut chars, 16, 8),
                    "\\U",
                );
            }
            Some('c') => {
                // Control character: backslash c X
                if let Some(c) = chars.next() {
                    output.push((c as u32 & 0x1f) as u8 as char);
                }
            }
            None => output.push('\\'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
        }
    }

    // GNU Bash stores words as NUL-terminated C strings, so a NUL byte inside a
    // dollar-single-quote ANSI-C string ends the word: everything from the
    // first NUL onward is dropped (e.g. $'ab\x{}cd' becomes ab). Truncate to
    // match that semantics.
    if let Some(nul) = output.find('\0') {
        output.truncate(nul);
    }

    output
}

fn read_ansi_c_digits<I>(chars: &mut std::iter::Peekable<I>, radix: u32, max: usize) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = String::new();
    while value.len() < max {
        let Some(next) = chars.peek().copied() else {
            break;
        };
        if next.to_digit(radix).is_none() {
            break;
        }
        value.push(next);
        chars.next();
    }

    if value.is_empty() {
        None
    } else {
        u32::from_str_radix(&value, radix).ok()
    }
}

/// Read up to `max` hex digits, returning both the parsed value and the
/// raw digit string. Preserves partial reads so that `\u` followed by
/// fewer than 4 hex digits re-emits the consumed digits as literal text.
fn read_ansi_c_digits_raw<I>(
    chars: &mut std::iter::Peekable<I>,
    radix: u32,
    max: usize,
) -> Option<(u32, String)>
where
    I: Iterator<Item = char>,
{
    let mut raw = String::new();
    while raw.len() < max {
        let Some(next) = chars.peek().copied() else {
            break;
        };
        if next.to_digit(radix).is_none() {
            break;
        }
        raw.push(next);
        chars.next();
    }
    if raw.is_empty() {
        None
    } else {
        u32::from_str_radix(&raw, radix).ok().map(|v| (v, raw))
    }
}

/// Push a `\u`/`\U` escape result into an ANSI-C output buffer.
/// Exact digit count is required for Unicode conversion; fewer digits
/// fall back to literal `\u` + the raw digits.
fn push_unicode_ansi_c(output: &mut String, value: Option<(u32, String)>, prefix: &str) {
    match value {
        Some((codepoint, raw)) if raw.len() == 4 || raw.len() == 8 => {
            push_ansi_c_codepoint(output, codepoint)
        }
        Some((_, raw)) => {
            output.push_str(prefix);
            output.push_str(&raw);
        }
        None => output.push_str(prefix),
    }
}

fn push_ansi_c_codepoint(output: &mut String, value: u32) {
    // GNU strtrans.c:161-181 decodes $'\uNNNN'/'\UNNNNNNNN' through the same
    // u32cconv as printf (lib/sh/unicode.c:239): wctomb on a 4-byte-wchar_t
    // UTF-8 platform encodes every value <= 0x7fffffff, including surrogates
    // (ED A0 80) and the 5/6-byte forms; larger values produce nothing.
    output.push_str(&crate::executor::substitution_metadata::u32cconv_utf8_text(
        value,
    ));
}

/// GNU strtrans.c ansicstr: `\xHH` and octal escapes emit one RAW byte
/// (`c &= 0xFF; *r++ = c;`) -- they never go through locale conversion, so
/// values >= 0x80 are single bytes, not UTF-8 encodings (nquote4.tests
/// `$'ab\x{cd}e'` -> `ab\xcd e`). Rubash words are Rust Strings, so bytes
/// >= 0x80 travel as the owner-tagged U+E000 raw-byte marker pair and are
/// decoded exactly once at the output boundary
/// (write_buffered_builtin_output / pipeline materialization).
fn push_ansi_c_byte(output: &mut String, byte: u32) {
    if byte < 0x80 {
        if let Some(ch) = char::from_u32(byte) {
            output.push(ch);
        }
    } else {
        output
            .push_str(&crate::executor::substitution_metadata::encode_raw_byte_marker(byte as u8));
    }
}
