pub(super) fn expand_format_escape<I>(chars: &mut std::iter::Peekable<I>) -> String
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

pub(super) fn expand_percent_b(value: &str) -> (String, bool) {
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

pub(super) fn shell_quote(value: &str) -> String {
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
