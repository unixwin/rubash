use super::ansi::decode_ansi_c_quoted;

pub(super) fn remove_shell_quotes(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                let mut quoted = String::new();
                let mut escaped = false;
                for quoted_ch in chars.by_ref() {
                    if escaped {
                        quoted.push('\\');
                        quoted.push(quoted_ch);
                        escaped = false;
                        continue;
                    }
                    if quoted_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if quoted_ch == '\'' {
                        break;
                    }
                    quoted.push(quoted_ch);
                }
                if escaped {
                    quoted.push('\\');
                }
                out.push_str(&decode_ansi_c_quoted(&quoted));
            }
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    if quoted == '$' {
                        out.push('\x1f');
                    } else {
                        out.push(quoted);
                    }
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    if quoted == '$' && chars.peek() == Some(&'{') {
                        copy_braced_parameter_after_dollar(&mut out, &mut chars);
                        continue;
                    }
                    match quoted {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                                chars.peek().copied()
                            {
                                chars.next();
                                if escaped != '\n' {
                                    match escaped {
                                        '$' => out.push('\x1f'),
                                        '`' => out.push('\x1a'),
                                        _ => out.push(escaped),
                                    }
                                }
                            } else {
                                out.push('\\');
                            }
                        }
                        _ => out.push(quoted),
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    if escaped == '$' {
                        out.push('\x1f');
                    } else if escaped == '\'' {
                        out.push('\x17');
                    } else {
                        out.push(escaped);
                    }
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

pub(super) fn remove_shell_quotes_outside_backticks(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                out.push(ch);
                while let Some(inner) = chars.next() {
                    out.push(inner);
                    if inner == '`' {
                        break;
                    }
                    if inner == '\\' {
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    }
                }
            }
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    out.push(quoted);
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    if quoted == '$' && chars.peek() == Some(&'{') {
                        copy_braced_parameter_after_dollar(&mut out, &mut chars);
                        continue;
                    }
                    match quoted {
                        '"' => break,
                        '`' => {
                            out.push(quoted);
                            while let Some(inner) = chars.next() {
                                out.push(inner);
                                if inner == '`' {
                                    break;
                                }
                                if inner == '\\' {
                                    if let Some(escaped) = chars.next() {
                                        out.push(escaped);
                                    }
                                }
                            }
                        }
                        '\\' => {
                            if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                                chars.peek().copied()
                            {
                                chars.next();
                                if escaped != '\n' {
                                    if escaped == '`' {
                                        out.push('\x1a');
                                    } else {
                                        out.push(escaped);
                                    }
                                }
                            } else {
                                out.push('\\');
                            }
                        }
                        _ => out.push(quoted),
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    if escaped == '\'' {
                        out.push('\x17');
                    } else {
                        out.push(escaped);
                    }
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

pub(super) fn copy_braced_parameter_after_dollar(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    out.push('$');
    if chars.next() != Some('{') {
        return;
    }
    out.push('{');
    let mut depth = 1usize;
    while let Some(ch) = chars.next() {
        out.push(ch);
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            out.push('{');
            depth += 1;
            continue;
        }
        if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
}
