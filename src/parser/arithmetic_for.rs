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

    let (body, body_end, body_kind) =
        if let Some((body, next_i)) = parse_arithmetic_for_brace_body(tokens, i) {
            (body, next_i, CommandBodyKind::BraceGroup)
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
                parse_arithmetic_for_body_commands(&tokens[body_start..i]),
                i + 1,
                CommandBodyKind::DoDone,
            )
        };
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
        body_kind,
        body,
    });
    let mut next_i = body_end;
    collect_trailing_redirections(tokens, &mut next_i, &mut command);
    Some((command, next_i))
}

fn parse_arithmetic_for_body_commands(tokens: &[Token]) -> Vec<CommandNode> {
    parse(tokens)
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}

fn parse_arithmetic_for_brace_body(
    tokens: &[Token],
    index: usize,
) -> Option<(Vec<CommandNode>, usize)> {
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
        return Some((parse_arithmetic_for_body_commands(&body_tokens), index + 1));
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

    Some((
        parse_arithmetic_for_body_commands(&tokens[index + 1..i]),
        i + 1,
    ))
}
