use super::FormatSpec;

pub(super) fn format_float(value: f64, spec: &FormatSpec, mode: char) -> String {
    let mut rendered = match mode {
        'e' | 'E' => {
            let precision = spec.precision.unwrap_or(6);
            let rendered = if mode == 'E' {
                format!("{value:.precision$E}")
            } else {
                format!("{value:.precision$e}")
            };
            normalize_float_exponent(rendered, mode == 'E')
        }
        'g' | 'G' => format_general_float(value, spec, mode == 'G'),
        'a' | 'A' => format_hex_float(value, spec, mode == 'A'),
        _ => {
            let precision = spec.precision.unwrap_or(6);
            format!("{value:.precision$}")
        }
    };

    if spec.alternate_form && matches!(mode, 'f' | 'e' | 'E') {
        ensure_float_decimal_point(&mut rendered);
    }

    if !rendered.starts_with('-') {
        if spec.explicit_sign {
            rendered.insert(0, '+');
        } else if spec.leading_space_sign {
            rendered.insert(0, ' ');
        }
    }

    rendered
}

fn format_hex_float(value: f64, spec: &FormatSpec, uppercase: bool) -> String {
    let bits = value.to_bits();
    let negative = bits >> 63 != 0;
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction_bits = bits & ((1_u64 << 52) - 1);

    let (prefix, exponent_marker) = if uppercase { ("0X", 'P') } else { ("0x", 'p') };
    let digit_case = if uppercase {
        HexCase::Upper
    } else {
        HexCase::Lower
    };

    if exponent_bits == 0 && fraction_bits == 0 {
        let mut rendered = format!("{prefix}0");
        if spec.alternate_form {
            rendered.push('.');
        }
        rendered.push(exponent_marker);
        rendered.push_str("+0");
        if negative {
            rendered.insert(0, '-');
        }
        return rendered;
    }

    let mut exponent = if exponent_bits == 0 {
        -1022
    } else {
        exponent_bits - 1023
    };
    let mantissa = if exponent_bits == 0 {
        fraction_bits
    } else {
        (1_u64 << 52) | fraction_bits
    };

    let precision = spec.precision;
    let (leading, mut fraction) =
        rounded_hex_mantissa(mantissa, precision, &mut exponent, digit_case);

    if precision.is_none() {
        while fraction.ends_with('0') {
            fraction.pop();
        }
    }

    let mut rendered = format!("{prefix}{leading}");
    if !fraction.is_empty() || spec.alternate_form || precision.unwrap_or(0) > 0 {
        rendered.push('.');
        rendered.push_str(&fraction);
    }
    rendered.push(exponent_marker);
    if exponent >= 0 {
        rendered.push('+');
    }
    rendered.push_str(&exponent.to_string());

    if negative {
        rendered.insert(0, '-');
    }
    rendered
}

#[derive(Clone, Copy)]
enum HexCase {
    Lower,
    Upper,
}

fn rounded_hex_mantissa(
    mantissa: u64,
    precision: Option<usize>,
    exponent: &mut i32,
    digit_case: HexCase,
) -> (char, String) {
    let precision = precision.unwrap_or(13);

    if precision <= 13 {
        let shift = 52 - precision * 4;
        let mut rounded = if shift == 0 {
            mantissa
        } else {
            (mantissa + (1_u64 << (shift - 1))) >> shift
        };
        let overflow = 1_u64 << (precision * 4 + 1);
        if rounded == overflow {
            rounded >>= 1;
            *exponent += 1;
        }

        let leading = hex_digit((rounded >> (precision * 4)) as u8, digit_case);
        let fraction_value = rounded & ((1_u64 << (precision * 4)) - 1);
        let fraction = if precision == 0 {
            String::new()
        } else {
            format_hex_fraction(fraction_value, precision, digit_case)
        };
        return (leading, fraction);
    }

    let leading = hex_digit((mantissa >> 52) as u8, digit_case);
    let mut fraction = format_hex_fraction(mantissa & ((1_u64 << 52) - 1), 13, digit_case);
    fraction.extend(std::iter::repeat('0').take(precision - 13));
    (leading, fraction)
}

fn format_hex_fraction(value: u64, width: usize, digit_case: HexCase) -> String {
    let raw = match digit_case {
        HexCase::Lower => format!("{value:0width$x}"),
        HexCase::Upper => format!("{value:0width$X}"),
    };
    raw
}

fn hex_digit(value: u8, digit_case: HexCase) -> char {
    let digits = match digit_case {
        HexCase::Lower => b"0123456789abcdef",
        HexCase::Upper => b"0123456789ABCDEF",
    };
    digits[value as usize] as char
}

fn format_general_float(value: f64, spec: &FormatSpec, uppercase: bool) -> String {
    let precision = spec.precision.unwrap_or(6).max(1);
    let exponent = decimal_exponent(value);
    let use_exponent = exponent < -4 || exponent >= precision as i32;

    let mut rendered = if use_exponent {
        let exponent_precision = precision.saturating_sub(1);
        let rendered = if uppercase {
            format!("{value:.exponent_precision$E}")
        } else {
            format!("{value:.exponent_precision$e}")
        };
        normalize_float_exponent(rendered, uppercase)
    } else {
        let fractional_precision = (precision as i32 - (exponent + 1)).max(0) as usize;
        format!("{value:.fractional_precision$}")
    };

    if spec.alternate_form {
        ensure_general_alternate_form(&mut rendered, precision);
    } else {
        trim_general_trailing_zeroes(&mut rendered);
    }

    rendered
}

fn decimal_exponent(value: f64) -> i32 {
    if value == 0.0 {
        return 0;
    }
    value.abs().log10().floor() as i32
}

fn normalize_float_exponent(mut rendered: String, uppercase: bool) -> String {
    let marker = if uppercase { 'E' } else { 'e' };
    let Some(index) = rendered.find(marker) else {
        return rendered;
    };

    let mantissa = rendered[..index].to_string();
    let exponent = &rendered[index + 1..];
    let value = exponent.parse::<i32>().unwrap_or_default();
    let sign = if value < 0 { '-' } else { '+' };
    rendered = format!("{mantissa}{marker}{sign}{:02}", value.unsigned_abs());
    rendered
}

fn ensure_float_decimal_point(rendered: &mut String) {
    let exponent_index = rendered.find(['e', 'E']);
    let mantissa_end = exponent_index.unwrap_or(rendered.len());
    if !rendered[..mantissa_end].contains('.') {
        rendered.insert(mantissa_end, '.');
    }
}

fn ensure_general_alternate_form(rendered: &mut String, precision: usize) {
    ensure_float_decimal_point(rendered);

    let exponent_index = rendered.find(['e', 'E']).unwrap_or(rendered.len());
    let mantissa = &rendered[..exponent_index];
    let digits = mantissa.chars().filter(|ch| ch.is_ascii_digit()).count();

    if digits >= precision {
        return;
    }

    let padding: String = std::iter::repeat('0').take(precision - digits).collect();
    rendered.insert_str(exponent_index, &padding);
}

fn trim_general_trailing_zeroes(rendered: &mut String) {
    let exponent_index = rendered.find(['e', 'E']).unwrap_or(rendered.len());
    let exponent = rendered[exponent_index..].to_string();
    let mut mantissa = rendered[..exponent_index].to_string();

    if mantissa.contains('.') {
        while mantissa.ends_with('0') {
            mantissa.pop();
        }
        if mantissa.ends_with('.') {
            mantissa.pop();
        }
    }

    *rendered = format!("{mantissa}{exponent}");
}
