use super::{command_boundary_keyword_allowed, is_case_end_keyword, parse, ProcessSubstitution};
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

pub(super) fn any_process_substitution_word_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    process_substitution_word_target(tokens, redirect_index)
        .or_else(|| output_process_substitution_word_target(tokens, redirect_index))
}

pub(super) fn process_substitutions_in_word(word: &str) -> Vec<ProcessSubstitution> {
    let tokens = crate::lexer::tokenize(word);
    let mut substitutions = Vec::new();
    let mut index = 0usize;

    while index < tokens.len() {
        if let Some((substitution, next_index)) =
            any_process_substitution_word_target(&tokens, index)
        {
            substitutions.push(substitution);
            index = next_index + 1;
        } else {
            index += 1;
        }
    }

    substitutions
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

pub(super) fn combined_process_substitution_redirect_target(
    tokens: &[Token],
    redirect_index: usize,
) -> Option<(ProcessSubstitution, usize)> {
    let redirect = tokens.get(redirect_index)?;
    if !matches!(redirect.kind, TokenKind::RedirectOut | TokenKind::Append)
        || !matches!(redirect.value.as_str(), "&>" | "&>>")
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
        let boundary = index == source_start || command_boundary_keyword_allowed(tokens, index);
        if boundary && tokens[index].kind == TokenKind::Keyword && tokens[index].value == "case" {
            case_depth += 1;
        } else if boundary && is_case_end_keyword(tokens, index) {
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

    let source = process_substitution_source(&tokens[source_start..index]);
    let operator = if output { ">" } else { "<" };
    let prefix = format!("{operator}(");
    let commands = parse(&crate::lexer::tokenize(&source)).commands;
    Some((
        ProcessSubstitution {
            target: format!("{prefix}{source})"),
            open_delimiter_metadata: delimiter_metadata(&prefix),
            open_delimiter: prefix,
            operator: operator.to_string(),
            operator_metadata: delimiter_metadata(operator),
            source,
            close_delimiter_metadata: delimiter_metadata(")"),
            close_delimiter: ")".to_string(),
            commands,
            output,
            word_index: None,
            redirect_fd: None,
        },
        index,
    ))
}

fn process_substitution_source(tokens: &[Token]) -> String {
    let mut source = String::new();
    let mut pending_heredoc_delimiter: Option<String> = None;
    let mut skip_next_semicolon = false;

    for (index, token) in tokens.iter().enumerate() {
        if skip_next_semicolon && token.kind == TokenKind::Semicolon {
            skip_next_semicolon = false;
            continue;
        }
        skip_next_semicolon = false;

        if token.kind == TokenKind::HereDocBody {
            if !source.ends_with('\n') {
                source.push('\n');
            }
            source.push_str(token.value.trim_start_matches(['\x1e', '\x1f']));
            if !source.ends_with('\n') {
                source.push('\n');
            }
            if let Some(delimiter) = pending_heredoc_delimiter.take() {
                source.push_str(&delimiter);
                source.push('\n');
            }
            skip_next_semicolon = true;
            continue;
        }

        if !source.is_empty() && !source.ends_with('\n') {
            source.push(' ');
        }
        source.push_str(&token.raw);

        if token.kind == TokenKind::HereDoc {
            pending_heredoc_delimiter = tokens.get(index + 1).map(|token| token.value.clone());
        }
    }

    source
}

fn delimiter_metadata(delimiter: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::new(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}
