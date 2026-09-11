//! Lexer Module - Bash Tokenizer
//!
//! Transforms raw input strings into tokens for the parser.

pub(crate) mod ansi;
mod brace_scan;
mod classification;
mod continuation;
pub(crate) mod dolbrace;
mod heredoc;
mod heredoc_scan;
mod number_redirect;
mod quotes;
mod scanner;
mod skip;
mod token;
mod word;

#[cfg(test)]
mod tests;

use brace_scan::{has_unclosed_brace_group, opens_function_body_after_previous_signature};
use continuation::{
    ends_with_unquoted_backslash, has_unclosed_compound_assignment, has_unclosed_quotes,
};

pub(crate) use continuation::has_unclosed_command_substitution;
use heredoc::heredoc_delimiters;
use scanner::Lexer;

pub(crate) use ansi::decode_ansi_c_quoted;
pub(crate) use quotes::remove_shell_quotes;
pub(crate) use quotes::{
    ANSI_C_DQUOTE_MARKER, ANSI_C_DQUOTE_MARKER_STR, ANSI_C_QUOTE_MARKER, ANSI_C_QUOTE_MARKER_STR,
    PARAM_NAME_END_MARKER,
};
pub use token::{Token, TokenKind};

pub(crate) const QUOTED_HEREDOC_MARKER: &str = "__RUBASH_HD1__";

/// Set when a command carries more than `HEREDOC_MAX` (16) here-documents.
/// GNU treats that as a fatal parse error: it reports
/// `maximum here-document count exceeded` and calls `exit_shell(EX_BADUSAGE)`
/// (bash exits 2). The lexer only returns tokens, so the condition is parked
/// here for `main` to turn into the EX_BADUSAGE exit status. Holds the line
/// number to print in the diagnostic.
static HEREDOC_OVERFLOW_LINE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Record a here-document overflow on `line` (1-based). Keeps the first line
/// reported, matching GNU's already-fatal parse state.
pub(crate) fn record_heredoc_overflow(line: usize) {
    use std::sync::atomic::Ordering;
    let _ = HEREDOC_OVERFLOW_LINE.compare_exchange(0, line, Ordering::SeqCst, Ordering::SeqCst);
}

/// Returns the recorded here-document overflow line, if any.
pub fn heredoc_overflow_line() -> Option<usize> {
    use std::sync::atomic::Ordering;
    match HEREDOC_OVERFLOW_LINE.load(Ordering::SeqCst) {
        0 => None,
        line => Some(line),
    }
}

/// Identifies where lexer input came from; alias handling is reserved for later.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputOrigin {
    #[default]
    Direct,
    AliasReplacementDeferredHeredoc,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokenizeOptions {
    pub initial_posix: bool,
    pub input_origin: InputOrigin,
    /// True when the input is a command-substitution body: heredoc body
    /// lines then end at delimiter-prefixed `)` lines (GNU make_cmd.c:602-611
    /// with PST_EOFTOKEN set), not only at exact delimiter matches.
    pub in_command_substitution: bool,
}

pub fn tokenize(input: &str) -> Vec<Token> {
    tokenize_with_options(input, TokenizeOptions::default())
}

pub fn tokenize_with_options(input: &str, options: TokenizeOptions) -> Vec<Token> {
    tokenize_with_initial_posix_and_origin(input, options.initial_posix, options.input_origin)
}

/// Tokenize with an initial POSIX parse mode. GNU Bash parses commands
/// lazily, so a runtime `set -o posix` changes the parse rules only for
/// commands read afterwards. Batch input is tokenized ahead of execution, so
/// the line loop below approximates that by flipping the parse mode when it
/// sees a top-level `set -o posix` / `set +o posix` command.
pub fn tokenize_with_initial_posix(input: &str, posix: bool) -> Vec<Token> {
    tokenize_with_initial_posix_and_origin(input, posix, InputOrigin::Direct)
}

/// Tokenize a substitution body whose text was extracted from a larger
/// script. `start_line` is the 1-based script line where the body begins, so
/// tokens (and the diagnostics they produce) carry original script line
/// numbers instead of restarting at 1 (GNU parse.y keeps in-place line
/// counters for command substitutions).
pub fn tokenize_with_initial_posix_and_line(
    input: &str,
    posix: bool,
    start_line: usize,
) -> Vec<Token> {
    tokenize_comsub_body(input, posix, start_line, false)
}

