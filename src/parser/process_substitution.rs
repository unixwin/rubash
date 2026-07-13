use super::{parse, ProcessSubstitution};
use crate::lexer::{Token, TokenKind};

pub(super) fn process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
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
) -> Option<(ProcessSubstitution, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::RedirectIn
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_process_substitution_target(tokens, redirect_index + 2)
}

pub(super) fn output_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::RedirectOut
        || !tokens
            .get(redirect_index)?
            .value
            .strip_suffix('>')
            .is_some_and(|prefix| prefix.chars().all(|ch| ch.is_ascii_digit()))
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::RedirectOut && token.value == ">")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 3)
}

pub(super) fn append_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if tokens.get(redirect_index)?.kind != TokenKind::Append
        || !tokens
            .get(redirect_index)?
            .value
            .strip_suffix(">>")
            .is_some_and(|prefix| prefix.chars().all(|ch| ch.is_ascii_digit()))
        || !tokens
            .get(redirect_index + 1)
            .is_some_and(|token| token.kind == TokenKind::RedirectOut && token.value == ">")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 3)
}

pub(super) fn stderr_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    if !matches!(
        tokens.get(redirect_index)?.kind,
        TokenKind::RedirectErr | TokenKind::RedirectErrAppend
    ) || !tokens
        .get(redirect_index + 1)
        .is_some_and(|token| token.kind == TokenKind::RedirectOut && token.value == ">")
        || !tokens
            .get(redirect_index + 2)
            .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == "(")
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 3)
}

pub(super) fn output_process_substitution_word_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    let redirect = tokens.get(redirect_index)?;
    let open = tokens.get(redirect_index + 1)?;
    if redirect.kind != TokenKind::RedirectOut
        || redirect.value != ">"
        || open.kind != TokenKind::Keyword
        || open.value != "("
    {
        return None;
    }

    collect_output_process_substitution_target(tokens, redirect_index + 2)
}

pub(super) fn collect_process_substitution_target(
    tokens: &[Token],
    source_start: usize,
) -> Option<(ProcessSubstitution, usize)> {
    collect_process_substitution_target_with_prefix(tokens, source_start, false)
}

fn collect_output_process_substitution_target(
    tokens: &[Token],
    source_start: usize,
) -> Option<(ProcessSubstitution, usize)> {
    collect_process_substitution_target_with_prefix(tokens, source_start, true)
}

fn collect_process_substitution_target_with_prefix(
    tokens: &[Token],
    source_start: usize,
    output: bool,
) -> Option<(ProcessSubstitution, usize)> {
    let mut index = source_start;
    let source_start = index;
    let mut depth = 1usize;
    let mut case_depth = 0usize;
    while index < tokens.len() {
        if tokens[index].kind == TokenKind::Keyword && tokens[index].value == "case" {
            case_depth += 1;
        } else if tokens[index].kind == TokenKind::Keyword && tokens[index].value == "esac" {
            case_depth = case_depth.saturating_sub(1);
        } else if case_depth == 0
            && tokens[index].kind == TokenKind::Keyword
            && tokens[index].value == "("
        {
            depth += 1;
        } else if case_depth == 0
            && tokens[index].kind == TokenKind::Keyword
            && tokens[index].value == ")"
        {
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
        .map(|token| token.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let operator = if output { ">" } else { "<" };
    let prefix = format!("{operator}(");
    let commands = parse(&crate::lexer::tokenize(&source)).commands;
    Some((
        ProcessSubstitution {
            target: format!("{prefix}{source})"),
            open_delimiter: prefix,
            operator: operator.to_string(),
            source,
            close_delimiter: ")".to_string(),
            commands,
            output,
            word_index: None,
            redirect_fd: None,
        },
        index,
    ))
}
