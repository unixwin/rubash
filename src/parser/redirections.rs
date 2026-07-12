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
            if let Some((target, next_i)) = process_substitution_redirect_target(tokens, *index) {
                command.redirect_in = Some(Redirect {
                    fd: None,
                    target,
                    append: false,
                    clobber: false,
                });
                *index = next_i + 1;
                continue;
            }
        }

        if token.kind == TokenKind::RedirectOut {
            if let Some((target, next_i)) =
                output_process_substitution_redirect_target(tokens, *index)
            {
                command.redirect_out = Some(Redirect {
                    fd: None,
                    target,
                    append: false,
                    clobber: false,
                });
                *index = next_i + 1;
                continue;
            }
        }

        let Some(target) = tokens.get(*index + 1).filter(|next| {
            matches!(
                next.kind,
                TokenKind::Word
                    | TokenKind::Variable
                    | TokenKind::CommandSubst
                    | TokenKind::HereDocBody
            )
        }) else {
            break;
        };

        match token.kind {
            TokenKind::RedirectIn => {
                let fd = redirect_operator_fd(&token.value)
                    .or_else(|| take_adjacent_redirect_fd_prefix(command, tokens, *index));
                command.redirect_in = Some(Redirect {
                    fd,
                    target: input_redirect_target(&token.value, &target.value),
                    append: false,
                    clobber: false,
                });
            }
            TokenKind::RedirectOut => {
                if token.value == "<>" {
                    command.redirect_in = Some(Redirect {
                        fd: None,
                        target: target.value.clone(),
                        append: true,
                        clobber: false,
                    });
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
                command.heredoc_redirects.push(HereDocRedirect {
                    fd,
                    delimiter: target.value.clone(),
                    body: None,
                });
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
    if let Some(fd) = fd {
        command.heredoc_redirects.push(HereDocRedirect {
            fd: Some(fd),
            delimiter: "<<<".to_string(),
            body: Some(format!("\x1d{target}")),
        });
    } else {
        command.here_string = Some(target.to_string());
    }
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
