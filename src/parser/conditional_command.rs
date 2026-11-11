use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_conditional_command(
    tokens: &[Token],
    start: usize,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(CommandNode, usize)> {
    if tokens.get(start)?.value != "[[" {
        return None;
    }

    let (end, merged_args) = conditional_end_and_args(tokens, start)?;
    let arg_parts: Vec<(String, String)> = merged_args
        .iter()
        .map(|(value, raw, _, _)| (value.clone(), raw.clone()))
        .collect();
    let args = arg_parts
        .iter()
        .map(|(arg, _)| arg.clone())
        .collect::<Vec<_>>();
    let arg_metadata = arg_parts
        .iter()
        .enumerate()
        .map(|(index, (arg, raw))| build_word_metadata(index, arg, raw))
        .collect::<Vec<_>>();
    let expression_arg_parts = merged_args
        .last()
        .is_some_and(|(arg, _, _, _)| arg == "]]")
        .then(|| &merged_args[..merged_args.len() - 1])
        .unwrap_or(merged_args.as_slice());
    let expression_args: Vec<(String, String)> = expression_arg_parts
        .iter()
        .map(|(value, raw, _, _)| (value.clone(), raw.clone()))
        .collect();
    let expression = conditional_expression(&expression_args);

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
    if let Some((spec, echo_line)) = conditional_syntax_error_spec(
        &merged_args,
        true,
        stream_eof_line(tokens, diagnostic_text, source_line_offset),
        cond_follower_text(tokens, end),
        diagnostic_text,
        source_line_offset,
    ) {
        command.insert_assignment("__RUBASH_PARSE_ERROR_COND__".to_string(), spec);
        if let Some(source) = conditional_offending_line(
            tokens,
            start,
            end,
            echo_line,
            diagnostic_text,
            source_line_offset,
        ) {
            command.insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
        }
    }
    command.words.push("[[".to_string());
    command.words.extend(args.clone());
    command.conditional_command = Some(Box::new(ConditionalCommand {
        open_delimiter: tokens[start].value.clone(),
        open_delimiter_metadata: token_metadata(&tokens[start]),
        args,
        arg_metadata,
        close_delimiter: tokens[end].value.clone(),
        close_delimiter_metadata: token_metadata(&tokens[end]),
        expression,
    }));

    Some(finish_compound_command(command, tokens, end + 1))
}

/// parse.y cond_term/cond_and/cond_or (5038-5246) + cond_error (5013-5029)
/// + report_syntax_error (6833-6900): run GNU's recursive-descent
/// conditional grammar over the collected arguments and encode the exact
/// diagnostic sequence GNU prints for `[[` syntax errors.
///
/// Marker spec for `__RUBASH_PARSE_ERROR_COND__` (PARSE_ERROR_FIELD_SEP):
///   `near` SEP near_token SEP near_line SEP (msg_line SOH msg)*
///   `eof`  SEP open_line  SEP eof_line  SEP (msg_line SOH msg)*
/// `near` prints each message, then `syntax error near `tok'` and echoes the
/// offending source line. `eof` prints each message, then
/// `syntax error: unexpected end of file from `[[' command on line N`.
/// The line GNU reports for an EOF discovered while scanning to the
/// end of the input: parse_matched_pair consumes raw text up to EOF,
/// so the report lands one past the last newline of the whole stream
/// (parse.y:5223-5227 at the EOF line). Falls back to the last token's
/// line + 1 when the source text is unavailable — the same fallback
/// shape conditional_eof_error_command already uses.
fn stream_eof_line(
    tokens: &[Token],
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> usize {
    if let Some(text) = diagnostic_text {
        // The text is this parse input's own source; its lines are
        // numbered from source_line_offset (token positions carry the
        // same offset), so the EOF line is offset + newlines + 1 — plus
        // one more when the final line is unterminated: GNU's reader
        // completes that line at EOF, so its EOF diagnostics (the
        // parse.y:5223-5227 report among them) land one past it either
        // way (`bash -c '[[ abc =~ ([a-z]]]'` reports line 2, as does
        // the same text as a file with or without a trailing newline).
        let unterminated_tail = !text.is_empty() && !text.ends_with('\n');
        return text.matches('\n').count()
            + 1
            + usize::from(unterminated_tail)
            + source_line_offset;
    }
    tokens.last().map(|token| token.position + 1).unwrap_or(1)
}

/// The source text of the token that follows the closing `]]' when it
/// is glued to it (no leading whitespace) and is an operator: GNU's
/// follow-on yyerror (parse.y:6727) runs error_token_from_text with the
/// reader index sitting after that lookahead token, so the backward
/// walk runs over the follower's text too — a glued `;' is the report,
/// a glued `&&' reports its break character `&', a glued `>' reports
/// the whole `]]>' run. A spaced follower starts a fresh token run and
/// the report stays the `]]' (rubash#2750). `>>' is capped to its first
/// character: GNU's reader index for the append operator sits after
/// the first `>' (`[[ a == ]]>>f' reports `]]>', like the single-`>'
/// form).
fn cond_follower_text(tokens: &[Token], end: usize) -> Option<String> {
    let follower = tokens.get(end + 1)?;
    if !follower.leading_ws.is_empty() || follower.line_break {
        return None;
    }
    let operator = matches!(
        follower.kind,
        TokenKind::Semicolon
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Pipe
            | TokenKind::PipeErr
            | TokenKind::Background
            | TokenKind::RedirectOut
            | TokenKind::RedirectIn
            | TokenKind::Append
            | TokenKind::RedirectErr
            | TokenKind::RedirectErrAppend
            | TokenKind::HereDoc
            | TokenKind::HereString
    ) || (follower.kind == TokenKind::Keyword
        && matches!(follower.value.as_str(), "(" | ")"))
        // Case-clause terminators lex as words in this position but
        // are operators to the reader: `[[ a == ]];;' reports the
        // follow-on `;' exactly like a glued separator.
        || (follower.kind == TokenKind::Word
            && matches!(follower.raw.as_str(), ";;" | ";&" | ";;;&"));
    if !operator {
        return None;
    }
    // Redirect followers contribute only their FIRST character to
    // the follow-on walk: GNU's reader position when the conditional
    // parser trips is just past the operator's first byte, so
    // `==]]>&f' walks to `==]]>' and `==]]<<<x' to `==]]<', exactly
    // like the `>>' → `>' cap (rubash#2750).
    if matches!(
        follower.kind,
        TokenKind::RedirectOut
            | TokenKind::RedirectIn
            | TokenKind::Append
            | TokenKind::RedirectErr
            | TokenKind::RedirectErrAppend
            | TokenKind::HereDoc
            | TokenKind::HereString
    ) {
        return Some(follower.raw.chars().take(1).collect());
    }
    Some(follower.raw.clone())
}

pub(super) fn conditional_syntax_error_spec(
    args: &[CondArg],
    closed: bool,
    stream_eof_line: usize,
    follower: Option<String>,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(String, Option<usize>)> {
    let open_line = args.first().map(|(_, _, _, line)| *line).unwrap_or(1);
    let mut cursor = CondCursor {
        args,
        index: 0,
        closed,
        stream_eof_line,
        newline_consumed_for: None,
        virtual_newline_seen: false,
        bang_term_position: true,
    };
    let fail = match cond_or(&mut cursor) {
        Err(fail) => fail,
        Ok(()) => match cursor.peek_skip_newlines() {
            // cond_token == COND_END after a clean cond_expr: no error.
            (CondTok::End, _, _, _) => return None,
            // cond_error EOF branch (parse.y:5018): reported at cond_lineno.
            (CondTok::Eof, _, _, _) => CondFail {
                messages: vec![(
                    open_line,
                    "unexpected EOF while looking for `]]'".to_string(),
                )],
                offending: CondOffending::Eof,
            },
            // cond_error token branch (parse.y:5020-5027): reported at
            // cond_lineno, the `[[` line.
            (tok, _, index, anchor) => CondFail {
                messages: vec![(
                    open_line,
                    format!(
                        "syntax error in conditional expression: unexpected token `{}'",
                        cond_offending_text(&cursor, &tok, index)
                    ),
                )],
                offending: CondOffending::of(&tok, index, anchor),
            },
        },
    };
    // Only a real collected argument can be the token GNU's reader had
    // just consumed when report_syntax_error ran; virtual/EOF offenders
    // anchor on a token boundary of their own and get no source char.
    let source_extra = match (&fail.offending, diagnostic_text) {
        (CondOffending::Index(index), Some(text)) => {
            source_char_after_arg(args, *index, text, source_line_offset)
        }
        _ => None,
    };
    Some(fail.encode(args, follower.as_deref(), source_extra))
}

/// The line of the `\n' token GNU's cond reader reports: a newline ending
/// line N is still line N (parser_error uses line_number before the
/// increment), so it is the following argument's line minus the newlines in
/// its leading whitespace.
fn newline_line(args: &[CondArg], anchor: usize) -> usize {
    args.get(anchor).map_or_else(
        || args.last().map(|a| a.3).unwrap_or(1),
        |(_, _, ws, line)| line.saturating_sub(ws.matches('\n').count()),
    )
}

/// The token that tripped the parser. `Index(n)` is a real collected
/// argument; `VirtualNewline(anchor)` is the `\n' GNU's reader returns in
/// operand slots — either a real newline before `args[anchor]` or the
/// end-of-input newline (anchor == args.len()); `Eof` is true end-of-input
/// (after the virtual newline is skipped); `End` is the `]]` closer itself.
enum CondOffending {
    Index(usize),
    VirtualNewline(usize),
    Eof,
    /// The `]]' closer itself: the index of the collected `]]'
    /// argument when it is a real argument, None for the virtual
    /// end-of-arguments closer.
    End(Option<usize>),
    /// A pre-shaped near token (rubash#1353's IO_NUMBER form): the text
    /// is the exact `syntax error near' payload and the index names the
    /// argument whose line the report uses.
    Fixed(String, usize),
    /// parse_matched_pair's EOF inside the `=~` regex word: the specific
    /// messages carry everything and PST_NOERROR suppresses the
    /// report_syntax_error tail — messages only, no echo, no `from [[` line.
    BareEof,
}

impl CondOffending {
    fn of(tok: &CondTok, index: Option<usize>, anchor: usize) -> Self {
        match tok {
            CondTok::Eof => CondOffending::Eof,
            CondTok::Newline => CondOffending::VirtualNewline(anchor),
            CondTok::End => CondOffending::End(index),
            _ => CondOffending::Index(index.unwrap_or(0)),
        }
    }
}

struct CondFail {
    messages: Vec<(usize, String)>,
    offending: CondOffending,
}

impl CondFail {
    /// Returns (spec, echo_line): echo_line is the offending input line GNU
    /// prints via print_offending_line (parse.y:6814) — None for the EOF
    /// shape, which echoes nothing.
    fn encode(
        self,
        args: &[CondArg],
        follower: Option<&str>,
        source_extra: Option<char>,
    ) -> (String, Option<usize>) {
        let sep = crate::executor::markers::PARSE_ERROR_FIELD_SEP;
        let last_line = args.last().map(|(_, _, _, line)| *line).unwrap_or(1);
        let open_line = args.first().map(|(_, _, _, line)| *line).unwrap_or(1);
        if matches!(self.offending, CondOffending::Eof) {
            // EOF shape (report_syntax_error EOF_Reached + compoundcmd_top,
            // parse.y:6884-6901): specific messages, then the
            // `unexpected end of file' tail naming the `[[` line.
            let eof_line = last_line + 1;
            let mut spec = format!("eof{sep}{open_line}{sep}{eof_line}");
            for (line, msg) in &self.messages {
                spec.push(sep);
                spec.push_str(&format!("{line}\u{1}{msg}"));
            }
            return (spec, None);
        }
        if matches!(self.offending, CondOffending::BareEof) {
            // PST_NOERROR shape (parse.y:3914 + 6724-6726): parse_matched_
            // pair already reported inside the word read, so yyerror prints
            // nothing — the carried messages are the whole diagnostic.
            let eof_line = last_line + 1;
            let mut spec = format!("bare{sep}{open_line}{sep}{eof_line}");
            for (line, msg) in &self.messages {
                spec.push(sep);
                spec.push_str(&format!("{line}\u{1}{msg}"));
            }
            return (spec, None);
        }
        // Near shape: error_token_from_text (parse.y:6772) reports the
        // offending token text; when the offender is virtual (newline or
        // `]]`) it falls back to the last real token before it — `(x'
        // reports `(x' because `(' is not a break character, and `x\n]]'
        // reports `x' not `]]'.
        // rubash#2750: when the offender is the closing `]]' or the
        // final collected argument, GNU's reader had already consumed
        // the glued operator token that follows the conditional as its
        // lookahead, and error_token_from_text (parse.y:6772-6808)
        // walks back over that follower's text too — the walk over
        // (arguments + follower) reproduces the report for every
        // operator shape (`;' alone, `&' for `&&', the whole `]]>' run
        // for a glued redirect).
        let follower_walk = |index: usize| -> Option<String> {
            let follower = follower?;
            if index != args.len().saturating_sub(1) {
                return None;
            }
            let mut text: String = args
                .iter()
                .flat_map(|(_, raw, ws, _)| [ws.as_str(), raw.as_str()])
                .collect();
            text.push_str(follower);
            Some(walk_back_over_text(&text))
        };
        let (near, near_line, echo_line) = match &self.offending {
            CondOffending::Index(index) => (
                glued_semicolon_near(args, *index)
                    .or_else(|| follower_walk(*index))
                    .or_else(|| {
                        // A bare `&&'/`||' offender reports its first
                        // character alone: the backward walk stops ON
                        // the token's own break character (parse.y:
                        // 6787-6793), where walk_back_near_token's
                        // fragment logic would keep both characters
                        // (rubash#2750: `[[ && a]]' names `&').
                        match args[*index].1.as_str() {
                            "&&" => Some("&".to_string()),
                            "||" => Some("|".to_string()),
                            _ => None,
                        }
                    })
                    .unwrap_or_else(|| walk_back_near_token(args, *index, source_extra)),
                args[*index].3,
                args[*index].3,
            ),
            CondOffending::Fixed(text, index) => (text.clone(), args[*index].3, args[*index].3),
            CondOffending::End(end_index) => {
                // GNU's reader stopped AT the offending `]]', so the
                // near report uses that argument's own line — an
                // unclosed collect that swallowed later lines' tokens
                // (rubash#1554's two-line form) still reports the
                // closer's line, not the last argument's.
                let end_line = end_index
                    .and_then(|index| args.get(index))
                    .map_or(last_line, |arg| arg.3);
                let near = end_index
                    .and_then(|index| glued_semicolon_near(args, index))
                    .or_else(|| follower_walk(end_index.unwrap_or(args.len().saturating_sub(1))))
                    .unwrap_or_else(|| end_near_token(args));
                (near, end_line, end_line)
            }
            CondOffending::VirtualNewline(anchor) => {
                let last = (*anchor).min(args.len()).saturating_sub(1);
                (
                    walk_back_near_token(args, last, None),
                    newline_line(args, *anchor),
                    newline_line(args, *anchor),
                )
            }
            // Unreachable — Eof encodes the `eof' shape above.
            CondOffending::Eof => (
                walk_back_near_token(args, args.len().saturating_sub(1), None),
                last_line,
                last_line,
            ),
            // Unreachable — BareEof encodes the `bare' shape above.
            CondOffending::BareEof => (
                walk_back_near_token(args, args.len().saturating_sub(1), None),
                last_line,
                last_line,
            ),
        };
        let mut spec = format!("near{sep}{near}{sep}{near_line}");
        for (line, msg) in &self.messages {
            spec.push(sep);
            spec.push_str(&format!("{line}\u{1}{msg}"));
        }
        (spec, Some(echo_line))
    }
}

/// parse.y:5443-5460 read_token_word PST_REGEXP: the first unescaped `(' in
/// the regex word opens parse_matched_pair (`(',`)') — when its group never
/// closes, the scan dies at EOF. Returns the line that `(' sits on (the
/// scan's start_lineno, parse.y:3901), or None when every unescaped group
/// closes. A backslash escapes the next character (LEX_PASSNEXT,
/// parse.y:3965-3984), so `\(` never opens a pair.
fn unclosed_regex_paren_line(word: &str, base_line: usize) -> Option<usize> {
    let mut depth = 0isize;
    let mut open_line = base_line;
    let mut line = base_line;
    let mut escaped = false;
    for ch in word.chars() {
        if ch == '\n' {
            line += 1;
        }
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '(' => {
                if depth == 0 {
                    open_line = line;
                }
                depth += 1;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    (depth > 0).then_some(open_line)
}

/// The same parse_matched_pair scan as `unclosed_regex_paren_line`, run
/// over a merged RHS argument's RAW text (rubash#1290 residual). The scan
/// GNU runs is over the raw input stream (parse.y:3901-3912): a backslash
/// protects the next character (LEX_PASSNEXT, parse.y:3965-3984), and a
/// quote character inside the scan is consumed as a nested matched pair
/// whose contents never count toward the paren depth (parse.y:4040-4051)
/// — which is also why a paren inside a quoted PREFIX of the raw form
/// (`a"(b"c') never opens a pair: shellquote has already consumed it
/// before the `(' special case (parse.y:5417). An unterminated quote
/// inside the scan swallows the rest of the text here; GNU would recurse
/// and report the quote's own EOF instead — a boundary this helper does
/// not model (the paren is still unclosed, only the wording differs).
fn unclosed_regex_paren_line_in_raw(raw: &str, base_line: usize) -> Option<usize> {
    let mut depth = 0isize;
    let mut open_line = base_line;
    let mut line = base_line;
    let mut escaped = false;
    let mut quote: Option<char> = None;
    // Whether the open '...' quote is the $'...' ANSI form, whose
    // backslashes escape (P_ALLOWESC, parse.y:3990-3996); a plain '...'
    // treats a backslash as a literal character.
    let mut quote_dollar_single = false;
    let mut prev = '\0';
    for ch in raw.chars() {
        if ch == '\n' {
            line += 1;
        }
        match quote {
            Some('\'') => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' && quote_dollar_single {
                    escaped = true;
                } else if ch == '\'' {
                    quote = None;
                }
            }
            Some(_) => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == quote.unwrap() {
                    quote = None;
                }
            }
            None => {
                if escaped {
                    escaped = false;
                } else {
                    match ch {
                        '\\' => escaped = true,
                        '\'' | '"' | '`' => {
                            quote_dollar_single = ch == '\'' && prev == '$';
                            quote = Some(ch);
                        }
                        '(' => {
                            if depth == 0 {
                                open_line = line;
                            }
                            depth += 1;
                        }
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
            }
        }
        prev = ch;
    }
    (depth > 0).then_some(open_line)
}

/// parse.y:5466-5489 read_token_word under PST_EXTPAT (`==` / `!=` /
/// `=`): the pattern word is read with the extglob group machinery
/// armed, so a PATTERN_CHAR (`@ * + ? !`, syntax.h:90-92) directly
/// before `(` hands the group to parse_matched_pair — when the group
/// never closes, the scan dies at EOF. Returns the line the group's
/// `(` sits on, or None when every group closes. The value form (the
/// word was one unquoted fragment): only backslash escapes state; a
/// `(` counts only as a PATTERN_CHAR tail at depth 0 — inside a group
/// every paren nests, exactly as parse_matched_pair counts.
fn unclosed_pattern_group_line(word: &str, base_line: usize) -> Option<usize> {
    let mut depth = 0isize;
    let mut open_line = base_line;
    let mut line = base_line;
    let mut escaped = false;
    let mut prev_op = false;
    for ch in word.chars() {
        if ch == '\n' {
            line += 1;
        }
        if escaped {
            escaped = false;
            prev_op = false;
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                prev_op = false;
            }
            '(' => {
                if depth == 0 {
                    if prev_op {
                        open_line = line;
                        depth = 1;
                    }
                } else {
                    depth += 1;
                }
                prev_op = false;
            }
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
                prev_op = false;
            }
            '@' | '*' | '+' | '?' | '!' => prev_op = true,
            _ => prev_op = false,
        }
    }
    (depth > 0).then_some(open_line)
}

/// The same group scan over a merged RHS argument's RAW text
/// (rubash#3229, mirroring `unclosed_regex_paren_line_in_raw`): quote
/// characters are consumed as nested matched pairs whose contents
/// never supply a PATTERN_CHAR (parse.y:4040-4051), so a quoted or
/// escaped operator never opens a group.
fn unclosed_pattern_group_line_in_raw(raw: &str, base_line: usize) -> Option<usize> {
    let mut depth = 0isize;
    let mut open_line = base_line;
    let mut line = base_line;
    let mut escaped = false;
    let mut quote: Option<char> = None;
    let mut quote_dollar_single = false;
    let mut prev_op = false;
    let mut prev = '\0';
    for ch in raw.chars() {
        if ch == '\n' {
            line += 1;
        }
        match quote {
            Some('\'') => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' && quote_dollar_single {
                    escaped = true;
                } else if ch == '\'' {
                    quote = None;
                }
                prev_op = false;
            }
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    quote = None;
                }
                prev_op = false;
            }
            None => {
                if escaped {
                    escaped = false;
                    prev_op = false;
                } else {
                    match ch {
                        '\\' => {
                            escaped = true;
                            prev_op = false;
                        }
                        '\'' | '"' | '`' => {
                            quote_dollar_single = ch == '\'' && prev == '$';
                            quote = Some(ch);
                            prev_op = false;
                        }
                        '(' => {
                            if depth == 0 {
                                if prev_op {
                                    open_line = line;
                                    depth = 1;
                                }
                            } else {
                                depth += 1;
                            }
                            prev_op = false;
                        }
                        ')' => {
                            if depth > 0 {
                                depth -= 1;
                            }
                            prev_op = false;
                        }
                        '@' | '*' | '+' | '?' | '!' => prev_op = true,
                        _ => prev_op = false,
                    }
                }
            }
        }
        prev = ch;
    }
    (depth > 0).then_some(open_line)
}

