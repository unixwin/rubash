use super::*;
use crate::lexer::Token;

pub(super) fn parse_conditional_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    if tokens.get(start)?.value != "[[" {
        return None;
    }

    let end = matching_conditional_end(tokens, start)?;
    let merged_args = merge_pattern_rhs_fragments(collect_conditional_args(tokens, start + 1, end));
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
    if let Some((spec, echo_line)) = conditional_syntax_error_spec(&merged_args, true) {
        command.insert_assignment("__RUBASH_PARSE_ERROR_COND__".to_string(), spec);
        if let Some(source) = conditional_error_source(tokens, start, end, echo_line) {
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
pub(super) fn conditional_syntax_error_spec(
    args: &[CondArg],
    closed: bool,
) -> Option<(String, Option<usize>)> {
    let open_line = args.first().map(|(_, _, _, line)| *line).unwrap_or(1);
    let mut cursor = CondCursor {
        args,
        index: 0,
        closed,
        newline_consumed_for: None,
        virtual_newline_seen: false,
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
    Some(fail.encode(args))
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
    End,
}

impl CondOffending {
    fn of(tok: &CondTok, index: Option<usize>, anchor: usize) -> Self {
        match tok {
            CondTok::Eof => CondOffending::Eof,
            CondTok::Newline => CondOffending::VirtualNewline(anchor),
            CondTok::End => CondOffending::End,
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
    fn encode(self, args: &[CondArg]) -> (String, Option<usize>) {
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
        // Near shape: error_token_from_text (parse.y:6772) reports the
        // offending token text; when the offender is virtual (newline or
        // `]]`) it falls back to the last real token before it — `(x'
        // reports `(x' because `(' is not a break character, and `x\n]]'
        // reports `x' not `]]'.
        let (near, near_line, echo_line) = match &self.offending {
            CondOffending::Index(index) => (
                walk_back_near_token(args, *index),
                args[*index].3,
                args[*index].3,
            ),
            CondOffending::End => ("]]".to_string(), last_line, last_line),
            CondOffending::VirtualNewline(anchor) => {
                let last = (*anchor).min(args.len()).saturating_sub(1);
                (
                    walk_back_near_token(args, last),
                    newline_line(args, *anchor),
                    newline_line(args, *anchor),
                )
            }
            // Unreachable — Eof encodes the `eof' shape above.
            CondOffending::Eof => (
                walk_back_near_token(args, args.len().saturating_sub(1)),
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

/// error_token_from_text (parse.y:6772-6808): the reported token extends
/// left across characters that are not ` \n\t;|&` breaks, so adjacent
/// arguments merge into one reported string (`(x' for `(x').
fn walk_back_near_token(args: &[CondArg], mut index: usize) -> String {
    let mut text = args
        .get(index)
        .map(|(value, _, _, _)| value.clone())
        .unwrap_or_default();
    while index > 0 && args[index].2.is_empty() {
        index -= 1;
        text = format!("{}{}", args[index].0, text);
    }
    text
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
    /// The arg index whose leading `\n' token was already consumed, so each
    /// physical newline surfaces exactly once (GNU read_token returns it as
    /// a token in operand slots).
    newline_consumed_for: Option<usize>,
    /// The virtual `\n' GNU's reader reports at end-of-input before EOF.
    virtual_newline_seen: bool,
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
                ("!", _) => CondTok::Bang,
                (_, false) => CondTok::Word,
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

/// cond_term (parse.y:5074-5247). `Err` carries the printed parser_error
/// messages and the offending token — equivalent to COND_RETURN_ERROR.
fn cond_term(cursor: &mut CondCursor) -> Result<(), CondFail> {
    let (tok, line, index, anchor) = cursor.skip_newlines();
    match tok {
        // `[[ ]]` — COND_END at term start is a silent COND_ERROR; cond_error
        // prints nothing and yyerror reports `syntax error near `]]''.
        CondTok::End => Err(CondFail {
            messages: Vec::new(),
            offending: CondOffending::End,
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
            let word = cursor.args[index.unwrap()].0.clone();
            if is_conditional_unary_operator(&word) && word.len() == 2 {
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
                    let op = &cursor.args[index.unwrap()].0;
                    is_conditional_binary_operator(op)
                        || op == "="
                        || op == "=="
                        || op == "!="
                        || op == "=~"
                }
                CondTok::Char('<') | CondTok::Char('>') => true,
                _ => false,
            };
            if binop {
                let (tok, line, index, anchor) = cursor.next();
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
                    let text = cond_offending_text(cursor, &tok, index);
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
        return cursor.args[i].0.clone();
    }
    match tok {
        CondTok::Char(c) => c.to_string(),
        other => etext(other).to_string(),
    }
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
    for token in &tokens[..=end.min(tokens.len() - 1)] {
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
            args.push((
                strip_conditional_quote_markers(&word),
                raw,
                ws,
                tokens[index].position,
            ));
            index = next_i;
            continue;
        }
        push_cond_arg(&mut args, &tokens[index], &mut pending_newline);
        index += 1;
    }
    args
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

        // The first fragment always begins the RHS word.
        let (mut value, mut raw_text, _, line) = arg_parts[index].clone();
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
            }
            value.push_str(ws);
            value.push_str(v);
            raw_text.push_str(ws);
            raw_text.push_str(r);
            depth += unquoted_paren_delta(&arg_parts[index]);
            index += 1;
        }
        args.push((value, raw_text, String::new(), line));
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

fn matching_conditional_end(tokens: &[Token], start: usize) -> Option<usize> {
    (start + 1..tokens.len()).find(|&index| tokens[index].raw == "]]")
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
) -> (CommandNode, usize) {
    let merged = merge_pattern_rhs_fragments(collect_conditional_args(
        tokens,
        start + 1,
        tokens.len() - 1,
    ));
    let (spec, echo_line) = conditional_syntax_error_spec(&merged, false).unwrap_or_else(|| {
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
    command.insert_assignment("__RUBASH_PARSE_ERROR_COND__".to_string(), spec);
    if let Some(source) = conditional_error_source(tokens, start, tokens.len() - 1, echo_line) {
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
