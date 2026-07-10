use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_arithmetic_for_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    let mut i = if tokens.get(start + 1)?.value == "((" {
        start + 2
    } else if is_keyword(tokens, start + 1, "(") && is_keyword(tokens, start + 2, "(") {
        start + 3
    } else {
        return None;
    };

    let mut parts = vec![Vec::new(), Vec::new(), Vec::new()];
    let mut part_index = 0usize;
    let mut paren_depth = 0usize;
    while i + 1 < tokens.len() {
        if paren_depth == 0 && tokens[i].value == "))" {
            i += 1;
            break;
        }

        if paren_depth == 0 && is_keyword(tokens, i, ")") && is_keyword(tokens, i + 1, ")") {
            i += 2;
            break;
        }

        if paren_depth == 0 && tokens[i].kind == TokenKind::Semicolon {
            part_index += 1;
            if part_index > 2 {
                return None;
            }
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, "(") {
            paren_depth += 1;
            parts[part_index].push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, ")") && paren_depth > 0 {
            paren_depth -= 1;
            parts[part_index].push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if let Some(combined) = arithmetic_combined_operator(&tokens[i], tokens.get(i + 1)) {
            parts[part_index].push(combined);
            i += 2;
            continue;
        }

        parts[part_index].push(tokens[i].value.clone());
        i += 1;
    }

    if part_index != 2 {
        return None;
    }

    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        i += 1;
    }

    if !is_keyword(tokens, i, "do") {
        return None;
    }
    i += 1;

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
    command.for_command = Some(ForCommand {
        variable: String::new(),
        words: Vec::new(),
        default_positional: false,
        arithmetic: Some(ArithmeticForCommand {
            init: parts[0].join(" "),
            test: parts[1].join(" "),
            update: parts[2].join(" "),
        }),
        body,
    });
    let mut next_i = i + 1;
    collect_trailing_redirections(tokens, &mut next_i, &mut command);
    Some((command, next_i))
}
