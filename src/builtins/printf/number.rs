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
        Some(value) => ParsedNumber {
            value,
            invalid: None,
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

pub(super) fn invalid_number_error(value: &str) -> String {
    format!("rubash: printf: {value}: invalid number")
}

fn printf_char_constant(value: &str) -> Option<char> {
    let mut chars = value.chars();
    match chars.next() {
        Some('\'') | Some('"') => chars.next(),
        _ => None,
    }
}

fn parse_integer_literal(value: &str) -> Option<i64> {
    let value = value.trim();
    let (sign, digits) = match value.as_bytes().first().copied() {
        Some(b'-') => (-1_i64, &value[1..]),
        Some(b'+') => (1_i64, &value[1..]),
        _ => (1_i64, value),
    };

    let parsed = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).ok()?
    } else if digits.len() > 1 && digits.starts_with('0') {
        i64::from_str_radix(&digits[1..], 8).ok()?
    } else {
        digits.parse::<i64>().ok()?
    };
    Some(sign * parsed)
}