/// The character at GNU's reader index immediately after the collected
/// argument at `index`, when that character never reached the argument
/// list. A token-initial `#' comments out the rest of the physical line
/// in rubash's lexer (scanner.rs, on parse.y:3630-3643), so a fragment
/// glued to the offending token can vanish entirely (`[[ a == (#i)bar ]]'
/// collects only `a', `==', `('). GNU's cond parser reports the `('
/// before reading that fragment, and error_token_from_text
/// (parse.y:6772-6808) derives the near token from the raw line — the
/// reader index still sits on the `#' — so consult the same raw line
/// here. Returns that one character when it is not one of
/// error_token_from_text's break characters (` \n\t;|&`); any
/// reconstruction mismatch falls back to no character (the args-only
/// behaviour).
fn source_char_after_arg(
    args: &[CondArg],
    index: usize,
    text: &str,
    source_line_offset: usize,
) -> Option<char> {
    let line = args.get(index)?.3;
    // Only this conditional's fragments on the offending argument's own
    // physical line can anchor the span: walk back over same-line
    // fragments (a fragment whose leading whitespace carries a newline
    // starts the line's span).
    let mut start = index;
    while start > 0 && args[start - 1].3 == line && !args[start].2.contains('\n') {
        start -= 1;
    }
    let mut span = String::new();
    for (_, raw, ws, _) in &args[start..=index] {
        span.push_str(ws);
        span.push_str(raw);
    }
    let span = span.rsplit('\n').next()?;
    if span.is_empty() {
        return None;
    }
    let line_text = line
        .checked_sub(source_line_offset)
        .and_then(|in_text| super::parse_loop::source_line_by_number(text, in_text))?;
    // Anchor the search at this conditional's own `[[` so an identical
    // fragment run elsewhere on the line cannot supply the character.
    let search_from = line_text.rfind("[[")?;
    let end = search_from + line_text[search_from..].find(span)? + span.len();
    let c = line_text[end..].chars().next()?;
    (!matches!(c, ' ' | '\n' | '\t' | ';' | '|' | '&')).then_some(c)
}

