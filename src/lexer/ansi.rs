/// C0 bytes the assignment walker uses as data-quote carriers
/// (U+0014 backslash, U+0017 quote, U+001A backtick, U+001F dollar). A byte
/// with one of these values that came out of ANSI-C decoding is DATA, not a
/// carrier, and must be tagged so the carrier restore cannot claim it.
pub(crate) fn is_assignment_carrier_byte(byte: u32) -> bool {
    // U+000C form feed and U+0013 are not quote carriers, but they are
    // control bytes that StorageWordIter's is_ascii_whitespace splitter
    // (0x0c) and PARAM_NAME_END_MARKER (0x13) would otherwise claim,
    // so they take the same owner-tagged carrier as the C0 quote bytes.
    // unicode1.sub [0x000c]=$'\f' and [0x0013]=$'\023' both collapsed to
    // empty elements without this.
    // U+0018 is the double-quote sentinel used by embedded-parameter
    // expansion; U+001C is the protected-whitespace / alternate-word
    // marker. Both must be tagged so the expansion walker treats them as
    // data, not markers (exp1.sub $'\c\\\001' and $'\034').
    // U+001D is the array-storage sentinel (declare/storage.rs, arrays/storage.rs);
    // a decoded \c] (0x1d) must be tagged so it is not misinterpreted as an
    // array marker prefix (nquote5.sub $'\c[\c\\\c]\c^\c_\c?').
    // U+001B is the quoted-tilde marker prefix (lexer/word.rs);
    // a decoded \c[ (0x1b, ESC) must be tagged so it is not misinterpreted
    // as a tilde marker (nquote5.sub $'\c[').
    // 0x0c is a plain data byte (the former PATSUB_QUOTED_VALUE_END carrier
    // moved to U+E311); it stays encoded so user \f input never collides
    // with residual text-layer consumers.
    // The tagged set must cover every text-layer carrier byte so a decoded
    // data byte can never be claimed by a carrier consumer. This is the same
    // set bytes_to_assignment_shell_text (substitution_metadata.rs) stores:
    // {0x0c, CTLESC(0x11), PARAM_NAME_END_MARKER(0x13)} plus the whole
    // 0x14..=0x1f carrier block — including PROTECTED_BACKSLASH(0x15),
    // PROTECTED_LITERAL_BACKSLASH(0x19), and SUBSCRIPT_CARRIER /
    // PARSE_ERROR_FIELD_SEP / HEREDOC_WARNED_BODY_PREFIX(0x1e). An
    // incomplete list is what made $'\025'/$'\031'/$'\036' decode to a raw
    // char that the backslash/subscript restores then mangled
    // (unicode1.sub U+00000015/U+00000019/U+0000001E).
    use crate::executor::markers::{CTLESC, PARAM_NAME_END_MARKER};
    [0x0c, CTLESC as u32, PARAM_NAME_END_MARKER as u32].contains(&byte)
        || (0x14..=0x1f).contains(&byte)
}

