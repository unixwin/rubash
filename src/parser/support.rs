use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn note_command_line(cmd: &mut CommandNode, token: &Token) {
    if cmd.line.is_none() {
        cmd.line = Some(token.position);
    }
}

pub(super) fn push_command_word(cmd: &mut CommandNode, token: &Token) {
    record_command_substitutions_for_word(cmd, cmd.words.len(), &token.value);
    record_arithmetic_expansions_for_word(cmd, cmd.words.len(), &token.value);
    record_parameter_expansions_for_word(cmd, cmd.words.len(), &token.value);
    record_brace_expansions_for_word(cmd, cmd.words.len(), &token.value);
    record_extglob_patterns_for_word(cmd, cmd.words.len(), &token.value);
    record_tilde_expansions_for_word(cmd, cmd.words.len(), &token.value);
    record_pathname_patterns_for_word(cmd, cmd.words.len(), &token.value, &token.raw);
    record_word_quotes_for_word(cmd, cmd.words.len(), &token.raw);
    record_array_element_assignment_for_word(cmd, cmd.words.len(), &token.value);
    cmd.words.push(token.value.clone());
    cmd.word_kinds.push(token.kind.clone());
}

pub(super) fn collect_compound_word_value(
    tokens: &[Token],
    index: usize,
) -> Option<(String, usize)> {
    if let Some((process_substitution, next_i)) = process_substitution_word_target(tokens, index) {
        return Some((process_substitution.target, next_i + 1));
    }

    if let Some((process_substitution, next_i)) =
        output_process_substitution_word_target(tokens, index)
    {
        return Some((process_substitution.target, next_i + 1));
    }

    let token = tokens.get(index)?;
    if matches!(
        token.kind,
        TokenKind::Word
            | TokenKind::Variable
            | TokenKind::Assignment
            | TokenKind::CommandSubst
            | TokenKind::BraceExpand
    ) {
        return Some((token.value.clone(), index + 1));
    }

    None
}

pub(super) fn collect_compound_or_keyword_word_value(
    tokens: &[Token],
    index: usize,
) -> Option<(String, usize)> {
    collect_compound_word_value(tokens, index).or_else(|| {
        tokens
            .get(index)
            .filter(|token| token.kind == TokenKind::Keyword)
            .map(|token| (token.value.clone(), index + 1))
    })
}

pub(super) fn set_body_line(body: &mut [CommandNode], line: usize) {
    // TODO(parse.y): Bash preserves source locations through compound command
    // parsing. Rubash reparses inline function bodies from text today, so
    // recover the definition line for diagnostics such as readonly errors.
    for command in body {
        command.line = Some(line);
    }
}

pub(super) fn is_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == value)
}

pub(super) fn update_compound_boundary_stack(
    tokens: &[Token],
    index: usize,
    stack: &mut Vec<&'static str>,
) {
    if stack
        .last()
        .is_some_and(|expected| is_keyword(tokens, index, expected))
    {
        stack.pop();
        return;
    }

    if stack.last().is_some_and(|expected| *expected == "esac")
        || !command_boundary_keyword_allowed(tokens, index)
    {
        return;
    }

    if is_keyword(tokens, index, "if") {
        stack.push("fi");
    } else if matches!(
        tokens.get(index).map(|token| token.value.as_str()),
        Some("for" | "select" | "while" | "until")
    ) {
        stack.push("done");
    } else if is_keyword(tokens, index, "case") {
        stack.push("esac");
    }
}

pub(super) fn command_boundary_keyword_allowed(tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
        return true;
    };

    if previous.kind == TokenKind::Keyword
        && previous.value.starts_with('{')
        && previous.value.ends_with('}')
        && previous.value.len() >= 2
    {
        return true;
    }

    matches!(
        previous.kind,
        TokenKind::Semicolon
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Pipe
            | TokenKind::PipeErr
            | TokenKind::Background
    ) || (previous.kind == TokenKind::Keyword
        && matches!(
            previous.value.as_str(),
            "{" | "then" | "do" | "else" | "elif" | "}"
        ))
        || (previous.kind == TokenKind::Word
            && matches!(previous.raw.as_str(), ";;" | ";&" | ";;&"))
}

pub(super) fn is_boundary_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    command_boundary_keyword_allowed(tokens, index) && is_keyword(tokens, index, value)
}

pub(super) fn matching_brace_group_end(tokens: &[Token], start: usize) -> Option<usize> {
    if !is_keyword(tokens, start, "{") {
        return None;
    }

    let mut depth = 1usize;
    let mut stack = Vec::new();
    let mut index = start + 1;
    while index < tokens.len() {
        update_compound_boundary_stack(tokens, index, &mut stack);
        if !stack.is_empty() {
            index += 1;
            continue;
        }

        if is_boundary_keyword(tokens, index, "{") {
            depth += 1;
        } else if is_boundary_keyword(tokens, index, "}") {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

pub(super) fn command_is_empty(cmd: &CommandNode) -> bool {
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.compound_assignments.is_empty()
        && cmd.array_element_assignments.is_empty()
        && cmd.process_substitutions.is_empty()
        && cmd.command_substitutions.is_empty()
        && cmd.arithmetic_expansions.is_empty()
        && cmd.parameter_expansions.is_empty()
        && cmd.brace_expansions.is_empty()
        && cmd.extglob_patterns.is_empty()
        && cmd.tilde_expansions.is_empty()
        && cmd.pathname_patterns.is_empty()
        && cmd.word_quotes.is_empty()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.heredoc_redirects.is_empty()
        && cmd.here_string.is_none()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.pipe.is_none()
        && !cmd.background
        && cmd.and_or.is_none()
        && !cmd.inverted
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.time_command.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && cmd.for_command.is_none()
        && cmd.arithmetic_command.is_none()
        && cmd.if_command.is_none()
        && cmd.loop_command.is_none()
        && cmd.conditional_command.is_none()
        && cmd.subshell_command.is_none()
        && cmd.case_command.is_none()
        && cmd.select_command.is_none()
        && cmd.function_command.is_none()
        && cmd.brace_group.is_none()
        && cmd.coproc_command.is_none()
}

pub(super) fn command_is_open_conditional(cmd: &CommandNode) -> bool {
    (cmd.words.first().map(String::as_str) == Some("[[")
        || (matches!(cmd.words.first().map(String::as_str), Some("if" | "elif"))
            && cmd.words.get(1).map(String::as_str) == Some("[[")))
        && !cmd.words.iter().any(|word| word == "]]")
}

pub(super) fn command_accepts_embedded_arithmetic_command(cmd: &CommandNode) -> bool {
    matches!(
        cmd.words.first().map(String::as_str),
        Some("if" | "elif" | "while" | "until" | "do" | "then" | "else")
    ) && cmd.words.len() == 1
}

pub(super) fn is_function_name(name: &str) -> bool {
    if name.is_empty() || name.contains('=') {
        return false;
    }

    !name
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

pub(super) fn is_function_keyword_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}
