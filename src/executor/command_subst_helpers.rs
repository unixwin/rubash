use crate::executor::markers::DATA_DOLLAR;
use crate::executor::substitution_metadata::bytes_to_shell_text;

pub(in crate::executor) fn collect_braced_parameter_name(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> String {
    let mut name = String::new();
    let mut nested = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_c = false;
    while let Some(ch) = chars.next() {
        // Inside single quotes everything is literal, including backslashes;
        // only the next single quote ends the quoted region.
        if single {
            if ch == '\'' {
                single = false;
            }
            name.push(ch);
            continue;
        }
        // Inside a $'...' ANSI-C span a backslash escapes the following
        // character, so '\'' is data rather than the closing quote (GNU parse.y
        // extract_dollar_brace_string). Without this the escaped quote ends the span
        // early and the trailing '}' is swallowed, leaving a heredoc body
        // such as 7: ${x%'$'a\t\\'b'}' unexpanded.
        if ansi_c {
            name.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    name.push(escaped);
                }
            } else if ch == '\'' {
                ansi_c = false;
            }
            continue;
        }
        if double {
            if ch == '"' {
                double = false;
                name.push(ch);
                continue;
            }
            if ch == '\\' {
                name.push(ch);
                if let Some(escaped) = chars.next() {
                    name.push(escaped);
                }
                continue;
            }
            name.push(ch);
            continue;
        }
        // GNU Bash scans `\` plus the following character as one unit
        // (extract_dollar_brace_string advances by two). `\\` is a literal
        // backslash, `\}` a literal closing brace; neither closes the name.
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                name.push('\\');
                name.push(escaped);
            } else {
                name.push('\\');
            }
            continue;
        }
        if ch == '$' && chars.peek().copied() == Some('\'') {
            // $'...' is an ANSI-C quoted span, a quoting mode distinct
            // from a plain single quote: backslashes inside it escape data.
            chars.next();
            ansi_c = true;
            name.push('$');
            name.push('\'');
            continue;
        }
        if ch == '\'' {
            single = true;
            name.push(ch);
            continue;
        }
        if ch == '"' {
            double = true;
            name.push(ch);
            continue;
        }
        if ch == '[' {
            name.push(ch);
            continue;
        }
        if ch == ']' {
            name.push(ch);
            continue;
        }
        if ch == '$' && chars.peek().copied() == Some('{') {
            chars.next();
            nested += 1;
            name.push('$');
            name.push('{');
            continue;
        }
        if ch == '}' {
            if nested == 0 {
                break;
            }
            nested -= 1;
            name.push(ch);
            continue;
        }
        name.push(ch);
    }
    name
}

pub(in crate::executor) fn decode_old_style_backtick_source(source: &str) -> String {
    let mut output = String::new();
    let mut chars = source.chars().peekable();
    let mut single = false;
    let mut double = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' && !double {
            single = !single;
            output.push(ch);
            continue;
        }

        if ch == '"' && !single {
            double = !double;
            output.push(ch);
            continue;
        }

        if ch != '\\' {
            push_backtick_source_char(&mut output, ch, single);
            continue;
        }

        if double {
            let mut lookahead = chars.clone();
            if lookahead.next() == Some('\\') && lookahead.next() == Some('"') {
                chars.next();
                chars.next();
                output.push(crate::executor::markers::DATA_DQUOTE);
                continue;
            }
        }

        if double && chars.peek().copied() == Some('"') {
            chars.next();
            output.push(crate::executor::markers::DATA_DQUOTE);
            continue;
        }

        match chars.next() {
            Some(next @ ('$' | '`' | '\\')) => {
                push_backtick_source_char(&mut output, next, single);
            }
            // GNU parse.y parse_matched_pair consumes the backslash before
            // any escaped character inside backticks, so `\"` becomes `"`
            // (not `\"`). This lets `echo \"Hello\"` inside backticks parse
            // as `echo "Hello"` with real double-quote delimiters.
            Some('"') => {
                push_backtick_source_char(&mut output, '"', single);
            }
            Some('\n') => {}
            Some('\r') if chars.peek().copied() == Some('\n') => {
                chars.next();
            }
            Some(next) => {
                output.push('\\');
                push_backtick_source_char(&mut output, next, single);
            }
            None => output.push('\\'),
        }
    }
    output
}

fn push_backtick_source_char(output: &mut String, ch: char, single: bool) {
    if single && ch == '$' {
        output.push(DATA_DOLLAR);
    } else {
        output.push(ch);
    }
}

pub(super) fn unescape_remaining_shell_escapes(value: &str) -> String {
    unescape_remaining_shell_escapes_inner(value)
}

fn unescape_remaining_shell_escapes_inner(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut lookahead = chars.clone();
            if lookahead.next() == Some('\\') && lookahead.next() == Some('\'') {
                chars.next();
                chars.next();
                output.push('\'');
                continue;
            }
            if let Some(
                next @ ('\'' | '"' | '\\' | '$' | '`' | '(' | ')' | '{' | '}' | ';' | '&' | '|'
                | '<' | '>' | '!' | '*' | '?' | '#' | ' '),
            ) = chars.peek().copied()
            {
                chars.next();
                output.push(next);
                continue;
            }
        }
        output.push(ch);
    }
    output
}

