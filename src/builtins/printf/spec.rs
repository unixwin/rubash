use super::number::{invalid_number_error, parse_i64};
use super::{next_arg, FormatSpec, ParsedFormat, ParsedNumber};

pub(super) fn parse_format_spec<I>(chars: &mut std::iter::Peekable<I>) -> ParsedFormat
where
    I: Iterator<Item = char>,
{
    let mut spec = FormatSpec::default();
    let mut raw = String::from("%");

    while let Some(flag) = chars.peek().copied() {
        match flag {
            '-' => spec.left_adjust = true,
            '0' => spec.zero_pad = true,
            '#' => spec.alternate_form = true,
            '+' => spec.explicit_sign = true,
            ' ' => spec.leading_space_sign = true,
            '\'' => {}
            _ => break,
        }
        raw.push(flag);
        chars.next();
    }

    if chars.peek() == Some(&'*') {
        chars.next();
        raw.push('*');
        spec.width_from_arg = true;
    } else {
        let (width, digits) = read_usize_with_digits(chars);
        raw.push_str(&digits);
        // GNU printf.def:897-901 decodeint(): inline field width is an
        // int; values outside i32 range return 0 (overflow_retval). Clamp
        // to avoid huge padding allocation (issue #90).
        match width.and_then(|w| i32::try_from(w).ok()) {
            Some(w) => spec.width = Some(w as usize),
            None if width.is_some() => {
                spec.width = Some(0);
                spec.inline_width_overflow = true;
                spec.width_digits = Some(digits);
            }
            None => {}
        }
    }
    if chars.peek() == Some(&'.') {
        chars.next();
        raw.push('.');
        if chars.peek() == Some(&'*') {
            chars.next();
            raw.push('*');
            spec.precision_from_arg = true;
        } else {
            let (precision, digits) = read_usize_with_digits(chars);
            raw.push_str(&digits);
            // GNU printf.def:912-918 decodeint(): inline precision is an
            // int; values outside i32 range return -1 (overflow_retval,
            // meaning "no precision"). A missing digit string is treated
            // as precision 0.
            match precision.and_then(|p| i32::try_from(p).ok()) {
                Some(p) => spec.precision = Some(p as usize),
                None if precision.is_some() => {
                    // Overflow: GNU's printstr adjusts pr back to
                    // `precision` (0 from printf_builtin), yielding
                    // precision 0, not -1. The %Q adjustment in
                    // printf_builtin instead decodes to -1 (no truncation)
                    // and re-derives the precision from the quoted length,
                    // so the overflow flag -- not this clamp -- drives it.
                    spec.precision = Some(0);
                    spec.inline_precision_overflow = true;
                    spec.precision_digits = Some(digits);
                }
                None => spec.precision = Some(0),
            }
        }
    }

    if chars.peek() == Some(&'(') {
        chars.next();
        raw.push('(');
        let mut time_format = String::new();
        let mut depth = 1;
        for ch in chars.by_ref() {
            raw.push(ch);
            match ch {
                '(' => {
                    depth += 1;
                    time_format.push(ch);
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    time_format.push(ch);
                }
                _ => time_format.push(ch),
            }
        }
        if depth != 0 {
            return ParsedFormat::Missing(raw);
        }
        spec.time_format = Some(time_format);
    }

    // GNU printf.def:472-477: `longform' is set when the `l' length
    // modifier precedes the conversion character; it selects the wide
    // (%ls/%lc) code path whenever the locale is multibyte.
    while let Some(length @ ('h' | 'j' | 'l' | 'L' | 't' | 'z')) = chars.peek().copied() {
        if length == 'l' {
            spec.wide = true;
        }
        raw.push(length);
        chars.next();
    }

    let Some(specifier) = chars.next() else {
        return ParsedFormat::Missing(raw);
    };
    raw.push(specifier);
    spec.specifier = specifier;
    spec.raw = raw;
    ParsedFormat::Spec(spec)
}

pub(super) fn resolve_dynamic_format_args(
    spec: &mut FormatSpec,
    args: &[&str],
    arg_index: &mut usize,
) -> Vec<String> {
    let mut errors = Vec::new();
    if spec.width_from_arg {
        let raw = next_arg(args, arg_index);
        let ParsedNumber {
            value: width,
            invalid,
        } = parse_i64(raw);
        if let Some(invalid) = invalid {
            errors.push(invalid_number_error(&invalid));
        }
        // GNU printf.def:1400-1424 getint(): field width is an int, so
        // values outside i32 range return 0 (the overflow_retval for
        // width) after chk_converror reports the argument with
        // printf_erange (conversion_error -> exit status 1). Without this
        // clamp, a huge width argument causes apply_width to allocate
        // gigabytes of padding, hanging the process (issue #90,
        // printf7.sub overflow tests).
        let width = match i32::try_from(width) {
            Ok(w) => w,
            Err(_) => {
                if !errors.iter().any(|error| error.contains(raw)) {
                    errors.push(invalid_number_error(&format!(
                        "__rubash_printf_overflow__:{raw}"
                    )));
                }
                0
            }
        };
        if width < 0 {
            spec.left_adjust = true;
            spec.width = Some(width.unsigned_abs() as usize);
        } else {
            spec.width = Some(width as usize);
        }
    }

    if spec.precision_from_arg {
        let raw = next_arg(args, arg_index);
        let ParsedNumber {
            value: precision,
            invalid,
        } = parse_i64(raw);
        if let Some(invalid) = invalid {
            errors.push(invalid_number_error(&invalid));
        }
        // GNU printf.def:1400-1424 getint(): precision is an int, so
        // values outside i32 range return -1 (the overflow_retval for
        // precision, meaning "no precision") after the same
        // chk_converror/printf_erange diagnostic.
        let precision = match i32::try_from(precision) {
            Ok(p) => p,
            Err(_) => {
                if !errors.iter().any(|error| error.contains(raw)) {
                    errors.push(invalid_number_error(&format!(
                        "__rubash_printf_overflow__:{raw}"
                    )));
                }
                -1
            }
        };
        spec.precision = (precision >= 0).then_some(precision as usize);
    }
    errors
}

fn read_usize_with_digits<I>(chars: &mut std::iter::Peekable<I>) -> (Option<usize>, String)
where
    I: Iterator<Item = char>,
{
    let mut digits = String::new();
    while let Some(ch) = chars.peek().copied() {
        if !ch.is_ascii_digit() {
            break;
        }
        digits.push(ch);
        chars.next();
    }

    (digits.parse().ok(), digits)
}

pub(super) fn valid_format_specifier(specifier: char) -> bool {
    matches!(
        specifier,
        's' | 'S'
            | 'b'
            | 'q'
            | 'Q'
            | 'c'
            | 'C'
            | 'd'
            | 'i'
            | 'u'
            | 'x'
            | 'X'
            | 'o'
            | 'f'
            | 'F'
            | 'e'
            | 'E'
            | 'g'
            | 'G'
            | 'a'
            | 'A'
            | 'n'
            | 'T'
    )
}
