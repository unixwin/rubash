use super::parse_loop::{unclosed_brace_eof_node, unclosed_paren_eof_node};
use super::*;
use crate::lexer::{Token, TokenKind};

/// GNU parse.y:3667 read_token: `(` and `)` are operator tokens only when
/// read UNQUOTED (the shellmeta branch returns the character itself); a
/// quoted or escaped paren is part of a WORD (parse.y:5305
/// read_token_word). The function_def grammar (parse.y:1054-1056
/// `WORD '(' ')'` / `FUNCTION WORD '(' ')'`) consumes the operator tokens
/// only. rubash's lexer emits the operator form as TokenKind::Keyword and
/// a quoted/escaped paren as a TokenKind::Word carrying the same value,
/// so every paren test on the function head must check the operator kind,
/// never the text alone (rubash#3388: `echo '(' ')'` and `[ '(' ')'`
/// parsed as function definitions and aborted the script).
fn is_paren_operator(token: &Token, text: &str) -> bool {
    token.kind == TokenKind::Keyword && token.value == text
}

pub(super) fn parse_function_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    parse_function_command_with_diagnostic(tokens, start, None, 0)
}

/// A `syntax error near unexpected token' node in the established
/// `__RUBASH_PARSE_ERROR__' encoding (message + physical source line),
/// rendered by the executor's parse-error arm with EX_BADUSAGE=2.
fn unexpected_token_error_node(
    token_text: &str,
    line: usize,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> CommandNode {
    let mut command = CommandNode::new();
    command.line = Some(line);
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR__".to_string(),
        format!("unexpected token `{token_text}'"),
    );
    if let Some(text) = diagnostic_text {
        if let Some(source) = line
            .checked_sub(source_line_offset)
            .and_then(|line_in_text| super::parse_loop::source_line_by_number(text, line_in_text))
        {
            command.insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
        }
    }
    command
}

/// A `syntax error: unexpected end of file' node: the executor's EOF arm
/// renders node.line + 1, so the anchor line is the last input line.
fn unexpected_eof_error_node(tokens: &[Token], start: usize) -> CommandNode {
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.insert_assignment("__RUBASH_PARSE_ERROR_EOF__".to_string(), "1".to_string());
    command
}

