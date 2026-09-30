use super::parse_loop::{
    parse_time_prefixed_compound_command, parse_time_prefixed_shell_command,
    unclosed_brace_eof_node, unclosed_paren_eof_node,
};
use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_function_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    parse_function_command_with_diagnostic(tokens, start, None, 0)
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
    let compact_parentheses = tokens.get(i).is_some_and(|token| token.value == "()");
    let separated_parentheses = tokens.get(i).is_some_and(|token| {
        token.value == "(" && tokens.get(i + 1).is_some_and(|next| next.value == ")")
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

    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        i += 1;
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
        let mut body_tokens = crate::lexer::tokenize(inner);
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
    if let Some((mut body_command, body_end)) = parse_function_compound_body(tokens, i) {
        if body_command.line.is_none() {
            body_command.line = tokens.get(start).map(|token| token.position);
        }
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
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

    if tokens.get(i).is_some_and(|token| token.value == "(") {
        // GNU parse.y:6890-6901: `name() (` reaching EOF unclosed reports
        // "unexpected end of file from `(' command on line N" — do not let
        // the `?` fall back to a `(`-unexpected simple command.
        let Some((mut body, close_i)) =
            parse_parenthesized_function_body(tokens, i, diagnostic_text, source_line_offset)
        else {
            let command = unclosed_paren_eof_node(tokens, i);
            return Some((command, tokens.len()));
        };
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
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
            let command = unclosed_brace_eof_node(tokens, i);
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
        if keyword_form
            && matches!(
                body_token.kind,
                TokenKind::Word
                    | TokenKind::Variable
                    | TokenKind::Assignment
                    | TokenKind::CommandSubst
                    | TokenKind::BraceExpand
            )
        {
            let mut command = CommandNode::new();
            command.line = Some(body_token.position);
            command.insert_assignment(
                "__RUBASH_PARSE_ERROR__".to_string(),
                format!("unexpected token `{}'", body_token.value),
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
        return None;
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
        let command = unclosed_brace_eof_node(tokens, open_brace);
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
    command: CommandNode,
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

fn parse_function_compound_body(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    if let Some(parsed) = parse_arithmetic_command(tokens, start) {
        return Some(parsed);
    }

    match tokens.get(start)?.value.as_str() {
        "time" => parse_time_prefixed_shell_command(tokens, start)
            .or_else(|| parse_time_prefixed_compound_command(tokens, start)),
        "for" => parse_for_command(tokens, start, None, 0),
        "if" => parse_if_command_standalone(tokens, start, None, 0),
        "while" | "until" => parse_loop_command(tokens, start, None, 0),
        "case" => parse_case_command(tokens, start, None, 0),
        "select" => parse_select_command(tokens, start),
        "coproc" => parse_coproc_command(tokens, start),
        "[[" => parse_conditional_command(tokens, start),
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
        } else if boundary && is_case_end_keyword(tokens, i) {
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

    let mut body = parse_function_body(&tokens[start + 1..i], diagnostic_text, source_line_offset);
    if let Some(first) = body.first_mut() {
        first.subshell = true;
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
        .is_some_and(|token| token.value == "()");
    let separated = tokens
        .get(start + 2)
        .is_some_and(|token| token.value == "(")
        && tokens
            .get(start + 3)
            .is_some_and(|token| token.value == ")");
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
            .is_some_and(|token| token.value == "(")
    {
        return None;
    }
    let mut depth = 0usize;
    let mut index = start + 1;
    while index < tokens.len() {
        let value = tokens[index].value.as_str();
        if value == "(" {
            depth += 1;
        } else if value == ")" {
            if depth == 1 {
                if tokens
                    .get(index + 1)
                    .is_some_and(|token| token.value == "(")
                    && tokens
                        .get(index + 2)
                        .is_some_and(|token| token.value == ")")
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
