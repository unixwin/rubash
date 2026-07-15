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
    command.subshell_command = Some(Box::new(SubshellCommand {
        open_delimiter: "(".to_string(),
        open_delimiter_metadata: token_metadata(&tokens[start]),
        close_delimiter: ")".to_string(),
        close_delimiter_metadata: token_metadata(&tokens[close]),
        body,
    }));
    Some(finish_compound_command(command, tokens, close + 1))
}

fn token_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn matching_subshell_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut case_depth = 0usize;
    let mut index = start + 1;
    while index < tokens.len() {
        let boundary = index == start + 1 || command_boundary_keyword_allowed(tokens, index);
        if boundary && is_keyword(tokens, index, "case") {
            case_depth += 1;
        } else if boundary && is_case_end_keyword(tokens, index) {
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
