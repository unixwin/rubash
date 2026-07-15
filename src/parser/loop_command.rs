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
        keyword_metadata: build_loop_keyword_metadata(&tokens[start]),
        condition,
        condition_terminator,
        do_keyword: tokens[do_index].value.clone(),
        do_keyword_metadata: build_loop_keyword_metadata(&tokens[do_index]),
        body_open_delimiter: tokens[do_index].value.clone(),
        body_close_delimiter: tokens[done_index].value.clone(),
        body,
        end_keyword: tokens[done_index].value.clone(),
        end_keyword_metadata: build_loop_keyword_metadata(&tokens[done_index]),
        kind,
        until,
    });

    Some(finish_compound_command(command, tokens, done_index + 1))
}

fn find_loop_do(tokens: &[Token], start: usize) -> Option<usize> {
    let mut stack = Vec::new();
    let mut index = start;
    while index < tokens.len() {
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && is_keyword(tokens, index, "do")
        {
            return Some(index);
        }
        update_compound_boundary_stack(tokens, index, &mut stack);
        index += 1;
    }
    None
}

fn parse_loop_body(tokens: &[Token], start: usize) -> Option<(Vec<CommandNode>, usize)> {
    let mut stack = Vec::new();
    let mut index = start;
    while index < tokens.len() {
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && is_keyword(tokens, index, "done")
        {
            break;
        }
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && is_keyword(tokens, index, "do")
        {
            return None;
        }
        update_compound_boundary_stack(tokens, index, &mut stack);
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

fn build_loop_keyword_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}
