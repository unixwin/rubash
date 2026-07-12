use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_coproc_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // Parse `coproc [NAME] command [args...]` or `coproc [NAME] { body; }` or `coproc [NAME] ( body )`
    let mut i = start + 1; // skip `coproc`

    // Determine if next token is a name followed by a compound command,
    // or if the next token is itself the command.
    let mut name: Option<String> = None;
    let lookahead = tokens.get(i);
    if let Some(lookahead) = lookahead {
        let is_brace = is_brace_group_token(lookahead) || is_keyword(tokens, i, "{");
        let is_subshell = is_keyword(tokens, i, "(");
        let is_compound = is_brace || is_subshell || is_coproc_shell_command_start(tokens, i);

        if !is_compound {
            // This might be a name. Check if the token *after* it is a brace group or subshell.
            let next_after = tokens.get(i + 1);
            let next_is_compound = next_after.is_some_and(|t| is_brace_group_token(t))
                || is_keyword(tokens, i + 1, "{")
                || is_keyword(tokens, i + 1, "(")
                || is_coproc_shell_command_start(tokens, i + 1);
            if next_is_compound {
                name = Some(lookahead.value.clone());
                i += 1; // consume the name
            }
            // Otherwise: no name, the token is part of the simple command
        }
    }

    // Parse the body
    if let Some(token) = tokens.get(i) {
        // Brace group body (single token from lexer)
        if is_brace_group_token(token) {
            let body = parse_inline_brace_body(token);
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                CoprocBodyKind::BraceGroup,
                Some(body),
            ));
            let mut next_i = i + 1;
            collect_trailing_redirections(tokens, &mut next_i, &mut command);
            while tokens
                .get(next_i)
                .is_some_and(|t| t.kind == TokenKind::Semicolon)
            {
                next_i += 1;
            }
            return Some((command, next_i));
        }

        if is_keyword(tokens, i, "{") {
            let (body, close_i) = parse_split_brace_body(tokens, i)?;
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                CoprocBodyKind::BraceGroup,
                Some(body),
            ));
            let mut next_i = close_i + 1;
            collect_trailing_redirections(tokens, &mut next_i, &mut command);
            while tokens
                .get(next_i)
                .is_some_and(|t| t.kind == TokenKind::Semicolon)
            {
                next_i += 1;
            }
            return Some((command, next_i));
        }

        // Subshell body: ( ... )
        if is_keyword(tokens, i, "(") {
            i += 1; // consume `(`
            let body_start = i;
            let mut depth = 1usize;
            while i < tokens.len() {
                if is_keyword(tokens, i, "(") {
                    depth += 1;
                } else if is_keyword(tokens, i, ")") {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                i += 1;
            }
            if i < tokens.len() {
                let body = parse(&tokens[body_start..i]).commands;
                let mut command = CommandNode::new();
                command.line = tokens.get(start).map(|t| t.position);
                command.coproc_command = Some(coproc_command(
                    name,
                    Vec::new(),
                    CoprocBodyKind::Subshell,
                    Some(body),
                ));
                let mut next_i = i + 1;
                collect_trailing_redirections(tokens, &mut next_i, &mut command);
                while tokens
                    .get(next_i)
                    .is_some_and(|t| t.kind == TokenKind::Semicolon)
                {
                    next_i += 1;
                }
                return Some((command, next_i));
            }
        }

        if let Some((body, body_end)) = parse_coproc_command_sequence_body(tokens, i) {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                CoprocBodyKind::CommandSequence,
                Some(body),
            ));
            let mut next_i = body_end;
            collect_trailing_redirections(tokens, &mut next_i, &mut command);
            while tokens
                .get(next_i)
                .is_some_and(|t| t.kind == TokenKind::Semicolon)
            {
                next_i += 1;
            }
            return Some((command, next_i));
        }

        if let Some((body_command, body_end)) = parse_coproc_compound_body(tokens, i) {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|t| t.position);
            command.coproc_command = Some(coproc_command(
                name,
                Vec::new(),
                CoprocBodyKind::CompoundCommand,
                Some(vec![body_command]),
            ));
            let mut next_i = body_end;
            collect_trailing_redirections(tokens, &mut next_i, &mut command);
            while tokens
                .get(next_i)
                .is_some_and(|t| t.kind == TokenKind::Semicolon)
            {
                next_i += 1;
            }
            return Some((command, next_i));
        }
    }

    // Simple command: collect remaining tokens as words
    let mut words = Vec::new();
    while i < tokens.len() {
        match tokens[i].kind {
            TokenKind::Word
            | TokenKind::Variable
            | TokenKind::BraceExpand
            | TokenKind::CommandSubst => {
                words.push(tokens[i].value.clone());
                i += 1;
            }
            _ => break,
        }
    }

    if words.is_empty() {
        return None;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|t| t.position);
    command.coproc_command = Some(coproc_command(
        name,
        words,
        CoprocBodyKind::SimpleCommand,
        None,
    ));
    let mut next_i = i;
    collect_trailing_redirections(tokens, &mut next_i, &mut command);
    while tokens
        .get(next_i)
        .is_some_and(|t| t.kind == TokenKind::Semicolon)
    {
        next_i += 1;
    }
    Some((command, next_i))
}

