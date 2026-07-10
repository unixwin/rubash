use super::escape::{expand_percent_b, shell_quote};
use super::float::format_float;
use super::number::{invalid_number_error, parse_f64, parse_i64};
use super::FormatSpec;

pub(super) fn format_value(value: &str, spec: &FormatSpec) -> (String, bool, Option<String>) {
    let mut stop_output = false;
    let mut invalid_number = None;
    let rendered = match spec.specifier {
        's' => truncate_precision(value.to_string(), spec.precision),
        'b' => {
            let (expanded, stop) = expand_percent_b(value);
            stop_output = stop;
            truncate_precision(expanded, spec.precision)
        }
        'q' => truncate_precision(shell_quote(value), spec.precision),
        'Q' => shell_quote(&truncate_precision(value.to_string(), spec.precision)),
        'c' => value.chars().next().unwrap_or('\0').to_string(),
        'd' | 'i' => {
            let parsed = parse_i64(value);
            invalid_number = parsed.invalid;
            format_signed_integer(parsed.value, spec)
        }
        'u' => {
            let parsed = parse_i64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value as u64, 10, false, spec)
        }
        'x' => {
            let parsed = parse_i64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value as u64, 16, false, spec)
        }
        'X' => {
            let parsed = parse_i64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value as u64, 16, true, spec)
        }
        'o' => {
            let parsed = parse_i64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value as u64, 8, false, spec)
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
        apply_width(rendered, &width_spec),
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
