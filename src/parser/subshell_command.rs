use super::*;
use crate::lexer::Token;

pub(super) fn parse_subshell_command(
    tokens: &[Token],
    start: usize,
    source: Option<&str>,
    source_line_offset: usize,
) -> Option<(CommandNode, usize)> {
    if !is_keyword(tokens, start, "(") {
        return None;
    }
    if is_keyword(tokens, start + 1, "(")
        && tokens[start + 1].column == tokens[start].column + tokens[start].raw.len()
        && dparen_lexically_arithmetic(tokens, start)
    {
        // GNU parse_dparen decided this is an arithmetic command, not a
        // nested subshell (the token after the balanced group is `)`).
        return None;
    }

    let close = matching_subshell_end(tokens, start)?;
    let body = super::parse_loop::parse_body_with_diagnostics(
        &tokens[start + 1..close],
        source,
        source_line_offset,
    );

    let mut command = CommandNode::new();
    // GNU make_cmd.c:784 sets temp->line = line_number, which is the
    // parser's current line when the `subshell: '(' compound_list ')'`
    // rule reduces — i.e. the line of the closing `)` token, not the
    // opening `(`. This matters for error reporting: execute_cmd.c
    // SET_LINE_NUMBER(command->value.Subshell->line) sets the executing
    // line to the `)` line, so diagnostics inside the subshell (e.g.
    // "break: is a special builtin" in func5.sub) report the `)` line.
    command.line = tokens.get(close).map(|token| token.position);
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