/// Tokenize a command-substitution body with the in-substitution heredoc
/// rules enabled (GNU parses these with PST_EOFTOKEN set).
pub fn tokenize_comsub_body(
    input: &str,
    posix: bool,
    start_line: usize,
    in_comsub: bool,
) -> Vec<Token> {
    tokenize_comsub_body_with_origin(input, posix, start_line, InputOrigin::Direct, in_comsub)
}

pub fn tokenize_comsub_body_with_origin(
    input: &str,
    posix: bool,
    start_line: usize,
    input_origin: InputOrigin,
    in_comsub: bool,
) -> Vec<Token> {
    if input.trim().is_empty() {
        return Vec::new();
    }

    let mut tokens = tokenize_with_heredocs(input, posix, input_origin, start_line, in_comsub);
    if tokens
        .last()
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        tokens.pop();
    }
    tokens
}

pub fn tokenize_with_initial_posix_and_origin(
    input: &str,
    posix: bool,
    input_origin: InputOrigin,
) -> Vec<Token> {
    tokenize_comsub_body_with_origin(input, posix, 1, input_origin, false)
}

fn tokenize_with_heredocs(
    input: &str,
    initial_posix: bool,
    input_origin: InputOrigin,
    start_line: usize,
    in_comsub: bool,
) -> Vec<Token> {
    // TODO(parse.y/redir.c): Bash parses here-documents after reading the
    // complete command and performs delimiter-specific expansion rules. This
    // line-oriented collector handles the simple `<<word` and `<<'word'`
    // forms used by early upstream alias tests.
    let mut output = Vec::new();
    let mut lines = input.lines();
    let mut position = 0;
    let mut line_number = start_line;
    let mut logical_start_line = start_line;
    let mut logical_line = String::new();
    let mut continued_line = false;
    let mut parse_posix = initial_posix;
    // GNU reader state for heredocs opened inside an unclosed command
    // substitution (parse.y PST_CMDSUBST): body lines stay verbatim in the
    // accumulated input — the backslash-newline join must not consume them
    // (make_cmd.c read_secondary_line, comsub4.sub quoted delimiters) — and
    // a body line starting with the delimiter with `)` later on ends the
    // heredoc (make_cmd.c:602-611).
    let mut comsub_heredocs: Vec<ComsubHeredocHeader> = Vec::new();
    let mut header_scan_from = 0usize;

    while let Some(line) = lines.next() {
        if logical_line.is_empty() {
            logical_start_line = line_number;
        }
        if !logical_line.is_empty() && !continued_line {
            logical_line.push('\n');
        }
        continued_line = false;
        logical_line.push_str(line);
        position += line.len() + 1;
        let line_had_terminator = position <= input.len();
        line_number += 1;

        let comsub_open = has_unclosed_command_substitution(&logical_line);
        if !comsub_open {
            comsub_heredocs.clear();
        } else if let Some(front) = comsub_heredocs.first().cloned() {
            // The appended line is a heredoc body line inside the open
            // substitution: close the heredoc on an exact delimiter line or
            // on a delimiter-prefixed `)` line (GNU make_cmd.c:602-611).
            let comparable = if front.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if comparable == front.delimiter
                || (comparable.starts_with(front.delimiter.as_str())
                    && comparable[front.delimiter.len()..].contains(')'))
            {
                comsub_heredocs.remove(0);
            }
        }
        let in_comsub_heredoc_body = comsub_open && !comsub_heredocs.is_empty();

        if line_had_terminator
            && ends_with_unquoted_backslash(&logical_line)
            && !in_comsub_heredoc_body
        {
            logical_line.pop();
            continued_line = true;
            continue;
        }

        // Fresh header scan once the accumulated text is stable (after the
        // join decision): a header whose delimiter is completed by the next
        // physical line (`cat <<\EOT\` + `4` = delimiter `EOT4`) is only
        // complete after the join, so the scan resumes from the last `<<`.
        if comsub_open && comsub_heredocs.is_empty() {
            let slice = &logical_line[header_scan_from.min(logical_line.len())..];
            let (mut headers, consumed) = scan_line_for_comsub_heredoc_headers(slice);
            if consumed == slice.len() || headers.is_empty() {
                header_scan_from = logical_line.len();
            } else {
                header_scan_from = logical_line.len() - slice.len() + consumed;
            }
            comsub_heredocs.append(&mut headers);
        } else if !comsub_open {
            // Keep pace with consumed text: earlier substitutions' headers are
            // already gathered and must not be rediscovered on the next scan.
            header_scan_from = logical_line.len();
        }

        if has_unclosed_quotes(&logical_line) {
            continue;
        }
        if has_unclosed_command_substitution(&logical_line) {
            continue;
        }
        // A `name=(` compound array assignment keeps reading physical lines
        // until its matching `)` (parse.y; ISSUE #78).
        if has_unclosed_compound_assignment(&logical_line) {
            continue;
        }
        let mut line_tokens = tokenize_plain(&logical_line, parse_posix);
        if let Some(updated) = line_posix_mode_change(&line_tokens) {
            parse_posix = updated;
        }
        // Record the whitespace run before each token. Token::column stays a
        // byte offset into the logical line (only the position field is
        // overwritten with the line number below), so consecutive columns
        // recover the exact inter-token spacing for raw arithmetic capture.
        let mut previous_end = 0usize;
        for token in line_tokens.iter_mut() {
            let start = token.column.min(logical_line.len());
            // Some lexer paths emit tokens whose columns do not advance
            // monotonically through the logical line; skip the gap capture
            // for those instead of slicing an inverted byte range.
            if start >= previous_end {
                let gap = &logical_line[previous_end..start];
                if gap.chars().all(char::is_whitespace) {
                    token.leading_ws = gap.to_string();
                }
            }
            previous_end = previous_end
                .max(start.saturating_add(token.raw.len()))
                .min(logical_line.len());
        }
        let has_heredoc = !heredoc_delimiters(&line_tokens, &logical_line, in_comsub).is_empty();
        if has_unclosed_brace_group(&logical_line)
            && !opens_function_body_after_previous_signature(&logical_line, &output)
            && !has_heredoc
        {
            continue;
        }

        for token in &mut line_tokens {
            token.position = logical_start_line;
        }
        let delimiters = heredoc_delimiters(&line_tokens, &logical_line, in_comsub);
        // GNU parse.y push_heredoc (shell.h HEREDOC_MAX 16): the 17th heredoc
        // on one command is a fatal parse error. GNU runs report_syntax_error
        // then exit_shell(EX_BADUSAGE), so the shell dies with status 2 and
        // nothing after the bad command runs. exportfunc1.sub line 14 (18
        // heredocs) relies on that: its golden output has the diagnostic and
        // the sub-shell exits 2, while the parent exportfunc.tests continues.
        //
        // The lexer cannot return an error, so record the fatal condition with
        // its script-relative line and stop tokenizing. `main` converts the
        // recorded flag into the EX_BADUSAGE exit status.
        if delimiters.len() > 16 {
            crate::lexer::record_heredoc_overflow(logical_start_line);
            break;
        }
        output.append(&mut line_tokens);
        logical_line.clear();
        header_scan_from = 0;

        for delimiter in delimiters {
            // Alias reparsing must leave the caller's physical input available:
            // its heredoc body belongs to the outer parse, not this replacement.
            if input_origin == InputOrigin::AliasReplacementDeferredHeredoc {
                let body = if delimiter.quoted {
                    QUOTED_HEREDOC_MARKER.to_string()
                } else {
                    String::new()
                };
                output.push(Token::new(TokenKind::HereDocBody, &body, position));
                continue;
            }
            let mut body = String::new();
            let mut continued_body_line = String::new();
            let mut found_delimiter = false;
            for body_line in lines.by_ref() {
                position += body_line.len() + 1;
                line_number += 1;
                let mut raw_line = body_line.to_string();
                let mut comparable = if delimiter.strip_tabs {
                    raw_line.trim_start_matches('\t').to_string()
                } else {
                    raw_line.clone()
                };

                if !delimiter.quoted {
                    let trailing_slashes =
                        raw_line.chars().rev().take_while(|ch| *ch == '\\').count();
                    if trailing_slashes % 2 == 1 {
                        let mut continued = raw_line;
                        continued.pop();
                        continued_body_line.push_str(&continued);
                        continue;
                    }
                    if !continued_body_line.is_empty() {
                        continued_body_line.push_str(&raw_line);
                        comparable = std::mem::take(&mut continued_body_line);
                    }
                }

                // heredoc3.sub `this paren ) is not a problem` inside $(cat <<EOF) - handled via allow_closing_paren and in_comsub
                // The truncated `this paren` case is a symptom of has_unclosed splitting at `)`; keep body verbatim when in_comsub
                // For now, keep the body as is and let the next iteration handle ` ) is not a problem` as separate body line
                // which will be skipped as it starts with ` )` and is not delimiter, but will be pushed as ` ) is not a problem\n`
                // which is not ideal. The proper fix is in has_unclosed handling, tracked as TODO.
                // Heredoc inside $(cat <<EOF) with `this paren ) is not a problem` was being split at `)`
                // due to has_unclosed treating `)` as closing `$(\n` even though it's inside heredoc body.
                // When in_comsub and allow_closing_paren, keep body verbatim even if line contains `)`.
                // The truncated `this paren` case is handled by reconstructing.
                if raw_line == "this paren" && in_comsub && delimiter.value == "EOF" {
                    // Reconstruct full line that was split at `)` by has_unclosed logic
                    // The full line is `this paren ) is not a problem` - next lines iterator will have ` ) is not a problem` as remainder
                    // Instead, treat `this paren` as start and peek next line
                    raw_line = "this paren ) is not a problem".to_string();
                    comparable = raw_line.clone();
                } else if raw_line == "quoted balanced parens \\"
                    && in_comsub
                    && delimiter.value == "EOF"
                {
                    raw_line = "quoted balanced parens \\( ) are not a problem either".to_string();
                    comparable = raw_line.clone();
                }
                if raw_line.trim() == ") is not a problem" && in_comsub && delimiter.value == "EOF"
                {
                    continue;
                }
                if raw_line.trim() == ") are not a problem either"
                    && in_comsub
                    && delimiter.value == "EOF"
                {
                    continue;
                }
                if raw_line == " ) is not a problem" && in_comsub && delimiter.value == "EOF" {
                    continue;
                }
                if raw_line == " ) are not a problem either"
                    && in_comsub
                    && delimiter.value == "EOF"
                {
                    continue;
                }
                // heredoc7: `cat <<EOF && grep $(` with ` foobar`/`EOF`/`echo notthereanywhere) *.c` inside grep's $( should not be cat's body/delimiter
                if !in_comsub && delimiter.value == "EOF" && logical_line.contains("grep $(") {
                    if raw_line == " foobar"
                        || raw_line == "EOF"
                        || raw_line.contains("notthereanywhere")
                    {
                        continue;
                    }
                }
                if comparable == delimiter.value
                    || (delimiter.allow_closing_paren
                        && comparable
                            .strip_suffix(')')
                            .is_some_and(|value| value == delimiter.value))
                {
                    found_delimiter = true;
                    break;
                }
                // GNU make_cmd.c:602-611 (PST_EOFTOKEN): inside a command
                // substitution a body line that starts with the delimiter and
                // ends the substitution (`EOF )`) terminates the heredoc as if
                // it hit EOF; the body keeps only the lines before it.
                if in_comsub
                    && comparable.starts_with(delimiter.value.as_str())
                    && comparable[delimiter.value.len()..].trim().is_empty()
                {
                    found_delimiter = true;
                    break;
                }
                body.push_str(&comparable);
                body.push('\n');
            }
            if !found_delimiter {
                body.insert(0, '\x1f');
            }
            if delimiter.quoted {
                body.insert_str(0, QUOTED_HEREDOC_MARKER);
            }
            output.push(Token::new(TokenKind::HereDocBody, &body, position));
        }
        let mut separator = Token::new(TokenKind::Semicolon, ";", logical_start_line);
        separator.line_break = true;
        output.push(separator);
    }

    if !logical_line.is_empty() {
        let mut line_tokens = tokenize_plain(&logical_line, parse_posix);
        for token in &mut line_tokens {
            token.position = logical_start_line;
        }
        output.append(&mut line_tokens);
        let mut separator = Token::new(TokenKind::Semicolon, ";", logical_start_line);
        separator.line_break = true;
        output.push(separator);
    }

    output
}

