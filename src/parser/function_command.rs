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
    let (name_index, mut i) = if keyword_form {
        (start + 1, start + 2)
    } else {
        (start, start + 1)
    };
    let name_token = tokens.get(name_index)?;
    let name = name_token.value.clone();
    let name_raw = name_token.raw.clone();
    if !(is_function_name(&name) || (keyword_form && is_function_keyword_name(&name))) {
        return None;
    }

    let has_parentheses = tokens.get(i).is_some_and(|token| {
        token.value == "(" && tokens.get(i + 1).is_some_and(|next| next.value == ")")
    });
    let keyword_metadata = keyword_form.then(|| build_token_metadata(&tokens[start]));
    let (open_paren_metadata, close_paren_metadata) = if has_parentheses {
        (
            Some(build_token_metadata(tokens.get(i)?)),
            Some(build_token_metadata(tokens.get(i + 1)?)),
        )
    } else {
        (None, None)
    };
    if has_parentheses {
        if tokens.get(i + 1)?.value != ")" {
            return None;
        }
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
        .map(|token| token.value.as_str())
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
        ));
        return Some(finish_function_command(command, tokens, body_end));
    }

    if tokens.get(i)?.value != "{" {
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
    let i = matching_brace_group_end(tokens, open_brace)?;

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