/// GNU parse.y function_def: the body is a `compound_command' parsed by the
/// SAME yyparse run — a stray `)' / `;;' inside it is the yacc
/// `syntax error near unexpected token' production (parse.y:6724 yyerror →
/// parse.y:6833 report_syntax_error → parse.y:6861 message +
/// parse.y:6867 print_offending_line, EX_BADUSAGE=2), aborting the whole
/// input. rubash parses bodies as nested token slices, so the nested parse
/// must run with the driver's strictness (`stray_close_is_error`) and the
/// original source text so the diagnostic echoes the physical offending
/// line (parse.y print_offending_line), not reconstructed token spacing.
pub(super) fn parse_function_command_with_diagnostic(
    tokens: &[Token],
    start: usize,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): Bash has full function_def grammar,
    // including `function name`, redirections, nested compound commands, and
    // parser-state-sensitive reserved words. This maps the upstream builtins
    // `name() { ...; }` and `function name { ...; }` forms onto a function
    // command node.
    let keyword_form = is_keyword(tokens, start, "function");
    // Probe admission: this parser is tried at every command start, and
    // almost every probe is an ordinary simple command. A non-`function`
    // definition can only begin with a `<(...)' name (RedirectIn `<'
    // directly followed by `('), a `!!' name (Keyword `!'), or a name
    // token directly followed by `()' / `(' — every other entry walks
    // the reassemblies and name checks below only to return None (after
    // cloning the candidate name). Bail before that walk when none of
    // the three shapes is present; the shapes admitted here are exactly
    // the ones the code below can answer Some for (GNU parse.y
    // function_def: WORD '(' ')' | `function' WORD ['(' ')']).
    if !keyword_form {
        let paren_follows = tokens
            .get(start + 1)
            .is_some_and(|token| is_paren_operator(token, "(") || is_paren_operator(token, "()"));
        let group_follows = tokens.get(start).is_some_and(|token| {
            (token.kind == TokenKind::RedirectIn && token.value == "<")
                || (token.kind == TokenKind::Keyword && token.value == "!")
        });
        if !paren_follows && !group_follows {
            return None;
        }
    }
    // GNU parse.y function_def: `function' must be followed by a WORD
    // name. A bare `function' — at end of input, before a newline, or
    // before a hard `;' — is a syntax error at that token, never a
    // command named `function' (rubash#1142: it fell through to the
    // word path and reported `function: command not found', rc 127,
    // and the script kept running).
    if keyword_form {
        let near_error = |name: &str, blame: usize, source_index: usize| {
            let mut command = CommandNode::new();
            command.line = Some(tokens[start].position);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                format!(
                    "{name}{}{}",
                    crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                    blame
                ),
            );
            command.insert_assignment(
                "__RUBASH_PARSE_SOURCE__".to_string(),
                super::parse_loop::offending_line_text(
                    tokens,
                    source_index,
                    diagnostic_text.map(|rc| &**rc),
                    source_line_offset,
                ),
            );
            Some((command, tokens.len()))
        };
        match tokens.get(start + 1) {
            None => {
                return near_error("newline", tokens[start].position, start);
            }
            Some(token) if token.kind == TokenKind::Eof => {
                return near_error("newline", tokens[start].position, start);
            }
            Some(token) if token.kind == TokenKind::Semicolon => {
                let name = if token.line_break { "newline" } else { ";" };
                return near_error(name, token.position, start + 1);
            }
            // `function ( ...' — a paren cannot start the name (GNU
            // reports the `(' itself); the word path would misblame the
            // token inside the group.
            Some(token) if token.kind == TokenKind::Keyword && token.value == "(" => {
                return near_error("(", token.position, start + 1);
            }
            _ => {}
        }
    }
    // GNU parse.y: function_def: WORD '(' ')' ... -- the name can be any
    // word, and a `<(...)' process-substitution-like token is read as one
    // WORD (so `<(:) () { ...; }' is a function definition whose name is
    // `<(:)', later rejected by valid_function_word). rubash's lexer emits
    // `<` (RedirectIn) plus the parenthesized group separately, so reassemble
    // the `<(...)' name here when the closing `)' is followed by `()'.
    let lt_group = if keyword_form {
        None
    } else {
        lt_group_function_name(tokens, start)
    };
    // `!!' lexes as two Keyword `!' tokens in rubash (GNU reads it as one
    // WORD), so `!! () { ...; }' needs the same reassembly as `<(...)'.
    let bang_group = if keyword_form {
        None
    } else {
        bang_group_function_name(tokens, start)
    };
    let special_group = lt_group.or(bang_group);
    let special_group_taken = special_group.is_some();
    let (name, name_raw, mut i) = if let Some((name, name_raw, next_i)) = special_group {
        (name, name_raw, next_i)
    } else {
        let (name_index, i) = if keyword_form {
            (start + 1, start + 2)
        } else {
            (start, start + 1)
        };
        if keyword_form {
            // GNU parse.y function_def demands WORD after `function':
            // end of input there is the yyerror EOF path, and a separator
            // in the name slot is itself the offending token — a line
            // break reports as `newline', a real `;' as `;' (rubash#1154:
            // bare `function' used to fall back to a simple command and
            // report `function: command not found', rc 127, and the
            // script kept running).
            match tokens.get(name_index) {
                None => {
                    return Some((unexpected_eof_error_node(tokens, start), tokens.len()));
                }
                Some(token) if token.kind == TokenKind::Semicolon => {
                    let display = if token.line_break { "newline" } else { ";" };
                    return Some((
                        unexpected_token_error_node(
                            display,
                            token.position,
                            diagnostic_text,
                            source_line_offset,
                        ),
                        tokens.len(),
                    ));
                }
                _ => {}
            }
        }
        let name_token = tokens.get(name_index)?;
        (name_token.value.clone(), name_token.raw.clone(), i)
    };
    if !special_group_taken
        && !(is_function_name(&name)
            || is_quoted_function_name(&name, &name_raw)
            || (keyword_form && is_function_keyword_name(&name)))
    {
        return None;
    }
    let compact_parentheses = tokens
        .get(i)
        .is_some_and(|token| is_paren_operator(token, "()"));
    let separated_parentheses = tokens.get(i).is_some_and(|token| {
        is_paren_operator(token, "(")
            && tokens
                .get(i + 1)
                .is_some_and(|next| is_paren_operator(next, ")"))
    });
    let has_parentheses = compact_parentheses || separated_parentheses;
    let keyword_metadata = keyword_form.then(|| build_token_metadata(&tokens[start]));
    let (open_paren_metadata, close_paren_metadata) = if compact_parentheses {
        (Some(build_token_metadata(tokens.get(i)?)), None)
    } else if separated_parentheses {
        (
            Some(build_token_metadata(tokens.get(i)?)),
            Some(build_token_metadata(tokens.get(i + 1)?)),
        )
    } else {
        (None, None)
    };
    if compact_parentheses {
        i += 1;
    } else if separated_parentheses {
        i += 2;
    } else if !keyword_form {
        return None;
    }

    // GNU parse.y function_def admits only newline_list between the
    // name/`()' and the body — a hard `;' there is a syntax error at
    // the `;' (`function f; ...', `f() ; { ... }'; rubash#1142), and
    // end of input is the plain unexpected-EOF report (`function f').
    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon && token.line_break)
    {
        i += 1;
    }
    if let Some(token) = tokens.get(i) {
        if token.kind == TokenKind::Semicolon {
            let mut command = CommandNode::new();
            command.line = Some(tokens[start].position);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                format!(
                    ";{}{}",
                    crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                    token.position
                ),
            );
            command.insert_assignment(
                "__RUBASH_PARSE_SOURCE__".to_string(),
                super::parse_loop::offending_line_text(
                    tokens,
                    i,
                    diagnostic_text.map(|rc| &**rc),
                    source_line_offset,
                ),
            );
            return Some((command, tokens.len()));
        }
    } else {
        let mut command = CommandNode::new();
        command.line = Some(
            tokens
                .iter()
                .map(|token| token.position)
                .max()
                .unwrap_or(tokens[start].position),
        );
        command.insert_assignment("__RUBASH_PARSE_ERROR_EOF__".to_string(), "1".to_string());
        return Some((command, tokens.len()));
    }
    if tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Eof)
    {
        let mut command = CommandNode::new();
        command.line = Some(
            tokens
                .iter()
                .map(|token| token.position)
                .max()
                .unwrap_or(tokens[start].position),
        );
        command.insert_assignment("__RUBASH_PARSE_ERROR_EOF__".to_string(), "1".to_string());
        return Some((command, tokens.len()));
    }
    if let Some(group) = tokens
        .get(i)
        .map(|token| token.value.trim())
        .filter(|value| value.starts_with('{') && value.ends_with('}'))
    {
        // TODO(parse.y): The lexer can currently preserve a full brace group
        // as one token. Recognize it as a function body for `name() { ...; }`
        // until the parser owns brace groups structurally.
        let inner = group.trim_start_matches('{').trim_end_matches('}').trim();
        // rubash#131: re-parse the folded body under the gate of the pass
        // that folded it (Token::extglob_gate) — GNU decides at read time.
        let saved_extglob = crate::lexer::parse_extended_glob();
        let group_gate = tokens.get(i).map_or(true, |token| token.extglob_gate);
        crate::lexer::set_parse_extended_glob(group_gate);
        // rubash#1544: same read-time discipline for the posix mode —
        // the folded body re-lexes under its fold pass's mode, locked
        // (see Token::posix_gate).
        let group_posix_gate = tokens.get(i).map_or(false, |token| token.posix_gate);
        let mut body_tokens = crate::lexer::tokenize_locked_body(inner, group_posix_gate);
        // GNU parse.y:1264-1290 list grammar, same gate as
        // parse_brace_group_command: a body whose last significant token is
        // a dangling connector (`f() { x && }') is not a terminated list —
        // the `}' token arrives where the grammar demands a pipeline, so
        // GNU's yacc error names `}' and print_offending_line
        // (parse.y:6813-6826) echoes that physical line (rubash#285: the
        // body used to re-parse as a bare dangling `&&' and report
        // `near unexpected end of file' instead).
        let tail_dangles = body_tokens.last().is_some_and(|last| {
            matches!(
                last.kind,
                TokenKind::And | TokenKind::Or | TokenKind::Pipe | TokenKind::PipeErr
            )
        });
        // GNU parse.y:1196 `group_command: '{' compound_list '}'` via
        // function_def: the body must hold at least one COMMAND — a
        // comment-only or empty body (`f() { \n # c \n }`) is a syntax
        // error at the `}' token, not an accepted definition (rubash#327;
        // comments are reader noise, line breaks are separators).
        let body_has_command = body_tokens
            .iter()
            .any(|token| !(token.kind == TokenKind::Semicolon && token.line_break));
        if tail_dangles || !body_has_command {
            crate::lexer::set_parse_extended_glob(saved_extglob);
            let mut command = CommandNode::new();
            let close_line = tokens.get(i).map_or(1, |token| {
                token.position
                    + token.raw[..token.raw.rfind('}').unwrap_or(0)]
                        .matches('\n')
                        .count()
            });
            command.line = Some(close_line);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR__".to_string(),
                "unexpected token `}'".to_string(),
            );
            let source_line = diagnostic_text.and_then(|text| {
                close_line
                    .checked_sub(source_line_offset)
                    .and_then(|line_in_text| {
                        super::parse_loop::source_line_by_number(text, line_in_text)
                    })
            });
            if let Some(line) = source_line {
                command.insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), line);
            }
            return Some((command, i + 1));
        }
        // GNU parse.y keeps absolute source lines inside function bodies:
        // `typeset -n v=$1` under a multi-line `function f1 { ... }` reports
        // its own line (nameref8.sub: line 16, not the `function` line 14).
        // The re-lexed tokens are relative to `inner`; inner line 1 is the
        // first non-whitespace byte after the group token's open brace.
        if let Some(group_token) = tokens.get(i) {
            let raw = group_token.raw.as_str();
            let open = raw.find('{').map(|at| at + 1).unwrap_or(0);
            let close = raw.rfind('}').unwrap_or(raw.len());
            let span = raw.get(open..close).unwrap_or("");
            let lead = span.len() - span.trim_start().len();
            let base = group_token.position + raw[..open + lead].matches('\n').count();
            for token in &mut body_tokens {
                token.position += base - 1;
            }
        }
        let mut body = parse_function_body(&body_tokens, diagnostic_text, source_line_offset);
        crate::lexer::set_parse_extended_glob(saved_extglob);
        // Alias-introduced compound openers (e.g. `alias forever='while
        // :;'`) expand in GNU's reader, before the body parse — this token
        // path cannot see the alias table, so keep the verbatim body text
        // for the executor's one alias-aware retry at definition time.
        let unparsed_body_source = find_body_parse_error(&body).map(|_| {
            (
                inner.to_string(),
                tokens
                    .get(i)
                    .map(|token| token.position)
                    .unwrap_or_default(),
            )
        });
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.logical_line = tokens.get(start).map(|token| token.logical_line);
        let mut function = function_command(
            name.clone(),
            name_raw.clone(),
            body,
            keyword_form,
            keyword_metadata.clone(),
            has_parentheses,
            open_paren_metadata.clone(),
            close_paren_metadata.clone(),
            FunctionBodyKind::BraceGroup,
            Some(i),
            Some(i),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
        );
        function.unparsed_body_source = unparsed_body_source;
        command.function_command = Some(function);
        return Some(finish_function_command(command, tokens, i + 1));
    }
    if let Some((mut body_command, body_end)) =
        parse_function_compound_body(tokens, i, diagnostic_text, source_line_offset)
    {
        if body_command.line.is_none() {
            body_command.line = tokens.get(start).map(|token| token.position);
            body_command.logical_line = tokens.get(start).map(|token| token.logical_line);
        }
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.logical_line = tokens.get(start).map(|token| token.logical_line);
        command.function_command = Some(function_command(
            name.clone(),
            name_raw.clone(),
            vec![body_command],
            keyword_form,
            keyword_metadata.clone(),
            has_parentheses,
            open_paren_metadata.clone(),
            close_paren_metadata.clone(),
            FunctionBodyKind::CompoundCommand,
            Some(i),
            body_end.checked_sub(1),
            tokens
                .get(body_end.saturating_sub(1))
                .map(|token| token.position),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
        ));
        return Some(finish_function_command(command, tokens, body_end));
    }

    if tokens
        .get(i)
        .is_some_and(|token| is_paren_operator(token, "("))
    {
        // GNU parse.y:6890-6901: `name() (` reaching EOF unclosed reports
        // "unexpected end of file from `(' command on line N" — do not let
        // the `?` fall back to a `(`-unexpected simple command.
        let Some((mut body, close_i)) =
            parse_parenthesized_function_body(tokens, i, diagnostic_text, source_line_offset)
        else {
            let command = unclosed_paren_eof_node(
                tokens,
                i,
                diagnostic_text.map(|s| &**s),
                source_line_offset,
            );
            return Some((command, tokens.len()));
        };
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.logical_line = tokens.get(start).map(|token| token.logical_line);
        command.function_command = Some(function_command(
            name.clone(),
            name_raw.clone(),
            body,
            keyword_form,
            keyword_metadata.clone(),
            has_parentheses,
            open_paren_metadata.clone(),
            close_paren_metadata.clone(),
            FunctionBodyKind::Subshell,
            Some(i),
            Some(close_i),
            tokens.get(close_i).map(|token| token.position),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
        ));
        return Some(finish_function_command(command, tokens, close_i + 1));
    }

    if let Some((mut body, body_end)) =
        parse_function_command_sequence_body(tokens, i, diagnostic_text, source_line_offset)
    {
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.logical_line = tokens.get(start).map(|token| token.logical_line);
        command.function_command = Some(function_command(
            name.clone(),
            name_raw.clone(),
            body,
            keyword_form,
            keyword_metadata.clone(),
            has_parentheses,
            open_paren_metadata.clone(),
            close_paren_metadata.clone(),
            FunctionBodyKind::CommandSequence,
            Some(i),
            body_end.checked_sub(1),
            tokens
                .get(body_end.saturating_sub(1))
                .map(|token| token.position),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
        ));
        return Some(finish_function_command(command, tokens, body_end));
    }

    let body_token = tokens.get(i)?;
    if body_token.value.trim() != "{" {
        // GNU parse.y:6890-6891 (yyerror EOF path): `name() { ...` or
        // `name() ( ...` that reaches EOF unclosed reports
        // "unexpected end of file from `X' command on line N" naming the
        // innermost unclosed compound — not a `(`-unexpected fallback to a
        // simple command.
        if body_token.kind == TokenKind::Keyword
            && body_token.value.starts_with('{')
            && !body_token.value.trim_end().ends_with('}')
        {
            let command = unclosed_brace_eof_node(
                tokens,
                i,
                diagnostic_text.map(|s| &**s),
                source_line_offset,
            );
            return Some((command, tokens.len()));
        }
        // parse.y:1054-1061 function_def: after `function WORD' (with or
        // without the `'()'` pair) the grammar demands a compound
        // shell_command body — a plain WORD there is a syntax error
        // naming that word (`syntax error near unexpected token `g'',
        // verified vs WSL GNU Bash 5.3.0; compound openers like `if` /
        // `while` / `for` / `case` are legal bodies). Falling through to a
        // simple command named `function' ran `function: command not
        // found' with rc=0 instead (rubash#302 family).
        // rubash#2748: a REDIRECTION in body position is the same
        // rejection — function_def admits redirections only AFTER the
        // body (function_body: shell_command [redirection_list]), so
        // `function f > /dev/null' names the operator (an fd-led form
        // names the fd digit, matching near_blame_text's rule). The
        // commitment is the same for the `name ()' form without the
        // `function' keyword: `f() > /dev/null { ...; }' names `>' too.
        if (keyword_form || has_parentheses)
            && matches!(
                body_token.kind,
                TokenKind::Word
                    | TokenKind::Variable
                    | TokenKind::Assignment
                    | TokenKind::CommandSubst
                    | TokenKind::BraceExpand
                    | TokenKind::RedirectIn
                    | TokenKind::RedirectOut
                    | TokenKind::Append
                    | TokenKind::RedirectErr
                    | TokenKind::RedirectErrAppend
                    | TokenKind::HereDoc
                    | TokenKind::HereString
            )
        {
            let blame = if matches!(
                body_token.kind,
                TokenKind::RedirectIn
                    | TokenKind::RedirectOut
                    | TokenKind::Append
                    | TokenKind::RedirectErr
                    | TokenKind::RedirectErrAppend
                    | TokenKind::HereDoc
                    | TokenKind::HereString
            ) {
                near_blame_text(body_token)
            } else {
                body_token.value.clone()
            };
            let mut command = CommandNode::new();
            command.line = Some(body_token.position);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR__".to_string(),
                format!("unexpected token `{blame}'"),
            );
            if let Some(text) = diagnostic_text {
                if let Some(source) = body_token
                    .position
                    .checked_sub(source_line_offset)
                    .and_then(|line_in_text| {
                        super::parse_loop::source_line_by_number(text, line_in_text)
                    })
                {
                    command.insert_assignment(
                        "__RUBASH_PARSE_SOURCE__".to_string(),
                        source.to_string(),
                    );
                }
            }
            return Some((command, tokens.len()));
        }
        // GNU parse.y function_def (parse.y:1054-1061): after the name
        // and `()' the grammar demands a compound_command body, and the
        // token that arrived instead is the yyerror token (parse.y:6727
        // -> report_syntax_error, parse.y:6833) — `f()}' names `}',
        // `f() > /dev/null { ... }' names `>' (redirections on a
        // definition belong AFTER the body), `f() | cat' names `|'.
        // Returning None here let the main loop reinterpret the `()'
        // as a subshell and name `(' instead (rubash#2747).
        let mut command = CommandNode::new();
        command.line = Some(tokens[start].position);
        command.insert_assignment(
            "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
            format!(
                "{}{}{}",
                body_token.value,
                crate::executor::markers::PARSE_ERROR_FIELD_SEP,
                body_token.position
            ),
        );
        command.insert_assignment(
            "__RUBASH_PARSE_SOURCE__".to_string(),
            super::parse_loop::offending_line_text(
                tokens,
                i,
                diagnostic_text.map(|rc| &**rc),
                source_line_offset,
            ),
        );
        return Some((command, tokens.len()));
    }
    let open_brace = i;
    i += 1;
    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        i += 1;
    }

    let body_start = i;
    let Some(i) = matching_brace_group_end(tokens, open_brace).or_else(|| {
        // A heredoc body is a complete lexical token, but older boundary
        // paths can omit the separator before the function closing brace.
        // Retain the final brace as the body terminator for serialized
        // functions when no nested delimiter was recognized.
        (open_brace + 1..tokens.len())
            .rev()
            .find(|&index| is_keyword(tokens, index, "}"))
    }) else {
        // GNU parse.y:6890-6901: the `{` body never closed — report the
        // compound-EOF error naming the innermost unclosed compound rather
        // than falling back to a `(`-unexpected simple command.
        let command = unclosed_brace_eof_node(
            tokens,
            open_brace,
            diagnostic_text.map(|s| &**s),
            source_line_offset,
        );
        return Some((command, tokens.len()));
    };

    let body = parse_function_body(&tokens[body_start..i], diagnostic_text, source_line_offset);
    // GNU parse.y:1196 `group_command: '{' compound_list '}'` via
    // function_def: the body must hold at least one COMMAND — comments are
    // reader noise and line breaks are separators, so a body slice of
    // nothing but line-break separators (`f() { <newline> # c <newline>
    // }`) is a syntax error at the `}' token, reported with the `}' line
    // echoed (rubash#327).
    if !tokens[body_start..i]
        .iter()
        .any(|token| !(token.kind == TokenKind::Semicolon && token.line_break))
    {
        let mut command = CommandNode::new();
        command.line = tokens.get(i).map(|token| token.position);
        command.logical_line = tokens.get(i).map(|token| token.logical_line);
        command.insert_assignment(
            "__RUBASH_PARSE_ERROR__".to_string(),
            "unexpected token `}'".to_string(),
        );
        let close_line = tokens.get(i).map_or(1, |token| token.position);
        if let Some(text) = diagnostic_text {
            if let Some(source) =
                close_line
                    .checked_sub(source_line_offset)
                    .and_then(|line_in_text| {
                        super::parse_loop::source_line_by_number(text, line_in_text)
                    })
            {
                command.insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
            }
        }
        return Some((command, i + 1));
    }
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
    command.function_command = Some(function_command(
        name,
        name_raw,
        body,
        keyword_form,
        keyword_metadata,
        has_parentheses,
        open_paren_metadata,
        close_paren_metadata,
        FunctionBodyKind::BraceGroup,
        Some(body_start),
        i.checked_sub(1),
        tokens.get(i).map(|token| token.position),
        tokens
            .get(open_brace)
            .map(|token| token.position + token.raw.matches('\n').count()),
    ));
    Some(finish_function_command(command, tokens, i + 1))
}

