use super::*;
use crate::lexer::Token;

pub(super) fn parse_loop_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    let (kind, until) = if is_keyword(tokens, start, "while") {
        (LoopKind::While, false)
    } else if is_keyword(tokens, start, "until") {
        (LoopKind::Until, true)
    } else {
        return None;
    };

    let do_index = find_loop_do(tokens, start + 1)?;
    let condition = parse_loop_body_commands(&tokens[start + 1..do_index]);
    let condition_terminator = tokens
        .get(do_index.saturating_sub(1))
        .filter(|token| token.kind == crate::lexer::TokenKind::Semicolon)
        .map(|token| token.value.clone());
    let (body, done_index) = parse_loop_body(tokens, do_index + 1)?;

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.loop_command = Some(LoopCommand {
        keyword: tokens[start].value.clone(),
        condition,
        condition_terminator,
        do_keyword: tokens[do_index].value.clone(),
        body_open_delimiter: tokens[do_index].value.clone(),
        body_close_delimiter: tokens[done_index].value.clone(),
        body,
        end_keyword: tokens[done_index].value.clone(),
        kind,
        until,
    });

    let mut next_i = done_index + 1;
    collect_trailing_redirections(tokens, &mut next_i, &mut command);
    Some((command, next_i))
}

fn find_loop_do(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = start;
    while index < tokens.len() {
        if opens_compound_body(tokens, index) {
            depth += 1;
        } else if is_keyword(tokens, index, "do") {
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

fn parse_loop_body(tokens: &[Token], start: usize) -> Option<(Vec<CommandNode>, usize)> {
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
        } else if depth == 0 && is_keyword(tokens, index, "do") {
            return None;
        }
        index += 1;
    }

    if !is_keyword(tokens, index, "done") {
        return None;
    }
    Some((parse_loop_body_commands(&tokens[start..index]), index))
}

fn parse_loop_body_commands(tokens: &[Token]) -> Vec<CommandNode> {
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