/// error_token_from_text (parse.y:6772-6808): the reported token is the
/// run of non-break characters (` \n\t;|&` are the breaks) ending at
/// token_end — glued fragments BEFORE the offending one merge into it
/// whole, but past the offending token the run extends by at most the
/// single character at the reader index (see the forward step below).
fn walk_back_near_token(args: &[CondArg], index: usize, source_extra: Option<char>) -> String {
    // Raw text (quote/escape forms intact): GNU's error_token_from_text
    // names the source form of the offending word (`syntax error near
    // `\<'', not `<').
    let orig = index;
    let mut start = index;
    // rubash#1353: an offending fragment that itself starts with a break
    // character (a single `&' or `|') BEGINS GNU's reported token — the
    // error_token_from_text scan cannot cross a break character
    // backwards, so glued predecessors stay out (`a&b' reports `&b',
    // not `a&b'; `-n x&y' reports `&y'). The `;' separator is the same
    // break-led shape (rubash#1537): `[[ a =~ (b)]]; ...' trips on the
    // `;' after the absorbed regex word and GNU reports near `;'',
    // never the glued run `(b)]];'.
    let offender_raw = &args[orig].1;
    let break_led = (offender_raw.starts_with('&')
        || offender_raw.starts_with('|')
        || offender_raw.starts_with(';'))
        && offender_raw != "&&"
        && offender_raw != "||";
    if !break_led {
        while start > 0 && args[start].2.is_empty() {
            start -= 1;
        }
    }
    // GNU's scan (parse.y:6787-6793) is anchored at the end of the token
    // the reader consumed and walks back over non-break characters
    // (" \n\t;|&" are the breaks): fragments glued BEFORE the offending
    // one merge through the walk above. The forward direction is NOT a
    // walk — token_end is fixed at shell_input_line_index + 1
    // (parse.y:6787-6789: the reader index sits on the next unread
    // character, and that one character is included when it is not
    // whitespace), so exactly ONE further character past the offending
    // token can join the report: `[[ a == (a ]]` trips on the `('
    // fragment and GNU reports `(a' (rubash#1125); `[[ a == (b) ]]`
    // reports `(b', never `(b)' — the run ends before the reader's
    // next token (rubash#1524). When the offending fragment is
    // break-led, the reader's token is the break character alone and
    // the one extra character comes from the rest of that same
    // fragment (`5&3]]' -> `&3', rubash#1353). A glued `]]' that the
    // lexer absorbed into a word fragment (`b]]', `3]]') IS part of
    // GNU's reader token and stays.
    let mut near: String = (start..orig)
        .filter_map(|i| args.get(i))
        .map(|(_, raw, _, _)| raw.as_str())
        .collect();
    let mut extra_spent = false;
    if break_led {
        let mut chars = args[orig].1.chars();
        if let Some(first) = chars.next() {
            near.push(first);
        }
        if let Some(c) = chars.next() {
            if !matches!(c, ' ' | '\t' | '\n' | ';' | '|' | '&') {
                near.push(c);
                extra_spent = true;
            }
        }
    } else {
        near.push_str(&args[orig].1);
    }
    let mut extra_taken = extra_spent;
    if !extra_taken {
        if let Some(next) = args.get(orig + 1).filter(|next| next.2.is_empty()) {
            if let Some(c) = next.1.chars().next() {
                if !matches!(c, ' ' | '\t' | '\n' | ';' | '|' | '&') {
                    near.push(c);
                    extra_taken = true;
                }
            }
        }
    }
    if !extra_taken {
        // No collected fragment carries the character at the reader
        // index — it may have been swallowed before tokenization (a
        // glued comment is the known shape). error_token_from_text
        // reads it from the raw line either way (parse.y:6787-6789);
        // `source_extra' is that character, precomputed by the caller
        // from the raw line (see source_char_after_arg).
        if let Some(c) = source_extra {
            near.push(c);
        }
    }
    near
}

/// rubash#1560: the near for an offending token that a glued `;'
/// immediately follows. error_token_from_text (parse.y:6772-6808) is
/// anchored at the reader index when report_syntax_error runs, and
/// the reader has already consumed that `;' as its lookahead: the
/// backward walk stops ON the break character and reports it alone
/// (`[[ a =~ (b)]] && echo T; ...' reports near `;'', not `T''). A
/// spaced `;' starts a fresh token run (its leading whitespace is
/// non-empty) and the near stays the offending token's own.
fn glued_semicolon_near(args: &[CondArg], offender_index: usize) -> Option<String> {
    let next = args.get(offender_index + 1)?;
    if next.2.is_empty() && next.1.starts_with(';') {
        Some(";".to_string())
    } else {
        None
    }
}

