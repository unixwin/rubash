use super::longdouble::{
    decimal_digit_char, exact_parts, hex_digit_char, mantissa_nibbles, FloatVal,
};
use super::FormatSpec;

/// Format a printf float operand, already converted through the 80-bit
/// extended format (GNU printf.def:208-213 floatmax_t, printf.def:774-798
/// float conversions via getfloatmax with the `L' modifier, so the C
/// library formats a long double).
///
/// Every finite engine works from the operand's EXACT decimal expansion
/// (`exact_parts`: binary fractions expand finitely) and rounds once to
/// the requested digits with round-half-even — the C library converts the
/// long double correctly rounded under the default rounding mode, so
/// ties break towards even (oracle: `%.0f` of 0.5/1.5/2.5 -> 0/2/2).
pub(super) fn format_float(value: FloatVal, spec: &FormatSpec, mode: char) -> String {
    // glibc printf renders non-finite values as inf/nan spelled per
    // conversion case (%f/%e/%g/%a -> "inf"/"nan", %F/%E/%G/%A -> "INF"/
    // "NAN"), keeps the sign bit as a literal minus ("-nan"), ignores
    // precision and the alternate-form decimal point, and still applies
    // the '+/space sign flags ("%+f" inf -> "+inf").
    let mut rendered = match value {
        FloatVal::Infinite { neg } | FloatVal::Nan { neg } => {
            let uppercase = matches!(mode, 'E' | 'F' | 'G' | 'A');
            let word = match value {
                FloatVal::Nan { .. } if uppercase => "NAN",
                FloatVal::Nan { .. } => "nan",
                _ if uppercase => "INF",
                _ => "inf",
            };
            if neg {
                format!("-{word}")
            } else {
                word.to_string()
            }
        }
        FloatVal::Finite { neg, mant, exp } => {
            let body = match mode {
                'e' | 'E' => format_exp(mant, exp, spec.precision.unwrap_or(6), mode == 'E'),
                'g' | 'G' => format_general(
                    mant,
                    exp,
                    spec.precision.unwrap_or(6).max(1),
                    mode == 'G',
                    spec.alternate_form,
                ),
                'a' | 'A' => format_hex_ld(mant, exp, spec, mode == 'A'),
                _ => format_fixed(mant, exp, spec.precision.unwrap_or(6)),
            };
            if neg {
                format!("-{body}")
            } else {
                body
            }
        }
    };

    let nonfinite = !matches!(value, FloatVal::Finite { .. });
    if spec.alternate_form && !nonfinite && matches!(mode, 'f' | 'e' | 'E') {
        ensure_float_decimal_point(&mut rendered);
    }

    // GNU snprintf.c:439-445: replace `.` with localeconv()->decimal_point[0]
    // so `printf '%.4f' 1` outputs `1,0000` in de_DE.UTF-8.
    let dp = crate::locale::decimal_point();
    if dp != '.' {
        if let Some(idx) = rendered.find('.') {
            rendered.replace_range(idx..idx + 1, &dp.to_string());
        }
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

/// Round the digit run in place, keeping the first `keep` digits and
/// folding `digits[keep..]` plus the `sticky` bit (value dropped before
/// this call, e.g. a capped digit window) into the decision, breaking
/// ties towards even. The caller guarantees at least `keep + 1` digits.
/// A carry past the most significant digit inserts a leading 1, so the
/// caller detects overflow as `digits.len() > keep` and renormalizes.
fn round_half_even(digits: &mut Vec<u8>, keep: usize, sticky: bool) {
    debug_assert!(digits.len() > keep);
    let guard = digits[keep];
    let rest_nonzero = sticky || digits[keep + 1..].iter().any(|&d| d != 0);
    let kept_odd = keep > 0 && digits[keep - 1] % 2 == 1;
    let round_up = guard > 5 || (guard == 5 && (rest_nonzero || kept_odd));
    digits.truncate(keep);
    if round_up {
        let mut i = keep;
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] < 9 {
                digits[i] += 1;
                break;
            }
            digits[i] = 0;
        }
    }
}