fn coproc_command(
    name: Option<String>,
    words: Vec<String>,
    body_kind: CoprocBodyKind,
    body: Option<Vec<CommandNode>>,
) -> Box<CoprocCommand> {
    Box::new(CoprocCommand {
        keyword: "coproc".to_string(),
        name,
        words,
        body_kind,
        body,
    })
}

fn is_coproc_shell_command_start(tokens: &[Token], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| {
        matches!(
            token.value.as_str(),
            "for" | "case" | "select" | "coproc" | "if" | "while" | "until" | "[["
        ) || token.value.starts_with("((")
    })
}

fn parse_coproc_command_sequence_body(
    tokens: &[Token],
    start: usize,
) -> Option<(Vec<CommandNode>, usize)> {
    let end = match tokens.get(start)?.value.as_str() {
        "[[" => matching_coproc_conditional_end(tokens, start)?,
        "if" => matching_coproc_if_end(tokens, start)?,
        "while" | "until" => matching_coproc_loop_end(tokens, start)?,
        _ => return None,
    };
    Some((parse(&tokens[start..=end]).commands, end + 1))
}

fn matching_coproc_conditional_end(tokens: &[Token], start: usize) -> Option<usize> {
    (start..tokens.len()).find(|&index| tokens[index].value == "]]")
}

fn matching_coproc_if_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..tokens.len() {
        if is_keyword(tokens, index, "if") {
            depth += 1;
        } else if is_keyword(tokens, index, "fi") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn matching_coproc_loop_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in start..tokens.len() {
        if is_keyword(tokens, index, "for")
            || is_keyword(tokens, index, "while")
            || is_keyword(tokens, index, "until")
            || is_keyword(tokens, index, "select")
        {
            depth += 1;
        } else if is_keyword(tokens, index, "done") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn parse_coproc_compound_body(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    match tokens.get(start)?.value.as_str() {
        "for" => parse_for_command(tokens, start),
        "case" => parse_case_command(tokens, start),
        "select" => parse_select_command(tokens, start),
        "coproc" => parse_coproc_command(tokens, start),
        _ if tokens[start].value.starts_with("((") => parse_arithmetic_command(tokens, start),
        _ => None,
    }
}

fn is_brace_group_token(token: &Token) -> bool {
    token.kind == TokenKind::BraceExpand
        || (token.kind == TokenKind::Keyword
            && token.value.starts_with('{')
            && token.value.ends_with('}')
            && token.value.len() > 1)
}

fn parse_inline_brace_body(token: &Token) -> Vec<CommandNode> {
    let inner = token
        .value
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    let body_tokens = crate::lexer::tokenize(inner);
    parse(&body_tokens).commands
}

fn parse_split_brace_body(tokens: &[Token], start: usize) -> Option<(Vec<CommandNode>, usize)> {
    if !is_keyword(tokens, start, "{") {
        return None;
    }

    let mut depth = 1usize;
    let mut i = start + 1;
    while i < tokens.len() {
        if is_keyword(tokens, i, "{") {
            depth += 1;
        } else if is_keyword(tokens, i, "}") {
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

    Some((parse(&tokens[start + 1..i]).commands, i))
}