/// The near token for a `]]' (COND_END) offender. error_token_from_text
/// (parse.y:6772-6808) walks the SOURCE TEXT backward from the end of
/// the offending token over non-break characters (` \n\t;|&' break),
/// stopping ON the first break character — which is therefore part of
/// the report when it is not whitespace (`&]]'), and glued word
/// fragments before it join the run (`(]]'). The collected arguments'
/// (leading_ws, raw) pairs reconstruct that source text exactly over
/// the span the walk can reach. Falls back to the bare `]]' when the
/// closer is the virtual end-of-arguments token rather than a
/// collected argument (rubash#1554).
fn end_near_token(args: &[CondArg]) -> String {
    if !args.last().is_some_and(|(_, raw, _, _)| raw == "]]") {
        return "]]".to_string();
    }
    let text: String = args
        .iter()
        .flat_map(|(_, raw, ws, _)| [ws.as_str(), raw.as_str()])
        .collect();
    walk_back_over_text(&text)
}

/// error_token_from_text's backward walk over a reconstructed source
/// text (parse.y:6772-6808): from the text's end, walk back over
/// non-break characters (` \n\t;|&' break), landing ON the break
/// character that stopped the walk — which therefore stays in the
/// report when it is not whitespace.
fn walk_back_over_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let bytes = text.as_bytes();
    let token_end = bytes.len();
    let mut i = token_end - 1;
    // Backward over the non-break run (parse.y:6787-6793): `i' lands on
    // the break character that stopped the walk, or on the first byte.
    while i > 0 && !matches!(bytes[i], b' ' | b'\n' | b'\t' | b';' | b'|' | b'&') {
        i -= 1;
    }
    // Forward over whitespace only (parse.y:6795-6796): a non-whitespace
    // break character stays as the report's first character.
    while i < token_end && matches!(bytes[i], b' ' | b'\n' | b'\t' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    text[i..token_end].to_string()
}

/// The GNU yacc-level token for one collected argument. Shell metacharacters
/// are structural only when unquoted (raw == value); `!` negates even quoted
/// (parse.y:5115 checks the WORD's text). End-of-input presents as a virtual
/// `newline` then `EOF`: operand slots see `newline` (GNU probes for
/// `bash -c '[[ -n'` report `unexpected argument `newline'`), while
/// newline-skipping positions consume it and see `EOF`.
#[derive(Clone, PartialEq)]
enum CondTok {
    Word,
    Bang,
    LParen,
    RParen,
    AndAnd,
    OrOr,
    End,
    Char(char),
    Newline,
    Eof,
}

struct CondCursor<'a> {
    args: &'a [CondArg],
    index: usize,
    closed: bool,
    /// Line of the input stream's EOF (one past its last newline).
    /// parse_matched_pair scans the raw regex word to EOF, so GNU's
    /// follow-up diagnostic lands on the stream's EOF line, not on
    /// the last collected conditional argument (rubash#1290 residual).
    stream_eof_line: usize,
    /// The arg index whose leading `\n' token was already consumed, so each
    /// physical newline surfaces exactly once (GNU read_token returns it as
    /// a token in operand slots).
    newline_consumed_for: Option<usize>,
    /// The virtual `\n' GNU's reader reports at end-of-input before EOF.
    virtual_newline_seen: bool,
    /// Whether the NEXT token sits at a term position — GNU's
    /// `reserved_word_acceptable(last_read_token)' restricted to the `[['
    /// token population (parse.y:5901-5945). read_token maps the word `!`
    /// to BANG only through CHECK_FOR_RESERVED_WORD (parse.y:3168-3181),
    /// so an unquoted `!` negates ONLY after a term opener (expression
    /// start, `(', `&&', `||', another `!', a newline); after a WORD or
    /// `<'/`>' it stays a WORD operand (`[[ $1 == -* || $1 == ! ]]`,
    /// bats-exec-test:67 — the RHS `!` is a literal pattern, not
    /// negation).
    bang_term_position: bool,
}

impl<'a> CondCursor<'a> {
    /// Current lookahead: (kind, error line, offending arg index — None for
    /// the virtual newline/EOF/closer, anchor arg index). The anchor is the
    /// argument a virtual newline precedes (or args.len() at end), used to
    /// find the last real token for error_token_from_text.
    fn peek(&self) -> (CondTok, usize, Option<usize>, usize) {
        if self.index < self.args.len() {
            let (value, raw, ws, line) = &self.args[self.index];
            if ws.contains('\n') && self.newline_consumed_for != Some(self.index) {
                return (
                    CondTok::Newline,
                    line.saturating_sub(ws.matches('\n').count()),
                    None,
                    self.index,
                );
            }
            let tok = match (value.as_str(), raw == value) {
                // `(_, false)` first: any quoted fragment is a WORD — GNU's
                // bang check (parse.y:5117) reads the parser's raw word
                // text, where a quoted `"!"` starts with `"` and is a term.
                (_, false) => CondTok::Word,
                ("!", _) if self.bang_term_position => CondTok::Bang,
                ("&&", true) => CondTok::AndAnd,
                ("||", true) => CondTok::OrOr,
                ("(", true) => CondTok::LParen,
                (")", true) => CondTok::RParen,
                ("]]", true) => CondTok::End,
                ("&" | "|" | ";" | "<" | ">", true) => CondTok::Char(value.chars().next().unwrap()),
                _ => CondTok::Word,
            };
            return (tok, *line, Some(self.index), self.index);
        }
        let last_line = self.args.last().map(|a| a.3).unwrap_or(1);
        if self.closed {
            (CondTok::End, last_line, None, self.args.len())
        } else if !self.virtual_newline_seen {
            (CondTok::Newline, last_line, None, self.args.len())
        } else {
            (CondTok::Eof, last_line + 1, None, self.args.len())
        }
    }

    /// Consume the lookahead token and return it. Newline/EOF/closer consume
    /// only their virtual state; real arguments advance the index.
    fn next(&mut self) -> (CondTok, usize, Option<usize>, usize) {
        let peeked = self.peek();
        // parse.y CHECK_FOR_RESERVED_WORD runs against last_read_token, so
        // the token just consumed decides whether a following `!' word is
        // eligible to become BANG: only the term openers (BANG itself, `(',
        // `&&', `||', and a newline — parse.y:5901-5945 lists all of them)
        // leave a term position; a WORD or `<'/`>' does not.
        self.bang_term_position = matches!(
            peeked.0,
            CondTok::Bang | CondTok::LParen | CondTok::AndAnd | CondTok::OrOr | CondTok::Newline
        );
        match peeked.0 {
            CondTok::Newline => {
                if self.index < self.args.len() {
                    self.newline_consumed_for = Some(self.index);
                } else {
                    self.virtual_newline_seen = true;
                }
            }
            _ if self.index < self.args.len() => self.index += 1,
            _ => {}
        }
        peeked
    }

    /// cond_skip_newlines (parse.y:5063): consume `\n' tokens and return the
    /// following token.
    fn skip_newlines(&mut self) -> (CondTok, usize, Option<usize>, usize) {
        loop {
            let peeked = self.next();
            if peeked.0 != CondTok::Newline {
                return peeked;
            }
        }
    }

    /// Look past newline tokens without consuming what follows them (the
    /// cond_token lookahead that cond_and/cond_or inspect).
    fn peek_skip_newlines(&mut self) -> (CondTok, usize, Option<usize>, usize) {
        loop {
            let peeked = self.peek();
            if peeked.0 != CondTok::Newline {
                return peeked;
            }
            self.next();
        }
    }
}

fn etext(tok: &CondTok) -> &'static str {
    match tok {
        CondTok::End => "]]",
        CondTok::Newline => "newline",
        CondTok::Eof => "EOF",
        CondTok::AndAnd => "&&",
        CondTok::OrOr => "||",
        _ => "",
    }
}

/// rubash#1353: the IO_NUMBER error shape. When the primary word is an
/// unquoted digit run glued to `<'/`>' characters (`1<', `12>>'), GNU's
/// cond reader has lexed it as IO_NUMBER + redirection operator, and
/// cond_error (parse.y:5010-5013) reports `unexpected token `DIGITS' in
/// conditional command' with a near of the digits plus ONE operator
/// character. Returns None for every other primary shape.
fn io_number_cond_fail(args: &[CondArg], word_index: usize, line: usize) -> Option<CondFail> {
    let (value, raw, _, _) = &args[word_index];
    if raw != value {
        return None;
    }
    let digits_len = raw.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_len == 0 || digits_len == raw.len() {
        return None;
    }
    let rest = &raw[digits_len..];
    if !rest.chars().all(|c| c == '<' || c == '>') {
        return None;
    }
    let digits = &raw[..digits_len];
    let op = rest.chars().next()?;
    Some(CondFail {
        messages: vec![(
            line,
            format!("unexpected token `{digits}' in conditional command"),
        )],
        offending: CondOffending::Fixed(format!("{digits}{op}"), word_index),
    })
}

