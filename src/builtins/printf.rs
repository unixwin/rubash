//! `printf` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/printf.def (`printf_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Default)]
struct FormatSpec {
    left_adjust: bool,
    zero_pad: bool,
    alternate_form: bool,
    explicit_sign: bool,
    leading_space_sign: bool,
    width: Option<usize>,
    width_from_arg: bool,
    precision: Option<usize>,
    precision_from_arg: bool,
    specifier: char,
}

#[derive(Debug, Clone)]
struct RenderedPrintf {
    output: String,
    status: i32,
    errors: Vec<String>,
    stop_output: bool,
}

enum ParsedFormat {
    Spec(FormatSpec),
    Missing(String),
}

struct ParsedNumber<T> {
    value: T,
    invalid: Option<String>,
}

/// Execute `printf` with arguments after the command name.
pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut output_var = None;
    let mut index = 0;

    let mut end_options = false;
    if args.get(index) == Some(&"--") {
        index += 1;
        end_options = true;
    }

    if !end_options
        && matches!(args.get(index), Some(option) if option.starts_with('-') && *option != "-v")
    {
        writeln!(stderr, "rubash: printf: {}: invalid option", args[index])?;
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    }

    if args.get(index) == Some(&"-v") {
        let Some(name) = args.get(index + 1) else {
            writeln!(stderr, "rubash: printf: -v: option requires an argument")?;
            return Ok(EX_USAGE);
        };

        if !valid_identifier(name) {
            writeln!(stderr, "rubash: printf: `{}`: not a valid identifier", name)?;
            return Ok(EX_USAGE);
        }

        output_var = Some(*name);
        index += 2;
        if args.get(index) == Some(&"--") {
            index += 1;
            end_options = true;
        }
    }

    if !end_options && matches!(args.get(index), Some(option) if option.starts_with('-')) {
        writeln!(stderr, "rubash: printf: {}: invalid option", args[index])?;
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    }

    let Some(format) = args.get(index) else {
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    };

    let rendered = render(format, &args[index + 1..], env_vars);
    if let Some(name) = output_var {
        env_vars.insert(name.to_string(), rendered.output);
    } else {
        stdout.write_all(rendered.output.as_bytes())?;
    }

    for error in rendered.errors {
        writeln!(stderr, "{error}")?;
    }

    Ok(rendered.status)
}

fn render(format: &str, args: &[&str], env_vars: &mut HashMap<String, String>) -> RenderedPrintf {
    let mut output = String::new();
    let mut arg_index = 0;
    let mut errors = Vec::new();

    if args.is_empty() {
        return render_one_pass(format, args, &mut arg_index, output, env_vars);
    }

    while arg_index < args.len() {
        let before_arg = arg_index;
        let rendered = render_one_pass(format, args, &mut arg_index, output, env_vars);
        output = rendered.output;
        errors.extend(rendered.errors);
        if rendered.stop_output {
            return RenderedPrintf {
                output,
                status: status_from_errors(&errors),
                errors,
                stop_output: true,
            };
        }

        if arg_index == before_arg {
            break;
        }
    }

    RenderedPrintf {
        output,
        status: status_from_errors(&errors),
        errors,
        stop_output: false,
    }
}

