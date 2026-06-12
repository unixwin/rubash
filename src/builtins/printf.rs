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
    width: Option<usize>,
    precision: Option<usize>,
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

    let rendered = render(format, &args[index + 1..]);
    if let Some(name) = output_var {
        env_vars.insert(name.to_string(), rendered);
    } else {
        stdout.write_all(rendered.as_bytes())?;
    }

    Ok(EXECUTION_SUCCESS)
}

fn render(format: &str, args: &[&str]) -> String {
    let mut output = String::new();
    let mut arg_index = 0;

    if args.is_empty() {
        render_one_pass(format, args, &mut arg_index, &mut output);
        return output;
    }

    while arg_index < args.len() {
        let before_arg = arg_index;
        render_one_pass(format, args, &mut arg_index, &mut output);

        if arg_index == before_arg {
            break;
        }
    }

    output
}

fn render_one_pass(format: &str, args: &[&str], arg_index: &mut usize, output: &mut String) {
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

                let Some(spec) = parse_format_spec(&mut chars) else {
                    output.push('%');
                    continue;
                };

                let value = next_arg(args, arg_index);
                output.push_str(&format_value(value, &spec));
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
            '+' | ' ' | '#' | '\'' => {}
            _ => break,
        }
        chars.next();
    }

    spec.width = read_usize(chars);
    if chars.peek() == Some(&'.') {
        chars.next();
        spec.precision = Some(read_usize(chars).unwrap_or(0));
    }

    while matches!(chars.peek(), Some('h' | 'j' | 'l' | 'L' | 't' | 'z')) {
        chars.next();
    }

    spec.specifier = chars.next()?;
    Some(spec)
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
        'q' | 'Q' => shell_quote(value),
        'c' => value.chars().next().unwrap_or('\0').to_string(),
        'd' | 'i' => parse_i64(value).to_string(),
        'u' => (parse_i64(value) as u64).to_string(),
        'x' => format!("{:x}", parse_i64(value)),
        'X' => format!("{:X}", parse_i64(value)),
        'o' => format!("{:o}", parse_i64(value)),
        'f' | 'F' => format_float(value, spec.precision, 'f'),
        'e' => format_float(value, spec.precision, 'e'),
        'E' => format_float(value, spec.precision, 'E'),
        'g' | 'G' => format_float(value, spec.precision, 'g'),
        other => {
            let mut fallback = String::from('%');
            fallback.push(other);
            fallback
        }
    };

    apply_width(rendered, spec)
}

fn truncate_precision(value: String, precision: Option<usize>) -> String {
    let Some(precision) = precision else {
        return value;
    };
    value.chars().take(precision).collect()
}

fn format_float(value: &str, precision: Option<usize>, mode: char) -> String {
    let value = value.parse::<f64>().unwrap_or(0.0);
    match (mode, precision) {
        ('e', Some(precision)) => format!("{value:.precision$e}"),
        ('E', Some(precision)) => format!("{value:.precision$E}"),
        (_, Some(precision)) => format!("{value:.precision$}"),
        ('e', None) => format!("{value:e}"),
        ('E', None) => format!("{value:E}"),
        _ => format!("{value}"),
    }
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
    } else {
        format!("{padding}{value}")
    }
}

fn parse_i64(value: &str) -> i64 {
    value
        .parse::<i64>()
        .unwrap_or_else(|_| value.chars().next().map(|ch| ch as i64).unwrap_or_default())
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
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }

    output
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '/' | '.' | '-' | ':'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
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
}
