use crate::lexer::{Token, TokenKind};

pub(super) fn process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(String, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::RedirectIn {
        return None;
    }

    let mut index = redirect_index + 1;
    if !tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::RedirectIn)
        || !tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }
    index += 2;

    collect_process_substitution_target(tokens, index)
}

pub(super) fn process_substitution_word_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(String, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::RedirectIn
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_process_substitution_target(tokens, redirect_index + 2)
}

pub(super) fn collect_process_substitution_target(
    tokens: &[Token],
    source_start: usize,
) -> Option<(String, usize)> {
    let mut index = source_start;
    let source_start = index;
    let mut depth = 1usize;
    while index < tokens.len() {
        if tokens[index].kind == TokenKind::Keyword && tokens[index].value == "(" {
            depth += 1;
        } else if tokens[index].kind == TokenKind::Keyword && tokens[index].value == ")" {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        index += 1;
    }
    if index >= tokens.len() {
        return None;
    }

    let source = tokens[source_start..index]
        .iter()
        .map(|token| token.value.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Some((format!("<({source})"), index))
}