fn finish_function_command(
    mut command: CommandNode,
    tokens: &[Token],
    index: usize,
) -> (CommandNode, usize) {
    // GNU parse.y function_def: a syntax error anywhere in the body makes
    // the entire definition fail — yyerror aborts the parse before the
    // function is ever defined. The strict body parse parks its
    // `__RUBASH_PARSE_ERROR__' node inside the body; return THAT node in
    // place of the definition so the executor's standard parse-error arm
    // reports both GNU lines (message + offending line) and aborts with
    // EX_BADUSAGE=2 instead of silently defining a broken function
    // (rubash#213: `f() { echo ); }' used to define and rc=0).
    if let Some(function) = command.function_command.as_ref() {
        if let Some(error) = find_body_parse_error(&function.body) {
            return (error, index);
        }
    }
    // rubash#3209: a bare `name()` definition plants the body's
    // first-command stamp at the name's line (GNU parse.y:5849-5851
    // via the command-position word capture; the `function` keyword
    // forms plant nothing because the name follows the FUNCTION
    // token). The definition node's own line is the name token's
    // line on every construction path above.
    if let Some(definition_line) = command.line {
        if let Some(function) = command.function_command.as_mut() {
            if !function.keyword && function.has_parentheses {
                restamp_function_body_first_command(&mut function.body, definition_line);
            }
        }
    }
    let (command, mut next_i) = finish_compound_command(command, tokens, index);
    while tokens
        .get(next_i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        next_i += 1;
    }
    (command, next_i)
}

