//! `echo` builtin.
//!
//! GNU Bash source ownership:
// - builtins/echo.def

use crate::executor::markers::DATA_DOLLAR;
use std::io::{self, Write};

/// Execute `echo` with arguments after the command name.
pub fn execute(args: &[String]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    write_echo_decoded(args.iter().map(String::as_str), &mut stdout)
}

pub(crate) fn write_echo<'a, I, W>(args: I, writer: &mut W) -> io::Result<()>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    // History (rubash#1585): a literal special case here rewrote a sole
    // `hi)` argument to `hi`, compensating for a heredoc-body synthesis
    // defect in the comsub-eof5 shape (`echo "$(\ncat <<\)\nhi\n))"`).
    // That defect is gone — the substitution itself now yields `hi` on
    // every route (direct echo, printf, assignment) — and the special
    // case had become a divergence of its own: GNU echo prints its
    // arguments verbatim (builtins/echo.def), so `echo 'hi)'` must print
    // `hi)`. No literal rewrites live in the builtin.
    let mut display_newline = true;
    let mut interpret_escapes = crate::builtins::shopt::xpg_echo_enabled();
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

    let writes_alias_definition = args.get(index).is_some_and(|arg| *arg == "alias");
    let mut suppress_remaining = false;
    for (position, arg) in args[index..].iter().enumerate() {
        if position > 0 {
            writer.write_all(b" ")?;
        }

        if interpret_escapes {
            let normalized =
                remove_residual_shell_quotes(arg, writes_alias_definition && position > 0);
            let expanded = expand_escapes(&normalized);
            writer.write_all(&expanded.output)?;
            if expanded.stop {
                suppress_remaining = true;
                display_newline = false;
                break;
            }
        } else {
            let normalized =
                remove_residual_shell_quotes(arg, writes_alias_definition && position > 0);
            writer.write_all(normalized.as_bytes())?;
        }
    }

    if display_newline && !suppress_remaining {
        writer.write_all(b"\n")?;
    }

    Ok(())
}

/// Write echo output after consuming shell-text raw-byte markers.
pub(crate) fn write_echo_decoded<'a, I, W>(args: I, writer: &mut W) -> io::Result<()>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let mut raw = Vec::new();
    write_echo(args, &mut raw)?;
    let decoded = crate::executor::substitution_metadata::decode_raw_byte_markers(&raw);
    writer.write_all(&decoded)
}

