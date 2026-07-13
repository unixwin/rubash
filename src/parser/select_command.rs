use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_select_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // Parse `select name [in words ...]; do body; done`
    let variable = tokens.get(start + 1)?.value.clone();
    if !matches!(
        tokens.get(start + 1)?.kind,
        TokenKind::Word | TokenKind::Variable
    ) {
        return None;
    }

    let mut i = start + 2;
    let mut words = Vec::new();
    let mut in_keyword = None;
    let mut list_terminator = None;

    // Optional `in words...`
    let default_positional = if is_keyword(tokens, i, "in") {
        in_keyword = Some(tokens[i].value.clone());
        i += 1;
        while i < tokens.len() && !is_keyword(tokens, i, "do") {
            if tokens[i].kind == TokenKind::Semicolon {
                if list_terminator.is_none() {
                    list_terminator = Some(tokens[i].value.clone());
                }
                i += 1;
                while tokens
                    .get(i)
                    .is_some_and(|token| token.kind == TokenKind::Semicolon)
                {
                    i += 1;
                }
                if select_brace_body_start(tokens, i) {
                    break;
                }
                continue;
            }
            if select_brace_body_start(tokens, i) {
                return None;
            }
            if let Some((word, next_i)) = collect_compound_word_value(tokens, i) {
                words.push(word);
                i = next_i;
                continue;
            }
            i += 1;
        }
        false
    } else {
        // Skip optional semicolons before `do`
        while tokens
            .get(i)
            .is_some_and(|token| token.kind == TokenKind::Semicolon)
        {
            if list_terminator.is_none() {
                list_terminator = Some(tokens[i].value.clone());
            }
            i += 1;
        }
        true
    };

    let (body, body_end, body_kind, do_keyword, end_keyword) =
        if let Some((body, next_i)) = parse_select_brace_body(tokens, i) {
            (body, next_i, CommandBodyKind::BraceGroup, None, None)
        } else {
            if !is_keyword(tokens, i, "do") {
                return None;
            }
            let do_keyword = Some(tokens[i].value.clone());
            i += 1;

            // Find matching `done`
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

            (
                parse_select_body_commands(&tokens[body_start..i]),
                i + 1,
                CommandBodyKind::DoDone,
                do_keyword,
                end_keyword,
            )
        };
    let (body_open_delimiter, body_close_delimiter) =
        command_body_delimiters(body_kind, do_keyword.as_deref(), end_keyword.as_deref());
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.select_command = Some(Box::new(SelectCommand {
        keyword: tokens[start].value.clone(),
        variable,
        in_keyword,
        words,
        default_positional,
        list_terminator,
        body_kind,
        body_open_delimiter,
        body_close_delimiter,
        do_keyword,
        end_keyword,
        body,
    }));
    Some(finish_compound_command(command, tokens, body_end))
}

fn parse_select_body_commands(tokens: &[Token]) -> Vec<CommandNode> {
    parse(tokens)
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}

fn select_brace_body_start(tokens: &[Token], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| {
        (token.kind == TokenKind::Keyword
            && token.value.starts_with('{')
            && token.value.ends_with('}')
            && token.value.len() >= 2)
            || is_keyword(tokens, index, "{")
    })
}

fn parse_select_brace_body(tokens: &[Token], index: usize) -> Option<(Vec<CommandNode>, usize)> {
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
        return Some((parse_select_body_commands(&body_tokens), index + 1));
    }

    if !is_keyword(tokens, index, "{") {
        return None;
    }

    let mut depth = 1usize;
    let mut i = index + 1;
    while i < tokens.len() {
        if is_boundary_keyword(tokens, i, "{") {
            depth += 1;
        } else if is_boundary_keyword(tokens, i, "}") {
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

    Some((parse_select_body_commands(&tokens[index + 1..i]), i + 1))
}
