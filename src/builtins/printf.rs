//! `printf` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/printf.def (`printf_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
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

    if args.get(index) == Some(&"--") {
        index += 1;
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
        }
    }

    let Some(format) = args.get(index) else {
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    };

    let rendered = render(format, &args[index + 1..], env_vars);
    if let Some(name) = output_var {
        env_vars.insert(name.to_string(), rendered);
    } else {
        stdout.write_all(rendered.as_bytes())?;
    }

    Ok(EXECUTION_SUCCESS)
}

fn render(format: &str, args: &[&str], env_vars: &mut HashMap<String, String>) -> String {
    let mut output = String::new();
    let mut arg_index = 0;

    if args.is_empty() {
        render_one_pass(format, args, &mut arg_index, &mut output, env_vars);
        return output;
    }

    while arg_index < args.len() {
        let before_arg = arg_index;
        render_one_pass(format, args, &mut arg_index, &mut output, env_vars);

        if arg_index == before_arg {
            break;
        }
    }

    output
}

fn render_one_pass(
    format: &str,
    args: &[&str],
    arg_index: &mut usize,
    output: &mut String,
    env_vars: &mut HashMap<String, String>,
) {
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => output.push(expand_format_escape(&mut chars)),
            '%' => {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    output.push('%');
                    continue;
                }

                let Some(mut spec) = parse_format_spec(&mut chars) else {
                    output.push('%');
                    continue;
                };
                resolve_dynamic_format_args(&mut spec, args, arg_index);

                if spec.specifier == 'n' {
                    let name = next_arg(args, arg_index);
                    if valid_identifier(name) {
                        env_vars.insert(name.to_string(), output.chars().count().to_string());
                    }
                } else {
                    let value = next_arg(args, arg_index);
                    output.push_str(&format_value(value, &spec));
                }
            }
            other => output.push(other),
        }
    }
}

fn next_arg<'a>(args: &'a [&str], arg_index: &mut usize) -> &'a str {
    let value = args.get(*arg_index).copied().unwrap_or("");
    *arg_index += 1;
    value
}

fn parse_format_spec<I>(chars: &mut std::iter::Peekable<I>) -> Option<FormatSpec>
where
    I: Iterator<Item = char>,
{
    let mut spec = FormatSpec::default();

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
        chars.next();
    }

    if chars.peek() == Some(&'*') {
        chars.next();
        spec.width_from_arg = true;
    } else {
        spec.width = read_usize(chars);
    }
    if chars.peek() == Some(&'.') {
        chars.next();
        if chars.peek() == Some(&'*') {
            chars.next();
            spec.precision_from_arg = true;
        } else {
            spec.precision = Some(read_usize(chars).unwrap_or(0));
        }
    }

    while matches!(chars.peek(), Some('h' | 'j' | 'l' | 'L' | 't' | 'z')) {
        chars.next();
    }

    spec.specifier = chars.next()?;
    Some(spec)
}

fn resolve_dynamic_format_args(spec: &mut FormatSpec, args: &[&str], arg_index: &mut usize) {
    if spec.width_from_arg {
        let width = parse_i64(next_arg(args, arg_index));
        if width < 0 {
            spec.left_adjust = true;
            spec.width = Some(width.unsigned_abs() as usize);
        } else {
            spec.width = Some(width as usize);
        }
    }

    if spec.precision_from_arg {
        let precision = parse_i64(next_arg(args, arg_index));
        spec.precision = (precision >= 0).then_some(precision as usize);
    }
}

fn read_usize<I>(chars: &mut std::iter::Peekable<I>) -> Option<usize>
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

    digits.parse().ok()
}

fn format_value(value: &str, spec: &FormatSpec) -> String {
    let rendered = match spec.specifier {
        's' => truncate_precision(value.to_string(), spec.precision),
        'b' => truncate_precision(expand_percent_b(value), spec.precision),
        'q' => truncate_precision(shell_quote(value), spec.precision),
        'Q' => shell_quote(&truncate_precision(value.to_string(), spec.precision)),
        'c' => value.chars().next().unwrap_or('\0').to_string(),
        'd' | 'i' => format_signed_integer(parse_i64(value), spec),
        'u' => format_unsigned_integer(parse_i64(value) as u64, 10, false, spec),
        'x' => format_unsigned_integer(parse_i64(value) as u64, 16, false, spec),
        'X' => format_unsigned_integer(parse_i64(value) as u64, 16, true, spec),
        'o' => format_unsigned_integer(parse_i64(value) as u64, 8, false, spec),
        'f' | 'F' => format_float(value, spec, 'f'),
        'e' => format_float(value, spec, 'e'),
        'E' => format_float(value, spec, 'E'),
        'g' | 'G' => format_float(value, spec, 'g'),
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
    apply_width(rendered, &width_spec)
}

fn truncate_precision(value: String, precision: Option<usize>) -> String {
    let Some(precision) = precision else {
        return value;
    };
    value.chars().take(precision).collect()
}

fn format_float(value: &str, spec: &FormatSpec, mode: char) -> String {
    let value = parse_f64(value);
    let mut rendered = match (mode, spec.precision) {
        ('e', Some(precision)) => format!("{value:.precision$e}"),
        ('E', Some(precision)) => format!("{value:.precision$E}"),
        (_, Some(precision)) => format!("{value:.precision$}"),
        ('e', None) => format!("{value:e}"),
        ('E', None) => format!("{value:E}"),
        _ => format!("{value}"),
    };

    if spec.alternate_form && matches!(mode, 'f') && !rendered.contains('.') {
        rendered.push('.');
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

fn parse_i64(value: &str) -> i64 {
    if let Some(ch) = printf_char_constant(value) {
        return ch as i64;
    }
    parse_integer_literal(value).unwrap_or_default()
}

fn parse_f64(value: &str) -> f64 {
    if let Some(ch) = printf_char_constant(value) {
        return ch as u32 as f64;
    }
    value.parse::<f64>().unwrap_or_default()
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

fn expand_format_escape<I>(chars: &mut std::iter::Peekable<I>) -> char
where
    I: Iterator<Item = char>,
{
    match chars.next() {
        Some('a') => '\x07',
        Some('b') => '\x08',
        Some('e') | Some('E') => '\x1b',
        Some('f') => '\x0c',
        Some('n') => '\n',
        Some('r') => '\r',
        Some('t') => '\t',
        Some('v') => '\x0b',
        Some('\\') => '\\',
        Some(other) => other,
        None => '\\',
    }
}

fn expand_percent_b(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('c') => break,
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

    output
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
}