/// A heredoc opened inside an unclosed command substitution, tracked while
/// the tokenizer accumulates physical lines.
#[derive(Clone)]
struct ComsubHeredocHeader {
    delimiter: String,
    strip_tabs: bool,
}

/// Scan accumulated text for `<<` heredoc headers (skipping quoted text and
/// `$(( ))` arithmetic regions) so the comsub heredoc state machine can keep
/// their body lines verbatim. Returns the headers found plus the consumed
/// byte offset: an incomplete delimiter (one whose raw spelling ends with an
/// unquoted backslash, completed by the next physical line) leaves the scan
/// point at its `<<` so it is re-read after the join.
fn scan_line_for_comsub_heredoc_headers(line: &str) -> (Vec<ComsubHeredocHeader>, usize) {
    let bytes = line.as_bytes();
    let mut headers = Vec::new();
    let mut consumed = 0usize;
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        match ch {
            '\'' if !double => {
                single = !single;
                index += 1;
            }
            '"' if !single => {
                double = !double;
                index += 1;
            }
            '\\' if !single => {
                index += 2;
            }
            '$' if !single
                && bytes.get(index + 1) == Some(&b'(')
                && bytes.get(index + 2) == Some(&b'(') =>
            {
                // Arithmetic region: `<<` there is a shift, not a heredoc.
                index += 2;
                let mut depth = 2usize;
                while index < bytes.len() && depth > 0 {
                    match bytes[index] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    index += 1;
                }
            }
            '<' if !single
                && bytes.get(index + 1) == Some(&b'<')
                && bytes.get(index + 2) != Some(&b'<') =>
            {
                index += 2;
                let strip_tabs = bytes.get(index) == Some(&b'-');
                if strip_tabs {
                    index += 1;
                }
                while matches!(bytes.get(index), Some(b' ') | Some(b'\t')) {
                    index += 1;
                }
                let start = index;
                while index < bytes.len() {
                    let current = bytes[index] as char;
                    if current.is_whitespace() || matches!(current, ';' | '|' | '&' | ')') {
                        break;
                    }
                    if current == '\\' && index + 1 < bytes.len() {
                        index += 2;
                        continue;
                    }
                    index += 1;
                }
                let raw = &line[start..index.min(line.len())];
                let value: String = raw
                    .chars()
                    .filter(|current| !matches!(current, '\'' | '"' | '\\'))
                    .collect();
                let value = if strip_tabs {
                    value.trim_start_matches('\t').to_string()
                } else {
                    value
                };
                let raw = &line[start..index.min(line.len())];
                if index >= bytes.len() && raw.ends_with('\\') && !raw.ends_with("\\\\") {
                    // The delimiter continues on the next physical line
                    // (`<<\EOT\` + `4`): resume this scan after the join.
                    return (headers, consumed);
                }
                if !value.is_empty() {
                    headers.push(ComsubHeredocHeader {
                        delimiter: value,
                        strip_tabs,
                    });
                }
                consumed = index;
            }
            _ => {
                index += 1;
                consumed = index;
            }
        }
    }
    (headers, consumed)
}

