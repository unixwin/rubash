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

        if paren_depth == 0 && tokens[i].value == ";;" {
            part_index += 2;
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

    let mut list_terminator = None;
    let mut list_terminator_metadata = None;
    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        if list_terminator.is_none() {
            list_terminator = Some(tokens[i].value.clone());
            list_terminator_metadata = Some(build_keyword_metadata(&tokens[i]));
        }
        i += 1;
    }

    let (
        body,
        body_end,
        body_kind,
        do_keyword,
        do_keyword_metadata,
        end_keyword,
        end_keyword_metadata,
    ) = if let Some((body, next_i)) = parse_arithmetic_for_brace_body(tokens, i) {
        (
            body,
            next_i,
            CommandBodyKind::BraceGroup,
            None,
            None,
            None,
            None,
        )
    } else {
        if !is_keyword(tokens, i, "do") {
            return None;
        }
        let do_keyword = Some(tokens[i].value.clone());
        let do_keyword_metadata = Some(build_keyword_metadata(&tokens[i]));
        i += 1;

        let body_start = i;
        let mut stack = Vec::new();
        while i < tokens.len() {
            if stack.is_empty()
                && command_boundary_keyword_allowed(tokens, i)
                && is_keyword(tokens, i, "done")
            {
                break;
            }
            update_compound_boundary_stack(tokens, i, &mut stack);
            i += 1;
        }

        if !is_keyword(tokens, i, "done") {
            return None;
        }
        let end_keyword = Some(tokens[i].value.clone());
        let end_keyword_metadata = Some(build_keyword_metadata(&tokens[i]));

        (
            parse_arithmetic_for_body_commands(&tokens[body_start..i]),
            i + 1,
            CommandBodyKind::DoDone,
            do_keyword,
            do_keyword_metadata,
            end_keyword,
            end_keyword_metadata,
        )
    };
    let (body_open_delimiter, body_close_delimiter) =
        command_body_delimiters(body_kind, do_keyword.as_deref(), end_keyword.as_deref());
    let init = parts[0].join(" ");
    let test = parts[1].join(" ");
    let update = parts[2].join(" ");

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.for_command = Some(ForCommand {
        keyword: tokens[start].value.clone(),
        keyword_metadata: build_keyword_metadata(&tokens[start]),
        variable: String::new(),
        variable_metadata: Box::new(build_word_metadata(0, "", "")),
        in_keyword: None,
        in_keyword_metadata: None,
        words: Vec::new(),
        word_metadata: Vec::new(),
        default_positional: false,
        list_terminator,
        list_terminator_metadata,
        arithmetic: Some(ArithmeticForCommand {
            open_delimiter: "((".to_string(),
            open_delimiter_metadata: delimiter_metadata("(("),
            init: init.clone(),
            init_metadata: ArithmeticExpressionMetadata::new(init),
            separators: vec![";".to_string(), ";".to_string()],
            separator_metadata: vec![separator_metadata(0, ";"), separator_metadata(1, ";")],
            test: test.clone(),
            test_metadata: ArithmeticExpressionMetadata::new(test),
            update: update.clone(),
            update_metadata: ArithmeticExpressionMetadata::new(update),
            close_delimiter: "))".to_string(),
            close_delimiter_metadata: delimiter_metadata("))"),
        }),
        body_kind,
        body_open_delimiter,
        body_close_delimiter,
        do_keyword,
        do_keyword_metadata,
        end_keyword,
        end_keyword_metadata,
        body,
    });
    Some(finish_compound_command(command, tokens, body_end))
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}

fn separator_metadata(index: usize, separator: &str) -> WordMetadata {
    build_word_metadata(index, separator, separator)
}

fn parse_arithmetic_for_body_commands(tokens: &[Token]) -> Vec<CommandNode> {
    parse(tokens)
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}

fn build_keyword_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
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

    let i = matching_brace_group_end(tokens, index)?;

    Some((
        parse_arithmetic_for_body_commands(&tokens[index + 1..i]),
        i + 1,
    ))
}
