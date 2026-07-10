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

        if !is_brace && !is_subshell {
            // This might be a name. Check if the token *after* it is a brace group or subshell.
            let next_after = tokens.get(i + 1);
            let next_is_compound = next_after.is_some_and(|t| {
                is_brace_group_token(t)
                    || (t.kind == TokenKind::Keyword && (t.value == "{" || t.value == "("))
            });
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
            command.coproc_command = Some(CoprocCommand {
                name,
                words: Vec::new(),
                body: Some(body),
            });
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
            command.coproc_command = Some(CoprocCommand {
                name,
                words: Vec::new(),
                body: Some(body),
            });
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
                command.coproc_command = Some(CoprocCommand {
                    name,
                    words: Vec::new(),
                    body: Some(body),
                });
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
    command.coproc_command = Some(CoprocCommand {
        name,
        words,
        body: None,
    });
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
