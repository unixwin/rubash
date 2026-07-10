use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_select_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // Parse `select name [in words ...]; do body; done`
    let variable = tokens.get(start + 1)?.value.clone();
    if !matches!(
        tokens.get(start + 1)?.kind,
        TokenKind::Word | TokenKind::Variable
    ) {
        return None;
    }

    let mut i = start + 2;
    let mut words = Vec::new();

    // Optional `in words...`
    let default_positional = if is_keyword(tokens, i, "in") {
        i += 1;
        while i < tokens.len() && !is_keyword(tokens, i, "do") {
            if tokens[i].kind == TokenKind::Semicolon {
                i += 1;
                continue;
            }
            if matches!(
                tokens[i].kind,
                TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
            ) {
                words.push(tokens[i].value.clone());
            }
            i += 1;
        }
        false
    } else {
        // Skip optional semicolons before `do`
        while tokens
            .get(i)
            .is_some_and(|token| token.kind == TokenKind::Semicolon)
        {
            i += 1;
        }
        true
    };

    if !is_keyword(tokens, i, "do") {
        return None;
    }
    i += 1;

    // Find matching `done`
    let body_start = i;
    let mut depth = 0usize;
    while i < tokens.len() {
        if is_keyword(tokens, i, "for")
            || is_keyword(tokens, i, "while")
            || is_keyword(tokens, i, "until")
            || is_keyword(tokens, i, "select")
        {
            depth += 1;
        } else if is_keyword(tokens, i, "done") {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        i += 1;
    }

    if !is_keyword(tokens, i, "done") {
        return None;
    }

    let body = parse(&tokens[body_start..i])
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect();
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.select_command = Some(SelectCommand {
        variable,
        words,
        default_positional,
        body,
    });
    let mut next_i = i + 1;
    collect_trailing_redirections(tokens, &mut next_i, &mut command);
    Some((command, next_i))
}