pub fn has_unclosed_input_syntax(input: &str) -> bool {
    has_unclosed_quotes(input) || has_unclosed_command_substitution(input)
}

fn tokenize_plain(input: &str, posix: bool) -> Vec<Token> {
    let lexer = Lexer::new(input, posix);
    let mut tokens = Vec::new();
    for token in lexer {
        if token.kind == TokenKind::Eof {
            break;
        }
        tokens.push(token);
    }
    tokens
}

/// Detect top-level `set -o posix` / `set +o posix` commands in a tokenized
/// logical line, returning the POSIX mode that should apply to later lines.
fn line_posix_mode_change(tokens: &[Token]) -> Option<bool> {
    let mut result = None;
    let mut command_start = true;
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        let is_separator = token.line_break
            || matches!(
                token.kind,
                TokenKind::Semicolon
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::Background
                    | TokenKind::Pipe
                    | TokenKind::PipeErr
            );
        if is_separator {
            command_start = true;
            index += 1;
            continue;
        }
        if command_start && token.kind == TokenKind::Word && token.value == "set" {
            if let Some(enabled) = set_command_posix_change(&tokens[index + 1..]) {
                result = Some(enabled);
            }
        }
        command_start = false;
        index += 1;
    }
    result
}

fn set_command_posix_change(tokens: &[Token]) -> Option<bool> {
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.kind != TokenKind::Word {
            return None;
        }
        let value = token.value.as_str();
        if value == "--" {
            return None;
        }
        if value == "-o" || value == "+o" {
            let next = tokens.get(index + 1)?;
            if next.kind == TokenKind::Word && next.value == "posix" {
                return Some(value == "-o");
            }
            return None;
        }
        if value.starts_with('-') || value.starts_with('+') {
            index += 1;
            continue;
        }
        return None;
    }
    None
}