/// Parse a function-body token slice with the driver's strictness: a `)' or
/// case terminator at command position is a syntax error node, not a
/// silently dropped token. `diagnostic_text' is the original source text
/// (parse.y shell_input_line) so the error echoes the physical offending
/// line; body token positions are absolute script lines, so the outer
/// parse's `source_line_offset' maps them into that text.
fn parse_function_body(
    tokens: &[Token],
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Vec<CommandNode> {
    crate::parser::parse_with_options(
        tokens,
        crate::parser::ParseLoopOptions {
            stray_close_is_error: true,
            source_text: diagnostic_text.cloned(),
            source_line_offset,
            ..Default::default()
        },
    )
    .commands
}

/// Depth-first search for a parse-error node anywhere in a function body
/// tree. Returns a clone of the node carrying the
/// `__RUBASH_PARSE_ERROR__'/`__RUBASH_PARSE_SOURCE__' pair (line numbers
/// and verbatim source ride on it), or None when the whole body parsed.
pub(crate) fn find_body_parse_error(commands: &[CommandNode]) -> Option<CommandNode> {
    for command in commands {
        if command
            .assignments
            .iter()
            .any(|(name, _)| name.starts_with("__RUBASH_PARSE_ERROR"))
        {
            return Some(command.clone());
        }
        let nested: Option<&Vec<CommandNode>> =
            if let Some(function) = command.function_command.as_ref() {
                Some(&function.body)
            } else if let Some(group) = command.brace_group.as_ref() {
                Some(&group.body)
            } else if let Some(subshell) = command.subshell_command.as_ref() {
                Some(&subshell.body)
            } else if let Some(coproc) = command.coproc_command.as_ref() {
                coproc.body.as_ref()
            } else {
                None
            };
        if let Some(found) = nested.and_then(|body| find_body_parse_error(body)) {
            return Some(found);
        }
        if let Some(for_command) = command.for_command.as_ref() {
            if let Some(found) = find_body_parse_error(&for_command.body) {
                return Some(found);
            }
        }
        if let Some(if_command) = command.if_command.as_ref() {
            for branch in if_command
                .condition
                .iter()
                .chain(if_command.then_body.iter())
                .chain(
                    if_command
                        .elif_branches
                        .iter()
                        .flat_map(|branch| branch.condition.iter().chain(branch.body.iter())),
                )
                .chain(if_command.else_body.iter().flatten())
            {
                if let Some(found) = find_body_parse_error(std::slice::from_ref(branch)) {
                    return Some(found);
                }
            }
        }
        if let Some(loop_command) = command.loop_command.as_ref() {
            for branch in loop_command
                .condition
                .iter()
                .chain(loop_command.body.iter())
            {
                if let Some(found) = find_body_parse_error(std::slice::from_ref(branch)) {
                    return Some(found);
                }
            }
        }
        if let Some(select_command) = command.select_command.as_ref() {
            if let Some(found) = find_body_parse_error(&select_command.body) {
                return Some(found);
            }
        }
        if let Some(case_command) = command.case_command.as_ref() {
            for clause in &case_command.clauses {
                if let Some(found) = find_body_parse_error(&clause.body) {
                    return Some(found);
                }
            }
        }
        if let Some(pipeline) = command.pipeline_command.as_ref() {
            if let Some(found) = find_body_parse_error(&pipeline.stages) {
                return Some(found);
            }
        }
        if let Some(and_or) = command.and_or_list.as_ref() {
            if let Some(found) = find_body_parse_error(&and_or.commands) {
                return Some(found);
            }
        }
        if let Some(time) = command.time_command.as_ref() {
            if let Some(found) = find_body_parse_error(std::slice::from_ref(&time.command)) {
                return Some(found);
            }
        }
        if let Some(background) = command.background_command.as_ref() {
            if let Some(found) = find_body_parse_error(std::slice::from_ref(&background.command)) {
                return Some(found);
            }
        }
        if let Some(inverted) = command.inverted_command.as_ref() {
            if let Some(found) = find_body_parse_error(std::slice::from_ref(&inverted.command)) {
                return Some(found);
            }
        }
    }
    None
}

fn function_command(
    name: String,
    name_raw: String,
    body: Vec<CommandNode>,
    keyword: bool,
    keyword_metadata: Option<Box<WordMetadata>>,
    has_parentheses: bool,
    open_paren_metadata: Option<Box<WordMetadata>>,
    close_paren_metadata: Option<Box<WordMetadata>>,
    body_kind: FunctionBodyKind,
    body_start: Option<usize>,
    body_end: Option<usize>,
    body_end_line: Option<usize>,
    body_open_line: Option<usize>,
) -> Box<FunctionCommand> {
    let (
        body_open_delimiter,
        body_open_delimiter_metadata,
        body_close_delimiter,
        body_close_delimiter_metadata,
    ) = match body_kind {
        FunctionBodyKind::BraceGroup => (
            Some("{".to_string()),
            Some(delimiter_metadata("{")),
            Some("}".to_string()),
            Some(delimiter_metadata("}")),
        ),
        FunctionBodyKind::Subshell => (
            Some("(".to_string()),
            Some(delimiter_metadata("(")),
            Some(")".to_string()),
            Some(delimiter_metadata(")")),
        ),
        FunctionBodyKind::CommandSequence | FunctionBodyKind::CompoundCommand => {
            (None, None, None, None)
        }
    };

    Box::new(FunctionCommand {
        name_metadata: build_word_metadata(0, &name, &name_raw),
        name,
        body,
        keyword,
        keyword_text: keyword.then(|| "function".to_string()),
        keyword_metadata,
        has_parentheses,
        open_paren: has_parentheses.then(|| "(".to_string()),
        open_paren_metadata,
        close_paren: has_parentheses.then(|| ")".to_string()),
        close_paren_metadata,
        body_kind,
        body_open_delimiter,
        body_open_delimiter_metadata,
        body_close_delimiter,
        body_close_delimiter_metadata,
        body_start,
        body_end,
        body_end_line,
        body_open_line,
        unparsed_body_source: None,
    })
}

fn build_token_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}

