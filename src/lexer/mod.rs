//! Lexer Module - Bash Tokenizer
//!
//! Transforms raw input strings into tokens for the parser.

mod alias_stream;
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

use brace_scan::{
    has_unclosed_parameter_expansion, opens_function_body_after_previous_signature,
    tokens_open_unclosed_brace_group,
};
use continuation::{
    ends_with_unquoted_backslash, has_unclosed_compound_assignment, has_unclosed_quotes,
};

pub(crate) use alias_stream::{expand_aliases_in_source, AliasLookup};
pub(crate) use continuation::has_unclosed_command_substitution;
pub(crate) use continuation::unclosed_command_substitution_depth;
pub(crate) use continuation::unclosed_input_close_char_posix;
use heredoc::heredoc_delimiters;
use scanner::{Lexer, LexerParseState};
pub(crate) use skip::skip_parenthesized_unit_corrected;

use crate::executor::markers::DATA_DOLLAR;
pub(crate) use ansi::decode_ansi_c_quoted;
pub(crate) use quotes::remove_shell_quotes;
pub(crate) use quotes::{
    escape_decoded_ansi_c_quotes, ANSI_C_DQUOTE_MARKER, ANSI_C_DQUOTE_MARKER_STR,
    ANSI_C_QUOTE_MARKER, ANSI_C_QUOTE_MARKER_STR, PARAM_NAME_END_MARKER,
};
pub use token::{Token, TokenKind};

pub(crate) const QUOTED_HEREDOC_MARKER: &str = crate::executor::markers::QUOTED_HEREDOC_MARKER;

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

