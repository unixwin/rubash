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
    match parse_integer_literal(value) {
        Some((parsed_value, issue)) => ParsedNumber {
            value: parsed_value,
            invalid: issue.map(|issue| match issue {
                IntegerIssue::InvalidSuffix => value.to_string(),
                IntegerIssue::Overflow => format!("__rubash_printf_overflow__:{value}"),
            }),
        },
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
    match parse_unsigned_integer_literal(value) {
        Some((parsed_value, issue)) => ParsedNumber {
            value: parsed_value,
            invalid: issue.map(|issue| match issue {
                IntegerIssue::InvalidSuffix => value.to_string(),
                IntegerIssue::Overflow => format!("__rubash_printf_overflow__:{value}"),
            }),
        },
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
    match value.parse::<f64>() {
        Ok(value) => ParsedNumber {
            value,
            invalid: None,
        },
        Err(_) => ParsedNumber {
            value: 0.0,
            invalid: Some(value.to_string()),
        },
    }
}

/// GNU printf.def:834-838 printf_erange()/1391-1398 chk_converror(): the
/// diagnostic is a builtin_error body (no "warning:") naming the offending
/// operand, and conversion_error makes the builtin exit status 1. The body
/// carries no shell prefix -- render_one_pass adds the `script: line N:`
/// prologue from builtin_error's error_prolog.
pub(super) fn invalid_number_error(value: &str) -> String {
    if let Some(value) = value.strip_prefix("__rubash_printf_overflow__:") {
        format!("printf: {value}: Numerical result out of range")
    } else if value.starts_with("0x") || value.starts_with("0X") {
        format!("printf: {value}: invalid hex number")
    } else if value.len() > 1
        && value.starts_with('0')
        && value
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'8' | b'9'))
    {
        format!("printf: {value}: invalid octal number")
    } else {
        format!("printf: {value}: invalid number")
    }
}

fn printf_char_constant(value: &str) -> Option<char> {
    let mut chars = value.chars();
    match chars.next() {
        Some('\'') | Some('"') => chars.next(),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum IntegerIssue {
    InvalidSuffix,
    Overflow,
}

fn parse_unsigned_integer_literal(value: &str) -> Option<(u64, Option<IntegerIssue>)> {
    let value = value.trim();
    let (negative, digits) = match value.as_bytes().first().copied() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (radix, digits) = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        (16, hex)
    } else if digits.len() > 1 && digits.starts_with('0') {
        (8, digits)
    } else {
        (10, digits)
    };
    let prefix_len = digits
        .char_indices()
        .take_while(|(_, ch)| ch.to_digit(radix).is_some())
        .map(|(index, ch)| index + ch.len_utf8())
        .last()
        .unwrap_or(0);
    if prefix_len == 0 {
        return None;
    }
    let mut parsed = 0_u64;
    let mut overflow = false;
    for ch in digits[..prefix_len].chars() {
        let digit = ch
            .to_digit(radix)
            .expect("prefix contains only radix digits") as u64;
        parsed = match parsed
            .checked_mul(radix as u64)
            .and_then(|n| n.checked_add(digit))
        {
            Some(value) => value,
            None => {
                overflow = true;
                u64::MAX
            }
        };
    }
    let number = if negative && !overflow {
        0_u64.wrapping_sub(parsed)
    } else {
        parsed
    };
    let issue = if prefix_len != digits.len() {
        Some(IntegerIssue::InvalidSuffix)
    } else if overflow {
        Some(IntegerIssue::Overflow)
    } else {
        None
    };
    Some((number, issue))
}

fn parse_integer_literal(value: &str) -> Option<(i64, Option<IntegerIssue>)> {
    let value = value.trim();
    let (negative, digits) = match value.as_bytes().first().copied() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };

    let (radix, digits) = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        (16, hex)
    } else if digits.len() > 1 && digits.starts_with('0') {
        (8, digits)
    } else {
        (10, digits)
    };

    // Bash consumes the valid prefix, saturates integer overflow, and reports
    // a warning without turning the printf status into failure.
    let prefix_len = digits
        .char_indices()
        .take_while(|(_, ch)| ch.to_digit(radix).is_some())
        .map(|(index, ch)| index + ch.len_utf8())
        .last()
        .unwrap_or(0);
    if prefix_len == 0 {
        return None;
    }

    let prefix = &digits[..prefix_len];
    let limit = if negative {
        (i64::MAX as u128) + 1
    } else {
        i64::MAX as u128
    };
    let mut parsed = 0_u128;
    let mut overflow = false;
    for ch in prefix.chars() {
        let digit = ch
            .to_digit(radix)
            .expect("prefix contains only radix digits") as u128;
        parsed = match parsed
            .checked_mul(radix as u128)
            .and_then(|n| n.checked_add(digit))
        {
            Some(value) if value <= limit => value,
            _ => {
                overflow = true;
                limit
            }
        };
    }

    let number = if negative {
        if parsed == (i64::MAX as u128) + 1 {
            i64::MIN
        } else {
            -(parsed as i64)
        }
    } else {
        parsed as i64
    };
    let issue = if overflow {
        Some(IntegerIssue::Overflow)
    } else if prefix_len != digits.len() {
        Some(IntegerIssue::InvalidSuffix)
    } else {
        None
    };
    Some((number, issue))
}