/// cond_term (parse.y:5074-5247). `Err` carries the printed parser_error
/// messages and the offending token — equivalent to COND_RETURN_ERROR.
fn cond_term(cursor: &mut CondCursor) -> Result<(), CondFail> {
    let (tok, line, index, anchor) = cursor.skip_newlines();
    match tok {
        // `[[ ]]` — COND_END at term start is a silent COND_ERROR; cond_error
        // prints nothing and yyerror reports `syntax error near `]]''.
        CondTok::End => Err(CondFail {
            messages: Vec::new(),
            offending: CondOffending::End(index),
        }),
        CondTok::Eof => Err(CondFail {
            messages: vec![(
                line,
                "unexpected token `EOF' in conditional command".to_string(),
            )],
            offending: CondOffending::Eof,
        }),
        CondTok::LParen => {
            let paren_line = line;
            if let Err(mut inner) = cond_or(cursor) {
                // parse.y:5100-5109: an inner COND_ERROR makes the expected
                // `)' report the bare `expected `)'' form.
                inner
                    .messages
                    .push((paren_line, "expected `)'".to_string()));
                return Err(inner);
            }
            let (tok, _, index, anchor) = cursor.skip_newlines();
            if tok == CondTok::RParen {
                Ok(())
            } else {
                let text = cond_offending_text(cursor, &tok, index);
                Err(CondFail {
                    messages: vec![(
                        paren_line,
                        format!("unexpected token `{text}', expected `)'"),
                    )],
                    offending: CondOffending::of(&tok, index, anchor),
                })
            }
        }
        CondTok::Bang => cond_term(cursor),
        CondTok::Word => {
            let word_index = index.unwrap();
            let word = cursor.args[word_index].0.clone();
            // GNU parse.y:5125 tests the parser's word text, which still
            // carries quote characters (quote removal is deferred to
            // expansion, subst.c dequote_string) — a quoted `"-a"` starts
            // with `"` and never matches a unary operator. Rubash strips
            // quote markers when collecting args, so gate the unary path on
            // the raw text being unquoted (rubash#129: `[[ "-a" =~ -a ]]`
            // mis-took the quoted LHS for unary `-a` and errored).
            let unquoted = cursor.args[index.unwrap()].1 == word;
            if is_conditional_unary_operator(&word) && word.len() == 2 && unquoted {
                let (tok, line, index, anchor) = cursor.next();
                if tok != CondTok::Word {
                    let text = cond_offending_text(cursor, &tok, index);
                    return Err(CondFail {
                        messages: vec![(
                            line,
                            format!("unexpected argument `{text}' to conditional unary operator"),
                        )],
                        offending: CondOffending::of(&tok, index, anchor),
                    });
                }
                return Ok(());
            }
            // Left argument of a binary operator, or the bare `-n` shortcut.
            let (tok, line, index, anchor) = cursor.next();
            let binop = match &tok {
                CondTok::Word => {
                    // GNU parse.y:5156-5174: the binary-operator slot
                    // matches the parser's word TEXT — quote removal has
                    // not run (parse.y cond_term reads cond_token words),
                    // so a quoted `'=='` / `'<'` or an escaped `\==` /
                    // `\<` matches no operator and falls to the error
                    // branch (`unexpected token `\<', conditional binary
                    // operator expected`, rubash#360). The unescaped char
                    // operators arrive as Char tokens below.
                    let (op, raw) = (
                        &cursor.args[index.unwrap()].0,
                        &cursor.args[index.unwrap()].1,
                    );
                    raw == op && is_conditional_binary_operator(op)
                }
                CondTok::Char('<') | CondTok::Char('>') => true,
                _ => false,
            };
            if binop {
                // parse.y:5163-5174: only `=~` sets PST_REGEXP (==/!= set
                // PST_EXTPAT), and the parse.y:5443 `(`-folding arm is
                // PST_REGEXP-only — gate the unclosed-paren shape on it.
                let op_is_regex = match &tok {
                    CondTok::Word => cursor.args[index.unwrap()].0 == "=~",
                    _ => false,
                };
                let op_is_pattern = match &tok {
                    CondTok::Word => {
                        matches!(cursor.args[index.unwrap()].0.as_str(), "==" | "!=" | "=")
                    }
                    _ => false,
                };
                let (tok, line, index, anchor) = cursor.next();
                // parse.y:5443-5460 read_token_word under PST_REGEXP: an
                // unquoted `(' in the regex word opens parse_matched_pair
                // (`(',`)') which scans to EOF — its own EOF report names `)'
                // at the `(' line (start_lineno, parse.y:3901-3912) and sets
                // PST_NOERROR, so the cond parser's "unexpected argument to
                // conditional binary operator" (parse.y:5223-5227, at the EOF
                // line) is the LAST diagnostic and yyerror's
                // report_syntax_error tail is suppressed (parse.y:6724-6726).
                // `[[ a =~ (b ]]` prints exactly two lines and no
                // offending-line echo (#551 C4, 2026-10-12 WSL GNU 5.3.0
                // probes n6 / `[[ x =~ (unclosed`).
                if op_is_regex && tok == CondTok::Word {
                    let (value, raw, _, arg_line) = &cursor.args[index.unwrap()];
                    // An unquoted first fragment makes raw == value and the
                    // cooked word is the scan text. When the merge swallowed
                    // following lines into the RHS (the unclosed scan's own
                    // doing), later fragments carry quoting, so raw != value
                    // — but parse_matched_pair scans the RAW stream, where
                    // those quote characters are just data inside the pair
                    // (parse.y:3901-3912); scan the raw form then
                    // (rubash#1290 residual: file-mode probes printed the
                    // generic `]]' EOF shape instead).
                    let paren_line = if raw == value {
                        unclosed_regex_paren_line(value, *arg_line)
                    } else {
                        unclosed_regex_paren_line_in_raw(raw, *arg_line)
                    };
                    if let Some(paren_line) = paren_line {
                        let eof_line = cursor.stream_eof_line;
                        return Err(CondFail {
                            messages: vec![
                                (
                                    paren_line,
                                    "unexpected EOF while looking for matching `)'".to_string(),
                                ),
                                (
                                    eof_line,
                                    "unexpected argument to conditional binary operator"
                                        .to_string(),
                                ),
                            ],
                            offending: CondOffending::BareEof,
                        });
                    }
                }
                // parse.y:5466-5489 read_token_word under PST_EXTPAT:
                // the pattern word is read with the extglob group
                // machinery armed, so a PATTERN_CHAR tail's group goes
                // to parse_matched_pair and its EOF is the same pair of
                // reports as the PST_REGEXP arm above (the pair EOF at
                // the group's line, then the dangling-operator report
                // at the EOF line, PST_NOERROR suppressing the tail) —
                // rubash#3229: `[[ ab == @(ab ]]` printed the generic
                // `]]' EOF shape instead.
                if op_is_pattern && tok == CondTok::Word {
                    let (value, raw, _, arg_line) = &cursor.args[index.unwrap()];
                    let group_line = if raw == value {
                        unclosed_pattern_group_line(value, *arg_line)
                    } else {
                        unclosed_pattern_group_line_in_raw(raw, *arg_line)
                    };
                    if let Some(group_line) = group_line {
                        let eof_line = cursor.stream_eof_line;
                        return Err(CondFail {
                            messages: vec![
                                (
                                    group_line,
                                    "unexpected EOF while looking for matching `)'".to_string(),
                                ),
                                (
                                    eof_line,
                                    "unexpected argument to conditional binary operator"
                                        .to_string(),
                                ),
                            ],
                            offending: CondOffending::BareEof,
                        });
                    }
                }
                if tok != CondTok::Word {
                    let text = cond_offending_text(cursor, &tok, index);
                    return Err(CondFail {
                        messages: vec![(
                            line,
                            format!("unexpected argument `{text}' to conditional binary operator"),
                        )],
                        offending: CondOffending::of(&tok, index, anchor),
                    });
                }
                return Ok(());
            }
            match tok {
                // `[[ x ]]` / `[[ x && .. ]]` — the `-n` shortcut keeps the
                // lookahead token for the caller (parse.y:5178-5187).
                CondTok::End | CondTok::AndAnd | CondTok::OrOr | CondTok::RParen => {
                    if let Some(i) = index {
                        cursor.index = i;
                    }
                    Ok(())
                }
                _ => {
                    // rubash#1353: a primary word of the form DIGITS+</>
                    // (`1<', `12>') is GNU's IO_NUMBER token — the cond
                    // reader lexes digit runs before a redirection
                    // operator as IO_NUMBER, which is illegal at term
                    // position, and cond_error reports the DIGITS with
                    // the `in conditional command' wording and a near of
                    // digits + the single operator character (`1<').
                    if let Some(fail) = io_number_cond_fail(cursor.args, word_index, line) {
                        return Err(fail);
                    }
                    let text = cond_offending_text(cursor, &tok, index);
                    // rubash#1353: an offending fragment LED by a single
                    // `&' or `|' names just that character in GNU's
                    // report (`unexpected token `&'' for `&3]]'), not the
                    // whole glued fragment.
                    let text = match text.chars().next() {
                        Some(c @ ('&' | '|')) if text != "&&" && text != "||" => c.to_string(),
                        _ => text,
                    };
                    Err(CondFail {
                        messages: vec![(
                            line,
                            format!(
                                "unexpected token `{text}', conditional binary operator expected"
                            ),
                        )],
                        offending: CondOffending::of(&tok, index, anchor),
                    })
                }
            }
        }
        _ => {
            let text = cond_offending_text(cursor, &tok, index);
            Err(CondFail {
                messages: vec![(
                    line,
                    format!("unexpected token `{text}' in conditional command"),
                )],
                offending: CondOffending::of(&tok, index, anchor),
            })
        }
    }
}

fn cond_and(cursor: &mut CondCursor) -> Result<(), CondFail> {
    cond_term(cursor)?;
    // parse.y:5052: cond_term leaves the next token in cond_token; only a
    // `&&` consumes it and starts another term.
    if cursor.peek_skip_newlines().0 == CondTok::AndAnd {
        cursor.next();
        return cond_and(cursor);
    }
    Ok(())
}

fn cond_or(cursor: &mut CondCursor) -> Result<(), CondFail> {
    cond_and(cursor)?;
    if cursor.peek_skip_newlines().0 == CondTok::OrOr {
        cursor.next();
        return cond_or(cursor);
    }
    Ok(())
}

/// error_token_from_token text for the diagnostic: real arguments render as
/// their text; virtual/newline/EOF and the `]]` closer use GNU's alist names.
fn cond_offending_text(cursor: &CondCursor, tok: &CondTok, index: Option<usize>) -> String {
    if let Some(i) = index {
        // GNU error_token_from_token prints the word's raw text — quote
        // characters and escapes are still attached (`\<', `'=='`), so the
        // operator-slot rejection names the exact source form.
        return cursor.args[i].1.clone();
    }
    match tok {
        CondTok::Char(c) => c.to_string(),
        other => etext(other).to_string(),
    }
}

