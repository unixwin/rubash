use super::*;
use crate::lexer::Token;

pub(super) fn parse_if_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    if !is_keyword(tokens, start, "if") {
        return None;
    }

    let then_index = find_if_then(tokens, start + 1)?;
    let then_keyword = tokens[then_index].value.clone();
    let condition = parse_if_body_commands(&tokens[start + 1..then_index]);
    let condition_terminator = condition_terminator_before(tokens, then_index);
    let mut index = then_index + 1;

    let (then_body, boundary) = parse_if_section(tokens, index)?;
    index = boundary;

    let mut elif_branches = Vec::new();
    while is_keyword(tokens, index, "elif") {
        let elif_keyword = tokens[index].value.clone();
        let elif_then = find_if_then(tokens, index + 1)?;
        let elif_then_keyword = tokens[elif_then].value.clone();
        let condition = parse_if_body_commands(&tokens[index + 1..elif_then]);
        let condition_terminator = condition_terminator_before(tokens, elif_then);
        let (body, next_boundary) = parse_if_section(tokens, elif_then + 1)?;
        elif_branches.push(ElifBranch {
            keyword: elif_keyword,
            condition,
            condition_terminator,
            then_keyword: elif_then_keyword,
            body,
        });
        index = next_boundary;
    }

    let (else_keyword, else_body) = if is_keyword(tokens, index, "else") {
        let else_keyword = tokens[index].value.clone();
        let (body, next_boundary) = parse_if_section(tokens, index + 1)?;
        index = next_boundary;
        (Some(else_keyword), Some(body))
    } else {
        (None, None)
    };

    if !is_keyword(tokens, index, "fi") {
        return None;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.if_command = Some(IfCommand {
        keyword: tokens[start].value.clone(),
        condition,
        condition_terminator,
        then_keyword,
        then_body,
        elif_branches,
        else_keyword,
        else_body,
        end_keyword: tokens[index].value.clone(),
    });

    Some(finish_compound_command(command, tokens, index + 1))
}

fn condition_terminator_before(tokens: &[Token], then_index: usize) -> Option<String> {
    tokens
        .get(then_index.saturating_sub(1))
        .filter(|token| token.kind == crate::lexer::TokenKind::Semicolon)
        .map(|token| token.value.clone())
}

fn find_if_then(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = start;
    while index < tokens.len() {
        if opens_compound_body(tokens, index) {
            depth += 1;
        } else if is_keyword(tokens, index, "then") {
            if depth == 0 {
                return Some(index);
            }
        } else if closes_compound_body(tokens, index) {
            depth = depth.saturating_sub(1);
        }
        index += 1;
    }
    None
}

fn parse_if_section(tokens: &[Token], start: usize) -> Option<(Vec<CommandNode>, usize)> {
    let mut depth = 0usize;
    let mut index = start;
    while index < tokens.len() {
        if opens_compound_body(tokens, index) {
            depth += 1;
        } else if closes_compound_body(tokens, index) {
            if depth == 0 {
                break;
            }
            depth -= 1;
        } else if depth == 0 && is_keyword(tokens, index, "then") {
            return None;
        } else if depth == 0
            && (is_keyword(tokens, index, "elif") || is_keyword(tokens, index, "else"))
        {
            break;
        }
        index += 1;
    }

    tokens.get(index)?;
    Some((parse_if_body_commands(&tokens[start..index]), index))
}

fn parse_if_body_commands(tokens: &[Token]) -> Vec<CommandNode> {
    parse(tokens)
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}

fn opens_compound_body(tokens: &[Token], index: usize) -> bool {
    matches!(
        tokens.get(index).map(|token| token.value.as_str()),
        Some("if" | "for" | "select" | "while" | "until" | "case")
    )
}

fn closes_compound_body(tokens: &[Token], index: usize) -> bool {
    matches!(
        tokens.get(index).map(|token| token.value.as_str()),
        Some("fi" | "done" | "esac")
    )
}
