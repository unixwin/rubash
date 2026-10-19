use super::longdouble::{parse_long_double_subject, FloatVal};
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

/// GNU printf.def:208-213 floatmax_t/strtofltmax (strtold on this
/// platform), printf.def:774-798 float conversions via getfloatmax, and
/// printf.def:1387-1398 chk_converror: the operand converts through the
/// 80-bit extended format, and a fully-consumed subject that overflows (or
/// underflows inexactly) raises ERANGE. A trailing suffix beats ERANGE
/// (`1e9999x` is a plain invalid number, not an overflow).
pub(super) fn parse_long_double(value: &str) -> ParsedNumber<FloatVal> {
    if let Some(ch) = printf_char_constant(value) {
        return ParsedNumber {
            value: longdouble_char_constant(ch),
            invalid: None,
        };
    }
    if value.is_empty() {
        return ParsedNumber {
            value: FloatVal::zero(),
            invalid: None,
        };
    }
    match scan_float_subject(value) {
        Some(subject) => {
            let end = subject.end;
            let (parsed, erange) = parse_long_double_subject(&value[subject.start..end]);
            // printf.def:1472-1495 getdouble(): strtod accumulates the
            // longest valid subject sequence and getdouble RETURNS it even
            // when chk_converror rejects the trailing bytes, so `printf %f
            // '5 '` prints 5.000000 (status 1) instead of 0.000000.
            ParsedNumber {
                value: parsed,
                invalid: if end < value.len() {
                    Some(value.to_string())
                } else if erange {
                    Some(format!("__rubash_printf_overflow__:{value}"))
                } else {
                    None
                },
            }
        }
        None => ParsedNumber {
            value: FloatVal::zero(),
            invalid: Some(value.to_string()),
        },
    }
}

/// printf.def:1519-1544 asciicode(): a quote operand yields arg[1]
/// converted with mbrtowc — a single BMP char is its own code point, so a
/// quoted char is the exact small integer of that code point.
fn longdouble_char_constant(ch: char) -> FloatVal {
    FloatVal::from_small_int(ch as u32 as u64, false)
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

    // inf / infinity / nan[(n-char-sequence)] -- C99 7.20.1.3; the parenthesised
    // part belongs to the subject only when it closes.
    let word_end = rest_bytes
        .iter()
        .position(|byte| !byte.is_ascii_alphabetic())
        .unwrap_or(rest_bytes.len());
    match rest[..word_end].to_ascii_lowercase().as_str() {
        "inf" | "infinity" | "nan" => {
            let mut end = leading + offset + word_end;
            if rest.as_bytes().get(word_end) == Some(&b'(') {
                let close = rest_bytes[word_end..]
                    .iter()
                    .position(|byte| *byte == b')')
                    .map(|position| word_end + position);
                if let Some(close) = close {
                    end = leading + offset + close + 1;
                }
            }
            return Some(FloatSubject { start: 0, end });
        }
        _ => {}
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