fn render_one_pass(
    format: &str,
    args: &[&str],
    arg_index: &mut usize,
    mut output: String,
    env_vars: &mut HashMap<String, String>,
) -> RenderedPrintf {
    let mut chars = format.chars().peekable();
    let mut errors = Vec::new();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => output.push_str(&expand_format_escape(&mut chars)),
            '%' => {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    output.push('%');
                    continue;
                }

                let mut spec = match parse_format_spec(&mut chars) {
                    ParsedFormat::Spec(spec) => spec,
                    ParsedFormat::Missing(format) => {
                        return RenderedPrintf {
                            output,
                            status: EXECUTION_FAILURE,
                            errors: vec![format!(
                                "rubash: printf: `{format}': missing format character"
                            )],
                            stop_output: true,
                        };
                    }
                };

                if !valid_format_specifier(spec.specifier) {
                    return RenderedPrintf {
                        output,
                        status: EXECUTION_FAILURE,
                        errors: vec![format!(
                            "rubash: printf: `{}': invalid format character",
                            spec.specifier
                        )],
                        stop_output: true,
                    };
                };
                errors.extend(resolve_dynamic_format_args(&mut spec, args, arg_index));

                if spec.specifier == 'n' {
                    let name = next_arg(args, arg_index);
                    if valid_identifier(name) {
                        env_vars.insert(name.to_string(), output.chars().count().to_string());
                    }
                } else {
                    let value = next_arg(args, arg_index);
                    let (rendered, stop_output, error) = format_value(value, &spec);
                    if let Some(error) = error {
                        errors.push(error);
                    }
                    output.push_str(&rendered);
                    if stop_output {
                        return RenderedPrintf {
                            output,
                            status: status_from_errors(&errors),
                            errors,
                            stop_output: true,
                        };
                    }
                }
            }
            other => output.push(other),
        }
    }
    RenderedPrintf {
        output,
        status: status_from_errors(&errors),
        errors,
        stop_output: false,
    }
}

fn status_from_errors(errors: &[String]) -> i32 {
    if errors.is_empty() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    }
}

fn next_arg<'a>(args: &'a [&str], arg_index: &mut usize) -> &'a str {
    let value = args.get(*arg_index).copied().unwrap_or("");
    *arg_index += 1;
    value
}

fn parse_format_spec<I>(chars: &mut std::iter::Peekable<I>) -> ParsedFormat
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

    while let Some(length @ ('h' | 'j' | 'l' | 'L' | 't' | 'z')) = chars.peek().copied() {
        raw.push(length);
        chars.next();
    }

    let Some(specifier) = chars.next() else {
        return ParsedFormat::Missing(raw);
    };
    spec.specifier = specifier;
    ParsedFormat::Spec(spec)
}

fn resolve_dynamic_format_args(
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

fn valid_format_specifier(specifier: char) -> bool {
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
            | 'n'
    )
}

