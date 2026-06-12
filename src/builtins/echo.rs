//! `echo` builtin.
//!
//! GNU Bash source ownership:
// - builtins/echo.def

use std::io::{self, Write};

/// Execute `echo` with arguments after the command name.
pub fn execute(args: &[String]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    write_echo(args.iter().map(String::as_str), &mut stdout)
}

fn write_echo<'a, I, W>(args: I, writer: &mut W) -> io::Result<()>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut display_newline = true;
    let mut interpret_escapes = false;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index];
        if !is_echo_option(arg) {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'n' => display_newline = false,
                'e' => interpret_escapes = true,
                'E' => interpret_escapes = false,
                _ => unreachable!("validated by is_echo_option"),
            }
        }
        index += 1;
    }

    let mut suppress_remaining = false;
    for (position, arg) in args[index..].iter().enumerate() {
        if position > 0 {
            writer.write_all(b" ")?;
        }

        if interpret_escapes {
            let normalized = remove_residual_shell_quotes(arg);
            let expanded = expand_escapes(&normalized);
            writer.write_all(expanded.output.as_bytes())?;
            if expanded.stop {
                suppress_remaining = true;
                display_newline = false;
                break;
            }
        } else {
            let normalized = remove_residual_shell_quotes(arg);
            writer.write_all(normalized.as_bytes())?;
        }
    }

    if display_newline && !suppress_remaining {
        writer.write_all(b"\n")?;
    }

    Ok(())
}

fn remove_residual_shell_quotes(arg: &str) -> String {
    // TODO(subst.c/parse.y): Quote removal belongs in the expansion pipeline,
    // not inside echo. This narrow cleanup covers alias replacement text that
    // Rubash has not yet re-read through a complete parser input stack.
    if arg.len() >= 2
        && ((arg.starts_with('"') && arg.ends_with('"'))
            || (arg.starts_with('\'') && arg.ends_with('\'')))
    {
        return arg[1..arg.len() - 1].to_string();
    }

    if arg.starts_with('"') || arg.starts_with('\'') {
        return arg[1..].to_string();
    }

    arg.to_string()
}

fn is_echo_option(arg: &str) -> bool {
    let Some(rest) = arg.strip_prefix('-') else {
        return false;
    };

    !rest.is_empty() && rest.chars().all(|ch| matches!(ch, 'n' | 'e' | 'E'))
}

#[derive(Debug, PartialEq, Eq)]
struct EscapeExpansion {
    output: String,
    stop: bool,
}

fn expand_escapes(input: &str) -> EscapeExpansion {
    let mut output = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        let Some(escaped) = chars.next() else {
            output.push('\\');
            break;
        };

        match escaped {
            'a' => output.push('\x07'),
            'b' => output.push('\x08'),
            'c' => return EscapeExpansion { output, stop: true },
            'e' | 'E' => output.push('\x1b'),
            'f' => output.push('\x0c'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'v' => output.push('\x0b'),
            '\\' => output.push('\\'),
            '0' => push_codepoint(&mut output, read_digits(&mut chars, 8, 3)),
            'x' => push_codepoint(&mut output, read_digits(&mut chars, 16, 2)),
            'u' => push_codepoint(&mut output, read_digits(&mut chars, 16, 4)),
            'U' => push_codepoint(&mut output, read_digits(&mut chars, 16, 8)),
            other => {
                output.push('\\');
                output.push(other);
            }
        }
    }

    EscapeExpansion {
        output,
        stop: false,
    }
}

fn read_digits<I>(chars: &mut std::iter::Peekable<I>, radix: u32, max: usize) -> Option<u32>
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

fn push_codepoint(output: &mut String, value: Option<u32>) {
    let Some(value) = value else {
        return;
    };

    if let Some(ch) = char::from_u32(value) {
        output.push(ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(args: &[&str]) -> String {
        let mut output = Vec::new();
        write_echo(args.iter().copied(), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn joins_arguments_with_spaces_and_newline() {
        assert_eq!(render(&["hello", "world"]), "hello world\n");
    }

    #[test]
    fn supports_no_newline_option() {
        assert_eq!(render(&["-n", "hello"]), "hello");
    }

    #[test]
    fn treats_invalid_option_as_operand() {
        assert_eq!(render(&["-x", "hello"]), "-x hello\n");
    }

    #[test]
    fn supports_escape_enable_and_disable_options() {
        assert_eq!(render(&["-e", "a\\nb"]), "a\nb\n");
        assert_eq!(render(&["-eE", "a\\nb"]), "a\\nb\n");
    }

    #[test]
    fn supports_stop_output_escape() {
        assert_eq!(render(&["-e", "one\\ctwo", "three"]), "one");
    }

    #[test]
    fn supports_numeric_escapes() {
        assert_eq!(render(&["-e", "\\0101\\x42\\u43\\U44"]), "ABCD\n");
    }
}
