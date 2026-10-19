use super::ParsedNumber;

pub(super) fn parse_i64(value: &str) -> ParsedNumber<i64> {
    if let Some(ch) = printf_char_constant(value) {
        return ParsedNumber {
            value: ch as i64,
            invalid: None,
        };
    }
    if value.is_empty() {
        return ParsedNumber {
            value: 0,
            invalid: None,
        };
    }
    match scan_integer(value) {
        Some(scan) => {
            let (parsed, overflow) = accumulate_signed(&scan);
            ParsedNumber {
                value: parsed,
                // printf.def:1388-1395 chk_converror(): the `*ep != '\0'`
                // branch (bytes left after the digit run) wins over ERANGE,
                // so a suffixed operand reports sh_invalidnum even when the
                // prefix also overflowed (`99999999999999999999x: invalid
                // number`, not "Numerical result out of range").
                invalid: if scan.suffixed {
                    Some(value.to_string())
                } else if overflow {
                    Some(format!("__rubash_printf_overflow__:{value}"))
                } else {
                    None
                },
            }
        }
        None => ParsedNumber {
            value: 0,
            invalid: Some(value.to_string()),
        },
    }
}

pub(super) fn parse_u64(value: &str) -> ParsedNumber<u64> {
    if let Some(ch) = printf_char_constant(value) {
        return ParsedNumber {
            value: ch as u64,
            invalid: None,
        };
    }
    if value.is_empty() {
        return ParsedNumber {
            value: 0,
            invalid: None,
        };
    }
    match scan_integer(value) {
        Some(scan) => {
            let (parsed, overflow) = accumulate_unsigned(&scan);
            ParsedNumber {
                value: parsed,
                invalid: if scan.suffixed {
                    Some(value.to_string())
                } else if overflow {
                    Some(format!("__rubash_printf_overflow__:{value}"))
                } else {
                    None
                },
            }
        }
        None => ParsedNumber {
            value: 0,
            invalid: Some(value.to_string()),
        },
    }
}

pub(super) fn parse_f64(value: &str) -> ParsedNumber<f64> {
    if let Some(ch) = printf_char_constant(value) {
        return ParsedNumber {
            value: ch as u32 as f64,
            invalid: None,
        };
    }
    if value.is_empty() {
        return ParsedNumber {
            value: 0.0,
            invalid: None,
        };
    }
    match scan_float_subject(value) {
        Some(subject) => {
            let end = subject.end;
            let parsed = convert_float_subject(&value[subject.start..end]);
            // printf.def:1472-1495 getdouble(): strtod accumulates the
            // longest valid subject sequence and getdouble RETURNS it even
            // when chk_converror rejects the trailing bytes, so `printf %f
            // '5 '` prints 5.000000 (status 1) instead of 0.000000.
            ParsedNumber {
                value: parsed,
                invalid: (end < value.len()).then(|| value.to_string()),
            }
        }
        None => ParsedNumber {
            value: 0.0,
            invalid: Some(value.to_string()),
        },
    }
}

/// GNU printf.def:834-838 printf_erange()/1391-1398 chk_converror(): the
/// diagnostic is a builtin_error body (no "warning:") naming the offending
/// operand, and conversion_error makes the builtin exit status 1. The body
/// carries no shell prefix -- render_one_pass adds the `script: line N:`
/// prologue from builtin_error's error_prolog. The classification of the
/// non-ERANGE branch is builtins/common.c:214-225 sh_invalidnum(): a
/// leading `0` followed by a digit is an octal complaint, a leading `0x`
/// (lowercase x only) a hex complaint, everything else a plain number.
pub(super) fn invalid_number_error(value: &str) -> String {
    if let Some(value) = value.strip_prefix("__rubash_printf_overflow__:") {
        format!("printf: {value}: Numerical result out of range")
    } else {
        let bytes = value.as_bytes();
        let message = match (bytes.first(), bytes.get(1)) {
            (Some(b'0'), Some(second)) if second.is_ascii_digit() => "invalid octal number",
            (Some(b'0'), Some(b'x')) => "invalid hex number",
            _ => "invalid number",
        };
        format!("printf: {value}: {message}")
    }
}