/// The offending-line echo for `[[` conditional syntax errors.
///
/// GNU parse.y:6814-6826 `print_offending_line` echoes the ENTIRE physical
/// `shell_input_line' of the error's line number — trailing newlines
/// stripped, everything else verbatim. That line regularly contains text
/// OUTSIDE the conditional's own token slice: a function-definition body is
/// re-parsed from a re-lexed slice (`c12() { [[ ((zz)) == zz ]] ...; }'
/// echoes with the `c12() { ' prefix and closing `}'), an inline section
/// (`while false; do [[ ... ]]; done') echoes with the loop head and
/// `done'. Token-slice reconstruction cannot see that text, so when the
/// parse carries the original source (ParseLoopOptions::diagnostic_text /
/// source_text), the echo is the verbatim physical line from it; the token
/// walk below remains the fallback for synthesized inputs (eval bodies,
/// alias-spliced groups without text).
fn conditional_offending_line(
    tokens: &[Token],
    start: usize,
    end: usize,
    echo_line: Option<usize>,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<String> {
    let line = echo_line.or_else(|| tokens.get(start).map(|token| token.position))?;
    if let Some(text) = diagnostic_text {
        if let Some(source) = line
            .checked_sub(source_line_offset)
            .and_then(|in_text| super::parse_loop::source_line_by_number(text, in_text))
        {
            return Some(source);
        }
    }
    conditional_error_source(tokens, start, end, echo_line)
}

/// Echo the input line containing the offending token for `syntax error
/// near` reports (print_offending_line, parse.y:6814): GNU echoes that one
/// input line verbatim; reconstruct it from the tokens sharing its line,
/// dropping `\n' separator pseudo-tokens (their raw is `;', not source text).
fn conditional_error_source(
    tokens: &[Token],
    start: usize,
    end: usize,
    echo_line: Option<usize>,
) -> Option<String> {
    let line = echo_line.or_else(|| tokens.get(start).map(|token| token.position))?;
    let mut source = String::new();
    let mut began = false;
    // GNU print_offending_line (parse.y:6814) echoes the WHOLE input line
    // of the offending token — including text after `]]' when the
    // conditional is followed on the same line (`[[ x \< y ]] || echo no'
    // echoes the full line), so the walk must not stop at the closer.
    for token in tokens {
        if token.position != line {
            if began {
                break;
            }
            continue;
        }
        if token.line_break {
            continue;
        }
        began = true;
        source.push_str(&token.leading_ws);
        source.push_str(&token.raw);
    }
    (!source.is_empty()).then_some(source)
}

/// (value, raw, leading_ws, line) — `leading_ws` is the whitespace that
/// preceded the fragment in the source (Token::leading_ws), needed to
/// reconstruct `=~`/`==` RHS words exactly the way parse_matched_pair
/// preserves them; `line` is the source line for GNU error reporting.
type CondArg = (String, String, String, usize);

fn collect_conditional_args(tokens: &[Token], mut index: usize, end: usize) -> Vec<CondArg> {
    let mut args: Vec<CondArg> = Vec::new();
    let mut pending_newline = false;
    // parse.y:5166 cond_term: after a `=~' binop the RHS is read as ONE
    // word under PST_REGEXP (parse.y:5443 read_token_word absorbs its
    // `(...)' groups through parse_matched_pair) — a folded `((...))'
    // token there is that single word, NOT a cond group. `=' / `==' /
    // `!=' (PST_EXTPAT) keep the expansion: GNU's word reader only joins
    // a `(' after a PATTERN_CHAR (parse.y:5464-5466), so a leading-`('
    // RHS reaches cond_term as the `(' token and errors.
    let mut regexp_rhs_pending = false;
    while index <= end {
        if tokens[index].line_break {
            // A physical newline is a `\n' token in GNU's cond reader —
            // visible in operand slots (`unexpected argument `newline''),
            // skipped in term positions. Model it as a `\n' marker in the
            // next argument's leading whitespace.
            pending_newline = true;
            index += 1;
            continue;
        }
        if index == end {
            push_cond_arg(&mut args, &tokens[index], &mut pending_newline);
            break;
        }
        if is_folded_arithmetic_token(&tokens[index]) {
            let regexp_rhs = regexp_rhs_pending;
            regexp_rhs_pending = false;
            if regexp_rhs {
                push_cond_arg(&mut args, &tokens[index], &mut pending_newline);
            } else {
                args.extend(folded_arithmetic_cond_args(
                    &tokens[index],
                    &mut pending_newline,
                ));
            }
            index += 1;
            continue;
        }
        if let Some((word, next_i)) = collect_compound_word_value(tokens, index) {
            let raw = if next_i == index + 1 {
                tokens[index].raw.clone()
            } else {
                word.clone()
            };
            let mut ws = tokens[index].leading_ws.clone();
            if pending_newline && !ws.contains('\n') {
                ws.push('\n');
            }
            pending_newline = false;
            let value = strip_conditional_quote_markers(&word);
            // merge_pattern_rhs_fragments' operator predicate: a `=~'
            // whose own predecessor is not a structural token opens a
            // single-word regexp RHS (parse.y:5166 PST_REGEXP).
            regexp_rhs_pending =
                value == "=~" && !args.last().is_some_and(|prev| is_cond_structure(prev));
            args.push((value, raw, ws, tokens[index].position));
            index = next_i;
            continue;
        }
        push_cond_arg(&mut args, &tokens[index], &mut pending_newline);
        index += 1;
    }
    args
}

/// The structural tokens that disqualify a preceding `=~' from opening a
/// regexp RHS (merge_pattern_rhs_fragments' exclusion list).
fn is_cond_structure(arg: &CondArg) -> bool {
    matches!(arg.0.as_str(), "(" | ")" | "!" | "&&" | "||")
}

/// A lexer-folded arithmetic command: the unquoted `((...))` Keyword the
/// lexer emits at a command-position `((` (balanced by construction).
fn is_folded_arithmetic_token(token: &Token) -> bool {
    token.kind == TokenKind::Keyword
        && token.raw == token.value
        && token.value.starts_with("((")
        && token.value.ends_with("))")
        && token.value.len() >= 4
}

/// Re-split a folded `((...))' token into the cond-group token population
/// GNU's reader produces inside a conditional.
///
/// GNU anchor: read_token delivers `(' as the grouping token inside `[[ ]]'
/// — the arithmetic-command special case is gated on
/// `(parser_state & (PST_CASEPAT|PST_CONDCMD)) == 0' (parse.y:3780) — so
/// `[[ ((x == 0)) ]]' parses as cond_term group(group(binary))
/// (parse.y:5094-5110). Every completed term ends with
/// `(void)cond_skip_newlines ()' (parse.y:5108 group close, 5140 unary,
/// 5243 binary rhs), which is why a newline before the next operand, `||',
/// or `]]' is invisible after a CLOSED GROUP while a bare word's binop
/// slot (raw read_token, parse.y:5179) errors on it — rubash#505's
/// multi-line `|| ((${#_ARGS_BOUNDS[@]} == 0))' continuation. rubash's
/// lexer folds the balanced `((...))' into one Keyword token regardless of
/// the conditional; modeling it as a single WORD operand surfaced the
/// continuation newline in the binop slot. Expanding restores GNU's token
/// population: the group opens, the inner words bind as operands, and the
/// group close runs the newline-skipping lookahead.
fn folded_arithmetic_cond_args(token: &Token, pending_newline: &mut bool) -> Vec<CondArg> {
    // Strip the outer parens the reader would have delivered as `('
    // tokens; what remains re-lexes to the group body (which still opens
    // with its own `(' for the nested group).
    let raw = token.raw.as_str();
    let inner_text = &raw[1..raw.len().saturating_sub(1)];
    // rubash#131: the fold's re-lex must run under the gate of the pass
    // that produced it — GNU decides extglob word-splits at read time
    // (parse.y:5466).
    let saved_extglob = crate::lexer::parse_extended_glob();
    crate::lexer::set_parse_extended_glob(token.extglob_gate);
    let inner_tokens = crate::lexer::tokenize(inner_text);
    crate::lexer::set_parse_extended_glob(saved_extglob);

    // The whitespace that preceded the whole fold in the source (and any
    // pending `\n' marker) belongs to the `(' the reader delivered first.
    let mut prefix = token.leading_ws.clone();
    if *pending_newline && !prefix.contains('\n') {
        prefix.push('\n');
    }
    *pending_newline = false;

    let mut out: Vec<CondArg> = Vec::new();
    let mut pending_inner = false;
    let mut first_pushed = true;
    for inner in &inner_tokens {
        if inner.line_break {
            // A multi-line `(( ... ))' fold: the inner newline is a `\n'
            // token between the group's tokens, same model as the outer
            // collection.
            pending_inner = true;
            continue;
        }
        if is_folded_arithmetic_token(inner) {
            // `[[ (( ((x)) )) ]]' — nested folds re-enter the same
            // expansion.
            let mut nested = folded_arithmetic_cond_args(inner, &mut pending_inner);
            if first_pushed {
                if let Some(first) = nested.first_mut() {
                    let mut combined = prefix.clone();
                    combined.push_str(&first.2);
                    first.2 = combined;
                }
                first_pushed = false;
            }
            out.extend(nested);
            continue;
        }
        let mut ws = inner.leading_ws.clone();
        if pending_inner && !ws.contains('\n') {
            ws.push('\n');
        }
        pending_inner = false;
        if first_pushed {
            let mut combined = prefix.clone();
            combined.push_str(&ws);
            ws = combined;
            first_pushed = false;
        }
        // Inner token columns are 1-based within `inner_text', which sits
        // one byte into `raw` — map through raw's newlines for the
        // physical line GNU's reader would report.
        let at = (inner.column + 1).min(raw.len());
        let line = token.position + raw[..at].matches('\n').count();
        out.push((
            strip_conditional_quote_markers(&inner.value),
            inner.raw.clone(),
            ws,
            line,
        ));
    }
    out
}

fn push_cond_arg(args: &mut Vec<CondArg>, token: &Token, pending_newline: &mut bool) {
    let mut ws = token.leading_ws.clone();
    if *pending_newline && !ws.contains('\n') {
        ws.push('\n');
    }
    *pending_newline = false;
    args.push((
        strip_conditional_quote_markers(&token.value),
        token.raw.clone(),
        ws,
        token.position,
    ));
}

/// GNU parse.y cond_term (parse.y:5150-5220): after `=~` (PST_REGEXP,
/// parse.y:5166) and `=`/`==`/`!=` (PST_EXTPAT, parse.y:5161-5163) the
/// right-hand side is read as ONE word — `read_token_word` absorbs
/// `(...)` groups verbatim through parse_matched_pair (parse.y:5443,
/// whitespace included) and treats `|` as a word character under
/// PST_REGEXP. A whitespace-separated fragment at paren depth 0 ends
/// the word; a bare `)` ends it too (it is the cond group's `)` or an
/// error token). For `==`/`!=` a `(` opens a group only when it
/// directly follows an extglob pattern char (`@ * + ? !` —
/// syntax.h PATTERN_CHAR); for `=~` every unquoted `(` opens a regex
/// group (ERE).
fn merge_pattern_rhs_fragments(arg_parts: Vec<CondArg>) -> Vec<CondArg> {
    let mut args: Vec<CondArg> = Vec::with_capacity(arg_parts.len());
    let mut index = 0usize;
    while index < arg_parts.len() {
        let current = arg_parts[index].clone();
        index += 1;
        let is_op = matches!(current.1.as_str(), "=~" | "==" | "=" | "!=")
            && args
                .last()
                .is_some_and(|prev| !matches!(prev.0.as_str(), "(" | ")" | "!" | "&&" | "||"));
        args.push(current.clone());
        if !is_op || index >= arg_parts.len() {
            continue;
        }
        let regexp = current.1 == "=~";

        // GNU parse.y:5464-5466: an extended-pattern `(` group only joins a
        // word after a PATTERN_CHAR (`@ * + ? !`); at word START the `(`
        // never joins — read_token returns it as the `(' token and
        // parse_cond_pair reports `unexpected argument `(' to conditional
        // binary operator` (parse.y:5223), with or without the extglob
        // shopt (the RHS read forces extended_glob, but that only arms the
        // PATTERN_CHAR-prefixed groups). A regexp RHS (`=~`) opens its ERE
        // group at any position (rubash#221: `[[ a == (b|c)* ]]`).
        if !regexp && arg_parts[index].1.starts_with('(') {
            continue;
        }

        // A leading bare `)' never starts the right-hand word either:
        // `)' is a shell break character, so read_token delivers it as
        // its own token (parse.y:5305 read_token_word) and cond_term's
        // argument slot reports it — letting it start the merge glued
        // the following fragments into a fake word and the
        // conditional silently evaluated (`[[ a == )abc ]]',
        // rubash#1538). Under `=~' the reader jumps straight to word
        // reading (parse.y:3663), where a leading break character ends
        // the read at once (parse.y:5688): the regex word is EMPTY and
        // the `)' stays in the stream, so cond_error's generic form —
        // not the binary-argument form — reports it. Model that empty
        // word and leave the `)' fragment for the next pass. A quoted
        // `")"' (raw != value) is word text and still joins below.
        if arg_parts[index].0 == ")" && arg_parts[index].1 == ")" {
            if regexp {
                let (_, _, ws, line) = arg_parts[index].clone();
                args.push((String::new(), String::new(), ws, line));
            }
            continue;
        }

        // The first fragment always begins the RHS word. The merged
        // word keeps the first fragment's leading whitespace: it is the
        // spacing the word actually has in the source, and consumers
        // (the near-token walk-back, the cursor's newline model) read
        // it as such — zeroing it fabricated a glue to the operator, so
        // `[[ a == x) echo ]]` reported near `==x)' instead of GNU's
        // `x)' (rubash#1524).
        let (mut value, mut raw_text, first_ws, line) = arg_parts[index].clone();
        let mut depth = unquoted_paren_delta(&arg_parts[index]);
        index += 1;
        while index < arg_parts.len() {
            let (v, r, ws, _) = &arg_parts[index];
            if depth <= 0 {
                if !ws.is_empty() || r == ")" {
                    break;
                }
                if r == "(" && !regexp && !last_rhs_char_is_pattern_char(&value) {
                    break;
                }
                // Unquoted `<` `>` `;` `&` end the RHS word in GNU
                // read_token_word — under PST_REGEXP only `(`, `)` (via
                // parse_matched_pair) and `|` (parse.y:5446 got_character)
                // are word-safe, so `[[ a =~ ^<a ]]` reports `syntax error
                // in conditional expression: unexpected token `<'` (rubash
                // #129 runtime probe). `|` stays joinable for regexps.
                if matches!(r.as_str(), "<" | ">" | ";" | "&") {
                    break;
                }
            }
            value.push_str(ws);
            value.push_str(v);
            raw_text.push_str(ws);
            raw_text.push_str(r);
            depth += unquoted_paren_delta(&arg_parts[index]);
            index += 1;
        }
        args.push((value, raw_text, first_ws, line));
    }
    args
}

fn last_rhs_char_is_pattern_char(value: &str) -> bool {
    value
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '@' | '*' | '+' | '?' | '!'))
}

