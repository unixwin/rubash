//! Lexer Module - Bash Tokenizer
//!
//! Transforms raw input strings into tokens for the parser.

mod alias_stream;
pub(crate) mod ansi;
mod brace_scan;
mod brace_scan_cache;
mod classification;
mod continuation;
pub(crate) mod dolbrace;
mod heredoc;
mod heredoc_scan;
mod number_redirect;
pub(crate) mod quotes;
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
use brace_scan_cache::BraceScanCache;
pub(crate) use continuation::has_unclosed_command_substitution;
pub(crate) use continuation::unclosed_command_substitution_depth;
pub(crate) use continuation::unclosed_input_close_char_posix;
use heredoc::heredoc_delimiters;
use scanner::{Lexer, LexerParseState};
pub(crate) use skip::command_substitutions_balanced;
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

/// Parse-time `extended_glob` (rubash#131).
///
/// GNU gates extglob pattern operators (`?(`, `*(`, `+(`, `@(`, `!(`) on the
/// runtime `extended_glob` variable inside `read_token_word`
/// (parse.y:5466 `if MBTEST(extended_glob && PATTERN_CHAR (character))`),
/// which `reset_parser` syncs from the `extglob` shopt flag (parse.y:3502
/// `extended_glob = extglob_flag`) and the shopt builtin updates live
/// (builtins/shopt.def). Bash 5.3 defaults to
/// `shell_compatibility_level` 53 (version.c DEFAULT_COMPAT_LEVEL =
/// `${dist_major}${dist_minor}`), so parse.y:4538's parse_comsub forcing
/// (`shell_compatibility_level <= 51`) does NOT apply: `$(` command
/// substitution bodies are gated by the shopt too (verified: GNU 5.3.0
/// rejects `r=$(case x in ?(a)) :;; esac)` with rc 2). The only exception
/// is the `[[ ... ]]` pattern/regexp right-hand side
/// (parse.y:5203-5210 parse_cond_command forces extended_glob under
/// PST_EXTPAT), which rubash's conditional parser already handles by
/// merging the RHS fragments (conditional_command.rs
/// merge_pattern_rhs_fragments).
///
/// Rubash tokenizes a whole script before anything executes, so this static
/// stands in for GNU's variable: the tokenizer's line loop flips it when it
/// sees a top-level `shopt -s/-u extglob` (mirroring GNU's parse-execute
/// cadence, the same granularity `line_posix_mode_change` uses for
/// `set -o posix`), and the shopt builtin keeps it current as commands
/// execute. Initialized off: `extglob` is off by default in GNU.
static PARSE_EXTENDED_GLOB: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the commands of the script being tokenized will actually
/// execute. Under `-n` (noexec) GNU still parses every command but never
/// runs the `shopt` builtin, so a top-level `shopt -s extglob` line must
/// NOT open the parse gate for later lines (verified: GNU 5.3.0 `bash -n`
/// on `shopt -s extglob` + `case x in ?(a)) ...` fails with rc 2).
static PARSE_EXECUTION_EXPECTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Current parse-time extglob gate (see `PARSE_EXTENDED_GLOB`).
pub(crate) fn parse_extended_glob() -> bool {
    PARSE_EXTENDED_GLOB.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set the parse-time extglob gate. Called by the shopt builtin when
/// `extglob` is turned on or off (GNU builtins/shopt.def updates
/// extglob_flag, and reset_parser parse.y:3502 propagates it to
/// extended_glob).
pub(crate) fn set_parse_extended_glob(enabled: bool) {
    PARSE_EXTENDED_GLOB.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Record whether the current shell invocation will execute commands
/// (`-n` means it will not; GNU shell.c reader_loop still parses).
pub fn set_parse_execution_expected(expected: bool) {
    PARSE_EXECUTION_EXPECTED.store(expected, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the line-loop shopt simulation may run (see
/// `PARSE_EXECUTION_EXPECTED`).
pub(crate) fn parse_execution_expected() -> bool {
    PARSE_EXECUTION_EXPECTED.load(std::sync::atomic::Ordering::Relaxed)
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
    let result =
        tokenize_with_heredocs_inner(input, initial_posix, input_origin, start_line, in_comsub);
    TOKENIZE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    result
}

fn tokenize_with_heredocs_inner(
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
    // rubash#131: flips of the parse-time extglob gate are detected only in
    // the outermost script tokenization. Nested tokenizations (a brace-group
    // body re-tokenized by the folding parser, a `$(` body parsed inline)
    // are not execution boundaries in GNU: the enclosing command is parsed
    // as a unit before any of its lines could run a `shopt'.
    let mut extglob_flips_allowed = true;
    let tokenize_depth = TOKENIZE_DEPTH.with(|depth| {
        let value = depth.get() + 1;
        depth.set(value);
        value
    });
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
    // rubash#176/#178: resumable brace-group scan cache for the accumulating
    // logical line (see brace_scan_cache.rs). Cleared on every mutation of
    // `logical_line` that is not a pure append, and when the line is
    // accepted and flushed.
    let mut brace_cache = BraceScanCache::default();
    // rubash#155 / #130: whether the previous physical line was joined by
    // the token-level brace-group signal (`tokens_open_unclosed_brace_group`
    // below) with every text-level scan closed at that point. While true, an
    // appended physical line that provably cannot open or close any
    // construct takes the fast path below and skips the O(buffer)
    // re-tokenization and re-scans entirely.
    let mut brace_join_active = false;
    // rubash#241 (perf2): checkpoint for the per-line `has_unclosed_quotes`
    // rescan, same append-only discipline as `brace_cache`. The captain's
    // scanner (continuation.rs:663) answers `single || double || ansi_single`
    // from a state whose ONLY quote-state transitions are the bytes
    // `\ " ' $ \`` (`$` pairing with `{`, `(`, `'` via one-byte lookahead;
    // `${`/`$(`/backtick spans never cross the text end — an unclosed span
    // falls through and toggles quotes within the already-scanned text).
    // Appending "\n" + an inert physical line therefore cannot change the
    // answer: `\n`, `#` and whitespace only move the comment sub-state, and
    // every other byte falls to the no-op arm. A line accepted by
    // `brace_join_fast_path_line` is inert under a SUPERSET of this byte
    // set, so the brace fast-path `continue` keeps the checkpoint valid.
    // Invalidated at every non-append mutation of `logical_line` (IFS_GLUE
    // insert, backslash pop, comsub-heredoc rotation, flush).
    let mut unclosed_quotes_cache: Option<bool> = None;

    while let Some(raw_line) = lines.next() {
        // niubash #106: a '\r' immediately before the '\n' belongs to the
        // CRLF line terminator, not to the last word — a Windows-native
        // shell must accept CRLF scripts (v1.1.1 did; the shipped
        // oh-my-niu bundle is 100% CRLF). The GNU-fidelity rule this loop
        // documents above still applies to a '\r' NOT followed by '\n':
        // `set ""<CR>` with a bare carriage return keeps $1 = "\r".
        // rubash#140: the tolerance is a Windows product decision
        // (CRT text-mode compensation, niubash#120 family). GNU on unix
        // keeps the '\r' as literal word data — a CRLF script there fails
        // with `$'getopts\r': command not found` (parse.y read_token /
        // read_secondary_line have no CR stripping) — so gate it.
        let line = if cfg!(windows) {
            raw_line.strip_suffix('\r').unwrap_or(raw_line)
        } else {
            raw_line
        };
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

        // rubash#155 / #130 fast path. GNU reads tokens sequentially
        // (parse.y:3557 read_token): one pass over the input, with the
        // reader state carried token to token. The full-buffer
        // re-tokenization below is this tokenizer's substitute for that
        // streaming model, and re-running it per appended physical line is
        // what made a single `{` group spanning N lines O(N^2) (nvm.sh
        // shape: 4000 lines = 14.3s vs GNU 9.7ms).
        //
        // While the only reason the logical line stays open is the
        // token-level brace-group signal from the previous pass (quotes,
        // command substitutions and compound assignments were all closed —
        // reaching the join `continue` below proves that), a physical line
        // without any quote, escape, expansion, brace, paren, comment,
        // heredoc or `posix` bytes provably cannot change any of the
        // decisions this iteration would recompute:
        //
        // - has_unclosed_quotes / _command_substitution /
        //   _compound_assignment / _parameter_expansion: opening any of
        //   them needs ' " ` $ ( { bytes; closing the already-open `${...}`
        //   alternative needs `}`.
        // - tokens_open_unclosed_brace_group: the standalone `{` keyword
        //   flag persists — the group only folds when skip_brace finds its
        //   `}`, and `brace_group_contains_heredoc_operator` can only gain
        //   a `<<` (both need `}` / `<` bytes; even then the `{` stays a
        //   standalone keyword either way).
        // - ends_with_unquoted_backslash: needs a `\`; the backslash join
        //   that popped one cannot have been taken on the previous pass.
        // - line_posix_mode_change: the `posix` token of `set -o posix`
        //   requires the contiguous substring once quoting and escaping
        //   bytes are excluded.
        // - heredoc_delimiters / relocate_comsub_heredoc_paren / the
        //   comsub-heredoc header scan: all need `<`, `$` or quote bytes;
        //   with the command substitution closed, keep header_scan_from
        //   pacing the accumulated text exactly as the slow path's closed
        //   branch does.
        //
        // Every other intermediate result (token gap capture, heredoc
        // delimiter line numbers, line_posix_mode_change) is discarded by
        // the join `continue` and recomputed by the final full pass, which
        // still runs the unchanged code below — so the accepted token
        // stream is byte-identical; only the per-line work drops from
        // O(accumulated buffer) to O(this line).
        if brace_join_active && brace_join_fast_path_line(line) {
            header_scan_from = logical_line.len();
            continue;
        }

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
                        // Mid-string rewrite: positional scan cache invalid.
                        brace_cache.clear();
                        unclosed_quotes_cache = None;
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
            // The popped byte changes the text every later offset depends
            // on: positional scan cache invalid.
            brace_cache.clear();
            unclosed_quotes_cache = None;
            continued_line = true;
            brace_join_active = false;
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

        let hq = match unclosed_quotes_cache.filter(|_| line_is_quote_inert(line)) {
            Some(cached) => cached,
            _ => {
                let value = has_unclosed_quotes(&logical_line);
                unclosed_quotes_cache = Some(value);
                value
            }
        };
        if hq {
            brace_join_active = false;
            continue;
        }
        if has_unclosed_command_substitution(&logical_line) {
            brace_join_active = false;
            continue;
        }
        // A `name=(` compound array assignment keeps reading physical lines
        // until its matching `)` (parse.y; ISSUE #78).
        if has_unclosed_compound_assignment(&logical_line) {
            brace_join_active = false;
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
            // Rotation rewrites the middle of the line: cache invalid.
            brace_cache.clear();
            unclosed_quotes_cache = None;
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
        let mut line_tokens = tokenize_plain(
            &logical_line,
            parse_posix,
            &mut line_lex_state,
            &mut brace_cache,
        );
        if let Some(updated) = line_posix_mode_change(&line_tokens) {
            parse_posix = updated;
        }
        // rubash#131: a top-level `shopt -s/-u extglob` executes before the
        // next line parses in GNU's read-execute loop; mirror that for the
        // parse-time gate (same per-line granularity as the posix flip
        // above). Disabled under -n (nothing executes) and after a top-level
        // `set -n` (GNU executes `set -n` and then only parses).
        // `tokenize_depth == 1` is the outermost script tokenization.
        if parse_execution_expected() && extglob_flips_allowed && tokenize_depth == 1 {
            match line_extglob_mode_change(&line_tokens) {
                ExtglobFlip::Enable => set_parse_extended_glob(true),
                ExtglobFlip::Disable => set_parse_extended_glob(false),
                ExtglobFlip::ExecutionOff => extglob_flips_allowed = false,
                ExtglobFlip::None => {}
            }
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
            // Reaching here proves quotes, command substitutions and
            // compound assignments are all closed: the join stands on the
            // token-level brace-group flag (and/or an open `${...}`, which
            // an inert line cannot close either). Arm the rubash#155 fast
            // path for the next physical line.
            brace_join_active = true;
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
        unclosed_quotes_cache = None;
        // Offsets restart for the next logical line: cache invalid.
        brace_cache.clear();
        header_scan_from = 0;
        brace_join_active = false;

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
                // rubash#140: Windows-only tolerance; GNU on unix keeps
                // the '\r' as delimiter/body data (make_here_document
                // compares the raw line, make_cmd.c).
                let body_line = if cfg!(windows) {
                    body_line
                        .strip_suffix('\r')
                        .unwrap_or(body_line)
                        .to_string()
                } else {
                    body_line.to_string()
                };
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
            brace_cache.clear();
        }
        let mut line_tokens = tokenize_plain(
            &logical_line,
            parse_posix,
            &mut lexer_parse_state,
            &mut brace_cache,
        );
        // GNU parse.y line_number is PHYSICAL: the reader increments it per
        // physical line consumed, and the unclosed-construct EOF diagnostics
        // ("unexpected end of file from `{' command on line N") number both
        // the innermost opener and the EOF position from it. The leftover
        // logical line at end of input spans every physical line the open
        // construct accumulated, so map each token's byte offset (its
        // `column`) back to its physical line instead of stamping the whole
        // run with the first line (`{ </n>cmd1 </n>cmd2` reported the EOF at
        // line 2 where GNU reports line 4, rubash#278).
        let leftover_spans_lines = logical_line.contains('\n');
        for token in &mut line_tokens {
            token.position = if leftover_spans_lines {
                logical_start_line
                    + logical_line[..token.column.min(logical_line.len())]
                        .matches('\n')
                        .count()
            } else {
                logical_start_line
            };
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
pub fn unclosed_array_subscript_line(input: &str) -> Option<(usize, bool)> {
    skip::unclosed_array_subscript_line(input)
}

pub fn has_unclosed_input_syntax_posix(input: &str, posix: bool) -> bool {
    has_unclosed_quotes(input)
        || (has_unclosed_command_substitution(input)
            && !skip::command_substitutions_balanced(input))
        // GNU parse.y:5635-5643: an unclosed array subscript `[` swallows
        // the rest of the input ahead of any other matched-pair construct
        // (`x=([a` reports `]`, not `)`) — check it before the generic
        // close-char scan (rubash#221).
        || skip::unclosed_array_subscript_line(input).is_some()
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

/// rubash#155 / #130: whether a physical line appended to a logical line
/// whose only open construct is the token-level brace-group join provably
/// cannot change any lexer decision the join iteration recomputes.
///
/// The line must contain no byte that any consumer between the append and
/// the join `continue` can react to: quote characters (has_unclosed_quotes,
/// and quoting state inside every scanner), backslash (line continuations,
/// escapes), `$` (parameter/command/arithmetic substitution openers),
/// `{`/`}` (brace-group depth: a `}` could fold the group and end the join),
/// `(`/`)` (compound assignments, subshells), `` ` `` (backtick
/// substitutions), `#` (comments), `<` (heredoc operators and delimiters).
/// The `posix` substring check keeps the per-pass `set -o posix` detection
/// (`line_posix_mode_change`) exact: without quoting or escaping bytes the
/// `posix` word token implies this contiguous substring in the line, and
/// with it the slow path runs and computes the real answer.
///
/// This is an admission whitelist, not a symptom blacklist (rubash#117
/// rule): a false negative only costs the O(buffer) slow path that the
/// unchanged code below already implements; every admitted line is proven
/// inert for the decisions listed in the fast-path comment in
/// `tokenize_with_heredocs`.
fn brace_join_fast_path_line(line: &str) -> bool {
    if line.bytes().any(|b| {
        matches!(
            b,
            b'\'' | b'"' | b'`' | b'$' | b'{' | b'}' | b'(' | b')' | b'#' | b'<' | b'\\'
        )
    }) {
        return false;
    }
    !line.contains("posix")
}

/// True when the physical line contains no byte that can move the quote
/// state of `has_unclosed_quotes` (continuation.rs:663): the scanner's only
/// quote-state transitions are `\ " ' $ \`` (with `$` pairing via one-byte
/// lookahead); everything else — including `#`, whitespace and newlines —
/// only touches the comment sub-state, which the answer ignores. The
/// checkpoint `unclosed_quotes_cache` reuses its previous
/// `has_unclosed_quotes` answer exactly when the appended physical line is
/// inert under this predicate (rubash#241).
fn line_is_quote_inert(line: &str) -> bool {
    !line
        .bytes()
        .any(|b| matches!(b, b'\\' | b'"' | b'\'' | b'$' | b'`'))
}

thread_local! {
    static TOKENIZE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What a top-level line's tokens say about the parse-time extglob gate.
enum ExtglobFlip {
    None,
    /// `shopt -s extglob` seen: later lines parse with the gate open.
    Enable,
    /// `shopt -u extglob` seen: later lines parse with the gate closed.
    Disable,
    /// A top-level `set -n` executed: from here on GNU parses without
    /// executing, so shopt lines must stop flipping the gate.
    ExecutionOff,
}

/// Detect a top-level `shopt -s/-u extglob` (or `set -n`) command in a
/// tokenized logical line. `shopt` commands nested inside a compound
/// (function body, brace group, subshell, if/while/for/case) do not count:
/// GNU parses the whole enclosing definition before any of its body
/// commands could execute (verified: GNU 5.3.0 rejects
/// `g() { shopt -s extglob; case x in ?(a)) :;; esac; }` with rc 2).
fn line_extglob_mode_change(tokens: &[Token]) -> ExtglobFlip {
    let mut result = ExtglobFlip::None;
    let mut command_start = true;
    // Compound opener stack (innermost last). A closer pops only when it
    // matches the top, so a case-clause `)` inside `case ... esac` never
    // pops the case frame, and a folded `{ ... }' keyword token (a complete
    // compound in one token) neither opens nor closes a frame.
    let mut compound: Vec<&'static str> = Vec::new();
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
        if token.kind == TokenKind::Keyword {
            let closes = match token.value.as_str() {
                "}" => Some("}"),
                ")" => Some(")"),
                "fi" => Some("fi"),
                "done" => Some("done"),
                "esac" => Some("esac"),
                "]]" => Some("]]"),
                _ => None,
            };
            let opens = match token.value.as_str() {
                "{" => Some("}"),
                "(" => Some(")"),
                "if" => Some("fi"),
                "while" | "until" | "for" | "select" => Some("done"),
                "case" => Some("esac"),
                "[[" => Some("]]"),
                _ => None,
            };
            if let Some(closer) = closes {
                if compound.last() == Some(&closer) {
                    compound.pop();
                }
            } else if let Some(closer) = opens {
                if !token.value.starts_with('{') || token.value.trim() == "{" {
                    compound.push(closer);
                }
            }
        }
        if command_start && compound.is_empty() {
            if token.kind == TokenKind::Word && token.value == "shopt" {
                if let Some(enabled) = shopt_extglob_change(&tokens[index + 1..]) {
                    result = if enabled {
                        ExtglobFlip::Enable
                    } else {
                        ExtglobFlip::Disable
                    };
                }
            } else if token.kind == TokenKind::Word && token.value == "set" {
                if tokens[index + 1..]
                    .iter()
                    .take_while(|next| next.kind == TokenKind::Word)
                    .any(|next| next.value == "-n")
                {
                    result = ExtglobFlip::ExecutionOff;
                }
            }
        }
        command_start = false;
        index += 1;
    }
    result
}

/// `shopt` argument scan: does this command turn `extglob` on or off?
/// Mirrors the argument walk of GNU `shopt_builtin`
/// (builtins/shopt.def:292-340) and `toggle_shopts` (shopt.def:469-485):
/// internal_getopt consumes the `-p/-s/-u/-o/-q` flags, then EVERY
/// remaining word is an option NAME — each recognized name is set or unset,
/// and an unrecognized name only fails that one entry (shopt_error,
/// shopt.def:472-476) while the walk continues. So `shopt -s nullglob
/// extglob` DOES turn extglob on for later parses (shopt.def:640 propagates
/// extglob_flag to the parser's extended_glob; read_token_word then accepts
/// `@(` at parse.y:5464-5466) — the printf.tests line-358 form. Two GNU
/// error paths set nothing and must report no flip: `-s` and `-u` together
/// ("cannot set and unset shell options simultaneously", shopt.def:327-330)
/// and an unknown flag letter (builtin_usage, shopt.def:317-318). A `-o`
/// invocation routes extglob through set_shopt_o_options, where it is not
/// a -o option name and is never set (shopt.def:337-341) — also no flip.
fn shopt_extglob_change(args: &[Token]) -> Option<bool> {
    // The single mode GNU collects from the flags (-s/-u; both = error).
    let mut mode = None;
    // `shopt -o(-s|-u) ...` sets only -o option names, never extglob.
    let mut o_names_only = false;
    let mut flags_ended = false;
    for token in args {
        if token.kind != TokenKind::Word {
            // A redirection (`shopt -s extglob 2>/dev/null`) is not part of
            // the builtin's WORD_LIST (GNU loptend skips it), so it only
            // ENDS the scan here — an `extglob' name already seen still
            // counts.
            break;
        }
        if !flags_ended && token.value.starts_with('-') && token.value != "-" {
            if token.value == "--" {
                flags_ended = true;
                continue;
            }
            for flag in token.value[1..].chars() {
                match flag {
                    's' => {
                        if mode == Some(false) {
                            // -s and -u together: GNU fails the whole builtin.
                            return None;
                        }
                        mode = Some(true);
                    }
                    'u' => {
                        if mode == Some(true) {
                            return None;
                        }
                        mode = Some(false);
                    }
                    'o' => o_names_only = true,
                    'p' | 'q' => {}
                    // Unknown flag letter: GNU prints usage and sets nothing.
                    _ => return None,
                }
            }
            continue;
        }
        match token.value.as_str() {
            "extglob" => return if o_names_only { None } else { mode },
            // Any other word is one more option NAME for the same mode;
            // GNU continues past names it does not recognize.
            _ => continue,
        }
    }
    None
}

fn tokenize_plain(
    input: &str,
    posix: bool,
    parse_state: &mut LexerParseState,
    brace_cache: &mut BraceScanCache,
) -> Vec<Token> {
    let mut lexer = Lexer::new_with_cache(input, posix, brace_cache);
    lexer.extended_glob = parse_extended_glob();
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
