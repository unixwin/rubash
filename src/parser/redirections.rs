use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn collect_trailing_redirections(
    tokens: &[Token],
    index: &mut usize,
    command: &mut CommandNode,
) {
    loop {
        let Some(token) = tokens.get(*index) else {
            break;
        };
        if token.kind == TokenKind::HereDocBody {
            fill_pending_heredoc_body(command, &token.value);
            *index += 1;
            continue;
        }

        if token.kind == TokenKind::RedirectIn {
            if let Some((mut process_substitution, next_i)) =
                process_substitution_redirect_target(tokens, *index)
            {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                process_substitution.redirect_fd = fd;
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                let redirect = redirect_node(&token.value, fd, &target, false, false);
                command.redirects.push(redirect.clone());
                command.redirect_in = Some(redirect);
                *index = next_i + 1;
                continue;
            }
        }

        if token.kind == TokenKind::RedirectOut {
            if let Some((process_substitution, next_i)) =
                combined_process_substitution_redirect_target(tokens, *index)
            {
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                assign_output_redirect(command, &token.value, &target, None);
                *index = next_i + 1;
                continue;
            }

            if let Some((mut process_substitution, next_i)) =
                output_process_substitution_redirect_target(tokens, *index)
            {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                process_substitution.redirect_fd = fd;
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                command.redirect_out = Some(redirect_node(&token.value, fd, &target, false, false));
                *index = next_i + 1;
                continue;
            }
        }

        if token.kind == TokenKind::Append {
            if let Some((process_substitution, next_i)) =
                combined_process_substitution_redirect_target(tokens, *index)
            {
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                assign_append_redirect(command, &token.value, &target, None);
                *index = next_i + 1;
                continue;
            }
        }

        if matches!(
            token.kind,
            TokenKind::RedirectErr | TokenKind::RedirectErrAppend
        ) {
            if let Some((mut process_substitution, next_i)) =
                stderr_process_substitution_redirect_target(tokens, *index)
            {
                process_substitution.redirect_fd = Some(2);
                let target = process_substitution.target.clone();
                command.process_substitutions.push(process_substitution);
                if token.kind == TokenKind::RedirectErrAppend {
                    let redirect = redirect_node(&token.value, Some(2), &target, true, false);
                    command.redirects.push(redirect.clone());
                    command.redirect_err_append = Some(redirect);
                } else {
                    let redirect =
                        redirect_node(&token.value, Some(2), &target, false, token.value == "2>|");
                    command.redirects.push(redirect.clone());
                    command.redirect_err = Some(redirect);
                }
                *index = next_i + 1;
                continue;
            }
        }

        if token.kind == TokenKind::HereString {
            if let Some((process_substitution, next_i)) =
                any_process_substitution_word_target(tokens, *index + 1)
            {
                assign_here_string_process_substitution(
                    command,
                    &token.value,
                    process_substitution,
                );
                *index = next_i + 1;
                continue;
            }
        }

        let Some(target) = redirect_target_token(tokens, *index) else {
            break;
        };

        match token.kind {
            TokenKind::RedirectIn => {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                let redirect = redirect_node(
                    &token.value,
                    fd,
                    &input_redirect_target(&token.value, &target.value),
                    false,
                    false,
                );
                command.redirects.push(redirect.clone());
                command.redirect_in = Some(redirect);
            }
            TokenKind::RedirectOut => {
                if token.value.ends_with("<>") {
                    let fd = redirect_operator_fd(&token.value)
                        .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                    let redirect = redirect_node(&token.value, fd, &target.value, true, false);
                    command.redirects.push(redirect.clone());
                    command.redirect_in = Some(redirect);
                } else {
                    assign_output_redirect(command, &token.value, &target.value, None);
                }
            }
            TokenKind::Append => {
                assign_append_redirect(command, &token.value, &target.value, None);
            }
            TokenKind::RedirectErr => {
                assign_redirect_err_target(tokens, *index, command);
            }
            TokenKind::RedirectErrAppend => {
                assign_redirect_err_append_target(tokens, *index, command);
            }
            TokenKind::HereString => {
                assign_here_string_redirect(command, &token.value, &target.value);
            }
            TokenKind::HereDoc => {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                command
                    .heredoc_redirects
                    .push(heredoc_redirect(&token.value, target, fd));
                if fd.is_none() {
                    command.heredoc_delimiter = Some(target.value.clone());
                }
                *index += 2;
                continue;
            }
            _ => break,
        }

        *index += 2;
    }
}

pub(super) fn take_heredoc_fd_prefix(cmd: &mut CommandNode) -> Option<u32> {
    take_redirect_fd_prefix(cmd)
}

pub(super) fn assign_here_string_redirect(command: &mut CommandNode, operator: &str, target: &str) {
    let fd = redirect_operator_fd(operator);
    command
        .redirects
        .push(redirect_node(operator, fd, target, false, false));
    if let Some(fd) = fd {
        command.heredoc_redirects.push(HereDocRedirect {
            fd: Some(fd),
            operator: operator.to_string(),
            delimiter: "<<<".to_string(),
            strip_tabs: false,
            quoted_delimiter: false,
            here_string: true,
            body: Some(format!("\x1d{target}")),
        });
    } else {
        command.here_string = Some(target.to_string());
    }
}