pub(crate) fn decode_ansi_c_quoted(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    // Buffer for consecutive raw bytes >= 0x80 emitted by octal/hex escapes.
    // GNU ansicstr (lib/sh/strtrans.c:130 `c &= 0xFF; *r++ = c;`) stores each
    // escape as a single raw byte; in a UTF-8 locale, consecutive raw bytes
    // that form a valid UTF-8 sequence are stored as those bytes and compare
    // equal to the same bytes produced by printf '\UNNNNNNNN' (u32cconv →
    // wctomb). Rubash words are Rust Strings (UTF-8), so we decode the
    // buffered bytes as UTF-8 when the sequence is interrupted or at end of
    // input: valid UTF-8 becomes Unicode scalar chars (matching
    // u32cconv_utf8_text's char path), invalid bytes become raw-byte marker
    // pairs (matching u32cconv_utf8_text's marker path for 5/6-byte forms
    // and surrogate encodings that have no Rust char form).
    let mut raw_byte_buf: Vec<u8> = Vec::new();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('a') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\x07');
            }
            Some('b') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\x08');
            }
            Some('e') | Some('E') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                push_ansi_c_byte(&mut output, 0x1b);
            }
            Some('f') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                push_ansi_c_byte(&mut output, 0x0c);
            }
            Some('n') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\n');
            }
            Some('r') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\r');
            }
            Some('t') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\t');
            }
            Some('v') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\x0b');
            }
            Some('\\') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\\');
            }
            Some('\'') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\'');
            }
            Some('"') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('"');
            }
            Some('?') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('?');
            }
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
                        flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                        output.push('\0');
                    } else {
                        let parsed = u32::from_str_radix(&value, 16).unwrap_or(0) & 0xFF;
                        push_or_buffer_raw_byte(&mut output, &mut raw_byte_buf, parsed);
                    }
                } else if let Some(value) = read_ansi_c_digits(&mut chars, 16, 2) {
                    push_or_buffer_raw_byte(&mut output, &mut raw_byte_buf, value & 0xFF);
                } else {
                    flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
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
                push_or_buffer_raw_byte(&mut output, &mut raw_byte_buf, value & 0xFF);
            }
            Some(c) if c.is_ascii_digit() => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\\');
                output.push(c);
            }
            Some('u') => {
                // GNU strtrans.c ansicstr requires exactly 4 hex digits
                // for \uNNNN; a short run is literal \u followed by digits.
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                push_unicode_ansi_c(
                    &mut output,
                    read_ansi_c_digits_raw(&mut chars, 16, 4),
                    "\\u",
                );
            }
            Some('U') => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                push_unicode_ansi_c(
                    &mut output,
                    read_ansi_c_digits_raw(&mut chars, 16, 8),
                    "\\U",
                );
            }
            Some('c') => {
                // GNU chartypes.h TOCTRL: \c? → 0x7f (DEL), otherwise
                // TOUPPER(c) & 0x1f.  An omitted operand (end of string)
                // passes \c through literally (strtrans.c ansic_quote).
                // Posix requires $'\c\\' to do backslash escaping: if the
                // operand is `\` and the next character is also `\`,
                // consume the second backslash (strtrans.c ansicstr 203-204).
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                if let Some(c) = chars.next() {
                    let operand = if c == '\\' && chars.peek() == Some(&'\\') {
                        chars.next();
                        '\\'
                    } else {
                        c
                    };
                    let value = if operand == '?' {
                        0x7f
                    } else {
                        (operand.to_ascii_uppercase() as u32) & 0x1f
                    };
                    push_ansi_c_byte(&mut output, value);
                } else {
                    output.push('\\');
                    output.push('c');
                }
            }
            None => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\\');
            }
            Some(other) => {
                flush_raw_byte_buf(&mut output, &mut raw_byte_buf);
                output.push('\\');
                output.push(other);
            }
        }
    }
    flush_raw_byte_buf(&mut output, &mut raw_byte_buf);

    // GNU Bash stores words as NUL-terminated C strings, so a NUL byte inside a
    // dollar-single-quote ANSI-C string ends the word: everything from the
    // first NUL onward is dropped (e.g. $'ab\x{}cd' becomes ab). Truncate to
    // match that semantics.
    if let Some(nul) = output.find('\0') {
        output.truncate(nul);
    }

    output
}

/// Push a raw byte from an octal/hex escape. Bytes >= 0x80 are buffered for
/// possible UTF-8 multi-byte decoding (matching GNU's byte-level storage in a
/// UTF-8 locale). Bytes < 0x80 flush the buffer first, then go through
/// `push_ansi_c_byte` directly (carrier bytes get marker pairs, others become
/// plain chars).
fn push_or_buffer_raw_byte(output: &mut String, buf: &mut Vec<u8>, byte: u32) {
    let byte = byte as u8;
    if byte >= 0x80 {
        buf.push(byte);
    } else {
        flush_raw_byte_buf(output, buf);
        push_ansi_c_byte(output, byte as u32);
    }
}

/// Flush the raw-byte buffer, attempting UTF-8 decoding. Valid UTF-8 sequences
/// become Unicode scalar chars (matching `u32cconv_utf8_text`'s char path, so
/// `$'\302\200'` and `printf '\U00000080'` produce the same internal String).
/// Invalid bytes become raw-byte marker pairs (matching `u32cconv_utf8_text`'s
/// marker path for 5/6-byte forms and lone continuation bytes).
fn flush_raw_byte_buf(output: &mut String, buf: &mut Vec<u8>) {
    if buf.is_empty() {
        return;
    }
    let bytes = std::mem::take(buf);
    let mut start = 0;
    while start < bytes.len() {
        match std::str::from_utf8(&bytes[start..]) {
            Ok(s) => {
                output.push_str(s);
                break;
            }
            Err(e) => {
                let valid_len = e.valid_up_to();
                if valid_len > 0 {
                    let s = std::str::from_utf8(&bytes[start..start + valid_len]).unwrap();
                    output.push_str(s);
                }
                // The byte at valid_up_to is invalid — emit as marker pair.
                let invalid_byte = bytes[start + valid_len];
                output.push_str(
                    &crate::executor::substitution_metadata::encode_raw_byte_marker(invalid_byte),
                );
                start += valid_len + 1;
            }
        }
    }
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
        Some((codepoint, _raw)) => push_ansi_c_codepoint(output, codepoint),
        None => output.push_str(prefix),
    }
}