fn parse_function_command_sequence_body(
    tokens: &[Token],
    start: usize,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(Vec<CommandNode>, usize)> {
    let end = match tokens.get(start)?.value.as_str() {
        "[[" => matching_function_conditional_end(tokens, start)?,
        "if" => matching_function_if_end(tokens, start)?,
        "while" | "until" => matching_function_loop_end(tokens, start)?,
        _ => return None,
    };
    Some((
        parse_function_body(&tokens[start..=end], diagnostic_text, source_line_offset),
        end + 1,
    ))
}

fn matching_function_conditional_end(tokens: &[Token], start: usize) -> Option<usize> {
    (start..tokens.len()).find(|&index| tokens[index].raw == "]]")
}

fn matching_function_if_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..tokens.len() {
        let boundary = index == start || command_boundary_keyword_allowed(tokens, index);
        if boundary && is_keyword(tokens, index, "if") {
            depth += 1;
        } else if boundary && is_keyword(tokens, index, "fi") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn matching_function_loop_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..tokens.len() {
        let boundary = index == start || command_boundary_keyword_allowed(tokens, index);
        if boundary
            && (is_keyword(tokens, index, "for")
                || is_keyword(tokens, index, "while")
                || is_keyword(tokens, index, "until")
                || is_keyword(tokens, index, "select"))
        {
            depth += 1;
        } else if boundary && is_keyword(tokens, index, "done") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn parse_function_compound_body(
    tokens: &[Token],
    start: usize,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(CommandNode, usize)> {
    if let Some(parsed) = parse_arithmetic_command(tokens, start) {
        return Some(parsed);
    }

    match tokens.get(start)?.value.as_str() {
        // No "time" arm: a function body is a compound command
        // (parse.y function_body), and `time' opens a PIPELINE, not a
        // compound — GNU rejects every `f() time ...' at the `time'
        // itself, whatever follows (`time echo', `time { ...; }',
        // `time if ...' all name `time'). Returning None lets the
        // caller's offender arm name it (rubash#2747).
        "time" => None,
        "for" => parse_for_command(tokens, start, None, 0),
        "if" => parse_if_command_standalone(tokens, start, None, 0),
        "while" | "until" => parse_loop_command(tokens, start, None, 0)
            .map(|(command, next, _unclaimed)| (command, next)),
        "case" => parse_case_command(tokens, start, None, 0),
        "select" => parse_select_command(tokens, start, None, 0),
        "coproc" => parse_coproc_command(tokens, start),
        "[[" => parse_conditional_command(tokens, start, diagnostic_text, source_line_offset),
        _ => None,
    }
}

pub(super) fn parse_parenthesized_function_body(
    tokens: &[Token],
    start: usize,
    diagnostic_text: Option<&std::rc::Rc<str>>,
    source_line_offset: usize,
) -> Option<(Vec<CommandNode>, usize)> {
    if !is_keyword(tokens, start, "(") {
        return None;
    }

    let mut depth = 1usize;
    let mut case_depth = 0usize;
    let mut i = start + 1;
    while i < tokens.len() {
        let boundary = i == start + 1 || command_boundary_keyword_allowed(tokens, i);
        if boundary && is_keyword(tokens, i, "case") {
            case_depth += 1;
        } else if (boundary || follows_case_in_keyword(tokens, i)) && is_case_end_keyword(tokens, i)
        {
            // `esac` right after the case's `in` closes the empty case
            // (parse.y:3428-3441 special_case_tokens) so `f() ( case x in
            // esac )` ends at its own `)` (rubash#336).
            case_depth = case_depth.saturating_sub(1);
        } else if case_depth == 0 && is_keyword(tokens, i, "(") {
            depth += 1;
        } else if case_depth == 0 && is_keyword(tokens, i, ")") {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        i += 1;
    }
    if i >= tokens.len() {
        return None;
    }

    // GNU parse.y:1097: a subshell function body is `'(' compound_list
    // ')'`, and compound_list must contain a command — `f() ( )` is a
    // syntax error near `)'`, exactly like the bare `( )` handled in
    // parse_subshell_command (rubash#221). Without this the empty body
    // defined the function silently (rubash#1310). The error node rides
    // as the body so finish_function_command's find_body_parse_error
    // surfaces it in place of the definition.
    if let Some(error) = empty_compound_body_error_node(tokens, start + 1, i) {
        return Some((vec![error], i));
    }

    let mut body = parse_function_body(&tokens[start + 1..i], diagnostic_text, source_line_offset);
    if let Some(first) = body.first_mut() {
        first.subshell = true;
        // GNU execute_cmd.c:696 (SET_LINE_NUMBER(command->value.Subshell->line)):
        // a body subshell's own line is its CLOSING `)` — the whole body
        // runs under it as the ambient line_number, and the parent-side ERR
        // adjudication after wait_for (execute_cmd.c:769) reports it as
        // $LINENO. The flat `f() ( list )` region has no subshell node to
        // carry that line, so the opener command records the close line as
        // its end_line (rubash#720 form 2).
        if first.end_line.is_none() {
            first.end_line = tokens.get(i).map(|token| token.position);
        }
    }
    if let Some(last) = body.last_mut() {
        last.subshell_end = true;
    }
    Some((body, i))
}

/// Reassemble a `!!' function name from the two Keyword `!' tokens that
/// rubash's lexer emits (GNU reads `!!' as a single WORD, so the function_def
/// grammar accepts `!! () { ...; }'; the executor rejects the name under
/// POSIX mode via err_invalidid). Returns (name, raw, next_token_index).
fn bang_group_function_name(tokens: &[Token], start: usize) -> Option<(String, String, usize)> {
    if !tokens
        .get(start)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "!")
        || !tokens
            .get(start + 1)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "!")
    {
        return None;
    }
    let compact = tokens
        .get(start + 2)
        .is_some_and(|token| is_paren_operator(token, "()"));
    let separated = tokens
        .get(start + 2)
        .is_some_and(|token| is_paren_operator(token, "("))
        && tokens
            .get(start + 3)
            .is_some_and(|token| is_paren_operator(token, ")"));
    if !compact && !separated {
        return None;
    }
    Some(("!!".to_string(), "!!".to_string(), start + 2))
}

/// Reassemble a `<(...)' function name from the lexer's split tokens.
/// GNU's lexer reads `<(:)' as one WORD while scanning a word, so the
/// function_def grammar accepts `<(:) () { ...; }' as a definition whose
/// name is `<(:)' (later rejected by valid_function_word). rubash's lexer
/// emits `<` (RedirectIn) followed by a `( ... )` group, so when the group's
/// closing `)' is directly followed by `()', treat the whole `<(...)' as the
/// function name. Returns (name, raw, next_token_index).
fn lt_group_function_name(tokens: &[Token], start: usize) -> Option<(String, String, usize)> {
    if tokens.get(start)?.kind != TokenKind::RedirectIn
        || tokens.get(start)?.value != "<"
        || !tokens
            .get(start + 1)
            .is_some_and(|token| is_paren_operator(token, "("))
    {
        return None;
    }
    let mut depth = 0usize;
    let mut index = start + 1;
    while index < tokens.len() {
        if is_paren_operator(&tokens[index], "(") {
            depth += 1;
        } else if is_paren_operator(&tokens[index], ")") {
            if depth == 1 {
                if tokens
                    .get(index + 1)
                    .is_some_and(|token| is_paren_operator(token, "("))
                    && tokens
                        .get(index + 2)
                        .is_some_and(|token| is_paren_operator(token, ")"))
                {
                    let name = tokens[start..=index]
                        .iter()
                        .map(|token| token.value.as_str())
                        .collect::<String>();
                    let name_raw = tokens[start..=index]
                        .iter()
                        .map(|token| token.raw.as_str())
                        .collect::<String>();
                    return Some((name, name_raw, index + 1));
                }
                return None;
            }
            depth -= 1;
        }
        index += 1;
    }
    None
}
