use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn note_command_line(cmd: &mut CommandNode, token: &Token) {
    if cmd.line.is_none() {
        cmd.line = Some(token.position);
    }
}

pub(super) fn push_command_word(cmd: &mut CommandNode, token: &Token) {
    cmd.words.push(token.value.clone());
    cmd.word_kinds.push(token.kind.clone());
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

pub(super) fn command_is_empty(cmd: &CommandNode) -> bool {
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.heredoc_redirects.is_empty()
        && cmd.here_string.is_none()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.for_command.is_none()
        && cmd.if_command.is_none()
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
