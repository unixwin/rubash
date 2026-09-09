use super::parse_loop::{parse_time_prefixed_compound_command, parse_time_prefixed_shell_command};
use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_function_command(
    tokens: &[Token],
    start: usize,
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
        let body_tokens = crate::lexer::tokenize(inner);
        let mut body = parse(&body_tokens).commands;
        if let Some(line) = tokens.get(start).map(|token| token.position) {
            set_body_line(&mut body, line);
        }
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
            FunctionBodyKind::BraceGroup,
            Some(i),
            Some(i),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
        ));
        return Some(finish_function_command(command, tokens, i + 1));
    }
    if let Some((mut body_command, body_end)) = parse_function_compound_body(tokens, i) {
        if let Some(line) = tokens.get(start).map(|token| token.position) {
            body_command.line = Some(line);
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
            tokens.get(body_end.saturating_sub(1)).map(|token| token.position),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
        ));
        return Some(finish_function_command(command, tokens, body_end));
    }

    if tokens.get(i).is_some_and(|token| token.value == "(") {
        let (mut body, close_i) = parse_parenthesized_function_body(tokens, i)?;
        if let Some(line) = tokens.get(start).map(|token| token.position) {
            set_body_line(&mut body, line);
        }
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

    if let Some((mut body, body_end)) = parse_function_command_sequence_body(tokens, i) {
        if let Some(line) = tokens.get(start).map(|token| token.position) {
            set_body_line(&mut body, line);
        }
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
            tokens.get(body_end.saturating_sub(1)).map(|token| token.position),
            tokens
                .get(i)
                .map(|token| token.position + token.raw.matches('\n').count()),
        ));
        return Some(finish_function_command(command, tokens, body_end));
    }

    if tokens.get(i)?.value.trim() != "{" {
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
    let i = matching_brace_group_end(tokens, open_brace).or_else(|| {
        // A heredoc body is a complete lexical token, but older boundary
        // paths can omit the separator before the function closing brace.
        // Retain the final brace as the body terminator for serialized
        // functions when no nested delimiter was recognized.
        (open_brace + 1..tokens.len())
            .rev()
            .find(|&index| is_keyword(tokens, index, "}"))
    })?;

    let body = parse(&tokens[body_start..i]).commands;
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
    let (command, mut next_i) = finish_compound_command(command, tokens, index);
    while tokens
        .get(next_i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        next_i += 1;
    }
    (command, next_i)
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
) -> Option<(Vec<CommandNode>, usize)> {
    let end = match tokens.get(start)?.value.as_str() {
        "[[" => matching_function_conditional_end(tokens, start)?,
        "if" => matching_function_if_end(tokens, start)?,
        "while" | "until" => matching_function_loop_end(tokens, start)?,
        _ => return None,
    };
    Some((parse(&tokens[start..=end]).commands, end + 1))
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
        "for" => parse_for_command(tokens, start),
        "if" => parse_if_command(tokens, start),
        "while" | "until" => parse_loop_command(tokens, start),
        "case" => parse_case_command(tokens, start),
        "select" => parse_select_command(tokens, start),
        "coproc" => parse_coproc_command(tokens, start),
        "[[" => parse_conditional_command(tokens, start),
        _ => None,
    }
}

pub(super) fn parse_parenthesized_function_body(
    tokens: &[Token],
    start: usize,
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

    let mut body = parse(&tokens[start + 1..i]).commands;
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
    if !tokens.get(start).is_some_and(|token| {
        token.kind == TokenKind::Keyword && token.value == "!"
    }) || !tokens.get(start + 1).is_some_and(|token| {
        token.kind == TokenKind::Keyword && token.value == "!"
    }) {
        return None;
    }
    let compact = tokens
        .get(start + 2)
        .is_some_and(|token| token.value == "()");
    let separated = tokens.get(start + 2).is_some_and(|token| token.value == "(")
        && tokens.get(start + 3).is_some_and(|token| token.value == ")");
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
        || !tokens.get(start + 1).is_some_and(|token| token.value == "(")
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
                if tokens.get(index + 1).is_some_and(|token| token.value == "(")
                    && tokens.get(index + 2).is_some_and(|token| token.value == ")")
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