/// Net unquoted-paren balance of a fragment. A fragment whose raw
/// differs from its value carries quoting, so its parens are data —
/// e.g. `'('` under `=~` does not open a group (PST_REGEXP quoting is
/// consumed by shellquote before the `(` special-case, parse.y:5417).
fn unquoted_paren_delta(part: &CondArg) -> isize {
    let (value, raw, _, _) = part;
    if raw != value {
        return 0;
    }
    let mut delta = 0isize;
    for ch in value.chars() {
        match ch {
            '(' => delta += 1,
            ')' => delta -= 1,
            _ => {}
        }
    }
    delta
}

fn strip_conditional_quote_markers(value: &str) -> String {
    value.replace(crate::executor::markers::CTLESC, "")
}

/// The `]]' candidates for the conditional's end, in stream order: a
/// `]]' token at paren depth 0. GNU parse.y:5443-5460 read_token_word
/// under PST_REGEXP: after `=~' an unquoted `(' consumes its balanced
/// `(...)' group into the regex word (parse_matched_pair), and a cond
/// grouping `(' (parse.y:5166 `cond_term: '(' cond_expr ')') does the
/// same — so a `]]' inside an open group is regex data, not the
/// conditional terminator (`[[ a =~ ( ]]<>;&) ]]' ends at the FINAL
/// `]]', rubash#222). A quoted/escaped paren (raw != value) never opens
/// a group (parse.y:5417 shellquote runs before the `(' special case).
/// The stream index of an OPERATOR fragment carrying a trailing
/// `]]' (`==]]', `!=]]', ...), or None. GNU's cond reader returns
/// such a fragment as ONE token (the classifier rejects it as the
/// binary operator it almost is); a plain WORD ending in `]]' is
/// just a word — `]]' closes only as an exact standalone token
/// (parse.y special_case_tokens) — so word fragments never count
/// here. Mirrors conditional_end_candidates' paren-depth walk.
fn conditional_absorbed_closer(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0isize;
    for index in start + 1..tokens.len() {
        let token = &tokens[index];
        if token.kind == TokenKind::Keyword && token.raw == "(" && token.value == "(" {
            depth += 1;
        } else if token.kind == TokenKind::Keyword && token.raw == ")" && token.value == ")" {
            depth = (depth - 1).max(0);
        } else if depth == 0 {
            let Some(operator) = token.raw.strip_suffix("]]") else {
                continue;
            };
            if matches!(
                operator,
                "==" | "!="
                    | "="
                    | "=~"
                    | "<"
                    | ">"
                    | "<="
                    | ">="
                    | "-eq"
                    | "-ne"
                    | "-lt"
                    | "-le"
                    | "-gt"
                    | "-ge"
                    | "-ef"
                    | "-nt"
                    | "-ot"
                    | "&&"
                    | "||"
            ) {
                return Some(index);
            }
        }
    }
    None
}

fn conditional_end_candidates(tokens: &[Token], start: usize) -> Vec<usize> {
    let mut depth = 0isize;
    let mut candidates = Vec::new();
    for index in start + 1..tokens.len() {
        let token = &tokens[index];
        if token.kind == TokenKind::Keyword && token.raw == "(" && token.value == "(" {
            depth += 1;
        } else if token.kind == TokenKind::Keyword && token.raw == ")" && token.value == ")" {
            depth = (depth - 1).max(0);
        } else if token.raw == "]]" && depth == 0 {
            candidates.push(index);
        }
    }
    candidates
}