pub(super) fn assign_here_string_process_substitution(
    command: &mut CommandNode,
    operator: &str,
    mut process_substitution: ProcessSubstitution,
) {
    let fd = redirect_operator_fd(operator);
    process_substitution.redirect_fd = fd;
    let target = process_substitution.target.clone();
    command.process_substitutions.push(process_substitution);
    assign_here_string_redirect(command, operator, &target);
}

pub(super) fn redirect_target_token(tokens: &[Token], index: usize) -> Option<&Token> {
    tokens
        .get(index + 1)
        .filter(|target| is_redirect_target_token(target))
}

pub(super) fn is_redirect_target_token(token: &Token) -> bool {
    matches!(
        token.kind,
        TokenKind::Word
            | TokenKind::Variable
            | TokenKind::Assignment
            | TokenKind::CommandSubst
            | TokenKind::BraceExpand
            | TokenKind::HereDocBody
    )
}

pub(super) fn take_adjacent_redirect_fd_prefix(
    cmd: &mut CommandNode,
    tokens: &[Token],
    redirect_index: usize,
) -> Option<u32> {
    let previous = redirect_index
        .checked_sub(1)
        .and_then(|index| tokens.get(index))?;
    let redirect = tokens.get(redirect_index)?;
    if previous.position + previous.value.len() != redirect.position {
        return None;
    }
    take_redirect_fd_prefix(cmd)
}

pub(super) fn redirect_operator_fd(operator: &str) -> Option<u32> {
    let digits = operator
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

pub(super) fn redirect_node(
    operator: &str,
    fd: Option<u32>,
    target: &str,
    append: bool,
    clobber: bool,
) -> Redirect {
    Redirect {
        fd,
        operator: operator.to_string(),
        kind: redirect_kind(operator, target),
        target: target.to_string(),
        append,
        clobber,
    }
}

pub(super) fn redirect_kind(operator: &str, target: &str) -> RedirectKind {
    if operator.ends_with("<&") {
        return if target == "&-" {
            RedirectKind::CloseInput
        } else {
            RedirectKind::DuplicateInput
        };
    }
    if operator.ends_with(">&") {
        return if target == "&-" {
            RedirectKind::CloseOutput
        } else {
            RedirectKind::DuplicateOutput
        };
    }
    if operator.ends_with("<>") {
        return RedirectKind::ReadWrite;
    }
    if operator == "&>" {
        return RedirectKind::CombinedOutput;
    }
    if operator == "&>>" {
        return RedirectKind::CombinedAppend;
    }
    if operator.ends_with("<<<") {
        return RedirectKind::HereString;
    }
    if operator.ends_with(">>") {
        return RedirectKind::Append;
    }
    if operator.ends_with(">|") {
        return RedirectKind::ClobberOutput;
    }
    if operator.ends_with('>') {
        return RedirectKind::Output;
    }
    if operator.ends_with('<') {
        return RedirectKind::Input;
    }
    RedirectKind::Unknown
}

pub(super) fn redirect_target(operator: &str, target: &str) -> String {
    if operator.ends_with(">&") {
        format!("&{target}")
    } else {
        target.to_string()
    }
}

pub(super) fn input_redirect_target(operator: &str, target: &str) -> String {
    if operator.ends_with("<&") {
        format!("&{target}")
    } else {
        target.to_string()
    }
}

pub(super) fn take_redirect_fd_prefix(cmd: &mut CommandNode) -> Option<u32> {
    let fd = cmd
        .words
        .last()
        .filter(|word| !word.is_empty() && word.chars().all(|ch| ch.is_ascii_digit()))?
        .parse::<u32>()
        .ok()?;
    cmd.words.pop();
    cmd.word_kinds.pop();
    Some(fd)
}

pub(super) fn assign_heredoc_body(current_cmd: &mut CommandNode, ast: &mut Ast, body: String) {
    if fill_pending_heredoc_body(current_cmd, &body) {
        return;
    }
    for command in ast.commands.iter_mut().rev() {
        if fill_pending_heredoc_body(command, &body) {
            return;
        }
    }
    current_cmd.heredoc = Some(body);
}

pub(super) fn fill_pending_heredoc_body(cmd: &mut CommandNode, body: &str) -> bool {
    let Some(redirect) = cmd
        .heredoc_redirects
        .iter_mut()
        .find(|redirect| redirect.body.is_none())
    else {
        return false;
    };

    redirect.body = Some(body.to_string());
    if redirect.fd.is_none() {
        cmd.heredoc = Some(body.to_string());
        cmd.heredoc_delimiter = Some(redirect.delimiter.clone());
    }
    true
}

pub(super) fn heredoc_redirect(
    operator: &str,
    delimiter: &Token,
    fd: Option<u32>,
) -> HereDocRedirect {
    HereDocRedirect {
        fd,
        operator: operator.to_string(),
        delimiter: delimiter.value.clone(),
        strip_tabs: operator.ends_with("<<-"),
        quoted_delimiter: delimiter
            .raw
            .chars()
            .any(|ch| matches!(ch, '\'' | '"' | '\\')),
        here_string: false,
        body: None,
    }
}
