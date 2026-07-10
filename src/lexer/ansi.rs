pub(super) fn decode_ansi_c_quoted(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();

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
                // Hex escape: \xHH
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(c) = chars.next() {
                        if c.is_ascii_hexdigit() {
                            hex.push(c);
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&hex, 16) {
                    output.push(val as char);
                } else {
                    output.push_str(&hex);
                }
            }
            Some('0') => {
                // Octal escape: \0NNN (up to 3 octal digits)
                let mut octal = String::new();
                for _ in 0..3 {
                    if let Some(c) = chars.next() {
                        if matches!(c, '0'..='7') {
                            octal.push(c);
                        } else {
                            // Push back by not consuming
                            break;
                        }
                    }
                }
                if !octal.is_empty() {
                    if let Ok(val) = u8::from_str_radix(&octal, 8) {
                        output.push(val as char);
                    } else {
                        output.push('\\');
                        output.push('0');
                        output.push_str(&octal);
                    }
                } else {
                    output.push('\0');
                }
            }
            Some(c) if c.is_ascii_digit() => {
                // Octal escape: \NNN (up to 3 octal digits)
                let mut octal = String::new();
                octal.push(c);
                for _ in 0..2 {
                    if let Some(c) = chars.next() {
                        if matches!(c, '0'..='7') {
                            octal.push(c);
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&octal, 8) {
                    output.push(val as char);
                } else {
                    output.push('\\');
                    output.push_str(&octal);
                }
            }
            Some('u') => {
                // Unicode escape: \uHHHH
                let mut hex = String::new();
                for _ in 0..4 {
                    if let Some(c) = chars.next() {
                        if c.is_ascii_hexdigit() {
                            hex.push(c);
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                    if let Some(c) = char::from_u32(val) {
                        output.push(c);
                    }
                } else {
                    output.push_str(&hex);
                }
            }
            Some('U') => {
                // Unicode escape: \UHHHHHHHH
                let mut hex = String::new();
                for _ in 0..8 {
                    if let Some(c) = chars.next() {
                        if c.is_ascii_hexdigit() {
                            hex.push(c);
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                    if let Some(c) = char::from_u32(val) {
                        output.push(c);
                    }
                } else {
                    output.push_str(&hex);
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