/// Find the conditional's real end and the merged argument list for it.
///
/// A depth-0 `]]' is only a CANDIDATE: GNU's reader delivers COND_END
/// for a `]]' that starts a token, but a `]]' reached while a word is
/// still being read is word text — read_token_word (parse.y:5443-5461)
/// keeps consuming after parse_matched_pair returns, and `]' is not a
/// shell break character, so `[[ a =~ (b)]]' has NO closer at that
/// `]]': the regex word is `(b)]]' (rubash#1537). rubash's lexer emits
/// the glued `]]' as its own token (its scan restarts after the `)'
/// fragment), so candidacy is verified with the same word model the
/// parse itself uses: collect and merge the arguments up to the
/// candidate — if the merged list's last argument is exactly the
/// candidate's standalone `]]', the candidate is the end; if the merge
/// absorbed it into a right-hand-side word, the reader is still inside
/// that word and the next candidate (if any) gets its chance. No
/// surviving candidate means the conditional is unclosed and the
/// caller falls back to the EOF diagnostic path.
fn conditional_end_and_args(tokens: &[Token], start: usize) -> Option<(usize, Vec<CondArg>)> {
    for end in conditional_end_candidates(tokens, start) {
        let merged = merge_pattern_rhs_fragments(collect_conditional_args(tokens, start + 1, end));
        // The collected arguments always end with the candidate token's
        // own text (collection never drops a token), so a standalone
        // closer is recognizable as a final argument that is exactly
        // the `]]' token: same value/raw and the candidate's own line.
        // An absorbed `]]' instead sits inside a longer merged word.
        let survives = merged.last().is_some_and(|(value, raw, _, line)| {
            value == "]]" && raw == "]]" && *line == tokens[end].position
        });
        if survives {
            return Some((end, merged));
        }
    }
    None
}

/// Unclosed `[[` (no `]]` in the token stream): collect the rest of the
/// tokens as conditional arguments and run the same GNU classifier with
/// `closed = false`. GNU's cond parser stops at the first offending token,
/// so commands after the error never matter; a clean prefix reports
/// `unexpected EOF while looking for `]]'' (cond_error, parse.y:5018) plus
/// the `unexpected end of file from `[[' command on line N` tail
/// (report_syntax_error EOF path, parse.y:6884-6901).
pub(super) fn conditional_eof_error_command(
    tokens: &[Token],
    start: usize,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> (CommandNode, usize) {
    // GNU never treats a `]]' SUFFIX as the closer — special_case_
    // tokens (parse.y) returns COND_END only for a word that is
    // exactly `]]' — so a WORD fragment (`x]]') bounds nothing: it is
    // an ordinary argument and the classifier must see the tokens
    // after it (`[[ x]] echo hi' fails at `echo'). An OPERATOR
    // fragment (`==]]') is different: the classifier fails AT that
    // token in stream order, so the collection may stop there, and
    // the token glued after it is the follow-on the report walks
    // over (`[[ a ==]]&' names `&') (rubash#2750).
    let absorbed_closer = conditional_absorbed_closer(tokens, start);
    let merged = merge_pattern_rhs_fragments(collect_conditional_args(
        tokens,
        start + 1,
        absorbed_closer.unwrap_or(tokens.len() - 1),
    ));
    let follower = absorbed_closer.and_then(|end| cond_follower_text(tokens, end));
    let (spec, echo_line) = conditional_syntax_error_spec(
        &merged,
        false,
        stream_eof_line(tokens, diagnostic_text, source_line_offset),
        follower,
        diagnostic_text,
        source_line_offset,
    )
    .unwrap_or_else(|| {
        let sep = crate::executor::markers::PARSE_ERROR_FIELD_SEP;
        let open_line = tokens.get(start).map(|token| token.position).unwrap_or(1);
        let eof_line = tokens
            .last()
            .map(|token| token.position + 1)
            .unwrap_or(open_line + 1);
        (
            format!(
                "eof{sep}{open_line}{sep}{eof_line}{sep}{open_line}\u{1}unexpected EOF while looking for `]]'"
            ),
            None,
        )
    });
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
    command.insert_assignment("__RUBASH_PARSE_ERROR_COND__".to_string(), spec);
    if let Some(source) = conditional_offending_line(
        tokens,
        start,
        tokens.len() - 1,
        echo_line,
        diagnostic_text,
        source_line_offset,
    ) {
        command.insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
    }
    (command, tokens.len())
}

fn conditional_expression(args: &[(String, String)]) -> ConditionalExpression {
    if args.is_empty() {
        return conditional_leaf(ConditionalExpressionKind::Empty, None, args);
    }
    if args[0].0 == ")" {
        // `[[ ) ]]` is a syntax error in Bash (a `)` cannot start a
        // conditional expression unless it closes a `(` group). Rubash has
        // no parser error channel yet, so treat it as an empty (false)
        // condition instead of evaluating `)` as a truthy word.
        return conditional_leaf(ConditionalExpressionKind::Empty, None, &[]);
    }

    if let Some(inner) = conditional_outer_group(args) {
        return ConditionalExpression {
            kind: ConditionalExpressionKind::Group,
            open_delimiter: Some("(".to_string()),
            open_delimiter_metadata: Some(delimiter_metadata("(")),
            operator: None,
            operands: Vec::new(),
            pattern_operand: None,
            children: vec![conditional_expression(inner)],
            close_delimiter: Some(")".to_string()),
            close_delimiter_metadata: Some(delimiter_metadata(")")),
        };
    }

    if let Some(index) = top_level_operator(args, "||") {
        return conditional_logical_expression(args, index);
    }
    if let Some(index) = top_level_operator(args, "&&") {
        return conditional_logical_expression(args, index);
    }

    if args[0].0 == "!" {
        return ConditionalExpression {
            kind: ConditionalExpressionKind::Negation,
            open_delimiter: None,
            open_delimiter_metadata: None,
            operator: Some("!".to_string()),
            operands: Vec::new(),
            pattern_operand: None,
            children: vec![conditional_expression(&args[1..])],
            close_delimiter: None,
            close_delimiter_metadata: None,
        };
    }

    if args.len() == 2 && is_conditional_unary_operator(&args[0].0) {
        return conditional_leaf(
            ConditionalExpressionKind::Unary,
            Some(args[0].0.clone()),
            &args[1..2],
        );
    }

    if args.len() == 3 && is_conditional_binary_operator(&args[1].0) {
        return conditional_leaf(
            ConditionalExpressionKind::Binary,
            Some(args[1].0.clone()),
            &[args[0].clone(), args[2].clone()],
        );
    }

    if args.len() > 3
        && is_conditional_binary_operator(&args[1].0)
        && conditional_rhs_fragments_can_join(&args[2..])
    {
        let joined_rhs = (
            args[2..]
                .iter()
                .map(|(arg, _)| arg.as_str())
                .collect::<String>(),
            args[2..]
                .iter()
                .map(|(_, raw)| raw.as_str())
                .collect::<String>(),
        );
        return conditional_leaf(
            ConditionalExpressionKind::Binary,
            Some(args[1].0.clone()),
            &[args[0].clone(), joined_rhs],
        );
    }

    if args.len() == 1 {
        return conditional_leaf(ConditionalExpressionKind::Word, None, &args[0..1]);
    }

    conditional_leaf(ConditionalExpressionKind::Unknown, None, args)
}

fn conditional_rhs_fragments_can_join(rhs: &[(String, String)]) -> bool {
    rhs.len() > 1
        && rhs
            .iter()
            .any(|(arg, _)| matches!(arg.as_str(), "(" | ")" | "|") || arg.contains('('))
}

fn conditional_logical_expression(
    args: &[(String, String)],
    index: usize,
) -> ConditionalExpression {
    ConditionalExpression {
        kind: ConditionalExpressionKind::Logical,
        open_delimiter: None,
        open_delimiter_metadata: None,
        operator: Some(args[index].0.clone()),
        operands: Vec::new(),
        pattern_operand: None,
        children: vec![
            conditional_expression(&args[..index]),
            conditional_expression(&args[index + 1..]),
        ],
        close_delimiter: None,
        close_delimiter_metadata: None,
    }
}

fn conditional_leaf(
    kind: ConditionalExpressionKind,
    operator: Option<String>,
    operands: &[(String, String)],
) -> ConditionalExpression {
    ConditionalExpression {
        kind,
        open_delimiter: None,
        open_delimiter_metadata: None,
        pattern_operand: conditional_pattern_operand(operator.as_deref(), operands),
        operator,
        operands: operands.iter().map(|(arg, _)| arg.clone()).collect(),
        children: Vec::new(),
        close_delimiter: None,
        close_delimiter_metadata: None,
    }
}

fn token_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}

fn conditional_pattern_operand(
    operator: Option<&str>,
    operands: &[(String, String)],
) -> Option<ConditionalPatternOperand> {
    let rhs = operands.get(1)?;
    let kind = match operator? {
        "=" | "==" | "!=" => ConditionalPatternKind::Glob,
        "=~" => ConditionalPatternKind::Regex,
        _ => return None,
    };
    Some(ConditionalPatternOperand::new_with_raw(
        rhs.0.clone(),
        rhs.1.clone(),
        kind,
    ))
}

fn top_level_operator(args: &[(String, String)], operator: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate() {
        match arg.0.as_str() {
            "(" => depth += 1,
            ")" => depth = depth.saturating_sub(1),
            _ if depth == 0 && arg.0 == operator => return Some(index),
            _ => {}
        }
    }
    None
}

fn conditional_outer_group(args: &[(String, String)]) -> Option<&[(String, String)]> {
    if args.first().map(|(arg, _)| arg.as_str()) != Some("(")
        || args.last().map(|(arg, _)| arg.as_str()) != Some(")")
    {
        return None;
    }

    let mut depth = 0usize;
    for (index, arg) in args.iter().enumerate() {
        match arg.0.as_str() {
            "(" => depth += 1,
            ")" => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index != args.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }

    (depth == 0).then_some(&args[1..args.len() - 1])
}

fn is_conditional_unary_operator(op: &str) -> bool {
    matches!(
        op,
        "-a" | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-G"
            | "-h"
            | "-k"
            | "-L"
            | "-n"
            | "-O"
            | "-o"
            | "-p"
            | "-R"
            | "-r"
            | "-S"
            | "-s"
            | "-t"
            | "-u"
            | "-N"
            | "-v"
            | "-w"
            | "-x"
            | "-z"
    )
}

fn is_conditional_binary_operator(op: &str) -> bool {
    matches!(
        op,
        "=" | "=="
            | "!="
            | "=~"
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-ef"
            | "-nt"
            | "-ot"
    )
}