/// The `precision` significant decimal digits of `mant * 2^exp` (rounded
/// once, half-even) plus `x`, the decimal exponent of the first digit.
/// `None` is an exact zero. The returned run holds exactly `p` digits.
fn significant_digits(mant: u64, exp: i32, precision: usize) -> Option<(Vec<u8>, i64)> {
    if mant == 0 {
        return None;
    }
    let p = precision.max(1);
    let (int_s, frac_s) = exact_parts(mant, exp);
    let mut digits: Vec<u8> = Vec::new();
    let mut x: i64;
    if int_s != "0" {
        x = int_s.len() as i64 - 1;
        digits.extend(int_s.bytes().map(|b| b - b'0'));
        digits.extend(frac_s.bytes().map(|b| b - b'0'));
    } else {
        let zeros = frac_s.bytes().take_while(|&b| b == b'0').count();
        x = -(zeros as i64) - 1;
        digits.extend(frac_s[zeros..].bytes().map(|b| b - b'0'));
    }
    // Beyond the guard digit the exact tail folds into the sticky bit.
    let sticky = digits.len() > p + 1 && digits[p + 1..].iter().any(|&d| d != 0);
    digits.resize(p + 1, 0);
    round_half_even(&mut digits, p, sticky);
    if digits.len() > p {
        digits.truncate(p);
        x += 1;
    }
    Some((digits, x))
}

/// `%f`/`%F`: full integer part plus exactly `precision` fraction digits.
fn format_fixed(mant: u64, exp: i32, precision: usize) -> String {
    let (int_s, frac_s) = exact_parts(mant, exp);
    let mut digits: Vec<u8> = Vec::with_capacity(int_s.len() + precision + 1);
    digits.extend(int_s.bytes().map(|b| b - b'0'));
    digits.extend(frac_s.bytes().map(|b| b - b'0'));
    // Exact expansion, so the tail past the guard digit is all zeros.
    digits.resize(int_s.len() + precision + 1, 0);
    round_half_even(&mut digits, int_s.len() + precision, false);
    let int_len = digits.len() - precision;
    let mut rendered = digits[..int_len]
        .iter()
        .map(|d| decimal_digit_char(*d))
        .collect::<String>();
    if precision > 0 {
        rendered.push('.');
        rendered.extend(digits[int_len..].iter().map(|d| decimal_digit_char(*d)));
    }
    rendered
}

/// `%e`/`%E`: one leading digit, `precision` fraction digits, `e+-XX`.
fn format_exp(mant: u64, exp: i32, precision: usize, uppercase: bool) -> String {
    // One extra significant digit feeds the rounding; only `precision`
    // fraction digits render.
    let Some((digits, x)) = significant_digits(mant, exp, precision + 1) else {
        let mut rendered = String::from("0");
        if precision > 0 {
            rendered.push('.');
            rendered.extend(std::iter::repeat('0').take(precision));
        }
        rendered.push_str(&format_exp_suffix(0, uppercase));
        return rendered;
    };
    let mut rendered = decimal_digit_char(digits[0]).to_string();
    if precision > 0 {
        rendered.push('.');
        rendered.extend(digits[1..=precision].iter().map(|d| decimal_digit_char(*d)));
    }
    rendered.push_str(&format_exp_suffix(x, uppercase));
    rendered
}

/// The `e+-XX` suffix: sign always, at least two exponent digits.
fn format_exp_suffix(x: i64, uppercase: bool) -> String {
    let marker = if uppercase { 'E' } else { 'e' };
    let sign = if x < 0 { '-' } else { '+' };
    let mag = x.unsigned_abs().to_string();
    if mag.len() < 2 {
        format!("{marker}{sign}0{mag}")
    } else {
        format!("{marker}{sign}{mag}")
    }
}

