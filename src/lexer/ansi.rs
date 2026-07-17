pub(super) fn decode_ansi_c_quoted(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('a') => output.push('\x07'),
            Some('b') => output.push('\x08'),
            Some('e') | Some('E') => output.push('\x1b'),
            Some('f') => output.push('\x0c'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('v') => output.push('\x0b'),
            Some('\\') => output.push('\\'),
            Some('\'') => output.push('\''),
            Some('"') => output.push('"'),
            Some('?') => output.push('?'),
            Some('x') => {
                if let Some(value) = read_ansi_c_digits(&mut chars, 16, 2) {
                    push_ansi_c_codepoint(&mut output, value);
                } else {
                    output.push('\\');
                    output.push('x');
                }
            }
            Some(octal @ '0'..='7') => {
                let mut value = octal.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    let Some(next) = chars.peek().copied() else {
                        break;
                    };
                    let Some(digit) = next.to_digit(8) else {
                        break;
                    };
                    value = value * 8 + digit;
                    chars.next();
                }
                push_ansi_c_codepoint(&mut output, value);
            }
            Some(c) if c.is_ascii_digit() => {
                output.push('\\');
                output.push(c);
            }
            Some('u') => {
                if let Some(value) = read_ansi_c_digits(&mut chars, 16, 4) {
                    push_ansi_c_codepoint(&mut output, value);
                } else {
                    output.push('\\');
                    output.push('u');
                }
            }
            Some('U') => {
                if let Some(value) = read_ansi_c_digits(&mut chars, 16, 8) {
                    push_ansi_c_codepoint(&mut output, value);
                } else {
                    output.push('\\');
                    output.push('U');
                }
            }
            Some('c') => {
                // Control character: \cx
                if let Some(c) = chars.next() {
                    output.push((c as u32 & 0x1f) as u8 as char);
                }
            }
            None => output.push('\\'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
        }
    }

    output
}

fn read_ansi_c_digits<I>(chars: &mut std::iter::Peekable<I>, radix: u32, max: usize) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = String::new();
    while value.len() < max {
        let Some(next) = chars.peek().copied() else {
            break;
        };
        if next.to_digit(radix).is_none() {
            break;
        }
        value.push(next);
        chars.next();
    }

    if value.is_empty() {
        None
    } else {
        u32::from_str_radix(&value, radix).ok()
    }
}

fn push_ansi_c_codepoint(output: &mut String, value: u32) {
    if let Some(ch) = char::from_u32(value) {
        output.push(ch);
    }
}
