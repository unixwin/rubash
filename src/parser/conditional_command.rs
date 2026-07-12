use super::*;
use crate::lexer::Token;

pub(super) fn parse_conditional_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    if tokens.get(start)?.value != "[[" {
        return None;
    }

    let end = matching_conditional_end(tokens, start)?;
    let args = tokens[start + 1..=end]
        .iter()
        .map(|token| token.value.clone())
        .collect::<Vec<_>>();

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.words.push("[[".to_string());
    command.words.extend(args.clone());
    command.conditional_command = Some(ConditionalCommand { args });

    Some(finish_compound_command(command, tokens, end + 1))
}

fn matching_conditional_end(tokens: &[Token], start: usize) -> Option<usize> {
    (start + 1..tokens.len()).find(|&index| tokens[index].value == "]]")
}
