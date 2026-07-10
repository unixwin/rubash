use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_for_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports all `for_command`
    // grammar alternatives, nested compound lists, redirections on compound
    // commands and reserved-word parsing state. This maps common
    // `for name [in words]; do body; done` forms.
    if let Some((command, next_i)) = parse_arithmetic_for_command(tokens, start) {
        return Some((command, next_i));
    }

    let variable = tokens.get(start + 1)?.value.clone();
    if !matches!(
        tokens.get(start + 1)?.kind,
        TokenKind::Word | TokenKind::Variable
    ) {
        return None;
    }

    let mut i = start + 2;
    let mut words = Vec::new();
    let default_positional = if is_keyword(tokens, i, "in") {
        i += 1;
        while i < tokens.len() && !is_keyword(tokens, i, "do") {
            if tokens[i].kind == TokenKind::Semicolon {
                i += 1;
                while tokens
                    .get(i)
                    .is_some_and(|token| token.kind == TokenKind::Semicolon)
                {
                    i += 1;
                }
                if for_brace_body_start(tokens, i) {
                    break;
                }
                continue;
            }
            if for_brace_body_start(tokens, i) {
                return None;
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
        while tokens
            .get(i)
            .is_some_and(|token| token.kind == TokenKind::Semicolon)
        {
            i += 1;
        }
        true
    };

    let (body, body_end) = if let Some((body, next_i)) = parse_for_brace_body(tokens, i) {
        (body, next_i)
    } else {
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

        (
            parse_for_body_commands(&tokens[body_start..i]),
            i + 1,
        )
    };
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.for_command = Some(ForCommand {
        variable,
        words,
        default_positional,
        arithmetic: None,
        body,
    });
    let mut next_i = body_end;
    collect_trailing_redirections(tokens, &mut next_i, &mut command);
    Some((command, next_i))
}

fn parse_for_body_commands(tokens: &[Token]) -> Vec<CommandNode> {
    parse(tokens)
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}

fn for_brace_body_start(tokens: &[Token], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| {
        (token.kind == TokenKind::Keyword
            && token.value.starts_with('{')
            && token.value.ends_with('}')
            && token.value.len() >= 2)
            || is_keyword(tokens, index, "{")
    })
}

fn parse_for_brace_body(tokens: &[Token], index: usize) -> Option<(Vec<CommandNode>, usize)> {
    let token = tokens.get(index)?;
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
        return Some((parse_for_body_commands(&body_tokens), index + 1));
    }

    if !is_keyword(tokens, index, "{") {
        return None;
    }

    let mut depth = 1usize;
    let mut i = index + 1;
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

    Some((parse_for_body_commands(&tokens[index + 1..i]), i + 1))
}