/// printf.def:1416-1419 getintmax(): an operand whose first byte is a quote
/// goes to asciicode() BEFORE any numeric parse, and asciicode
/// (printf.def:1519-1544) converts arg[1] with mbrtowc without any
/// chk_converror -- so `'a ` yields 'a' (97) with no diagnostic, and a lone
/// quote yields the NUL at arg[1] (0, also no diagnostic).
fn printf_char_constant(value: &str) -> Option<char> {
    let mut chars = value.chars();
    match chars.next() {
        Some('\'') | Some('"') => chars.next().or(Some('\0')),
        _ => None,
    }
}

/// One strtoimax/strtoumax subject scan (printf.def:1437 getintmax and
/// :1459 getuintmax call strtoimax/strtoumax with base 0).
struct IntScan {
    radix: u32,
    negative: bool,
    digits: String,
    /// chk_converror's `*ep != '\0'`: bytes remain after the digit run.
    suffixed: bool,
}

/// C locale isspace: the exact character set strtoimax/strtod skip as
/// LEADING white space. Trailing white space is never skipped -- it is what
/// makes `'5 '` an invalid-number operand (issue #704 R6.1; the previous
/// full `str::trim` silently accepted it).
fn skip_leading_space(value: &str) -> &str {
    value.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
}

fn scan_integer(value: &str) -> Option<IntScan> {
    let body = skip_leading_space(value);
    let offset = if matches!(body.as_bytes().first(), Some(b'-') | Some(b'+')) {
        1
    } else {
        0
    };
    let negative = body.as_bytes().first() == Some(&b'-');
    let rest = &body[offset..];
    let rest_bytes = rest.as_bytes();
    // Base-0 radix selection. A 0x/0X or 0b/0B prefix only counts when a
    // digit of that radix follows; otherwise strto* falls back to the
    // leading `0` as an octal digit (`0xg` converts 0 with `xg` as the
    // rejected suffix, WSL GNU bash 5.3.0 probe). 0b/0B is the C23 binary
    // literal that glibc 2.38+ strto* accepts in base 0 (`0b101` -> 5,
    // oracle-verified).
    let (radix, digits_offset) = if matches!(rest_bytes.get(..2), Some(b"0x") | Some(b"0X"))
        && rest_bytes.get(2).is_some_and(u8::is_ascii_hexdigit)
    {
        (16, 2)
    } else if matches!(rest_bytes.get(..2), Some(b"0b") | Some(b"0B"))
        && matches!(rest_bytes.get(2), Some(b'0' | b'1'))
    {
        (2, 2)
    } else if rest_bytes.first() == Some(&b'0') {
        (8, 0)
    } else {
        (10, 0)
    };
    let digits = rest[digits_offset..]
        .chars()
        .take_while(|ch| ch.is_digit(radix))
        .collect::<String>();
    if digits.is_empty() {
        // No conversion: strtoimax leaves ep == nptr and chk_converror
        // reports the WHOLE original operand (`+ 5: invalid number`).
        return None;
    }
    let consumed = value.len() - body.len() + offset + digits_offset + digits.len();
    Some(IntScan {
        radix,
        negative,
        digits,
        suffixed: consumed < value.len(),
    })
}

fn accumulate_digits(scan: &IntScan, limit: u128) -> (u128, bool) {
    let mut parsed: u128 = 0;
    let mut overflow = false;
    for ch in scan.digits.chars() {
        let digit = ch
            .to_digit(scan.radix)
            .expect("scan collected only radix digits") as u128;
        parsed = match parsed
            .checked_mul(scan.radix as u128)
            .and_then(|n| n.checked_add(digit))
        {
            Some(value) if value <= limit => value,
            _ => {
                overflow = true;
                // strtoimax/strtoumax saturate at the type bound and raise
                // ERANGE; the returned value stays at the bound.
                limit
            }
        };
    }
    (parsed, overflow)
}

fn accumulate_signed(scan: &IntScan) -> (i64, bool) {
    let limit: u128 = if scan.negative {
        1_u128 << 63
    } else {
        (1_u128 << 63) - 1
    };
    let (magnitude, overflow) = accumulate_digits(scan, limit);
    let value = if scan.negative {
        if magnitude == 1_u128 << 63 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    };
    (value, overflow)
}