/// Command-substitution capture: strips the line terminator(s) GNU bash
/// removes from a substitution's output.
///
/// GNU (subst.c command_substitute) deletes every trailing newline and keeps
/// any other trailing byte, so on Unix `o=$(printf 'a\r\n')` is `a\r`
/// (WSL 5.3.0 baseline confirms: od prints `a \r`). Windows additionally
/// treats a CRLF pair as one terminator: niubash runs Windows-native tools
/// whose CRT text mode turns every `\n` into `\r\n` (measured: gawk, GoAWK's
/// awk, ugrep, python, jq, xz, curl, 7z), and a native shell reads those
/// pipes without the MSYS text-mode translation that turns `\r\n` back into
/// `\n`. The stray `\r` therefore survives into `$o` and breaks
/// `[ "$o" = x ]`, `case` labels, array keys and concatenated paths with an
/// invisible byte (niubash #120).
///
/// Rule: strip every trailing `\n`; on Windows also a `\r` that immediately
/// precedes a stripped `\n` (part of the CRLF terminator). A lone trailing
/// `\r` not followed by `\n` is preserved everywhere, so the GNU-fidelity
/// case is intact. This is the same policy the lexer applies to CRLF script
/// lines (niubash #106) and the same one `read` applies to its input.
pub(in crate::executor) trait CaptureTerminator {
    fn trim_capture_terminator(&self) -> &str;
}

impl CaptureTerminator for str {
    fn trim_capture_terminator(&self) -> &str {
        let bytes = self.as_bytes();
        let mut end = bytes.len();
        while end > 0 && bytes[end - 1] == b'\n' {
            end -= 1;
            if cfg!(windows) && end > 0 && bytes[end - 1] == b'\r' {
                end -= 1;
            }
        }
        &self[..end]
    }
}

/// Byte-level form of the same rule, for capture paths that carry `Vec<u8>`
/// until the final text conversion (GNU subst.c read_comsub).
pub(in crate::executor) fn trim_capture_terminator_bytes(bytes: &mut Vec<u8>) {
    while bytes.last() == Some(&b'\n') {
        bytes.pop();
        if cfg!(windows) && bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
}

pub(in crate::executor) fn echo_command_substitution_output(args: &[String]) -> String {
    let mut bytes = echo_raw_output_bytes(args);
    bytes.retain(|byte| *byte != 0);
    bytes_to_shell_text(&bytes)
        .trim_capture_terminator()
        .to_string()
}

pub(in crate::executor) fn echo_raw_output(args: &[String]) -> String {
    bytes_to_shell_text(&echo_raw_output_bytes(args))
}

fn echo_raw_output_bytes(args: &[String]) -> Vec<u8> {
    let mut output = Vec::new();
    let _ = crate::builtins::echo::write_echo(args.iter().map(String::as_str), &mut output);
    output
}

pub(in crate::executor) fn split_pipeline_words(words: &[String]) -> Option<Vec<&[String]>> {
    let mut stages = Vec::new();
    let mut start = 0usize;
    for (index, word) in words.iter().enumerate() {
        if word == "|" {
            if start == index {
                return None;
            }
            stages.push(&words[start..index]);
            start = index + 1;
        }
    }
    if start >= words.len() {
        return None;
    }
    stages.push(&words[start..]);
    (stages.len() > 1).then_some(stages)
}

/// True when any word is a shell control/redirection operator that the
/// word-based command-substitution shortcuts cannot interpret. GNU subst.c
/// parses the whole substitution body into a command list before executing
/// it; a word shortcut that assumes a lone simple command would hand the
/// operators to the command as literal arguments (`$(f a b | wc -l)` runs
/// f with `| wc -l` in its positional params, issue #70).
pub(in crate::executor) fn command_substitution_words_have_operators(words: &[String]) -> bool {
    words.iter().any(|word| command_word_is_operator(word))
}

/// True when the word itself carries redirection syntax: either a bare
/// operator token or an attached form without whitespace (`2>/dev/null`,
/// `>out`, `<in`, `<<EOF`).
pub(in crate::executor) fn command_word_carries_redirect(word: &str) -> bool {
    let body = word.trim_start_matches(|ch: char| ch.is_ascii_digit());
    body.starts_with('<') || body.starts_with('>')
}

fn command_word_is_operator(word: &str) -> bool {
    match word {
        "|" | "|&" | "||" | "&&" | "&" | ";" | ";;" | ";&" | ";;&" => true,
        "<" | ">" | ">>" | ">|" | "<>" | "<<" | "<<<" | "<&" | ">&" | "&>" | "&>>" | "1>"
        | "1>>" | "1>|" | "1<&" | "1>&" | "2>" | "2>>" | "2>|" | "2<&" | "2>&" => true,
        _ => command_word_carries_redirect(word) || word.contains('|'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn echo_command_substitution_uses_builtin_escape_rules() {
        assert_eq!(
            echo_command_substitution_output(&args(&["-e", "a\\nb"])),
            "a\nb"
        );
        assert_eq!(
            echo_command_substitution_output(&args(&["-e", "\\xz"])),
            "\\xz"
        );
        assert_eq!(
            echo_command_substitution_output(&args(&["--help"])),
            "--help"
        );
    }

    #[test]
    fn echo_command_substitution_removes_nul_bytes() {
        assert_eq!(
            echo_command_substitution_output(&args(&["-e", "a\\0b"])),
            "ab"
        );
    }

    #[test]
    fn echo_raw_output_keeps_builtin_stdout_shape() {
        assert_eq!(echo_raw_output(&args(&["-n", "hello"])), "hello");
        assert_eq!(echo_raw_output(&args(&["hello"])), "hello\n");
    }

    #[test]
    fn braced_parameter_collection_closes_at_first_brace_in_bracket_pattern() {
        // GNU parse.y uses P_FIRSTCLOSE for ${...}: the first unquoted '}'
        // closes the expression, even inside a bracket pattern. The
        // remaining `]` is literal text outside the expansion.
        let mut chars = "o%[}]}]".chars().peekable();
        assert_eq!(collect_braced_parameter_name(&mut chars), "o%[");
        assert_eq!(chars.collect::<String>(), "]}]");
    }
}
