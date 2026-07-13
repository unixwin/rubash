use super::*;
use crate::lexer::Token;

pub(super) fn parse_subshell_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    if !is_keyword(tokens, start, "(") || is_keyword(tokens, start + 1, "(") {
        return None;
    }

    let close = matching_subshell_end(tokens, start)?;
    let body = parse(&tokens[start + 1..close]).commands;

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.subshell_command = Some(SubshellCommand {
        open_delimiter: "(".to_string(),
        close_delimiter: ")".to_string(),
        body,
    });
    Some(finish_compound_command(command, tokens, close + 1))
}

fn matching_subshell_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut case_depth = 0usize;
    let mut index = start + 1;
    while index < tokens.len() {
        if is_keyword(tokens, index, "case") {
            case_depth += 1;
        } else if is_keyword(tokens, index, "esac") {
            case_depth = case_depth.saturating_sub(1);
        } else if case_depth == 0 && is_keyword(tokens, index, "(") {
            depth += 1;
        } else if case_depth == 0 && is_keyword(tokens, index, ")") {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}
