use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_arithmetic_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    let first = tokens.get(start)?.value.as_str();

    if let Some(inner) = first
        .strip_prefix("((")
        .and_then(|value| value.strip_suffix("))"))
    {
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        set_arithmetic_command_words(&mut command, inner.to_string());
        return Some(finish_arithmetic_command(command, tokens, start + 1));
    }

    let mut i;
    let mut parts = Vec::new();
    let mut paren_depth = 0usize;
    if first == "((" {
        i = start + 1;
    } else if is_keyword(tokens, start, "(") && is_keyword(tokens, start + 1, "(") {
        i = start + 2;
    } else {
        return None;
    }

    while i + 1 < tokens.len() {
        if paren_depth == 0 && tokens[i].value == "))" {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            set_arithmetic_command_words(&mut command, parts.join(" "));
            return Some(finish_arithmetic_command(command, tokens, i + 1));
        }

        if paren_depth == 0 && is_keyword(tokens, i, ")") && is_keyword(tokens, i + 1, ")") {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            set_arithmetic_command_words(&mut command, parts.join(" "));
            return Some(finish_arithmetic_command(command, tokens, i + 2));
        }

        if is_keyword(tokens, i, "(") {
            paren_depth += 1;
            parts.push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, ")") && paren_depth > 0 {
            paren_depth -= 1;
            parts.push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if let Some(combined) = arithmetic_combined_operator(&tokens[i], tokens.get(i + 1)) {
            parts.push(combined);
            i += 2;
            continue;
        }

        if tokens[i].kind == TokenKind::Semicolon {
            i += 1;
            continue;
        }

        parts.push(tokens[i].value.clone());
        i += 1;
    }

    None
}

fn set_arithmetic_command_words(command: &mut CommandNode, expression: String) {
    command.words.push("((".to_string());
    command.words.push(expression.clone());
    command.words.push("))".to_string());
    command.arithmetic_command = Some(ArithmeticCommand { expression });
}

pub(super) fn finish_arithmetic_command(
    mut command: CommandNode,
    tokens: &[Token],
    index: usize,
) -> (CommandNode, usize) {
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::And) => {
            command.and_or = Some(true);
            (command, index + 1)
        }
        Some(TokenKind::Or) => {
            command.and_or = Some(false);
            (command, index + 1)
        }
        Some(TokenKind::Background) => {
            command.background = true;
            (command, index + 1)
        }
        Some(TokenKind::Semicolon) => (command, index + 1),
        _ => (command, index),
    }
}

pub(super) fn finish_compound_command(
    mut command: CommandNode,
    tokens: &[Token],
    mut index: usize,
) -> (CommandNode, usize) {
    collect_trailing_redirections(tokens, &mut index, &mut command);
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::And) => {
            command.and_or = Some(true);
            (command, index + 1)
        }
        Some(TokenKind::Or) => {
            command.and_or = Some(false);
            (command, index + 1)
        }
        Some(TokenKind::Background) => {
            command.background = true;
            (command, index + 1)
        }
        Some(TokenKind::Semicolon) => (command, index + 1),
        _ => (command, index),
    }
}

pub(super) fn arithmetic_combined_operator(token: &Token, next: Option<&Token>) -> Option<String> {
    let op = token.value.as_str();
    if !matches!(op, ">" | "<" | "!" | "&" | "|" | "<<" | ">>") {
        return None;
    }

    let next = next?;
    if next.value == "=" {
        return Some(format!("{op}="));
    }

    next.value
        .strip_prefix('=')
        .map(|rhs| format!("{op}={rhs}"))
}