/// Returns the recorded here-document overflow line, if any, and clears it.
/// ${THIS_SH} scripts run in-process (external_finish.rs
/// execute_direct_shell_script), so a consumed flag must not leak into a
/// later tokenize in the same process.
pub fn heredoc_overflow_line() -> Option<usize> {
    use std::sync::atomic::Ordering;
    match HEREDOC_OVERFLOW_LINE.swap(0, Ordering::SeqCst) {
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
    // GNU bash does NOT strip CR from CRLF line endings: a '\r' left by a
    // Windows checkout is ordinary word text (e.g. `set ""\r` makes $1 = \r,
    // not the empty string). Rust's str::lines() strips trailing '\r', which
    // silently drops the byte. Split on '\n' only and keep '\r' in the line
    // content, matching GNU parse.y read_secondary_line. The trailing empty
    // string that split('\n') produces when the input ends with '\n' is
    // skipped below to match str::lines() semantics.
    let mut lines = input.split('\n').peekable();
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
    // GNU parse.y keeps one parser_state for the whole input: PST_CASEPAT,
    // last_read_token and expecting_in_command persist across physical
    // lines. Carry the same state between the per-logical-line Lexer
    // instances so `{` in a case pattern on its own line is still word
    // text, not a group opener.
    let mut lexer_parse_state = LexerParseState::default();

    while let Some(raw_line) = lines.next() {
        // niubash #106: a '\r' immediately before the '\n' belongs to the
        // CRLF line terminator, not to the last word — a Windows-native
        // shell must accept CRLF scripts (v1.1.1 did; the shipped
        // oh-my-niu bundle is 100% CRLF). The GNU-fidelity rule this loop
        // documents above still applies to a '\r' NOT followed by '\n':
        // `set ""<CR>` with a bare carriage return keeps $1 = "\r".
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        // str::lines() drops the trailing empty string that split('\n')
        // produces when the input ends with '\n'. Replicate that here.
        if line.is_empty() && lines.peek().is_none() {
            break;
        }
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
            // GNU make_cmd.c:602-611: when the `)` closing the command
            // substitution sits on the heredoc delimiter line (e.g. `EOF)`),
            // the heredoc is "delimited by end-of-file" and a warning is
            // issued. Mark the logical line with \x1c before the `)` so
            // command_substitution_heredoc_output_mut_typed can detect this
            // case. Only mark when the current line matches a tracked heredoc
            // delimiter followed by `)` — NOT when `)` is on the header line
            // (e.g. `cat << EOF)`), which is a different case handled by
            // heredoc_header_closes_command_substitution.
            if let Some(front) = comsub_heredocs.first().cloned() {
                let comparable = if front.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if comparable.starts_with(front.delimiter.as_str())
                    && comparable[front.delimiter.len()..].contains(')')
                {
                    // Insert \x1c before the first `)` after the delimiter
                    // in the logical line. command_substitution_heredoc_output_mut_typed
                    // will detect and remove it before parsing. The search is
                    // scoped to the physical line just appended: `rfind` on
                    // the whole logical line could land on the pushed-back
                    // `)` itself (delim `)`) or on a `)` from an earlier line.
                    let line_start = logical_line.len() - line.len();
                    let delim_end =
                        line_start + (line.len() - comparable.len()) + front.delimiter.len();
                    if let Some(rel_pos) = logical_line[delim_end..].find(')') {
                        logical_line
                            .insert(delim_end + rel_pos, crate::executor::markers::IFS_GLUE);
                    }
                }
            }
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
        // parse.y:5379-5384: a backslash before EOF is NOT removed — GNU's
        // read_token_word ungets EOF and keeps the `\` as a quoted literal
        // (`echo a\` prints `a\`), so no EOF-pop branch exists here. Only
        // `\`+`\n` joins lines (handled above).

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
        // GNU parse.y parse_comsub (PST_EOFTOKEN) + print_comsub
        // (parse.y:4632): a `)` on a heredoc header line inside `$(...)`
        // closes the substitution while the still-pending body was gathered
        // from the following input lines, and the substitution text is
        // reprinted with the body inside the closing `)`. Rotate
        // `$(cat <<EOF)\nfoo\nEOF` into `$(cat <<EOF\nfoo\nEOF)` so every
        // downstream consumer sees the GNU reprint order (heredoc7.sub).
        if let Some(rotated) = relocate_comsub_heredoc_paren(&logical_line) {
            logical_line = rotated;
        }
        // GNU reads tokens sequentially (parse.y read_token): the reader
        // state feeding reserved_word_acceptable (parse.y:5899) is the state
        // after the tokens BEFORE this logical line — newlines are just
        // whitespace between them. The join loop below re-tokenizes the
        // whole accumulated logical line after each joined physical line,
        // so every retry must replay from the SAME line-start state; the
        // end state of a partial tokenization describes the end of the
        // partial text, not the start of the longer one. Feeding it back
        // made `{)\t: brace ;;` fold after `esac` joined (the previous
        // partial ended with last=esac, so `{` sat in reserved-word
        // position) and swallowed the rest of the function body.
        let mut line_lex_state = lexer_parse_state.clone();
        let mut line_tokens = tokenize_plain(&logical_line, parse_posix, &mut line_lex_state);
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
        // Join forward only on signals the tokens themselves prove: an
        // unclosed reserved-word `{` group (see tokens_open_unclosed_brace_group)
        // or an unterminated `${...}` parameter expansion. The old text-level
        // has_unclosed_brace_group counted `case x in {)`'s pattern brace as a
        // group opener, joining the pattern line to far-away text.
        if (tokens_open_unclosed_brace_group(&line_tokens)
            || has_unclosed_parameter_expansion(&logical_line))
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
        // Commit only when the logical line is accepted: `continue` above
        // discards the partial state so the next, longer retry replays from
        // the same line-start state (see the comment at tokenize_plain).
        lexer_parse_state = line_lex_state;
        logical_line.clear();
        header_scan_from = 0;

        for delimiter in delimiters {
            // GNU parse.y:3120-3135 gather_here_documents passes the parser's
            // current line_number to make_here_document for each redirect: the
            // physical line on which the logical command line ended (this
            // line_number is already one past it), advanced by the body lines
            // any earlier heredoc of the same command consumed.  The
            // "here-document at line N" warning reports this gather line, not
            // the `<<` line, so it must travel with the body token.
            let gather_line = line_number.saturating_sub(1);
            // Alias reparsing must leave the caller's physical input available:
            // its heredoc body belongs to the outer parse, not this replacement.
            if input_origin == InputOrigin::AliasReplacementDeferredHeredoc {
                let body = if delimiter.quoted {
                    QUOTED_HEREDOC_MARKER.to_string()
                } else {
                    String::new()
                };
                output.push(Token::new(TokenKind::HereDocBody, &body, gather_line));
                continue;
            }
            let mut body = String::new();
            let mut continued_body_line = String::new();
            let mut found_delimiter = false;
            let mut found_with_warning = false;
            while let Some(body_line) = lines.next() {
                // Skip the trailing empty string produced by split('\n')
                // when the input ends with '\n' (matching the main loop's
                // str::lines() semantics). Without this, an unterminated
                // heredoc at EOF gets an extra empty body line, making the
                // warning line number off by 1.
                if body_line.is_empty() && lines.peek().is_none() {
                    break;
                }
                // niubash #106: heredoc bodies share the main loop's CRLF
                // rule — a trailing '\r' is the line terminator, so CRLF
                // scripts can match their own delimiters and bodies stay
                // clean. Bare '\r' (no following '\n') is preserved.
                let body_line = body_line
                    .strip_suffix('\r')
                    .unwrap_or(body_line)
                    .to_string();
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

                if comparable == delimiter.value {
                    found_delimiter = true;
                    break;
                }
                // GNU make_cmd.c:602-611 (PST_EOFTOKEN): inside a command
                // substitution, a body line that starts with the delimiter
                // and contains `)` (the shell_eof_token) terminates the
                // heredoc as if it hit EOF; the body keeps only the lines
                // before it, and a warning is issued (full_line=0).  This
                // covers `EOF)`, `EOF )`, and `))` (when the delimiter is
                // `)` itself).
                if delimiter.allow_closing_paren
                    && comparable.starts_with(delimiter.value.as_str())
                    && comparable[delimiter.value.len()..].contains(')')
                {
                    found_delimiter = true;
                    found_with_warning = true;
                    break;
                }
                // GNU make_cmd.c:602: a body line that is exactly the
                // delimiter followed by `)` (e.g. `EOF)`) also terminates
                // the heredoc with a warning on the non-comsub path.
                if delimiter.allow_closing_paren
                    && comparable
                        .strip_suffix(')')
                        .is_some_and(|value| value == delimiter.value)
                {
                    found_delimiter = true;
                    found_with_warning = true;
                    break;
                }
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
                body.insert(0, DATA_DOLLAR);
            } else if found_with_warning {
                body.insert(0, crate::executor::markers::HEREDOC_WARNED_BODY_PREFIX);
            }
            if delimiter.quoted {
                body.insert_str(0, QUOTED_HEREDOC_MARKER);
            }
            output.push(Token::new(TokenKind::HereDocBody, &body, gather_line));
        }
        let mut separator = Token::new(TokenKind::Semicolon, ";", logical_start_line);
        separator.line_break = true;
        output.push(separator);
        // GNU read_token reads this line break as a '\n' token before the
        // next line's first token (reserved_word_acceptable, parse.y:5902);
        // the separator above is emitted downstream of the Lexer, so the
        // carried reader state must record the break itself.
        lexer_parse_state.note_line_break();
    }

    if !logical_line.is_empty() {
        // GNU parse.y parse_comsub (PST_EOFTOKEN) + print_comsub
        // (parse.y:4632): a `)` on a heredoc header line inside `$(...)`
        // closes the substitution while the still-pending body was gathered
        // from the following input lines, and the substitution text is
        // reprinted with the body inside the closing `)`. Rotate
        // `$(cat <<EOF)\nfoo\nEOF` into `$(cat <<EOF\nfoo\nEOF)` so every
        // downstream consumer sees the GNU reprint order (heredoc7.sub).
        if let Some(rotated) = relocate_comsub_heredoc_paren(&logical_line) {
            logical_line = rotated;
        }
        let mut line_tokens = tokenize_plain(&logical_line, parse_posix, &mut lexer_parse_state);
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
pub(crate) struct ComsubHeredocHeader {
    pub(crate) delimiter: String,
    pub(crate) strip_tabs: bool,
}

/// Scan accumulated text for `<<` heredoc headers (skipping quoted text and
/// `$(( ))` arithmetic regions) so the comsub heredoc state machine can keep
/// their body lines verbatim. Returns the headers found plus the consumed
/// byte offset: an incomplete delimiter (one whose raw spelling ends with an
/// unquoted backslash, completed by the next physical line) leaves the scan
/// point at its `<<` so it is re-read after the join.
pub(crate) fn scan_line_for_comsub_heredoc_headers(
    line: &str,
) -> (Vec<ComsubHeredocHeader>, usize) {
    let bytes = line.as_bytes();
    let mut headers = Vec::new();
    let mut consumed = 0usize;
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    // `index` always sits on a char boundary: ASCII tokens step one byte and
    // every other char steps by its UTF-8 length. Widening bytes with
    // `bytes[i] as char` let continuation bytes 0x85/0xa0 (inside e.g.
    // U+60A0 悠 or the U+E0A0 powerline glyph) match is_whitespace and made
    // delimiter slices land mid-char (panic, same class as niubash#92).
    while index < bytes.len() {
        let ch = line[index..].chars().next().expect("index is a boundary");
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
                index += 1 + char_len_at(line, index + 1);
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
                // GNU read_token_word: quoting inside the delimiter word
                // makes metacharacters literal — `<< ')'` names `)` as the
                // delimiter, so a quoted `)` (or `;`, `|`, `&`) is delimiter
                // text, not the substitution closer (comsub-posix.tests).
                let mut delimiter_single = false;
                let mut delimiter_double = false;
                while index < bytes.len() {
                    let current = line[index..].chars().next().expect("index is a boundary");
                    match current {
                        '\'' if !delimiter_double => delimiter_single = !delimiter_single,
                        '"' if !delimiter_single => delimiter_double = !delimiter_double,
                        _ if !delimiter_single
                            && !delimiter_double
                            && (current.is_whitespace()
                                || matches!(current, ';' | '|' | '&' | ')')) =>
                        {
                            break;
                        }
                        '\\' if !delimiter_single
                            && !delimiter_double
                            && index + 1 < bytes.len() =>
                        {
                            index += 1 + char_len_at(line, index + 1);
                            continue;
                        }
                        _ => {}
                    }
                    index += current.len_utf8();
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
                index += ch.len_utf8();
                consumed = index;
            }
        }
    }
    (headers, consumed)
}

/// UTF-8 length of the char starting at `index`; 1 when `index` is out of
/// range (trailing escape at end of line), matching the old `+= 2` step.
fn char_len_at(line: &str, index: usize) -> usize {
    line.get(index..)
        .and_then(|rest| rest.chars().next())
        .map_or(1, char::len_utf8)
}

pub fn has_unclosed_input_syntax(input: &str) -> bool {
    has_unclosed_input_syntax_posix(input, false)
}

/// POSIX-aware variant: `set -o posix` changes how `'` inside `"${...}"`
/// scans (Interp 221), so the unclosed-delimiter probe must know the mode.
pub fn has_unclosed_input_syntax_posix(input: &str, posix: bool) -> bool {
    has_unclosed_quotes(input)
        || (has_unclosed_command_substitution(input)
            && !skip::command_substitutions_balanced(input))
        // A bare `(`/`{`-class delimiter can also keep a command open:
        // `ddd=(aaa` array lists and `( cmd` subshells continue on the
        // next line (GNU parse.y reads until the matching close).
        || unclosed_input_close_char_posix(input, posix).is_some()
}

/// parse.y:5379-5384 read_token_word: a backslash before the newline is
/// removed with it ("ignored in all cases except when quoted with single
/// quotes"), so a physical line ending in an unquoted backslash is an
/// unfinished token -- the incremental stdin driver must keep reading (PS2),
/// not submit the line with the backslash dropped. Drivers accumulate with
/// the newline attached, so probe the text before it; a retained CR (CRLF
/// line) then correctly reads as an ESCAPED carriage return, which GNU also
/// does not treat as a continuation.
pub fn stdin_line_ends_with_continuation(input: &str) -> bool {
    ends_with_unquoted_backslash(input.strip_suffix('\n').unwrap_or(input))
}

/// Rotate the `)`-that-closed-on-the-header-line segment of a command
/// substitution's heredoc past the gathered body, mirroring GNU
/// print_comsub's reprint order. Returns None when the input carries no such
/// pattern.
fn relocate_comsub_heredoc_paren(input: &str) -> Option<String> {
    if !input.contains("$(") || !input.contains("<<") {
        return None;
    }
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut depth = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => {
                single = true;
                index += 1;
            }
            '"' if !single => {
                double = !double;
                index += 1;
            }
            '$' if !double
                && chars.get(index + 1) == Some(&'(')
                && chars.get(index + 2) != Some(&'(') =>
            {
                depth += 1;
                index += 2;
            }
            '(' if depth > 0 && !double => {
                depth += 1;
                index += 1;
            }
            ')' if depth > 0 && !double => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            '<' if depth > 0
                && !double
                && chars.get(index + 1) == Some(&'<')
                && chars.get(index + 2) != Some(&'<') =>
            {
                let (next, closure) =
                    heredoc_scan::skip_heredoc_in_chars_with_closure(&chars, index);
                if let Some((paren, header_end)) = closure {
                    // `)` + its header-line tail move past the gathered body:
                    // `$(cat <<EOF)\nbody\nEOF` reads as GNU's reprint
                    // `$(cat <<EOF\nbody\nEOF)`.
                    let mut out: String = chars[..paren].iter().collect();
                    out.extend(chars[header_end..next].iter());
                    out.extend(chars[paren..header_end].iter());
                    out.extend(chars[next..].iter());
                    return Some(out);
                }
                index = next.max(index + 1);
            }
            _ => index += 1,
        }
    }
    None
}

fn tokenize_plain(input: &str, posix: bool, parse_state: &mut LexerParseState) -> Vec<Token> {
    let mut lexer = Lexer::new(input, posix);
    // parse.y keeps a single parser_state for the whole input — resume the
    // PST_CASEPAT / last_read_token state left by the previous logical line.
    lexer.restore_parse_state(parse_state.clone());
    let mut tokens = Vec::new();
    for token in &mut lexer {
        if token.kind == TokenKind::Eof {
            break;
        }
        tokens.push(token);
    }
    *parse_state = lexer.take_parse_state();
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
