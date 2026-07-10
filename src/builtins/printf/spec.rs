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
        spec.width = width;
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
            spec.precision = Some(precision.unwrap_or(0));
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

    while let Some(length @ ('h' | 'j' | 'l' | 'L' | 't' | 'z')) = chars.peek().copied() {
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
        's' | 'b'
            | 'q'
            | 'Q'
            | 'c'
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