fn push_ansi_c_codepoint(output: &mut String, value: u32) {
    // GNU strtrans.c:161-181 decodes $'\uNNNN'/'\UNNNNNNNN' through the same
    // u32cconv as printf (lib/sh/unicode.c:239): wctomb on a 4-byte-wchar_t
    // UTF-8 platform encodes every value <= 0x7fffffff, including surrogates
    // (ED A0 80) and the 5/6-byte forms; larger values produce nothing.
    //
    // Locale gate (rubash#353): strtrans.c:174 translates values <= 0x7f
    // directly (ASCII needs no conversion), everything else goes through
    // u32cconv, whose non-UTF-8-locale fallback is u32tocesc
    // (unicode.c:141: `\u%04X` / `\U%08X`, ISO C99 escape with UPPERCASE
    // hex) — under `LC_ALL=C` GNU keeps `$'\u00e9'` as the six literal
    // characters `\u00E9`. The locale name is read dynamically (see
    // locale::ansi_lex_locale_name: a re-derivation under a line's
    // read-time locale pins it), so an `export LC_ALL=...` earlier in the
    // script is honored at the word's GNU read time.
    // Only an EXPLICITLY selected non-UTF-8 locale closes the gate: with
    // no locale variables rubash's platform default stays UTF-8 (its whole
    // text layer is UTF-8 on Windows; GNU would default to the C locale —
    // a documented platform divergence).
    let explicit_non_utf8_locale = {
        let name = crate::locale::ansi_lex_locale_name();
        !name.is_empty() && !crate::locale::is_utf8_locale_name(&name)
    };
    if value > 0x7f && explicit_non_utf8_locale {
        if value < 0x1_0000 {
            output.push_str(&format!("\\u{value:04X}"));
        } else {
            output.push_str(&format!("\\U{value:08X}"));
        }
        return;
    }
    let decoded = crate::executor::substitution_metadata::u32cconv_utf8_text(value);
    // Registry-zone chars (U+E000..=U+E3FF) are user-reachable here —
    // $'\uE314' is real data, but the same codepoint is a transport guard.
    // E000-escape them at this entry so the decode boundary restores the
    // char instead of reading it as a marker (governance 3.1 user_reachable
    // rule; the E1xx byte-pair/UTF-8 mixed-decoding ambiguity).
    for ch in decoded.chars() {
        crate::executor::markers::push_literal_char(output, ch);
    }
}

/// GNU strtrans.c ansicstr: `\xHH` and octal escapes emit one RAW byte
/// (`c &= 0xFF; *r++ = c;`) -- they never go through locale conversion, so
/// values >= 0x80 are single bytes, not UTF-8 encodings (nquote4.tests
/// `$'ab\x{cd}e'` -> `ab\xcd e`). Rubash words are Rust Strings, so bytes
/// >= 0x80 travel as the owner-tagged U+E000 raw-byte marker pair and are
/// decoded exactly once at the output boundary
/// (write_buffered_builtin_output / pipeline materialization).
///
/// A decoded control byte that collides with one of the assignment walker's
/// C0 carriers must be tagged the same way. `$'\037'` is a genuine U+001F
/// data byte, but U+001F is also how the lexer stores a literal `$` inside a
/// single-quoted word (`'$$'` -> 1f 1f). Both reach the assignment expander
/// as the same bytes with the same `\x1c` quoted prefix, so no downstream
/// pass can tell them apart by inspection -- verified by dumping the value
/// at expand_assignment_value_inner, where `a='$$'` and `x=$'\037'` are
/// byte-identical (`1c 1f 1f` vs `1c 1f`). Restoring the carrier is correct
/// for the single-quote case and corrupts the ANSI-C case; tagging the
/// decoded byte keeps the two distinguishable so the carrier restore can
/// run unconditionally. GNU stores 24 24 for `'$$'` (real dollars), so the
/// restore is what makes rubash's storage bytes match GNU's.
fn push_ansi_c_byte(output: &mut String, byte: u32) {
    if byte < 0x80 {
        if let Some(ch) = char::from_u32(byte) {
            if is_assignment_carrier_byte(byte) {
                output.push_str(
                    &crate::executor::substitution_metadata::encode_raw_byte_marker(byte as u8),
                );
            } else {
                output.push(ch);
            }
        }
    } else {
        output
            .push_str(&crate::executor::substitution_metadata::encode_raw_byte_marker(byte as u8));
    }
}