fn accumulate_unsigned(scan: &IntScan) -> (u64, bool) {
    let (magnitude, overflow) = accumulate_digits(scan, u64::MAX as u128);
    // glibc saturates the magnitude at UINTMAX_MAX and signals ERANGE
    // WITHOUT negating: `printf %u -99999999999999999999` prints
    // 18446744073709551615 (oracle probe). Only an in-range magnitude is
    // negated into uintmax_t (`-1` -> 18446744073709551615).
    let value = if scan.negative && !overflow {
        (magnitude as u64).wrapping_neg()
    } else {
        magnitude as u64
    };
    (value, overflow)
}

/// One strtod subject sequence: the byte range of `value` that C grammar
/// would convert, so the caller can detect the `*ep != '\0'` suffix.
struct FloatSubject {
    start: usize,
    end: usize,
}

fn scan_float_subject(value: &str) -> Option<FloatSubject> {
    let leading = value.len() - skip_leading_space(value).len();
    let body = &value[leading..];
    let bytes = body.as_bytes();
    let mut offset = if matches!(bytes.first(), Some(b'-') | Some(b'+')) {
        1
    } else {
        0
    };
    let rest = &body[offset..];
    let rest_bytes = rest.as_bytes();

    // inf / infinity / nan[(n-char-sequence)] -- C99 7.20.1.3. These are
    // word PREFIXES in glibc strtod: the leading letters are consumed as the
    // subject and any trailing alphabetic run is left to *ep, so `infin`,
    // `infx`, `nann`, `nanx`, `inff`, `nana` all convert to inf/nan while
    // the tail is reported as the unparsed suffix (rubash#871 — the scanner
    // used to require the exact words `inf`/`infinity`/`nan`, so the
    // letter-suffixed forms fell through to the decimal path and printed
    // 0.000000). The parenthesised nan(n-char-sequence) part belongs to the
    // subject only when it closes.
    let word_end = rest_bytes
        .iter()
        .position(|byte| !byte.is_ascii_alphabetic())
        .unwrap_or(rest_bytes.len());
    let lower_word = rest[..word_end].to_ascii_lowercase();
    // glibc strtod tries `infinity` as a WHOLE word first and falls back to
    // the `inf` prefix when the letters run past it (`infinitY` is `inf` +
    // the suffix `initY`, NOT infinity — the oracle reports
    // `infinitY: invalid number`). `nan` is always a bare prefix.
    let prefix_len = if lower_word == "infinity" {
        Some("infinity".len())
    } else if lower_word.starts_with("inf") {
        Some("inf".len())
    } else if lower_word.starts_with("nan") {
        Some("nan".len())
    } else {
        None
    };
    if let Some(prefix_len) = prefix_len {
        // For the nan family, an immediately-following `(n-char-sequence)`
        // that closes is also part of the subject (C99 nan(n-char-seq)).
        let is_nan = lower_word.starts_with("nan");
        let mut end = leading + offset + prefix_len;
        if is_nan {
            if let Some(open) = rest.as_bytes().get(word_end).filter(|b| **b == b'(') {
                let _ = open;
                let close = rest_bytes[word_end..]
                    .iter()
                    .position(|byte| *byte == b')')
                    .map(|position| word_end + position);
                if let Some(close) = close {
                    end = leading + offset + close + 1;
                }
            }
        }
        return Some(FloatSubject { start: 0, end });
    }

    // Hexadecimal significand: 0x/0X plus at least one hex digit with the
    // single dot allowed on either side of it (`0x1`, `0x.8p1`, `0x1.`).
    // Without a hex digit the x is a suffix on the octal `0` (`0xg` -> 0.0
    // + "invalid hex number").
    let hex_digits_end = if matches!(rest_bytes.get(..2), Some(b"0x") | Some(b"0X")) {
        let mut index = 2;
        let mut any = false;
        let mut seen_dot = false;
        while let Some(byte) = rest_bytes.get(index) {
            if byte.is_ascii_hexdigit() {
                any = true;
                index += 1;
            } else if *byte == b'.' && !seen_dot {
                seen_dot = true;
                index += 1;
            } else {
                break;
            }
        }
        any.then_some(index)
    } else {
        None
    };

    if let Some(digits_end) = hex_digits_end {
        let mut end = leading + offset + digits_end;
        // Optional binary exponent: p/P [sign] digit+ ; without a digit the
        // p stays a suffix.
        if matches!(bytes.get(end - leading), Some(b'p') | Some(b'P')) {
            let mut exp = end - leading + 1;
            if matches!(bytes.get(exp), Some(b'-') | Some(b'+')) {
                exp += 1;
            }
            if bytes.get(exp).is_some_and(u8::is_ascii_digit) {
                while bytes.get(exp).is_some_and(u8::is_ascii_digit) {
                    exp += 1;
                }
                end = leading + exp;
            }
        }
        return Some(FloatSubject { start: 0, end });
    }

    // Decimal significand: digits with an optional single dot, then an
    // optional e/E exponent that needs at least one digit (`5e` -> subject
    // `5`, suffix `e`).
    let mut index = 0_usize;
    let mut any_digits = false;
    while rest_bytes.get(index).is_some_and(u8::is_ascii_digit) {
        any_digits = true;
        index += 1;
    }
    if rest_bytes.get(index) == Some(&b'.') {
        index += 1;
        while rest_bytes.get(index).is_some_and(u8::is_ascii_digit) {
            any_digits = true;
            index += 1;
        }
    }
    if !any_digits {
        return None;
    }
    if matches!(rest_bytes.get(index), Some(b'e') | Some(b'E')) {
        let mut exp = index + 1;
        if matches!(rest_bytes.get(exp), Some(b'-') | Some(b'+')) {
            exp += 1;
        }
        if rest_bytes.get(exp).is_some_and(u8::is_ascii_digit) {
            while rest_bytes.get(exp).is_some_and(u8::is_ascii_digit) {
                exp += 1;
            }
            index = exp;
        }
    }
    Some(FloatSubject {
        start: 0,
        end: leading + offset + index,
    })
}

