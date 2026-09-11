use super::escape::{expand_percent_b, shell_quote};
use super::float::format_float;
use super::number::{invalid_number_error, parse_f64, parse_i64, parse_u64};
use super::FormatSpec;

pub(super) fn format_value(value: &str, spec: &FormatSpec) -> (String, bool, Option<String>) {
    let mut stop_output = false;
    let mut invalid_number = None;
    // GNU printf measures width and precision in bytes when the locale is
    // single-byte (MB_CUR_MAX == 1): %.2ls of a three-byte character returns
    // two bytes and %lc returns one byte (intl4.sub under LC_CTYPE=C). The
    // numeric specifiers render ASCII in either mode, so only the string forms
    // switch unit.
    let byte_mode =
        !crate::locale::is_multi_byte() && matches!(spec.specifier, 's' | 'b' | 'q' | 'Q' | 'c');
    let rendered = match spec.specifier {
        's' => truncate_precision_locale(value.to_string(), spec.precision, byte_mode),
        'b' => {
            let (expanded, stop) = expand_percent_b(value);
            stop_output = stop;
            truncate_precision_locale(expanded, spec.precision, byte_mode)
        }
        'q' => truncate_precision_locale(shell_quote(value), spec.precision, byte_mode),
        'Q' => shell_quote(&truncate_precision_locale(
            value.to_string(),
            spec.precision,
            byte_mode,
        )),
        'c' => first_char_or_byte(value, byte_mode),
        'd' | 'i' => {
            let parsed = parse_i64(value);
            invalid_number = parsed.invalid;
            format_signed_integer(parsed.value, spec)
        }
        'u' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 10, false, spec)
        }
        'x' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 16, false, spec)
        }
        'X' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 16, true, spec)
        }
        'o' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 8, false, spec)
        }
        'f' | 'F' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, 'f')
        }
        'e' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, 'e')
        }
        'E' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, 'E')
        }
        'g' | 'G' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, spec.specifier)
        }
        'a' | 'A' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, spec.specifier)
        }
        other => {
            let mut fallback = String::from('%');
            fallback.push(other);
            fallback
        }
    };

    let mut width_spec = spec.clone();
    if spec.precision.is_some() && matches!(spec.specifier, 'd' | 'i' | 'u' | 'x' | 'X' | 'o') {
        width_spec.zero_pad = false;
    }
    (
        apply_width_locale(rendered, &width_spec, byte_mode),
        stop_output,
        invalid_number.map(|value| invalid_number_error(&value)),
    )
}

pub(super) fn truncate_precision(value: String, precision: Option<usize>) -> String {
    let Some(precision) = precision else {
        return value;
    };
    value.chars().take(precision).collect()
}

/// Locale-aware precision: character count in a multibyte locale, raw byte
/// count otherwise. Byte truncation is not rewound to a character boundary,
/// which is why a cut can leave a dangling multibyte lead byte behind.
fn truncate_precision_locale(value: String, precision: Option<usize>, byte_mode: bool) -> String {
    let Some(precision) = precision else {
        return value;
    };
    if !byte_mode {
        return value.chars().take(precision).collect();
    }
    let raw = locale_byte_span(&value);
    if raw.len() <= precision {
        return value;
    }
    crate::executor::substitution_metadata::bytes_to_shell_text(&raw[..precision])
}

/// `%c`/`%lc`: the first character, or the first raw byte in a single-byte
/// locale.
fn first_char_or_byte(value: &str, byte_mode: bool) -> String {
    if !byte_mode {
        return value.chars().next().unwrap_or('\0').to_string();
    }
    let raw = locale_byte_span(value);
    if raw.is_empty() {
        return String::new();
    }
    crate::executor::substitution_metadata::bytes_to_shell_text(&raw[..1])
}

/// Locale-aware width: counts bytes in a single-byte locale so the padding
/// reaches the byte target, not the character target.
fn apply_width_locale(value: String, spec: &FormatSpec, byte_mode: bool) -> String {
    let Some(width) = spec.width else {
        return value;
    };

    let len = if byte_mode {
        locale_byte_span(&value).len()
    } else {
        value.chars().count()
    };
    if len >= width {
        return value;
    }

    let pad = width - len;
    let pad_char = if spec.zero_pad && !spec.left_adjust {
        '0'
    } else {
        ' '
    };
    let padding: String = std::iter::repeat(pad_char).take(pad).collect();

    if spec.left_adjust {
        format!("{value}{padding}")
    } else if spec.zero_pad && matches!(value.chars().next(), Some('+' | '-' | ' ')) {
        let mut chars = value.chars();
        let sign = chars.next().unwrap_or_default();
        let rest: String = chars.collect();
        format!("{sign}{padding}{rest}")
    } else {
        format!("{padding}{value}")
    }
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

fn format_unsigned_integer(value: u64, radix: u32, uppercase: bool, spec: &FormatSpec) -> String {
    let mut rendered = match (radix, uppercase) {
        (10, _) => value.to_string(),
        (8, _) => format!("{value:o}"),
        (16, false) => format!("{value:x}"),
        (16, true) => format!("{value:X}"),
        _ => value.to_string(),
    };

    rendered = apply_integer_precision(rendered, value == 0, spec.precision);

    if !spec.alternate_form {
        return rendered;
    }

    match (radix, uppercase) {
        (8, _) if !rendered.starts_with('0') => format!("0{rendered}"),
        (16, false) if value != 0 => format!("0x{rendered}"),
        (16, true) if value != 0 => format!("0X{rendered}"),
        _ => rendered,
    }
}

fn format_signed_integer(value: i64, spec: &FormatSpec) -> String {
    let mut rendered =
        apply_integer_precision(value.unsigned_abs().to_string(), value == 0, spec.precision);
    if value < 0 {
        rendered.insert(0, '-');
    } else if spec.explicit_sign {
        rendered.insert(0, '+');
    } else if spec.leading_space_sign {
        rendered.insert(0, ' ');
    }
    rendered
}

fn apply_integer_precision(mut digits: String, is_zero: bool, precision: Option<usize>) -> String {
    let Some(precision) = precision else {
        return digits;
    };
    if precision == 0 && is_zero {
        return String::new();
    }
    let len = digits.chars().count();
    if len < precision {
        let padding: String = std::iter::repeat('0').take(precision - len).collect();
        digits = format!("{padding}{digits}");
    }
    digits
}

pub(super) fn apply_width(value: String, spec: &FormatSpec) -> String {
    let Some(width) = spec.width else {
        return value;
    };

    let len = value.chars().count();
    if len >= width {
        return value;
    }

    let pad = width - len;
    let pad_char = if spec.zero_pad && !spec.left_adjust {
        '0'
    } else {
        ' '
    };
    let padding: String = std::iter::repeat(pad_char).take(pad).collect();

    if spec.left_adjust {
        format!("{value}{padding}")
    } else if spec.zero_pad && matches!(value.chars().next(), Some('+' | '-' | ' ')) {
        let mut chars = value.chars();
        let sign = chars.next().unwrap_or_default();
        let rest: String = chars.collect();
        format!("{sign}{padding}{rest}")
    } else {
        format!("{padding}{value}")
    }
}
