use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_brace_group_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    let token = tokens.get(start)?;
    if token.kind == TokenKind::Keyword
        && token.value.starts_with('{')
        && token.value.ends_with('}')
        && token.value.len() >= 2
    {
        let inner = token
            .value
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim();
        let body_tokens = crate::lexer::tokenize(inner);
        let mut command = CommandNode::new();
        command.line = Some(token.position);
        command.brace_group = Some(BraceGroupCommand {
            body: parse(&body_tokens).commands,
        });
        return Some(finish_compound_command(command, tokens, start + 1));
    }

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

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.brace_group = Some(BraceGroupCommand {
        body: parse(&tokens[start + 1..i]).commands,
    });
    Some(finish_compound_command(command, tokens, i + 1))
}