fn convert_float_subject(subject: &str) -> f64 {
    let body = skip_leading_space(subject);
    let negative = body.starts_with('-');
    let unsigned = body.trim_start_matches(['-', '+']);
    let lower = unsigned.to_ascii_lowercase();

    let value = if let Some(digits) = lower.strip_prefix("0x") {
        hex_significand_to_f64(digits)
    } else if lower.starts_with("infinity") || lower.starts_with("inf") {
        f64::INFINITY
    } else if lower.starts_with("nan") {
        f64::NAN
    } else {
        unsigned.parse::<f64>().unwrap_or(0.0)
    };
    if negative {
        -value
    } else {
        value
    }
}

/// Convert the hex digit run (integer digits, an optional dot, fraction
/// digits) into an f64. Mantissa bits beyond f64's 53-bit precision round
/// through the u128 -> f64 conversion; the binary exponent is applied as
/// exact power-of-two scaling. ERANGE is deliberately not modelled: the
/// oracle's conversion goes through strtofltmax into long double, so even
/// `1e999`-scale decimal input produces no diagnostic (WSL GNU bash 5.3.0
/// probe: `printf '%f\n' 1e999` -> the expanded value, rc=0).
fn hex_significand_to_f64(digits: &str) -> f64 {
    let mut mantissa: u128 = 0;
    let mut fraction_digits: i32 = 0;
    let mut seen_dot = false;
    for ch in digits.chars().take_while(|ch| *ch != 'p' && *ch != 'P') {
        if ch == '.' {
            seen_dot = true;
            continue;
        }
        let digit = ch.to_digit(16).unwrap_or(0) as u128;
        if mantissa < (1_u128 << 120) {
            mantissa = mantissa * 16 + digit;
        }
        if seen_dot {
            fraction_digits += 4;
        }
    }
    let mut exponent: i32 = 0;
    if let Some(position) = digits.find(['p', 'P']) {
        let parsed = digits[position + 1..].trim().parse::<i32>().unwrap_or(0);
        exponent = parsed;
    }
    let mut value = mantissa as f64;
    let mut shift = exponent - fraction_digits;
    // Scale in bounded steps so intermediate products stay finite; powers
    // of two are exact in binary floating point.
    while shift > 0 {
        let step = shift.min(1023);
        value *= 2_f64.powi(step);
        shift -= step;
    }
    while shift < 0 {
        let step = (-shift).min(1022);
        value /= 2_f64.powi(step);
        shift += step;
    }
    value
}