/// `%g`/`%G`: `precision` significant digits, `%e` style when the decimal
/// exponent is below -4 or at/above the precision, else `%f` style; the
/// style decision uses the ROUNDED exponent, so a carry can flip it
/// (oracle: `%.2g` of 99.9 -> `1e+02`).
fn format_general(
    mant: u64,
    exp: i32,
    precision: usize,
    uppercase: bool,
    alternate: bool,
) -> String {
    let Some((digits, x)) = significant_digits(mant, exp, precision) else {
        // Oracle: `%g` of 0 -> `0`, `%#g` of 0 -> `0.00000` (precision 6).
        if alternate {
            let mut rendered = String::from("0.");
            rendered.extend(std::iter::repeat('0').take(precision - 1));
            return rendered;
        }
        return String::from("0");
    };
    let mut rendered = if x < -4 || x >= precision as i64 {
        let mut body = decimal_digit_char(digits[0]).to_string();
        if precision > 1 {
            body.push('.');
            body.extend(digits[1..].iter().map(|d| decimal_digit_char(*d)));
        }
        body.push_str(&format_exp_suffix(x, uppercase));
        body
    } else if x >= 0 {
        let int_len = x as usize + 1;
        let mut body: String = digits[..int_len]
            .iter()
            .map(|d| decimal_digit_char(*d))
            .collect();
        if digits.len() > int_len {
            body.push('.');
            body.extend(digits[int_len..].iter().map(|d| decimal_digit_char(*d)));
        }
        body
    } else {
        let mut body = String::from("0.");
        body.extend(std::iter::repeat('0').take(-(x + 1) as usize));
        body.extend(digits.iter().map(|d| decimal_digit_char(*d)));
        body
    };
    if alternate {
        ensure_general_alternate_form(&mut rendered, precision);
    } else {
        trim_general_trailing_zeroes(&mut rendered);
    }
    rendered
}

/// `%a`/`%A`: the 64-bit long-double mantissa as hex nibbles (glibc `%La`
/// style) — point after the first nibble, trailing-zero nibbles stripped
/// unless a precision is given, exponent `D = exp + 60` so that
/// `lead.rest * 2^D` is the value (oracle: `%a` of 5 -> `0xap-1`,
/// `%.2a` of 4.2 -> `0x8.66p-1`).
fn format_hex_ld(mant: u64, exp: i32, spec: &FormatSpec, uppercase: bool) -> String {
    let (prefix, exponent_marker) = if uppercase { ("0X", 'P') } else { ("0x", 'p') };
    let hex = |nibble: u8| hex_digit_char(nibble, uppercase);
    let nibbles = mantissa_nibbles(mant);
    let mut lead = nibbles[0];
    let mut frac: Vec<u8> = nibbles[1..].to_vec();
    // D is guard-bounded: exp in [-16445, ~16321] keeps D in i64 range.
    // An exact zero prints exponent +0 (glibc %La of 0.0L -> 0x0p+0).
    let mut d = if mant == 0 { 0 } else { exp as i64 + 60 };
    match spec.precision {
        None => {
            while frac.last() == Some(&0) {
                frac.pop();
            }
        }
        Some(n) => {
            if frac.len() > n {
                let guard = frac[n];
                let sticky = frac[n + 1..].iter().any(|&x| x != 0);
                let kept_odd = if n == 0 {
                    lead % 2 == 1
                } else {
                    frac[n - 1] % 2 == 1
                };
                let round_up = guard > 8 || (guard == 8 && (sticky || kept_odd));
                frac.truncate(n);
                if round_up {
                    let mut carry = true;
                    for slot in frac.iter_mut().rev() {
                        if !carry {
                            break;
                        }
                        if *slot < 0xF {
                            *slot += 1;
                            carry = false;
                        } else {
                            *slot = 0;
                        }
                    }
                    if carry {
                        lead += 1;
                        if lead == 0x10 {
                            // Carried past the leading nibble:
                            // renormalize (0x1.0 * 2^(D+4)).
                            lead = 1;
                            frac = vec![0; n];
                            d += 4;
                        }
                    }
                }
            } else {
                while frac.len() < n {
                    frac.push(0);
                }
            }
        }
    }
    let mut rendered = format!("{prefix}{}", hex(lead));
    if !frac.is_empty() || spec.alternate_form || spec.precision.unwrap_or(0) > 0 {
        rendered.push('.');
        rendered.extend(frac.iter().map(|&nibble| hex(nibble)));
    }
    rendered.push(exponent_marker);
    if d >= 0 {
        rendered.push('+');
    }
    rendered.push_str(&d.to_string());
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