fn format_value(value: &str, spec: &FormatSpec) -> (String, bool, Option<String>) {
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

fn truncate_precision(value: String, precision: Option<usize>) -> String {
    let Some(precision) = precision else {
        return value;
    };
    value.chars().take(precision).collect()
}

fn format_float(value: f64, spec: &FormatSpec, mode: char) -> String {
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
        _ => {
            let precision = spec.precision.unwrap_or(6);
            format!("{value:.precision$}")
        }
    };

    if spec.alternate_form && matches!(mode, 'f' | 'e' | 'E') {
        ensure_float_decimal_point(&mut rendered);
    }

    if value >= 0.0 {
        if spec.explicit_sign {
            rendered.insert(0, '+');
        } else if spec.leading_space_sign {
            rendered.insert(0, ' ');
        }
    }

    rendered
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

fn apply_width(value: String, spec: &FormatSpec) -> String {
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

fn parse_i64(value: &str) -> ParsedNumber<i64> {
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

fn parse_f64(value: &str) -> ParsedNumber<f64> {
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

fn invalid_number_error(value: &str) -> String {
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

fn expand_format_escape<I>(chars: &mut std::iter::Peekable<I>) -> String
where
    I: Iterator<Item = char>,
{
    match chars.next() {
        Some('a') => "\x07".to_string(),
        Some('b') => "\x08".to_string(),
        Some('e') | Some('E') => "\x1b".to_string(),
        Some('f') => "\x0c".to_string(),
        Some('n') => "\n".to_string(),
        Some('r') => "\r".to_string(),
        Some('t') => "\t".to_string(),
        Some('v') => "\x0b".to_string(),
        Some('\\') => "\\".to_string(),
        Some('x') => format_escape_codepoint(read_escape_digits(chars, 16, 2), "\\x"),
        Some('u') => format_escape_codepoint(read_escape_digits(chars, 16, 4), "\\u"),
        Some('U') => format_escape_codepoint(read_escape_digits(chars, 16, 8), "\\U"),
        Some('0') => format_escape_codepoint(read_escape_digits(chars, 8, 3).or(Some(0)), ""),
        Some(octal @ '1'..='7') => {
            format_escape_codepoint(read_prefixed_escape_digits(chars, octal, 8, 3), "")
        }
        Some(other) => format!("\\{other}"),
        None => "\\".to_string(),
    }
}

fn format_escape_codepoint(value: Option<u32>, fallback: &str) -> String {
    value
        .and_then(char::from_u32)
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn expand_percent_b(value: &str) -> (String, bool) {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    let mut stop_output = false;

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('c') => {
                stop_output = true;
                break;
            }
            Some('a') => output.push('\x07'),
            Some('b') => output.push('\x08'),
            Some('e') | Some('E') => output.push('\x1b'),
            Some('f') => output.push('\x0c'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('v') => output.push('\x0b'),
            Some('\\') => output.push('\\'),
            Some('x') => {
                push_escape_codepoint(&mut output, read_escape_digits(&mut chars, 16, 2), "\\x")
            }
            Some('u') => {
                push_escape_codepoint(&mut output, read_escape_digits(&mut chars, 16, 4), "\\u")
            }
            Some('U') => {
                push_escape_codepoint(&mut output, read_escape_digits(&mut chars, 16, 8), "\\U")
            }
            Some('0') => {
                let value = read_escape_digits(&mut chars, 8, 3).or(Some(0));
                push_escape_codepoint(&mut output, value, "");
            }
            Some(octal @ '1'..='7') => {
                let value = read_prefixed_escape_digits(&mut chars, octal, 8, 3);
                push_escape_codepoint(&mut output, value, "");
            }
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }

    (output, stop_output)
}

fn read_prefixed_escape_digits<I>(
    chars: &mut std::iter::Peekable<I>,
    first: char,
    radix: u32,
    max: usize,
) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = first.to_string();
    while value.len() < max {
        let Some(ch) = chars.peek().copied() else {
            break;
        };
        if ch.to_digit(radix).is_none() {
            break;
        }
        value.push(ch);
        chars.next();
    }
    u32::from_str_radix(&value, radix).ok()
}

fn read_escape_digits<I>(chars: &mut std::iter::Peekable<I>, radix: u32, max: usize) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = String::new();
    while value.len() < max {
        let Some(ch) = chars.peek().copied() else {
            break;
        };
        if ch.to_digit(radix).is_none() {
            break;
        }
        value.push(ch);
        chars.next();
    }
    if value.is_empty() {
        None
    } else {
        u32::from_str_radix(&value, radix).ok()
    }
}

fn push_escape_codepoint(output: &mut String, value: Option<u32>, fallback: &str) {
    match value.and_then(char::from_u32) {
        Some(ch) => output.push(ch),
        None => output.push_str(fallback),
    }
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    if value == "~" {
        return "\\~".to_string();
    }

    let mut quoted = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '/' | '.' | '-' | ':') {
            quoted.push(ch);
        } else {
            quoted.push('\\');
            quoted.push(ch);
        }
    }
    quoted
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (i32, String, String, HashMap<String, String>) {
        let mut env_vars = HashMap::new();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = execute_with_io(
            args.iter().copied(),
            &mut env_vars,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
            env_vars,
        )
    }

    #[test]
    fn prints_plain_and_escaped_format() {
        assert_eq!(run(&["a\\nb"]).1, "a\nb");
    }

    #[test]
    fn format_string_escapes_match_bash() {
        assert_eq!(run(&["\\045\\x41\\u0042\\101"]).1, "%ABA");
        assert_eq!(run(&["4\\.2 one\\ctwo"]).1, "4\\.2 one\\ctwo");
        assert_eq!(run(&["\\0101"]).1, "A");
    }

    #[test]
    fn invalid_format_characters_fail_like_bash() {
        let (status, stdout, stderr, _) = run(&["ab%Mcd\n"]);

        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "ab");
        assert!(stderr.contains("`M': invalid format character"));

        let (status, stdout, stderr, _) = run(&["%10"]);

        assert_eq!(status, EXECUTION_FAILURE);
        assert!(stdout.is_empty());
        assert!(stderr.contains("`%10': missing format character"));
    }

    #[test]
    fn invalid_options_fail_but_double_dash_allows_dash_format() {
        let (status, stdout, stderr, _) = run(&["-x"]);

        assert_eq!(status, EX_USAGE);
        assert!(stdout.is_empty());
        assert!(stderr.contains("invalid option"));

        let (status, stdout, stderr, _) = run(&["--", "-x"]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "-x");
        assert!(stderr.is_empty());
    }

    #[test]
    fn invalid_numeric_arguments_render_zero_and_fail() {
        let (status, stdout, stderr, _) = run(&[
            "%d|%o|%x|%.2f|%*s|%.*s",
            "z",
            "+",
            "GNU",
            "nope",
            "bad",
            "x",
            "bad",
            "abc",
        ]);

        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "0|0|0|0.00|x|");
        assert!(stderr.contains("z: invalid number"));
        assert!(stderr.contains("+: invalid number"));
        assert!(stderr.contains("GNU: invalid number"));
        assert!(stderr.contains("nope: invalid number"));
        assert_eq!(stderr.matches("bad: invalid number").count(), 2);
    }

    #[test]
    fn numeric_errors_do_not_stop_reused_formats() {
        let (status, stdout, stderr, _) = run(&["%d ", "z", "1"]);

        assert_eq!(status, EXECUTION_FAILURE);
        assert_eq!(stdout, "0 1 ");
        assert!(stderr.contains("z: invalid number"));

        let (status, stdout, stderr, _) = run(&["%d", ""]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "0");
        assert!(stderr.is_empty());
    }

    #[test]
    fn reuses_format_until_arguments_are_consumed() {
        assert_eq!(run(&["%s ", "a", "b"]).1, "a b ");
    }

    #[test]
    fn supports_string_numeric_and_b_formats() {
        assert_eq!(
            run(&["%s:%03d:%x:%b", "x", "7", "15", "a\\nb"]).1,
            "x:007:f:a\nb"
        );
    }

    #[test]
    fn assigns_output_with_v() {
        let (_status, stdout, _stderr, env_vars) = run(&["-v", "NAME", "%s", "value"]);

        assert!(stdout.is_empty());
        assert_eq!(env_vars.get("NAME"), Some(&"value".to_string()));
    }

    #[test]
    fn percent_n_assigns_character_count_without_output() {
        let (_status, stdout, _stderr, env_vars) = run(&["abc%n:%s", "COUNT", "done"]);

        assert_eq!(stdout, "abc:done");
        assert_eq!(env_vars.get("COUNT"), Some(&"3".to_string()));
    }

    #[test]
    fn percent_n_works_with_v_assignment() {
        let (_status, stdout, _stderr, env_vars) = run(&["-v", "OUT", "ab%ncd", "COUNT"]);

        assert!(stdout.is_empty());
        assert_eq!(env_vars.get("OUT"), Some(&"abcd".to_string()));
        assert_eq!(env_vars.get("COUNT"), Some(&"2".to_string()));
    }

    #[test]
    fn supports_dynamic_width_and_precision() {
        assert_eq!(run(&["<%*.*s>", "10", "4", "abcdef"]).1, "<      abcd>");
        assert_eq!(run(&["<%*s>", "-6", "ab"]).1, "<ab    >");
        assert_eq!(run(&["<%.*s>", "-1", "abcdef"]).1, "<abcdef>");
    }

    #[test]
    fn percent_q_uses_backslash_quoting_for_printable_shell_metacharacters() {
        assert_eq!(
            run(&["<%q><%q><%q>", "a b", "this&that", "~"]).1,
            "<a\\ b><this\\&that><\\~>"
        );
    }

    #[test]
    fn percent_q_and_upper_q_apply_precision_like_bash() {
        assert_eq!(run(&["<%.2q><%.2Q>", "a b", "a b"]).1, "<a\\><a\\ >");
    }

    #[test]
    fn percent_b_decodes_numeric_escapes() {
        assert_eq!(
            run(&["%b", "\\01017 \\1017 \\x417 \\u0041"]).1,
            "A7 A7 A7 A"
        );
    }

    #[test]
    fn numeric_formats_accept_bash_character_constants() {
        assert_eq!(
            run(&[
                "%d:%o:%x:%.2f:%d",
                "'string'",
                "\"string\"",
                "'string'",
                "'string'",
                "GNU"
            ])
            .1,
            "115:163:73:115.00:0"
        );
    }

    #[test]
    fn alternate_integer_formats_add_bash_prefixes() {
        assert_eq!(
            run(&["%#o:%#x:%#X:%#o:%#x", "115", "115", "115", "0", "0"]).1,
            "0163:0x73:0X73:0:0"
        );
    }

    #[test]
    fn signed_integer_formats_honor_sign_flags_and_zero_padding() {
        assert_eq!(
            run(&[
                "<%+d><% d><%+5d><%05d><%+05d>",
                "42",
                "42",
                "42",
                "-42",
                "42"
            ])
            .1,
            "<+42>< 42><  +42><-0042><+0042>"
        );
    }

    #[test]
    fn float_formats_honor_sign_flags_and_alternate_decimal_point() {
        assert_eq!(
            run(&[
                "<%+010.0f><% 010.0f><%#4.0f><%#.0f><%+10.0f>",
                "123",
                "123",
                "123",
                "123",
                "123"
            ])
            .1,
            "<+000000123>< 000000123><123.><123.><      +123>"
        );
    }

    #[test]
    fn float_formats_use_bash_default_precision_and_exponents() {
        assert_eq!(
            run(&["<%f><%F><%e><%E>", "4", "4", "4", "4"]).1,
            "<4.000000><4.000000><4.000000e+00><4.000000E+00>"
        );
    }

    #[test]
    fn general_float_formats_match_bash_significant_digits() {
        assert_eq!(
            run(&[
                "<%.4g><%.4g><%.4g><%.4g><%.4G><%6.2g><%6.2G>",
                "12345",
                "0.00012345",
                "123.44",
                "0",
                "12345",
                "4.2",
                "4.2"
            ])
            .1,
            "<1.234e+04><0.0001234><123.4><0><1.234E+04><   4.2><   4.2>"
        );
    }

    #[test]
    fn alternate_general_float_formats_keep_decimal_zeroes() {
        assert_eq!(
            run(&["<%#.0g><%#.4g><%#.4e><%#.0e>", "4", "123.44", "4", "4"]).1,
            "<4.><123.4><4.0000e+00><4.e+00>"
        );
    }

    #[test]
    fn integer_formats_parse_bash_numeric_bases() {
        assert_eq!(
            run(&[
                "%d:%d:%d:%i:%u:%x:<%*s>",
                "0x1a",
                "032",
                "-010",
                "010",
                "0x10",
                "032",
                "010",
                "x"
            ])
            .1,
            "26:26:-8:8:16:1a:<       x>"
        );
    }

    #[test]
    fn integer_formats_apply_precision_like_bash() {
        assert_eq!(
            run(&[
                "<%.5d><%8.5d><%08.5d><%.0d><%+.0d><% .0d><%#.5o><%#.5x><%#.0o>",
                "42",
                "42",
                "42",
                "0",
                "0",
                "0",
                "9",
                "26",
                "0"
            ])
            .1,
            "<00042><   00042><   00042><><+>< ><00011><0x0001a><0>"
        );
    }

    #[test]
    fn percent_b_backslash_c_stops_all_output() {
        assert_eq!(run(&["<%b>tail\n", "a\\cB"]).1, "<a");
        assert_eq!(run(&["X%bY%sZ\n", "a\\c", "later"]).1, "Xa");
    }
}