fn remove_residual_shell_quotes(arg: &str, unescape_alias_quotes: bool) -> String {
    // TODO(subst.c/parse.y): Quote removal belongs in the expansion pipeline,
    // not inside echo. This narrow cleanup covers alias replacement text that
    // Rubash has not yet re-read through a complete parser input stack.
    if unescape_alias_quotes && arg.contains("\\'") {
        return arg.replace("\\'", "'");
    }

    // GNU echo prints `"..."`-wrapped arguments verbatim: expansion output
    // carrying literal quotes (`x='"lit"'; echo $x` -> `"lit"`,
    // `${a[@]@K}` -> `"v"`) is data, and quote removal belongs to the
    // expansion pipeline that produced the word, not to echo.

    // Workaround: $'...' inside $(...) is not recognized by the word-based
    // command substitution shortcut (split_shell_words_with_quote_info in
    // alias_helpers.rs treats $ as literal and '...' as single-quoted,
    // converting \ to \x15). When echo receives an argument starting with $
    // followed by \x15 markers (protected backslashes from single quotes),
    // decode the ANSI-C escapes to recover the original $'...' content.
    // The \x1f and \x1a markers from push_single_quoted_shell_word_char
    // (literal $ and ` inside single quotes) are restored after decoding;
    // they are NOT produced by decode_ansi_c_quoted's push_ansi_c_byte
    // (which emits raw-byte markers for carrier bytes), so the replace
    // only affects the single-quote-origin markers.
    if arg.starts_with('$') && arg.contains(crate::executor::markers::PROTECTED_BACKSLASH) {
        let body = arg[1..].replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\");
        let decoded = crate::lexer::decode_ansi_c_quoted(&body);
        return decoded
            .replace(DATA_DOLLAR, "$")
            .replace(crate::executor::markers::DATA_BACKTICK, "`");
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
    output: Vec<u8>,
    stop: bool,
}

fn expand_escapes(input: &str) -> EscapeExpansion {
    let mut output = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            push_char(&mut output, ch);
            continue;
        }

        let Some(escaped) = chars.next() else {
            output.push(b'\\');
            break;
        };

        match escaped {
            'a' => output.push(0x07),
            'b' => output.push(0x08),
            'c' => return EscapeExpansion { output, stop: true },
            'e' | 'E' => output.push(0x1b),
            'f' => output.push(0x0c),
            'n' => output.push(b'\n'),
            'r' => output.push(b'\r'),
            't' => output.push(b'\t'),
            'v' => output.push(0x0b),
            '\\' => output.push(b'\\'),
            '0' => output.push((read_digits(&mut chars, 8, 3).unwrap_or(0) & 0xff) as u8),
            'x' => match read_digits(&mut chars, 16, 2) {
                Some(value) => output.push((value & 0xff) as u8),
                None => output.extend_from_slice(b"\\x"),
            },
            'u' => match read_digits(&mut chars, 16, 4) {
                Some(value) => push_codepoint(&mut output, value),
                None => output.extend_from_slice(b"\\u"),
            },
            'U' => match read_digits(&mut chars, 16, 8) {
                Some(value) => push_codepoint(&mut output, value),
                None => output.extend_from_slice(b"\\U"),
            },
            other => {
                output.push(b'\\');
                push_char(&mut output, other);
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

fn push_char(output: &mut Vec<u8>, ch: char) {
    let mut buf = [0; 4];
    output.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

fn push_codepoint(output: &mut Vec<u8>, value: u32) {
    // GNU echo -e decodes \u/\U through ansicstr (builtins/echo.def:169 ->
    // lib/sh/strtrans.c:161-181 -> lib/sh/unicode.c:239 u32cconv), the same
    // path as $'\u': values <= 0x7f are emitted directly, everything else
    // goes through u32cconv, whose non-UTF-8-locale fallback is u32tocesc
    // (unicode.c:141: `\u%04X` / `\U%08X`, ISO C99 escape with UPPERCASE
    // hex) — under LC_ALL=C GNU keeps `echo -e '\u00e9'` as the six literal
    // characters `\u00E9` (rubash#1488). The ANSI-C decoder in lexer/ansi.rs
    // applies exactly this gate (#353); mirror it here — through the ONE
    // shared locale decision (locale::locale_name_selects_utf8, the same
    // predicate behind the `${#var}` length face), so a non-UTF-8 decision
    // closes the gate here too, including the no-locale-variables case,
    // where setlocale(LC_ALL, "") falls back to the C locale and GNU keeps
    // the escape literal (rubash#1837).
    if value > 0x7f {
        let name = crate::locale::active_locale_name();
        if !crate::locale::locale_name_selects_utf8(&name) {
            if value < 0x1_0000 {
                output.extend_from_slice(format!("\\u{value:04X}").as_bytes());
            } else {
                output.extend_from_slice(format!("\\U{value:08X}").as_bytes());
            }
            return;
        }
    }
    if let Some(ch) = char::from_u32(value) {
        push_char(output, ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(args: &[&str]) -> String {
        String::from_utf8(render_bytes(args)).unwrap()
    }

    fn render_bytes(args: &[&str]) -> Vec<u8> {
        let mut output = Vec::new();
        write_echo(args.iter().copied(), &mut output).unwrap();
        output
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
        assert_eq!(render(&["--help"]), "--help\n");
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

    #[test]
    fn supports_nul_and_raw_byte_escapes() {
        assert_eq!(render_bytes(&["-e", "\\0"]), b"\0\n");
        assert_eq!(render_bytes(&["-e", "\\0z"]), b"\0z\n");
        assert_eq!(render_bytes(&["-e", "\\0400"]), b"\0\n");
        assert_eq!(render_bytes(&["-e", "\\0777"]), vec![0xff, b'\n']);
        assert_eq!(render_bytes(&["-e", "\\xff"]), vec![0xff, b'\n']);
    }

    #[test]
    fn decodes_read_markers_without_changing_echo_raw_escapes() {
        let marker = crate::executor::substitution_metadata::encode_raw_byte_marker(0xff);
        let argument = format!("a{marker}b");
        let mut output = Vec::new();
        write_echo_decoded([argument.as_str()], &mut output).unwrap();
        assert_eq!(output, b"a\xffb\n");
        let start = output.len();
        write_echo_decoded(["-e", "\\xff"], &mut output).unwrap();
        assert_eq!(&output[start..], b"\xff\n");
    }

    #[test]
    fn preserves_incomplete_hex_and_unicode_escapes() {
        assert_eq!(render(&["-e", "\\x"]), "\\x\n");
        assert_eq!(render(&["-e", "\\xz"]), "\\xz\n");
        assert_eq!(render(&["-e", "\\u"]), "\\u\n");
        assert_eq!(render(&["-e", "\\U"]), "\\U\n");
    }
}
